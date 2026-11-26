//! `translate` node: translates Markdown chunk by chunk with the LLM.

use std::collections::BTreeMap;

use serde::Deserialize;

use super::{read_text, write_md};
use crate::clients::llm::{LlmClient, LlmRequest, UserBlock};
use crate::executor::runner::{NodeContext, NodeError, NodeInputs, NodeOutputs};

/// Maximum characters per translation request (blocks are never split below this size).
pub const MAX_CHUNK_CHARS: usize = 8000;

const BASE_SYSTEM_PROMPT: &str = "\
You are a professional document translator. The user message is a fragment of a Markdown \
document. Translate it and output ONLY the translated Markdown.

Rules:
- Keep the Markdown structure exactly: headings, lists, tables (columns and separator rows), \
links, emphasis and line breaks.
- Do not modify fenced code blocks, inline code, URLs, or `<!-- image -->` placeholders.
- Do not add a preface, explanation, notes or a summary; do not wrap the output in a code block.
- Follow the glossary exactly when it is given.";

/// Translation settings read from the `settings` JSON input.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslateSettings {
    pub target_language: String,
    #[serde(default)]
    pub source_language: Option<String>,
    #[serde(default)]
    pub glossary: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub tone: Option<String>,
    #[serde(default)]
    pub instructions: Option<String>,
}

/// Parses and validates the settings JSON; the error is a human-readable reason.
pub fn parse_settings(text: &str) -> Result<TranslateSettings, String> {
    // Parse via `Value` first: serde would otherwise also accept a positional JSON array.
    let value: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    if !value.is_object() {
        return Err("JSON 객체여야 합니다".to_owned());
    }
    let settings: TranslateSettings = serde_json::from_value(value).map_err(|e| e.to_string())?;
    if settings.target_language.trim().is_empty() {
        return Err("targetLanguage 가 비어 있습니다".to_owned());
    }
    Ok(settings)
}

pub fn build_system_prompt(settings: &TranslateSettings) -> String {
    let mut prompt = String::from(BASE_SYSTEM_PROMPT);
    prompt.push_str(&format!(
        "\n\nTarget language: {}",
        settings.target_language.trim()
    ));
    if let Some(source) = settings
        .source_language
        .as_deref()
        .filter(|s| !s.trim().is_empty())
    {
        prompt.push_str(&format!("\nSource language: {}", source.trim()));
    }
    if let Some(tone) = settings.tone.as_deref().filter(|s| !s.trim().is_empty()) {
        prompt.push_str(&format!("\nTone/style: {}", tone.trim()));
    }
    if let Some(glossary) = settings.glossary.as_ref().filter(|g| !g.is_empty()) {
        prompt.push_str("\n\nGlossary (always translate these terms as given):");
        for (term, translation) in glossary {
            prompt.push_str(&format!("\n- {term} => {translation}"));
        }
    }
    if let Some(extra) = settings
        .instructions
        .as_deref()
        .filter(|s| !s.trim().is_empty())
    {
        prompt.push_str(&format!("\n\nAdditional instructions:\n{}", extra.trim()));
    }
    prompt
}

/// Splits Markdown into chunks of at most `max_chars` characters.
///
/// Blocks are separated by blank lines; fenced code blocks (``` / ~~~) are one block even if
/// they contain blank lines, and tables stay whole because their rows are contiguous. Blocks
/// are accumulated greedily; a block longer than `max_chars` becomes a chunk of its own.
/// Chunks are joined back with a blank line.
pub fn split_chunks(md: &str, max_chars: usize) -> Vec<String> {
    let mut chunks: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut current_len = 0usize;

    for block in split_blocks(md) {
        let block_len = block.chars().count();
        if current.is_empty() {
            current = block;
            current_len = block_len;
        } else if current_len + 2 + block_len <= max_chars {
            current.push_str("\n\n");
            current.push_str(&block);
            current_len += 2 + block_len;
        } else {
            chunks.push(std::mem::take(&mut current));
            current = block;
            current_len = block_len;
        }
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

fn split_blocks(md: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut lines: Vec<&str> = Vec::new();
    let mut fence: Option<char> = None;

    let mut flush = |lines: &mut Vec<&str>| {
        if !lines.is_empty() {
            blocks.push(lines.join("\n"));
            lines.clear();
        }
    };

    for line in md.lines() {
        let trimmed = line.trim_start();
        if let Some(marker) = fence {
            lines.push(line);
            if trimmed.starts_with(&marker.to_string().repeat(3)) {
                fence = None;
            }
        } else if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fence = trimmed.chars().next();
            lines.push(line);
        } else if line.trim().is_empty() {
            flush(&mut lines);
        } else {
            lines.push(line);
        }
    }
    flush(&mut lines);
    blocks
}

pub async fn run(
    llm: &LlmClient,
    ctx: &NodeContext,
    inputs: &NodeInputs,
) -> Result<NodeOutputs, NodeError> {
    let md_path = first_path(inputs, "md")?;
    let settings_path = first_path(inputs, "settings")?;

    let settings_text = read_text(settings_path, "settings").await?;
    let settings = parse_settings(&settings_text).map_err(|reason| {
        NodeError::Failed(format!("번역 설정 JSON 이 올바르지 않습니다: {reason}"))
    })?;
    let markdown = read_text(md_path, "md").await?;

    let system = build_system_prompt(&settings);
    let mut translated = Vec::new();
    for chunk in split_chunks(&markdown, MAX_CHUNK_CHARS) {
        let answer = llm
            .complete(LlmRequest {
                system: system.clone(),
                user_content: vec![UserBlock::Text(chunk)],
            })
            .await
            .map_err(|e| NodeError::Failed(e.to_string()))?;
        translated.push(answer.trim_end().to_owned());
    }

    write_md(ctx, &translated.join("\n\n")).await
}

fn first_path<'a>(
    inputs: &'a NodeInputs,
    socket: &str,
) -> Result<&'a std::path::PathBuf, NodeError> {
    inputs
        .get(socket)
        .and_then(|a| a.paths.first())
        .ok_or_else(|| NodeError::Failed(format!("입력 {socket} 이(가) 없습니다")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_documents_are_one_chunk() {
        assert_eq!(
            split_chunks("# Title\n\nHello\n\nWorld\n", 8000),
            ["# Title\n\nHello\n\nWorld"]
        );
        assert!(split_chunks("", 100).is_empty());
        assert!(split_chunks("\n\n  \n", 100).is_empty());
    }

    #[test]
    fn blocks_accumulate_up_to_the_limit_in_order() {
        let a = "a".repeat(40);
        let b = "b".repeat(40);
        let c = "c".repeat(40);
        let md = format!("{a}\n\n{b}\n\n{c}");
        // 40 + 2 + 40 = 82 <= 85 fits, adding a third (82 + 2 + 40) does not.
        let chunks = split_chunks(&md, 85);
        assert_eq!(chunks, [format!("{a}\n\n{b}"), c.clone()]);
        // Exactly at the boundary.
        assert_eq!(split_chunks(&md, 82), [format!("{a}\n\n{b}"), c]);
        // Joining chunks restores the (normalized) document.
        assert_eq!(split_chunks(&md, 41).join("\n\n"), md);
    }

    #[test]
    fn oversized_block_becomes_its_own_chunk() {
        let big = "x".repeat(300);
        let md = format!("intro\n\n{big}\n\noutro");
        assert_eq!(
            split_chunks(&md, 100),
            ["intro".to_owned(), big, "outro".to_owned()]
        );
    }

    #[test]
    fn fenced_code_blocks_are_never_split() {
        let code = "```rust\nfn main() {\n\n    println!(\"hi\");\n}\n```";
        let md = format!("before\n\n{code}\n\nafter");
        // Even with a tiny limit the fence (with its internal blank line) stays whole.
        let chunks = split_chunks(&md, 10);
        assert_eq!(
            chunks,
            ["before".to_owned(), code.to_owned(), "after".to_owned()]
        );
        // Tilde fences and indented fences behave the same.
        let tilde = "~~~\na\n\nb\n~~~";
        assert!(split_chunks(&format!("x\n\n{tilde}"), 3).contains(&tilde.to_owned()));
        let indented = "  ```\n  a\n\n  b\n  ```";
        assert!(split_chunks(&format!("x\n\n{indented}"), 3).contains(&indented.to_owned()));
    }

    #[test]
    fn fence_markers_inside_a_different_fence_do_not_close_it() {
        let md = "~~~\n```\n\nstill code\n~~~\n\nnext";
        assert_eq!(
            split_chunks(md, 5),
            ["~~~\n```\n\nstill code\n~~~".to_owned(), "next".to_owned()]
        );
    }

    #[test]
    fn tables_stay_whole() {
        let table = "| a | b |\n|---|---|\n| 1 | 2 |\n| 3 | 4 |";
        let md = format!("intro\n\n{table}\n\nouttro");
        let chunks = split_chunks(&md, 20);
        assert!(chunks.contains(&table.to_owned()), "{chunks:?}");
    }

    #[test]
    fn unterminated_fence_swallows_the_rest_as_one_block() {
        let md = "a\n\n```\ncode\n\nmore";
        assert_eq!(
            split_chunks(md, 3),
            ["a".to_owned(), "```\ncode\n\nmore".to_owned()]
        );
    }

    #[test]
    fn counts_characters_not_bytes() {
        let korean = "가".repeat(40);
        let md = format!("{korean}\n\n{korean}");
        assert_eq!(split_chunks(&md, 82).len(), 1);
        assert_eq!(split_chunks(&md, 81).len(), 2);
    }

    #[test]
    fn settings_parse_and_validate() {
        let s = parse_settings(r#"{"targetLanguage":"ko","sourceLanguage":"en","glossary":{"run":"실행"},"tone":"formal","instructions":"keep names"}"#).unwrap();
        assert_eq!(s.target_language, "ko");
        assert_eq!(s.source_language.as_deref(), Some("en"));
        assert_eq!(s.glossary.unwrap()["run"], "실행");
        assert_eq!(
            parse_settings(r#"{"targetLanguage":"ko","extra":1}"#)
                .unwrap()
                .tone,
            None
        );

        for bad in [
            "not json",
            "{}",
            r#"{"targetLanguage":""}"#,
            r#"{"targetLanguage":"  "}"#,
            r#"{"targetLanguage":5}"#,
            r#"{"targetLanguage":"ko","glossary":{"a":1}}"#,
            r#"{"targetLanguage":"ko","glossary":["a"]}"#,
            r#"["ko"]"#,
        ] {
            assert!(parse_settings(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn system_prompt_includes_settings() {
        let settings = parse_settings(
            r#"{"targetLanguage":"Korean","sourceLanguage":"English","glossary":{"node":"노드"},"tone":"formal","instructions":"Keep product names."}"#,
        )
        .unwrap();
        let prompt = build_system_prompt(&settings);
        assert!(prompt.contains("Target language: Korean"));
        assert!(prompt.contains("Source language: English"));
        assert!(prompt.contains("Tone/style: formal"));
        assert!(prompt.contains("- node => 노드"));
        assert!(prompt.contains("Keep product names."));
        assert!(prompt.contains("<!-- image -->"));
        let minimal = build_system_prompt(&parse_settings(r#"{"targetLanguage":"ko"}"#).unwrap());
        assert!(!minimal.contains("Glossary") && !minimal.contains("Source language"));
    }
}
