//! `POST /api/rag/{indexId}/query`: retrieval + answer generation (CONTEXT §4.3).

use axum::Json;
use axum::extract::State;
use uuid::Uuid;

use super::extract::{ApiJson, ApiPath};
use crate::clients::llm::{LlmRequest, UserBlock};
use crate::db;
use crate::error::ApiError;
use crate::state::AppState;
use crate::types::{RagQueryRequest, RagQueryResponse, RagSource};

const QUESTION_MAX_CHARS: usize = 2000;
const DEFAULT_TOP_K: u32 = 5;
const MAX_TOP_K: u32 = 20;

const SYSTEM_PROMPT: &str = "\
You answer questions about a document using only the numbered context passages provided by \
the user.

Rules:
- Use only the information in the context passages; never add outside knowledge.
- Answer in the same language as the question.
- Cite the passages you used as [n] right after the statements they support.
- If the context does not contain the answer, say clearly that the document does not \
contain relevant information instead of guessing.";

/// Validates the request; returns the trimmed question and the effective `topK`.
fn validate(req: &RagQueryRequest) -> Result<(String, i64), ApiError> {
    let question = req.question.trim();
    let len = question.chars().count();
    if len == 0 || len > QUESTION_MAX_CHARS {
        return Err(ApiError::BadRequest(format!(
            "question 은 1~{QUESTION_MAX_CHARS}자여야 합니다"
        )));
    }
    let top_k = req.top_k.unwrap_or(DEFAULT_TOP_K);
    if !(1..=MAX_TOP_K).contains(&top_k) {
        return Err(ApiError::BadRequest(format!(
            "topK 는 1~{MAX_TOP_K} 이어야 합니다"
        )));
    }
    Ok((question.to_owned(), i64::from(top_k)))
}

fn user_prompt(question: &str, sources: &[RagSource]) -> String {
    let mut prompt = String::from("<context>\n");
    for (i, source) in sources.iter().enumerate() {
        prompt.push_str(&format!("[{}]\n{}\n\n", i + 1, source.content));
    }
    prompt.push_str(&format!("</context>\n\nQuestion: {question}"));
    prompt
}

pub async fn query(
    State(state): State<AppState>,
    ApiPath(index_id): ApiPath<Uuid>,
    ApiJson(req): ApiJson<RagQueryRequest>,
) -> Result<Json<RagQueryResponse>, ApiError> {
    let (question, top_k) = validate(&req)?;

    db::rag::get_index(&state.db, index_id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("RAG index {index_id} not found")))?;

    let embedding = state
        .embedder
        .embed(std::slice::from_ref(&question))
        .await
        .map_err(|e| ApiError::Upstream(e.to_string()))?
        .into_iter()
        .next()
        .ok_or_else(|| ApiError::Upstream("임베딩 응답이 비어 있습니다".to_owned()))?;

    let hits = db::rag::search(&state.db, index_id, &embedding, top_k).await?;
    let sources: Vec<RagSource> = hits
        .into_iter()
        .map(|hit| RagSource {
            chunk_index: hit.chunk_index,
            content: hit.content,
            score: hit.score,
        })
        .collect();

    let answer = state
        .llm
        .complete(LlmRequest {
            system: SYSTEM_PROMPT.to_owned(),
            user_content: vec![UserBlock::Text(user_prompt(&question, &sources))],
        })
        .await
        .map_err(|e| ApiError::Upstream(e.to_string()))?;

    Ok(Json(RagQueryResponse { answer, sources }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(question: &str, top_k: Option<u32>) -> RagQueryRequest {
        RagQueryRequest {
            question: question.to_owned(),
            top_k,
        }
    }

    #[test]
    fn validation_rules() {
        assert_eq!(
            validate(&req("  hi  ", None)).unwrap(),
            ("hi".to_owned(), 5)
        );
        assert_eq!(validate(&req("q", Some(1))).unwrap().1, 1);
        assert_eq!(validate(&req("q", Some(20))).unwrap().1, 20);
        for bad in [
            req("", None),
            req("   ", None),
            req(&"q".repeat(2001), None),
            req("q", Some(0)),
            req("q", Some(21)),
        ] {
            assert!(
                matches!(validate(&bad), Err(ApiError::BadRequest(_))),
                "{bad:?}"
            );
        }
        assert!(validate(&req(&"가".repeat(2000), None)).is_ok());
    }

    #[test]
    fn prompt_numbers_the_passages_in_order() {
        let sources = vec![
            RagSource {
                chunk_index: 7,
                content: "first".into(),
                score: 0.9,
            },
            RagSource {
                chunk_index: 2,
                content: "second".into(),
                score: 0.5,
            },
        ];
        let prompt = user_prompt("why?", &sources);
        assert!(prompt.starts_with("<context>\n[1]\nfirst\n\n[2]\nsecond\n\n</context>"));
        assert!(prompt.ends_with("Question: why?"));
    }
}
