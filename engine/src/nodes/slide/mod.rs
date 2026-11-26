//! `slide` node: Claude drafts the deck structure (JSON), the engine renders a 16:9 PDF.

pub mod render;
pub mod spec;

use std::path::Path;

use super::{MAX_IMAGE_BYTES, read_text};
use crate::clients::llm::{
    LlmClient, LlmError, LlmRequest, MAX_REQUEST_BYTES, UserBlock, base64_len,
    estimate_request_bytes,
};
use crate::executor::runner::{Artifact, NodeContext, NodeError, NodeInputs, NodeOutputs};
use crate::storage::{single_output_path, work_dir};
use crate::types::{DataType, ScalarType};
use spec::{SlideDeck, parse_deck};

const SYSTEM_PROMPT: &str = "\
You turn a document into a presentation outline. Output ONLY one JSON object, no code fences \
and no commentary, in exactly this shape:

{\"title\": string, \"subtitle\": string (optional), \"slides\": [{\"title\": string, \"bullets\": [string, ...]}, ...]}

Rules:
- `title` is the presentation title; `subtitle` is a short tagline (omit it if there is none).
- 1 to 15 slides; each slide has a short `title` and 1 to 7 `bullets`.
- Every string is at most 120 characters. Bullets are concise phrases, not paragraphs.
- Follow the document's structure and keep numbers and names exact; do not invent facts.
- Write in the same language as the document.
- If an image is attached, it is the document's representative picture; it is placed on the \
cover and on a closing slide by the renderer, so do not describe layout.";

fn user_prompt(markdown: &str, has_image: bool, feedback: Option<&str>) -> String {
    let mut prompt =
        String::from("아래 <document> 의 문서를 발표 슬라이드 구성(JSON)으로 만들어 주세요.");
    if has_image {
        prompt.push_str("\n첨부된 이미지는 문서의 대표 이미지입니다.");
    }
    prompt.push_str(&format!("\n\n<document>\n{markdown}\n</document>"));
    if let Some(reason) = feedback {
        prompt.push_str(&format!(
            "\n\n[이전 응답은 해석에 실패했습니다: {reason}]\n지정한 JSON 형식의 객체 하나만 다시 출력해 주세요."
        ));
    }
    prompt
}

pub async fn run(
    llm: &LlmClient,
    font_dir: &Path,
    ctx: &NodeContext,
    inputs: &NodeInputs,
) -> Result<NodeOutputs, NodeError> {
    let md_path = inputs
        .get("md")
        .and_then(|a| a.paths.first())
        .ok_or_else(|| NodeError::Failed("입력 md 이(가) 없습니다".to_owned()))?;
    let markdown = read_text(md_path, "md").await?;

    // Text first, then the picture only if it still fits the request budget.
    let base = estimate_request_bytes(
        SYSTEM_PROMPT,
        &[UserBlock::Text(user_prompt(&markdown, true, None))],
    ) + 512; // room for the retry feedback
    if base > MAX_REQUEST_BYTES {
        return Err(NodeError::Failed(LlmError::RequestTooLarge.to_string()));
    }
    let image = load_image(inputs, MAX_REQUEST_BYTES - base).await?;

    let deck = request_deck(llm, &markdown, image.as_deref()).await?;

    let work = work_dir(&ctx.out_dir);
    tokio::fs::create_dir_all(&work).await.map_err(|e| {
        NodeError::Other(anyhow::Error::new(e).context("failed to create work dir"))
    })?;
    let json = serde_json::to_vec_pretty(&deck).map_err(anyhow::Error::from)?;
    tokio::fs::write(work.join("slides.json"), json)
        .await
        .map_err(|e| {
            NodeError::Other(anyhow::Error::new(e).context("failed to save slides.json"))
        })?;

    let font_dir = font_dir.to_path_buf();
    let image_for_render = image.clone();
    let bytes = tokio::task::spawn_blocking(move || {
        render::render_deck(&deck, image_for_render.as_deref(), &font_dir)
    })
    .await
    .map_err(|e| NodeError::Other(e.into()))?
    .map_err(|e| NodeError::Failed(format!("슬라이드 PDF 를 만들 수 없습니다: {e}")))?;

    let path = single_output_path(&ctx.out_dir, "pdf", ScalarType::Pdf);
    tokio::fs::write(&path, bytes).await.map_err(|e| {
        NodeError::Other(anyhow::Error::new(e).context("failed to write pdf output"))
    })?;
    Ok(NodeOutputs::from([(
        "pdf".to_owned(),
        Artifact {
            data_type: DataType::single(ScalarType::Pdf),
            paths: vec![path],
        },
    )]))
}

/// The representative image, or `None` (with a warning) when it is too large to send.
async fn load_image(inputs: &NodeInputs, budget: usize) -> Result<Option<Vec<u8>>, NodeError> {
    let Some(path) = inputs.get("image").and_then(|a| a.paths.first()) else {
        return Ok(None);
    };
    let size = tokio::fs::metadata(path)
        .await
        .map_err(|e| {
            NodeError::Failed(format!(
                "이미지를 읽을 수 없습니다 ({}): {e}",
                path.display()
            ))
        })?
        .len();
    let too_big =
        size > MAX_IMAGE_BYTES || base64_len(usize::try_from(size).unwrap_or(usize::MAX)) > budget;
    if too_big {
        tracing::warn!(
            size,
            "representative image too large; building the deck without it"
        );
        return Ok(None);
    }
    let bytes = tokio::fs::read(path).await.map_err(|e| {
        NodeError::Failed(format!(
            "이미지를 읽을 수 없습니다 ({}): {e}",
            path.display()
        ))
    })?;
    Ok(Some(bytes))
}

/// Asks for the deck; if the answer cannot be used, asks once more with the reason.
async fn request_deck(
    llm: &LlmClient,
    markdown: &str,
    image: Option<&[u8]>,
) -> Result<SlideDeck, NodeError> {
    let mut feedback: Option<String> = None;
    for attempt in 0..2 {
        let mut user_content: Vec<UserBlock> = image
            .map(|bytes| UserBlock::ImageJpeg(bytes.to_vec()))
            .into_iter()
            .collect();
        user_content.push(UserBlock::Text(user_prompt(
            markdown,
            image.is_some(),
            feedback.as_deref(),
        )));

        let answer = llm
            .complete(LlmRequest {
                system: SYSTEM_PROMPT.to_owned(),
                user_content,
            })
            .await
            .map_err(|e| NodeError::Failed(e.to_string()))?;
        match parse_deck(&answer) {
            Ok(deck) => return Ok(deck),
            Err(reason) if attempt == 0 => {
                tracing::warn!(%reason, "slide outline rejected; retrying once");
                feedback = Some(reason);
            }
            Err(reason) => {
                return Err(NodeError::Failed(format!(
                    "슬라이드 구성을 해석하지 못했습니다: {reason}"
                )));
            }
        }
    }
    unreachable!("the loop returns on the second attempt")
}
