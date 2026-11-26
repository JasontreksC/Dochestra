//! `rag` node: chunks Markdown, embeds the chunks, stores them in pgvector and outputs the
//! JSON description of the query API (CONTEXT §4.3).

use serde_json::json;
use sqlx::PgPool;

use super::{read_text, translate::split_chunks};
use crate::clients::embedding::EmbeddingClient;
use crate::executor::runner::{Artifact, NodeContext, NodeError, NodeInputs, NodeOutputs};
use crate::storage::single_output_path;
use crate::types::{DataType, ScalarType};

/// Maximum characters per stored chunk (including the heading context).
pub const MAX_CHUNK_CHARS: usize = 1500;
/// The section heading prepended to chunks that do not start with one is cut to this length.
const HEADING_CONTEXT_MAX: usize = 100;
const BLOCK_SEPARATOR: &str = "\n\n";
/// Room for the body text of a chunk: the cap minus a heading context and its separator, so a
/// chunk with context still stays within [`MAX_CHUNK_CHARS`].
const BODY_MAX: usize = MAX_CHUNK_CHARS - HEADING_CONTEXT_MAX - BLOCK_SEPARATOR.len();

/// Splits Markdown into retrieval chunks of at most [`MAX_CHUNK_CHARS`] characters.
///
/// - Blocks (separated by blank lines; fenced code and tables count as one block) are packed
///   greedily.
/// - A heading is never left at the end of a chunk: it stays with the block that follows it.
/// - A block too long for one chunk is split at sentence (and table-row) boundaries into pieces
///   of similar size — never mid-sentence unless a single sentence exceeds the limit, and then at
///   a word boundary — so there are no tiny tail fragments.
/// - Every chunk that does not start with a heading is prefixed with the section heading it
///   belongs to, so a fragment such as a table or a continued paragraph still says what it is
///   about (this matters for retrieval and for the context given to the answering model).
pub fn chunk_markdown(md: &str) -> Vec<String> {
    // A limit of 1 makes `split_chunks` return every block on its own.
    let blocks = split_chunks(md, 1);
    let mut chunks: Vec<String> = Vec::new();
    let mut current: Option<(String, usize, Option<String>)> = None; // (body, chars, context)
    let mut last_heading: Option<String> = None;

    let flush = |current: &mut Option<(String, usize, Option<String>)>,
                 chunks: &mut Vec<String>| {
        if let Some((body, _, context)) = current.take() {
            chunks.push(with_context(&body, context.as_deref()));
        }
    };

    for unit in group_headings(blocks) {
        let context_at_start = if unit.starts_with_heading {
            None
        } else {
            last_heading.clone()
        };
        let len = unit.text.chars().count();
        if len > BODY_MAX {
            flush(&mut current, &mut chunks);
            let inner_context = unit.last_heading.clone().or_else(|| last_heading.clone());
            for (i, piece) in balanced_split(&unit.text, BODY_MAX).into_iter().enumerate() {
                let context = if i == 0 {
                    context_at_start.clone()
                } else {
                    inner_context.clone()
                };
                chunks.push(with_context(&piece, context.as_deref()));
            }
        } else {
            match &mut current {
                Some((body, chars, _)) if *chars + BLOCK_SEPARATOR.len() + len <= BODY_MAX => {
                    body.push_str(BLOCK_SEPARATOR);
                    body.push_str(&unit.text);
                    *chars += BLOCK_SEPARATOR.len() + len;
                }
                _ => {
                    flush(&mut current, &mut chunks);
                    current = Some((unit.text.clone(), len, context_at_start));
                }
            }
        }
        if unit.last_heading.is_some() {
            last_heading = unit.last_heading;
        }
    }
    flush(&mut current, &mut chunks);
    chunks
}

/// One or more consecutive heading blocks glued to the block that follows them.
struct Unit {
    text: String,
    starts_with_heading: bool,
    /// The last heading inside the unit, truncated for use as a chunk prefix.
    last_heading: Option<String>,
}

fn is_heading(block: &str) -> bool {
    let line = block.trim();
    let hashes = line.chars().take_while(|c| *c == '#').count();
    (1..=6).contains(&hashes) && !line.contains('\n') && line[hashes..].starts_with(' ')
}

fn group_headings(blocks: Vec<String>) -> Vec<Unit> {
    let mut units = Vec::new();
    let mut pending: Vec<String> = Vec::new();
    let finish = |pending: &mut Vec<String>, body: Option<String>, units: &mut Vec<Unit>| {
        let last_heading = pending.last().map(|h| {
            h.trim()
                .chars()
                .take(HEADING_CONTEXT_MAX)
                .collect::<String>()
        });
        let starts_with_heading = !pending.is_empty();
        let mut parts = std::mem::take(pending);
        parts.extend(body);
        if !parts.is_empty() {
            units.push(Unit {
                text: parts.join(BLOCK_SEPARATOR),
                starts_with_heading,
                last_heading,
            });
        }
    };
    for block in blocks {
        if is_heading(&block) {
            pending.push(block);
        } else {
            finish(&mut pending, Some(block), &mut units);
        }
    }
    finish(&mut pending, None, &mut units); // headings with nothing after them
    units
}

fn with_context(body: &str, context: Option<&str>) -> String {
    match context {
        Some(heading) => format!("{heading}{BLOCK_SEPARATOR}{body}"),
        None => body.to_owned(),
    }
}

/// Splits `text` (longer than `limit` characters) into pieces of similar size, each at most
/// `limit`, breaking only at sentence boundaries where possible.
fn balanced_split(text: &str, limit: usize) -> Vec<String> {
    let total = text.chars().count();
    let pieces_needed = total.div_ceil(limit).max(1);
    let target = total / pieces_needed;

    let segments: Vec<String> = sentence_segments(text)
        .into_iter()
        .flat_map(|segment| wrap_oversized(&segment, limit))
        .collect();

    let mut pieces: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut current_len = 0usize;
    for segment in segments {
        let len = segment.chars().count();
        // Join the segment when that keeps the piece within the limit and no further from the
        // target size than it already is; otherwise start the next piece.
        let joins = current_len == 0
            || (current_len + len <= limit
                && (current_len + len).abs_diff(target) <= current_len.abs_diff(target));
        if !joins {
            pieces.push(std::mem::take(&mut current));
            current_len = 0;
        }
        current.push_str(&segment);
        current_len += len;
    }
    pieces.push(current);
    pieces
        .into_iter()
        .map(|p| p.trim().to_owned())
        .filter(|p| !p.is_empty())
        .collect()
}

/// Sentences and lines, each with its trailing whitespace: a sentence ends after `.`/`!`/`?`
/// followed by whitespace (or the end), after `。`/`！`/`？`, and after a newline.
fn sentence_segments(text: &str) -> Vec<String> {
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        current.push(c);
        let ends = match c {
            '\n' | '。' | '！' | '？' => true,
            '.' | '!' | '?' => chars.peek().is_none_or(|next| next.is_whitespace()),
            _ => false,
        };
        if ends {
            // Keep the whitespace that follows with this sentence.
            while chars.peek().is_some_and(|next| next.is_whitespace()) {
                current.extend(chars.next());
            }
            segments.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        segments.push(current);
    }
    segments
}

/// A single sentence longer than `limit`: wrapped at word boundaries (or hard-cut for text
/// without spaces, such as Korean without word gaps or long identifiers).
fn wrap_oversized(segment: &str, limit: usize) -> Vec<String> {
    if segment.chars().count() <= limit {
        return vec![segment.to_owned()];
    }
    let chars: Vec<char> = segment.chars().collect();
    let mut pieces = Vec::new();
    let mut start = 0usize;
    while start < chars.len() {
        let hard_end = (start + limit).min(chars.len());
        let end = if hard_end == chars.len() {
            hard_end
        } else {
            // Prefer the last whitespace inside the window (but not too early).
            chars[start..hard_end]
                .iter()
                .rposition(|c| c.is_whitespace())
                .map(|i| start + i + 1)
                .filter(|end| *end > start + limit / 2)
                .unwrap_or(hard_end)
        };
        pieces.push(chars[start..end].iter().collect());
        start = end;
    }
    pieces
}

pub async fn run(
    db: &PgPool,
    embedder: &EmbeddingClient,
    ctx: &NodeContext,
    inputs: &NodeInputs,
) -> Result<NodeOutputs, NodeError> {
    let md_path = inputs
        .get("md")
        .and_then(|a| a.paths.first())
        .ok_or_else(|| NodeError::Failed("입력 md 이(가) 없습니다".to_owned()))?;
    let markdown = read_text(md_path, "md").await?;
    if markdown.trim().is_empty() {
        return Err(NodeError::Failed("문서 내용이 비어 있습니다".to_owned()));
    }

    let chunks = chunk_markdown(&markdown);
    let embeddings = embedder
        .embed(&chunks)
        .await
        .map_err(|e| NodeError::Failed(e.to_string()))?;

    let index_id = crate::db::rag::create_index_with_chunks(
        db,
        ctx.run_id,
        &ctx.node_id,
        embedder.model(),
        &chunks,
        &embeddings,
    )
    .await
    .map_err(anyhow::Error::from)?;

    let description = json!({
        "type": "rag_api",
        "indexId": index_id,
        "method": "POST",
        "endpoint": format!("/api/rag/{index_id}/query"),
        "request": {
            "question": "string",
            "topK": "number (선택, 기본 5, 1~20)",
        },
        "chunkCount": chunks.len(),
        "embeddingModel": embedder.model(),
    });
    let bytes = serde_json::to_vec_pretty(&description).map_err(anyhow::Error::from)?;
    let path = single_output_path(&ctx.out_dir, "api", ScalarType::Json);
    tokio::fs::write(&path, bytes).await.map_err(|e| {
        NodeError::Other(anyhow::Error::new(e).context("failed to write api output"))
    })?;

    Ok(NodeOutputs::from([(
        "api".to_owned(),
        Artifact {
            data_type: DataType::single(ScalarType::Json),
            paths: vec![path],
        },
    )]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chars(s: &str) -> usize {
        s.chars().count()
    }

    /// `n` sentences of about 60 characters each.
    fn sentences(n: usize) -> String {
        (0..n)
            .map(|i| format!("Sentence number {i:03} talks about robots and their fleets."))
            .collect::<Vec<_>>()
            .join(" ")
    }

    #[test]
    fn short_documents_are_a_single_chunk() {
        assert_eq!(
            chunk_markdown("# T\n\nhello\n\nworld"),
            ["# T\n\nhello\n\nworld"]
        );
        assert!(chunk_markdown("").is_empty());
        assert!(chunk_markdown("\n\n  \n").is_empty());
    }

    #[test]
    fn blocks_are_packed_up_to_the_limit() {
        let a = "a".repeat(1000);
        let small = "c".repeat(BODY_MAX - 1000 - 2); // exactly fills the chunk
        let b = "b".repeat(1000);
        let chunks = chunk_markdown(&format!("{a}\n\n{small}\n\n{b}"));
        assert_eq!(chunks, [format!("{a}\n\n{small}"), b]);
        assert!(chunks.iter().all(|c| chars(c) <= MAX_CHUNK_CHARS));
    }

    #[test]
    fn a_long_paragraph_is_split_at_sentences_into_even_pieces() {
        let paragraph = sentences(60); // ~3,700 characters
        assert!(chars(&paragraph) > 2 * BODY_MAX);
        let chunks = chunk_markdown(&paragraph);

        assert_eq!(chunks.len(), chars(&paragraph).div_ceil(BODY_MAX));
        for chunk in &chunks {
            assert!(chars(chunk) <= MAX_CHUNK_CHARS, "{}", chars(chunk));
            assert!(
                chunk.starts_with("Sentence number"),
                "must start at a sentence: {chunk:?}"
            );
            assert!(
                chunk.ends_with("fleets."),
                "must end at a sentence: {chunk:?}"
            );
        }
        // Balanced: no tiny tail fragment.
        let (min, max) = chunks
            .iter()
            .map(|c| chars(c))
            .fold((usize::MAX, 0), |(lo, hi), n| (lo.min(n), hi.max(n)));
        assert!(max - min < 200, "sizes {min}..{max}");
        // Nothing lost, nothing duplicated.
        assert_eq!(chunks.join(" "), paragraph);
    }

    #[test]
    fn a_long_paragraph_after_a_heading_keeps_the_heading_on_every_piece() {
        let md = format!("## 8.4 Upgrades and Support\n\n{}", sentences(60));
        let chunks = chunk_markdown(&md);
        assert!(chunks.len() >= 3);
        assert!(
            chunks
                .iter()
                .all(|c| c.starts_with("## 8.4 Upgrades and Support\n\nSentence number")),
            "{chunks:?}"
        );
        assert!(chunks.iter().all(|c| chars(c) <= MAX_CHUNK_CHARS));
        // The heading is not duplicated inside the first chunk.
        assert_eq!(chunks[0].matches("## 8.4").count(), 1);
    }

    #[test]
    fn headings_stay_with_the_block_that_follows_them() {
        // The first paragraph nearly fills a chunk: the heading must move on with its paragraph
        // instead of being stranded at the end of the first chunk.
        let first = "x".repeat(BODY_MAX - 20);
        let second = "y".repeat(500);
        let chunks = chunk_markdown(&format!("{first}\n\n## Next section\n\n{second}"));
        assert_eq!(chunks.len(), 2);
        assert!(
            !chunks[0].contains("Next section"),
            "{:?}",
            &chunks[0][chunks[0].len() - 40..]
        );
        assert_eq!(chunks[1], format!("## Next section\n\n{second}"));
        // Consecutive headings travel together.
        let chunks = chunk_markdown(&format!("{first}\n\n# Part\n\n## Chapter\n\n{second}"));
        assert_eq!(chunks[1], format!("# Part\n\n## Chapter\n\n{second}"));
    }

    #[test]
    fn chunks_that_do_not_start_with_a_heading_get_the_section_heading_as_context() {
        let a = "a".repeat(1000);
        let b = "b".repeat(1000);
        let c = "c".repeat(1000);
        let chunks = chunk_markdown(&format!("## Alpha\n\n{a}\n\n{b}\n\n## Beta\n\n{c}"));
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0], format!("## Alpha\n\n{a}"));
        assert_eq!(chunks[1], format!("## Alpha\n\n{b}")); // continues the section: context added
        assert_eq!(chunks[2], format!("## Beta\n\n{c}")); // starts with its own heading: no prefix
        // A very long heading is truncated in the context, and the cap still holds.
        let long_heading = format!("## {}", "H".repeat(400));
        let chunks = chunk_markdown(&format!("{long_heading}\n\n{a}\n\n{b}"));
        assert!(
            chunks.iter().all(|c| chars(c) <= MAX_CHUNK_CHARS),
            "{:?}",
            chunks.iter().map(|c| chars(c)).collect::<Vec<_>>()
        );
        assert!(
            chunks[1].starts_with(
                &long_heading
                    .chars()
                    .take(HEADING_CONTEXT_MAX)
                    .collect::<String>()
            )
        );
    }

    #[test]
    fn a_long_table_is_split_between_rows_never_inside_one() {
        let rows: Vec<String> = (0..80)
            .map(|i| format!("| zone_{i:03} | {i}.5 | m/s |"))
            .collect();
        let table = format!(
            "| Parameter | Default | Unit |\n|---|---|---|\n{}",
            rows.join("\n")
        );
        assert!(chars(&table) > BODY_MAX);
        let chunks = chunk_markdown(&format!("## Parameters\n\n{table}"));
        assert!(chunks.len() >= 2);
        for chunk in &chunks {
            assert!(chunk.starts_with("## Parameters"));
            for line in chunk.lines().filter(|l| l.starts_with('|')) {
                assert!(line.ends_with('|'), "row cut in the middle: {line:?}");
            }
        }
        let all_rows: usize = chunks
            .iter()
            .map(|c| c.lines().filter(|l| l.starts_with("| zone_")).count())
            .sum();
        assert_eq!(all_rows, 80);
    }

    #[test]
    fn code_blocks_that_fit_are_not_split() {
        let code = format!("```\n{}\n\n{}\n```", "l1".repeat(300), "l2".repeat(300));
        let table = "| a | b |\n|---|---|\n| 1 | 2 |";
        let chunks = chunk_markdown(&format!("{}\n\n{code}\n\n{table}", "p".repeat(900)));
        assert!(chunks.contains(&format!("{code}\n\n{table}")), "{chunks:?}");
    }

    #[test]
    fn a_single_oversized_sentence_is_wrapped_at_words_then_hard_cut() {
        // One 3,000-character "sentence" made of words: cut at spaces.
        let words = "alpha beta gamma delta ".repeat(130);
        let chunks = chunk_markdown(words.trim());
        assert!(chunks.len() >= 3);
        for chunk in &chunks {
            assert!(chars(chunk) <= MAX_CHUNK_CHARS);
            assert!(
                chunk.starts_with("alpha")
                    || chunk.starts_with("beta")
                    || chunk.starts_with("gamma")
                    || chunk.starts_with("delta"),
                "{:?}",
                &chunk[..20]
            );
            assert!(
                !chunk.ends_with("alph") && !chunk.ends_with("gam"),
                "cut inside a word"
            );
        }
        // No spaces at all (long identifier, Korean without word gaps): hard cut at the limit.
        let blob = "가".repeat(3000);
        let chunks = chunk_markdown(&blob);
        assert!(chunks.iter().all(|c| chars(c) <= MAX_CHUNK_CHARS));
        assert_eq!(chunks.concat(), blob);
    }

    #[test]
    fn korean_text_is_split_at_sentence_ends() {
        let paragraph: String = (0..120)
            .map(|i| format!("{i}번째 문장은 로봇 함대의 운영 방법을 설명합니다. "))
            .collect();
        let chunks = chunk_markdown(paragraph.trim());
        assert!(chunks.len() >= 2);
        for chunk in &chunks {
            assert!(chars(chunk) <= MAX_CHUNK_CHARS);
            assert!(chunk.ends_with("설명합니다."), "{chunk:?}");
        }
    }

    #[test]
    fn counts_characters_not_bytes() {
        let korean = "가".repeat(BODY_MAX);
        let chunks = chunk_markdown(&korean);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], korean);
        assert_eq!(chunk_markdown(&"가".repeat(BODY_MAX + 1)).len(), 2);
    }

    #[test]
    fn heading_only_documents_keep_their_headings() {
        assert_eq!(
            chunk_markdown("# Title\n\n## Subtitle"),
            ["# Title\n\n## Subtitle"]
        );
    }
}
