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
- Keep numbers as Arabic numerals; do not spell them out in words.\n- Follow the glossary exactly when it is given.";

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
/// Chunks are joined back with a blank line. (Also used by the `rag` node.)
pub fn split_chunks(md: &str, max_chars: usize) -> Vec<String> {
    pack_blocks(split_blocks(md), max_chars)
}

/// Chunking for translation: like [`split_chunks`], but first (1) re-joins paragraphs that a
/// page break cut in the middle of a sentence ([`join_continuations`]) and (2) glues headings to
/// the block that follows them ([`attach_headings`]), so a chunk never ends with an orphaned
/// heading and no sentence is translated in two halves.
pub fn split_translation_chunks(md: &str, max_chars: usize) -> Vec<String> {
    pack_blocks(
        attach_headings(join_continuations(split_blocks(md))),
        max_chars,
    )
}

fn pack_blocks(blocks: Vec<String>, max_chars: usize) -> Vec<String> {
    let mut chunks: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut current_len = 0usize;

    for block in blocks {
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

fn is_heading(block: &str) -> bool {
    !block.contains('\n') && block.trim_start().starts_with('#')
}

/// A plain prose paragraph (as opposed to a heading, table, list, quote, code or HTML block).
fn is_prose(block: &str) -> bool {
    block.chars().next().is_some_and(char::is_alphabetic)
}

/// Re-joins a paragraph that a page break split in the middle of a sentence: the previous block
/// is prose without terminal punctuation and the next block starts with a lowercase letter.
/// (PDF extraction emits such fragments as separate blocks; translating them separately — or
/// in different chunks — produces broken sentences.)
fn join_continuations(blocks: Vec<String>) -> Vec<String> {
    const TERMINAL: &[char] = &[
        '.', '!', '?', ':', ';', '。', '！', '？', '…', '"', '”', '」', ')', '*', '|',
    ];
    let mut out: Vec<String> = Vec::with_capacity(blocks.len());
    for block in blocks {
        let continues = block.chars().next().is_some_and(char::is_lowercase)
            && out
                .last()
                .is_some_and(|prev| is_prose(prev) && !prev.trim_end().ends_with(TERMINAL));
        if continues {
            if let Some(prev) = out.last_mut() {
                prev.push(' ');
                prev.push_str(&block);
            }
        } else {
            out.push(block);
        }
    }
    out
}

/// Glues each run of headings to the next non-heading block (one atomic block), so a chunk
/// boundary never separates a heading from its body. A trailing heading stays on its own.
fn attach_headings(blocks: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(blocks.len());
    let mut pending: Vec<String> = Vec::new();
    for block in blocks {
        if is_heading(&block) {
            pending.push(block);
        } else {
            if pending.is_empty() {
                out.push(block);
            } else {
                pending.push(block);
                out.push(pending.join("\n\n"));
                pending.clear();
            }
        }
    }
    if !pending.is_empty() {
        out.push(pending.join("\n\n"));
    }
    out
}

/// Line ranges `[start, end]` (inclusive) of fenced code blocks (``` / ~~~) in `text`.
fn fenced_ranges(lines: &[&str]) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    let mut open: Option<(char, usize)> = None;
    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        match open {
            Some((marker, start)) => {
                if trimmed.starts_with(&marker.to_string().repeat(3)) {
                    ranges.push((start, i));
                    open = None;
                }
            }
            None => {
                if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
                    open = trimmed.chars().next().map(|c| (c, i));
                }
            }
        }
    }
    ranges
}

/// Guards against the model damaging what it was told to keep:
/// - an answer wrapped as one fenced block (``` / ```markdown) around the whole translation is
///   unwrapped when the source chunk was not itself a single fenced block;
/// - when the answer has as many fenced code blocks as the source chunk, each one is replaced
///   by the source's verbatim block (code must never be translated or reformatted).
pub fn sanitize_answer(source_chunk: &str, answer: &str) -> String {
    let mut text = answer.trim_end().to_owned();

    let src_lines: Vec<&str> = source_chunk.lines().collect();
    let src_ranges = fenced_ranges(&src_lines);

    // Whole-answer wrapper: ```, ```markdown or ```md on the first line and a bare ``` on the
    // last, while the source chunk itself does not start with a fence. (Inner fences of the
    // same length make the wrapper ambiguous for a real fence parser, so check the ends only.)
    let source_starts_with_fence = src_lines
        .iter()
        .find(|l| !l.trim().is_empty())
        .is_some_and(|l| l.trim_start().starts_with("```") || l.trim_start().starts_with("~~~"));
    if !source_starts_with_fence {
        let lines: Vec<&str> = text.lines().collect();
        if let (Some(first), Some(last)) = (lines.first(), lines.last()) {
            let first = first.trim();
            let wrapper_open = matches!(first, "```" | "```markdown" | "```md" | "~~~");
            if lines.len() >= 3
                && wrapper_open
                && last.trim() == if first == "~~~" { "~~~" } else { "```" }
            {
                text = lines[1..lines.len() - 1].join("\n");
            }
        }
    }

    let lines: Vec<&str> = text.lines().collect();
    let ranges = fenced_ranges(&lines);
    if !src_ranges.is_empty() && ranges.len() == src_ranges.len() {
        let mut rebuilt: Vec<&str> = Vec::with_capacity(lines.len());
        let mut cursor = 0usize;
        for (i, &(start, end)) in ranges.iter().enumerate() {
            rebuilt.extend_from_slice(&lines[cursor..start]);
            let (s_start, s_end) = src_ranges[i];
            rebuilt.extend_from_slice(&src_lines[s_start..=s_end]);
            cursor = end + 1;
        }
        rebuilt.extend_from_slice(&lines[cursor..]);
        text = rebuilt.join("\n");
    }
    text
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
    for chunk in split_translation_chunks(&markdown, MAX_CHUNK_CHARS) {
        let answer = llm
            .complete(LlmRequest {
                system: system.clone(),
                user_content: vec![UserBlock::Text(chunk.clone())],
            })
            .await
            .map_err(|e| NodeError::Failed(e.to_string()))?;
        translated.push(sanitize_answer(&chunk, &answer));
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
        // Uppercase: lowercase-initial blocks would be re-joined as page-break fragments.
        let a = "A".repeat(40);
        let b = "B".repeat(40);
        let c = "C".repeat(40);
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
        let big = "X".repeat(300);
        let md = format!("Intro\n\n{big}\n\nOutro");
        assert_eq!(
            split_chunks(&md, 100),
            ["Intro".to_owned(), big, "Outro".to_owned()]
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
        assert!(prompt.contains("Arabic numerals"));
        let minimal = build_system_prompt(&parse_settings(r#"{"targetLanguage":"ko"}"#).unwrap());
        assert!(!minimal.contains("Glossary") && !minimal.contains("Source language"));
    }

    #[test]
    fn plain_chunking_is_unchanged_for_rag() {
        // split_chunks (shared with rag) keeps headings and fragments as separate blocks.
        assert_eq!(split_chunks("a\n\n## T\n\nb", 4), ["a", "## T", "b"]);
        assert_eq!(
            split_chunks("Text with no end\n\nlower start", 10),
            ["Text with no end", "lower start"]
        );
    }

    #[test]
    fn headings_stay_with_the_following_block() {
        let a = "a".repeat(60);
        let b = "b".repeat(60);
        let md = format!("{a}\n\n## Title\n\n{b}");
        // Without gluing, `## Title` would end chunk 1 (60 + 2 + 8 <= 75) and `b` start chunk 2.
        let chunks = split_translation_chunks(&md, 75);
        assert_eq!(chunks, [a.clone(), format!("## Title\n\n{b}")]);
        // Consecutive headings travel together with the next body block.
        let md = format!("{a}\n\n## One\n\n### Two\n\n{b}");
        let chunks = split_translation_chunks(&md, 75);
        assert_eq!(chunks, [a, format!("## One\n\n### Two\n\n{b}")]);
        // A heading at the very end stays (alone) rather than vanishing.
        assert_eq!(
            split_translation_chunks("text\n\n## End", 5),
            ["text", "## End"]
        );
        // Joining chunks restores the document.
        let md = "# A\n\nx\n\n## B\n\ny";
        assert_eq!(split_translation_chunks(md, 4).join("\n\n"), md);
    }

    #[test]
    fn page_break_fragments_are_rejoined() {
        let md = "The vision system detects damaged packaging and routes the affected item to a\n\nmanual inspection station. Next sentence ends here.\n\nAnother paragraph.";
        let chunks = split_translation_chunks(md, 8000);
        assert_eq!(
            chunks,
            [
                "The vision system detects damaged packaging and routes the affected item to a manual inspection station. Next sentence ends here.\n\nAnother paragraph."
            ]
        );
        // Terminal punctuation, a capital letter, or a non-prose previous block: no join.
        for md in [
            "Ends here.\n\nlower start",
            "No terminal punctuation\n\nCapital start",
            "# heading\n\nlower start",
            "| a | b |\n|---|---|\n| 1 | 2 |\n\nlower start",
            "<!-- image -->\n\nlower start",
            "- item\n\nlower start",
        ] {
            assert_eq!(split_translation_chunks(md, 8000), [md.to_owned()], "{md}");
            assert_eq!(join_continuations(split_blocks(md)).len(), 2, "{md}");
        }
    }

    #[test]
    fn answer_wrapped_in_a_code_fence_is_unwrapped() {
        let src = "# Title\n\nHello";
        assert_eq!(
            sanitize_answer(src, "```markdown\n# 제목\n\n안녕\n```\n"),
            "# 제목\n\n안녕"
        );
        // A source that is itself one fenced block is left alone.
        let code = "```\nfn main() {}\n```";
        assert_eq!(sanitize_answer(code, code), code);
        // A fence in the middle of an answer is not a wrapper.
        let answer = "텍스트\n\n```\ncode\n```\n\n끝";
        assert_eq!(
            sanitize_answer("Text\n\n```\ncode\n```\n\nEnd", answer),
            answer
        );
    }

    #[test]
    fn code_blocks_are_restored_verbatim() {
        let src =
            "Run it:\n\n```python\n# add numbers\nprint(1 + 2)\n```\n\nThen:\n\n~~~\nls -l\n~~~";
        let answer = "실행하세요:\n\n```python\n# 숫자를 더한다\nprint(1+2)\n```\n\n그다음:\n\n~~~\nls -l   \n~~~";
        assert_eq!(
            sanitize_answer(src, answer),
            "실행하세요:\n\n```python\n# add numbers\nprint(1 + 2)\n```\n\n그다음:\n\n~~~\nls -l\n~~~"
        );
        // Mismatched block counts: leave the answer untouched (do not guess).
        assert_eq!(sanitize_answer(src, "번역만"), "번역만");
        assert_eq!(sanitize_answer("plain", "그냥 텍스트\n"), "그냥 텍스트");
    }
}
