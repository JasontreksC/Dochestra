//! `summary` node: summarizes Markdown (plus optional images) with the LLM.

use std::path::PathBuf;

use super::{SelectedImages, read_text, select_images, write_md};
use crate::clients::llm::{
    LlmClient, LlmError, LlmRequest, MAX_REQUEST_BYTES, UserBlock, estimate_request_bytes,
};
use crate::executor::runner::{NodeContext, NodeError, NodeInputs, NodeOutputs};

pub use super::MAX_IMAGES;

const SYSTEM_PROMPT: &str = "\
너는 문서 요약 전문가다. 주어진 문서(와 첨부 이미지)를 바탕으로 정확하고 읽기 쉬운 한국어 \
Markdown 요약만 출력한다. 문서에 없는 내용을 지어내지 않고, 서문이나 사족은 붙이지 않는다. \
요약문은 반드시 한국어로 쓰며, 원문이 영어여도 일반 어휘와 단위(hours, minutes, meters, \
percent 등)는 한국어로 옮긴다. 고유명사·제품명·코드·식별자만 원문 표기를 유지한다.";

fn instruction(image_count: usize) -> String {
    let mut text = String::from(
        "아래 <document> 안의 문서를 한국어 Markdown 으로 요약해 주세요.\n\
         - 먼저 '## 핵심 요점' 아래에 가장 중요한 내용을 글머리 기호 목록으로 정리합니다.\n\
         - 이어서 문서의 섹션(장) 구조에 따라 섹션별 요약을 작성하며, 각 섹션(장)마다 \
         소제목을 따로 둡니다. 여러 섹션에서 같은 내용이 반복되면 그 내용은 한 번만 \
         설명하고 나머지 섹션에서는 달라지는 부분만 씁니다.\n\
         - 섹션별 요약에는 핵심 요점에 이미 쓴 문장을 되풀이하지 말고, 핵심 요점에 없는 \
         세부 내용만 씁니다.\n\
         - 요약은 원문보다 분명히 짧아야 합니다(대체로 원문 분량의 20~40%, 1,000자 이하의 짧은 \
         문서는 핵심 요점 위주로 더 짧게). 표를 통째로 옮기지 말고 필요한 수치만 문장이나 \
         목록에 반영합니다.\n\
         - 수치·고유명사·결론은 정확히 보존하고, 문서에 없는 수치는 쓰지 않습니다. 영어 원문의 \
         일반 어휘와 단위는 한국어로 옮깁니다.",
    );
    if image_count > 0 {
        text.push_str(&format!(
            "\n- 첨부된 이미지 {image_count}개는 문서에서 추출한 그림이며, 문서의 `<!-- image -->` \
             위치에 나오는 순서대로 첨부되어 있습니다. 그림 안의 글자·수치가 중요하면 요약에 반영하고, \
             이미지에서 읽은 내용은 문서 본문의 내용과 혼동하지 않습니다."
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
    let remaining = MAX_REQUEST_BYTES - base;

    // `images` is optional (D-4): absent or empty both mean "no images".
    let image_paths: &[PathBuf] = inputs.get("images").map_or(&[], |a| a.paths.as_slice());
    let SelectedImages { images, excluded } = select_images(image_paths, remaining).await?;

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
