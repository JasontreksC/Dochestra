//! `voice` node: writes a spoken narration script for a document with Claude, then
//! synthesizes it to MP3 with OpenAI TTS.

use std::path::PathBuf;

use super::{MAX_IMAGES, SelectedImages, read_text, select_images};
use crate::clients::llm::{
    LlmClient, LlmError, LlmRequest, MAX_REQUEST_BYTES, UserBlock, estimate_request_bytes,
};
use crate::clients::tts::TtsClient;
use crate::executor::runner::{Artifact, NodeContext, NodeError, NodeInputs, NodeOutputs};
use crate::storage::{single_output_path, work_dir};
use crate::types::{DataType, ScalarType};

const SYSTEM_PROMPT: &str = "\
너는 문서를 소리 내어 설명하는 해설 내레이터다. 주어진 문서(와 첨부 이미지)를 바탕으로, 청취자가 \
귀로만 듣고도 이해할 수 있는 해설 대본을 쓴다.

규칙:
- 한국어 구어체(\"~입니다\", \"~해요\" 등 자연스러운 말투)로 쓴다.
- 마크다운 문법, 글머리 기호, 번호 목록, 표, 이모지, 괄호 속 보충 표기를 쓰지 않고 순수한 문장만 쓴다.
- 분량은 약 1500~2500자로 한다. 핵심 내용을 먼저 짚고 흐름에 따라 설명한다.
- 수치와 고유명사는 정확히 말하고, 문서에 없는 내용을 지어내지 않는다.
- 이미지가 첨부된 경우 그 내용도 자연스럽게 설명에 녹인다.
- 서문(\"다음은 대본입니다\" 등)이나 맺음 설명 없이 대본 본문만 출력한다.";

fn instruction(image_count: usize) -> String {
    let mut text = String::from(
        "아래 <document> 안의 문서를 해설하는 대본을 작성해 주세요. 시스템 지침의 규칙을 따르고 \
         대본 본문만 출력합니다.",
    );
    if image_count > 0 {
        text.push_str(&format!(
            "\n첨부된 이미지 {image_count}개는 문서에서 추출한 그림입니다. 설명에 필요한 경우 그 내용도 반영합니다."
        ));
    }
    text
}

pub async fn run(
    llm: &LlmClient,
    tts: &TtsClient,
    ctx: &NodeContext,
    inputs: &NodeInputs,
) -> Result<NodeOutputs, NodeError> {
    let md_path = inputs
        .get("md")
        .and_then(|a| a.paths.first())
        .ok_or_else(|| NodeError::Failed("입력 md 이(가) 없습니다".to_owned()))?;
    let markdown = read_text(md_path, "md").await?;

    // Same request budget rules as `summary`: text first, then as many images as fit.
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
    let image_paths: &[PathBuf] = inputs.get("images").map_or(&[], |a| a.paths.as_slice());
    let SelectedImages { images, excluded } =
        select_images(image_paths, MAX_REQUEST_BYTES - base).await?;
    if excluded > 0 {
        // Not added to the script: it would be read aloud.
        tracing::warn!(excluded, "images left out of the narration request");
    }

    let mut user_content: Vec<UserBlock> =
        images.iter().cloned().map(UserBlock::ImageJpeg).collect();
    user_content.push(UserBlock::Text(document(images.len())));
    let script = llm
        .complete(LlmRequest {
            system: SYSTEM_PROMPT.to_owned(),
            user_content,
        })
        .await
        .map_err(|e| NodeError::Failed(e.to_string()))?;
    let script = script.trim();
    if script.is_empty() {
        return Err(NodeError::Failed("해설 대본이 비어 있습니다".to_owned()));
    }

    let work = work_dir(&ctx.out_dir);
    tokio::fs::create_dir_all(&work).await.map_err(|e| {
        NodeError::Other(anyhow::Error::new(e).context("failed to create work dir"))
    })?;
    tokio::fs::write(work.join("script.txt"), script)
        .await
        .map_err(|e| NodeError::Other(anyhow::Error::new(e).context("failed to save script")))?;

    let audio = tts
        .synthesize(script)
        .await
        .map_err(|e| NodeError::Failed(e.to_string()))?;
    let path = single_output_path(&ctx.out_dir, "mp3", ScalarType::Mp3);
    tokio::fs::write(&path, audio).await.map_err(|e| {
        NodeError::Other(anyhow::Error::new(e).context("failed to write mp3 output"))
    })?;

    Ok(NodeOutputs::from([(
        "mp3".to_owned(),
        Artifact {
            data_type: DataType::single(ScalarType::Mp3),
            paths: vec![path],
        },
    )]))
}
