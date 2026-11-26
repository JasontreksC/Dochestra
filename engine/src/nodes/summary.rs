//! `summary` node: summarizes Markdown (plus optional images) with Claude.

use std::path::PathBuf;

use super::{MAX_IMAGE_BYTES, read_image, read_text, write_md};
use crate::clients::llm::{
    LlmClient, LlmError, LlmRequest, MAX_REQUEST_BYTES, UserBlock, base64_len,
    estimate_request_bytes,
};
use crate::executor::runner::{NodeContext, NodeError, NodeInputs, NodeOutputs};

/// At most this many images are sent along with the document.
pub const MAX_IMAGES: usize = 20;

const SYSTEM_PROMPT: &str = "\
너는 문서 요약 전문가다. 주어진 문서(와 첨부 이미지)를 바탕으로 정확하고 읽기 쉬운 한국어 \
Markdown 요약만 출력한다. 문서에 없는 내용을 지어내지 않고, 서문이나 사족은 붙이지 않는다.";

fn instruction(image_count: usize) -> String {
    let mut text = String::from(
        "아래 <document> 안의 문서를 한국어 Markdown 으로 요약해 주세요.\n\
         - 먼저 '## 핵심 요점' 아래에 가장 중요한 내용을 글머리 기호 목록으로 정리합니다.\n\
         - 이어서 문서의 섹션 구조에 따라 섹션별 요약을 작성합니다.\n\
         - 수치·고유명사·결론은 정확히 보존합니다.",
    );
    if image_count > 0 {
        text.push_str(&format!(
            "\n- 첨부된 이미지 {image_count}개는 문서에서 추출한 그림입니다. 요약에 필요한 경우 그 내용도 반영합니다."
        ));
    }
    text
}

/// The note appended to the summary when images were left out.
pub fn exclusion_note(excluded: usize) -> String {
    format!("※ 이미지 {excluded}개는 크기/개수 제한으로 제외됨")
}

pub async fn run(
    llm: &LlmClient,
    ctx: &NodeContext,
    inputs: &NodeInputs,
) -> Result<NodeOutputs, NodeError> {
    let md_path = inputs
        .get("md")
        .and_then(|a| a.paths.first())
        .ok_or_else(|| NodeError::Failed("입력 md 이(가) 없습니다".to_owned()))?;
    let markdown = read_text(md_path, "md").await?;

    // The document and system prompt are accounted for first; images only get what is left.
    // The instruction is measured as if all MAX_IMAGES were attached (its upper bound).
    let document = |image_count: usize| {
        format!(
            "{}\n\n<document>\n{markdown}\n</document>",
            instruction(image_count)
        )
    };
    let base = estimate_request_bytes(SYSTEM_PROMPT, &[UserBlock::Text(document(MAX_IMAGES))]);
    if base > MAX_REQUEST_BYTES {
        return Err(NodeError::Failed(LlmError::RequestTooLarge.to_string()));
    }
    let mut remaining = MAX_REQUEST_BYTES - base;

    // `images` is optional (D-4): absent or empty both mean "no images". Images are taken in
    // order; one that is too large on its own is skipped, while the first one that no longer
    // fits the request budget (or exceeds the count limit) ends the selection.
    let image_paths: &[PathBuf] = inputs.get("images").map_or(&[], |a| a.paths.as_slice());
    let mut images: Vec<Vec<u8>> = Vec::new();
    let mut excluded = 0usize;
    let mut budget_exhausted = false;
    for path in image_paths {
        if budget_exhausted || images.len() >= MAX_IMAGES {
            excluded += 1;
            continue;
        }
        let size = tokio::fs::metadata(path)
            .await
            .map_err(|e| {
                NodeError::Failed(format!(
                    "이미지를 읽을 수 없습니다 ({}): {e}",
                    path.display()
                ))
            })?
            .len();
        if size > MAX_IMAGE_BYTES {
            excluded += 1;
            continue;
        }
        let cost = base64_len(usize::try_from(size).unwrap_or(usize::MAX));
        if cost > remaining {
            budget_exhausted = true;
            excluded += 1;
            continue;
        }
        remaining -= cost;
        images.push(read_image(path).await?);
    }

    let mut user_content: Vec<UserBlock> =
        images.iter().cloned().map(UserBlock::ImageJpeg).collect();
    user_content.push(UserBlock::Text(document(images.len())));

    let mut summary = llm
        .complete(LlmRequest {
            system: SYSTEM_PROMPT.to_owned(),
            user_content,
        })
        .await
        .map_err(|e| NodeError::Failed(e.to_string()))?;
    if excluded > 0 {
        summary = format!("{}\n\n{}", summary.trim_end(), exclusion_note(excluded));
    }

    write_md(ctx, &summary).await
}
