//! `analyze` node: describes an image with the LLM.

use super::{read_image, write_md};
use crate::clients::llm::{LlmClient, LlmRequest, UserBlock};
use crate::executor::runner::{NodeContext, NodeError, NodeInputs, NodeOutputs};

const SYSTEM_PROMPT: &str = "\
너는 문서에서 추출된 이미지를 분석하는 도우미다. 이미지의 내용을 정확하고 구조적으로 설명하는 \
Markdown 만 출력한다. 서문이나 사족은 붙이지 않는다.";

const INSTRUCTION: &str = "\
이 이미지를 분석해 한국어 Markdown 으로 설명해 주세요.
- 무엇을 보여 주는 이미지인지 한두 문장으로 먼저 요약합니다.
- 이미지 안의 텍스트는 가능한 한 그대로 옮깁니다.
- 도표·그래프·표가 있다면 축, 범례, 수치와 추세를 구조화해서(표나 목록으로) 정리합니다.
- 보이지 않거나 불확실한 내용은 추측하지 말고 그렇게 표시합니다.";

pub async fn run(
    llm: &LlmClient,
    ctx: &NodeContext,
    inputs: &NodeInputs,
) -> Result<NodeOutputs, NodeError> {
    let path = inputs
        .get("image")
        .and_then(|a| a.paths.first())
        .ok_or_else(|| NodeError::Failed("입력 image 이(가) 없습니다".to_owned()))?;
    let bytes = read_image(path).await?;

    let answer = llm
        .complete(LlmRequest {
            system: SYSTEM_PROMPT.to_owned(),
            user_content: vec![
                UserBlock::ImageJpeg(bytes),
                UserBlock::Text(INSTRUCTION.to_owned()),
            ],
        })
        .await
        .map_err(|e| NodeError::Failed(e.to_string()))?;

    write_md(ctx, &answer).await
}
