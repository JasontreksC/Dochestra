//! `analyze` node: describes an image with the LLM.

use super::{read_image, write_md};
use crate::clients::llm::{LlmClient, LlmRequest, UserBlock};
use crate::executor::runner::{NodeContext, NodeError, NodeInputs, NodeOutputs};

const SYSTEM_PROMPT: &str = "\
너는 문서에서 추출된 이미지를 분석하는 도우미다. 이미지의 내용을 정확하고 구조적으로 설명하는 \
한국어 Markdown 만 출력한다. 서문이나 사족은 붙이지 않고, 이미지에 없는 내용을 지어내지 않는다.";

const INSTRUCTION: &str = "\
이 이미지를 분석해 한국어 Markdown 으로 설명해 주세요. 출력은 정확히 아래 구조를 따릅니다. \
`#`(H1) 제목은 쓰지 말고, 첫 줄은 반드시 `## 요약` 입니다.

## 요약
무엇을 보여 주는 이미지인지 한두 문장으로 씁니다.

## 이미지 속 텍스트
이미지 안의 모든 글자(제목, 범례, 축 이름, 눈금, 캡션, 작은 글씨 포함)를 읽은 그대로 옮깁니다. \
없으면 '없음'이라고 씁니다.

## 도표·수치
그래프·차트·표가 있을 때만 이 섹션을 씁니다. 축, 범례, 계열, 막대·점의 모든 수치와 추세를 표나 \
목록으로 정리합니다. 이미지에 적혀 있지 않은 값을 계산해서 쓸 때는 '(계산값)'이라고 표시합니다.

## 판독 불가·불확실
흐리거나 잘려서 읽을 수 없는 부분, 확신할 수 없는 값을 목록으로 씁니다. 없으면 이 섹션을 생략합니다.

규칙:
- 제목·범례·축 이름·캡션이 있는지 이미지 가장자리까지 꼼꼼히 확인한 뒤에만 '보이지 않음'이라고 \
씁니다. 보이는 글자를 빠뜨리지 않습니다.
- 읽을 수 없는 글자나 수치는 추측하지 말고 '판독 불가'로 표시합니다. 이미지에 없는 대상은 서술하지 않습니다.";

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
