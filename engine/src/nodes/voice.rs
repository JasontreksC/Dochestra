//! `voice` node: writes a spoken narration script for a document with the LLM, then
//! synthesizes it to MP3 with the TTS model.

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
- 영문 상호·이름·약어는 한국어로 읽는 대로 한글로 적고(예: Aurora Robotics → 오로라 로보틱스), 같은 대상은 \
처음부터 끝까지 한 가지 표기로만 쓴다.
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
    let script = clean_script(&script);
    if script.is_empty() {
        return Err(NodeError::Failed("해설 대본이 비어 있습니다".to_owned()));
    }

    let work = work_dir(&ctx.out_dir);
    tokio::fs::create_dir_all(&work).await.map_err(|e| {
        NodeError::Other(anyhow::Error::new(e).context("failed to create work dir"))
    })?;
    tokio::fs::write(work.join("script.txt"), &script)
        .await
        .map_err(|e| NodeError::Other(anyhow::Error::new(e).context("failed to save script")))?;

    let audio = tts
        .synthesize(&script)
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

/// Characters a narrator would read aloud as noise: emoji, pictographs and their joiners.
fn is_pictograph(c: char) -> bool {
    matches!(c as u32,
        0x1F000..=0x1FAFF | 0x2600..=0x27BF | 0x2B50 | 0x2B55 | 0x200D | 0xFE0F)
}

/// Leading markdown of one line: heading hashes, bullet marker or `1.` / `1)` list number.
fn strip_line_marker(line: &str) -> &str {
    let line = line.trim_start();
    let hashes = line.chars().take_while(|&c| c == '#').count();
    if (1..=6).contains(&hashes) && line[hashes..].starts_with(' ') {
        return line[hashes..].trim_start();
    }
    let mut chars = line.chars();
    if let (Some(first), Some(' ')) = (chars.next(), chars.next())
        && "-*•·▪●○■□▶►".contains(first)
    {
        return line[first.len_utf8()..].trim_start();
    }
    let digits = line.chars().take_while(char::is_ascii_digit).count();
    if (1..=2).contains(&digits)
        && let Some(rest) = line[digits..].strip_prefix(['.', ')'])
        && rest.starts_with(' ')
    {
        return rest.trim_start();
    }
    line
}

/// Safety net for the "plain sentences only" rule of the prompt: the script is read aloud as
/// is, so leftover markdown (`#`, `**`, bullets, rules), emoji and bare URLs would be spoken.
/// Paragraph breaks are kept (at most one blank line); everything else stays untouched.
pub(crate) fn clean_script(script: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    for raw in script.lines() {
        let line = strip_line_marker(raw);
        let is_rule = line.len() >= 3
            && ["-", "*", "="]
                .iter()
                .any(|m| line.chars().all(|c| m.contains(c)));
        let line: String = if is_rule {
            String::new()
        } else {
            line.replace("**", "")
                .replace("__", "")
                .replace("~~", "")
                .replace('`', "")
                .chars()
                .filter(|&c| !is_pictograph(c))
                .collect()
        };
        let line = line
            .split_whitespace()
            .filter(|word| !word.starts_with("http://") && !word.starts_with("https://"))
            .collect::<Vec<_>>()
            .join(" ");
        if line.is_empty() && lines.last().is_none_or(String::is_empty) {
            continue; // leading blank lines and repeated blank lines
        }
        lines.push(line);
    }
    lines.join("\n").trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::clean_script;

    #[test]
    fn plain_narration_is_left_untouched() {
        let text = "안녕하세요. 2025년 매출은 4억 8천만 달러입니다.\n\n두 번째 문단입니다. 1.5배 늘었어요.";
        assert_eq!(clean_script(text), text);
        assert_eq!(clean_script("  \n문장입니다.\n\n"), "문장입니다.");
        assert_eq!(clean_script(""), "");
    }

    #[test]
    fn markdown_markers_are_removed_but_the_words_stay() {
        let text = "## 개요\n**핵심**은 `성장`입니다.\n- 첫째 항목\n* 둘째 항목\n• 셋째 항목\n1. 하나\n2) 둘\n---\n끝.";
        assert_eq!(
            clean_script(text),
            "개요\n핵심은 성장입니다.\n첫째 항목\n둘째 항목\n셋째 항목\n하나\n둘\n\n끝."
        );
    }

    #[test]
    fn emoji_and_urls_are_dropped() {
        assert_eq!(
            clean_script(
                "좋은 소식입니다 🎉 자세한 내용은 https://example.com/a 에서 ✅ 확인하세요 👍🏽."
            ),
            "좋은 소식입니다 자세한 내용은 에서 확인하세요 ."
        );
    }

    #[test]
    fn numbers_and_ordinary_symbols_are_not_mistaken_for_markup() {
        // "2025. " starts with three digits (not a list number); %, +, -, / and a lone * stay.
        let text = "2025. 연도입니다.\n-3도에서 +5% 상승, 5/6 지점, 별표 * 하나.\n#1 제품입니다.";
        assert_eq!(clean_script(text), text);
    }

    #[test]
    fn blank_lines_collapse_to_one_paragraph_break() {
        assert_eq!(clean_script("가.\n\n\n\n나.\n \n다."), "가.\n\n나.\n\n다.");
    }
}
