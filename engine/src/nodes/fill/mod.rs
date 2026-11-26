//! `fill` node: fills the AcroForm fields of a PDF from a JSON object of key-value pairs.

pub mod form;
pub mod labels;
pub mod matching;

use std::collections::{BTreeMap, HashSet};

use lopdf::Document;
use serde::Serialize;
use serde_json::Value;

use super::read_text;
use crate::clients::llm::{LlmClient, LlmRequest, UserBlock};
use crate::executor::runner::{Artifact, NodeContext, NodeError, NodeInputs, NodeOutputs};
use crate::storage::{single_output_path, work_dir};
use crate::types::{DataType, ScalarType};
use form::{FormField, apply, discover, plan_write_with, set_need_appearances};
use labels::Hints;
use matching::{
    LLM_SYSTEM_PROMPT, Match, Method, expand_same_name, llm_prompt, match_keys, parse_llm_mapping,
};

/// How many field names the "nothing matched" error lists.
const LISTED_FIELDS: usize = 20;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FillReport {
    pub filled: Vec<Filled>,
    pub unmatched_keys: Vec<String>,
    pub untouched_fields: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct Filled {
    pub key: String,
    pub field: String,
    pub value: String,
    pub method: Method,
}

fn failed(message: impl Into<String>) -> NodeError {
    NodeError::Failed(message.into())
}

pub async fn run(
    llm: &LlmClient,
    ctx: &NodeContext,
    inputs: &NodeInputs,
) -> Result<NodeOutputs, NodeError> {
    let pdf_path = inputs
        .get("pdf")
        .and_then(|a| a.paths.first())
        .ok_or_else(|| failed("입력 pdf 이(가) 없습니다"))?;
    let values_path = inputs
        .get("values")
        .and_then(|a| a.paths.first())
        .ok_or_else(|| failed("입력 values 이(가) 없습니다"))?;

    // --- values
    let values_text = read_text(values_path, "values").await?;
    let values: Value = serde_json::from_str(&values_text)
        .map_err(|e| failed(format!("폼 값 JSON 을 읽을 수 없습니다: {e}")))?;
    let Value::Object(values) = values else {
        return Err(failed("폼 값 JSON 은 객체여야 합니다"));
    };

    let mut warnings: Vec<String> = Vec::new();
    // `null` means "nothing to fill"; arrays and objects have no form representation.
    let mut keys: BTreeMap<String, Value> = BTreeMap::new();
    for (key, value) in values {
        match value {
            Value::Null => {}
            Value::Array(_) | Value::Object(_) => {
                warnings.push(format!(
                    "키 '{key}' 의 값 형식(배열/객체)은 지원하지 않아 무시했습니다"
                ));
            }
            scalar => {
                keys.insert(key, scalar);
            }
        }
    }

    // --- pdf and its fields
    let pdf_bytes = tokio::fs::read(pdf_path)
        .await
        .map_err(|e| failed(format!("PDF 를 읽을 수 없습니다: {e}")))?;
    let (mut doc, fields, hints) = tokio::task::spawn_blocking(
        move || -> Result<(Document, Vec<FormField>, Vec<Hints>), String> {
            let doc = Document::load_mem(&pdf_bytes)
                .map_err(|e| format!("PDF 를 읽을 수 없습니다: {e}"))?;
            if doc.is_encrypted() {
                return Err("암호화된 PDF 는 지원하지 않습니다".to_owned());
            }
            let fields = discover(&doc);
            let hints = labels::collect(&doc, &fields);
            Ok((doc, fields, hints))
        },
    )
    .await
    .map_err(|e| NodeError::Other(e.into()))?
    .map_err(failed)?;
    if fields.is_empty() {
        return Err(failed("PDF 에 입력 가능한 폼 필드가 없습니다"));
    }

    // --- matching: exact, normalized, then the LLM for what is left
    let mut matches = match_keys(&keys, &fields);
    matches.extend(llm_matches(llm, &keys, &fields, &hints, &matches, &mut warnings).await);
    // Fields repeating a matched field's name (the same value on several pages) are filled too.
    let matches = expand_same_name(matches, &fields);

    // --- write
    let mut filled: Vec<Filled> = Vec::new();
    let mut touched = vec![false; fields.len()];
    for Match {
        key,
        field: index,
        method,
    } in &matches
    {
        let field = &fields[*index];
        let planned = match plan_write_with(field, &keys[key], &hints[*index]) {
            Ok(planned) => planned,
            Err(reason) => {
                warnings.push(reason);
                continue;
            }
        };
        if let Err(reason) = apply(&mut doc, field, &planned.write) {
            warnings.push(reason);
            continue;
        }
        warnings.extend(planned.warning);
        touched[*index] = true;
        filled.push(Filled {
            key: key.clone(),
            field: field.name.clone(),
            value: planned.shown,
            method: *method,
        });
    }

    let matched_keys: Vec<&String> = matches.iter().map(|m| &m.key).collect();
    let report = FillReport {
        unmatched_keys: keys
            .keys()
            .filter(|k| !matched_keys.contains(k))
            .cloned()
            .collect(),
        untouched_fields: fields
            .iter()
            .zip(&touched)
            .filter(|(_, t)| !**t)
            .map(|(f, _)| f.name.clone())
            .collect(),
        filled,
        warnings,
    };
    write_report(ctx, &report).await?;

    if report.filled.is_empty() {
        let listed: Vec<&str> = fields
            .iter()
            .take(LISTED_FIELDS)
            .map(|f| f.name.as_str())
            .collect();
        let more = fields.len().saturating_sub(LISTED_FIELDS);
        let suffix = if more > 0 {
            format!(" 외 {more}개")
        } else {
            String::new()
        };
        return Err(failed(format!(
            "입력 값과 일치하는 폼 필드가 없습니다 (필드: {}{suffix})",
            listed.join(", ")
        )));
    }

    let bytes = tokio::task::spawn_blocking(move || -> Result<Vec<u8>, String> {
        set_need_appearances(&mut doc)?;
        let mut out = Vec::new();
        doc.save_to(&mut out)
            .map_err(|e| format!("PDF 를 저장할 수 없습니다: {e}"))?;
        Ok(out)
    })
    .await
    .map_err(|e| NodeError::Other(e.into()))?
    .map_err(failed)?;

    let path = single_output_path(&ctx.out_dir, "pdf", ScalarType::Pdf);
    tokio::fs::write(&path, bytes).await.map_err(|e| {
        NodeError::Other(anyhow::Error::new(e).context("failed to write pdf output"))
    })?;
    Ok(NodeOutputs::from([(
        "pdf".to_owned(),
        Artifact {
            data_type: DataType::single(ScalarType::Pdf),
            paths: vec![path],
        },
    )]))
}

/// Asks the LLM to match the keys and fields that exact/normalized matching left over. Any
/// failure (no key, API error, unusable answer) only skips this step, with a warning.
async fn llm_matches(
    llm: &LlmClient,
    keys: &BTreeMap<String, Value>,
    fields: &[FormField],
    hints: &[Hints],
    done: &[Match],
    warnings: &mut Vec<String>,
) -> Vec<Match> {
    let remaining_keys: Vec<(&String, &Value)> = keys
        .iter()
        .filter(|(k, _)| !done.iter().any(|m| &m.key == *k))
        .collect();
    // A name the form repeats is offered once, and never when an earlier match already took that name.
    let mut listed: HashSet<&str> = done.iter().map(|m| fields[m.field].name.as_str()).collect();
    let remaining_fields: Vec<(usize, &FormField)> = fields
        .iter()
        .enumerate()
        .filter(|(i, _)| !done.iter().any(|m| m.field == *i))
        .filter(|(_, f)| listed.insert(f.name.as_str()))
        .collect();
    if remaining_keys.is_empty() || remaining_fields.is_empty() {
        return Vec::new();
    }

    let prompt = llm_prompt(
        &remaining_keys,
        &remaining_fields
            .iter()
            .map(|(i, f)| (*f, &hints[*i]))
            .collect::<Vec<_>>(),
    );
    let answer = match llm
        .complete(LlmRequest {
            system: LLM_SYSTEM_PROMPT.to_owned(),
            user_content: vec![UserBlock::Text(prompt)],
        })
        .await
    {
        Ok(answer) => answer,
        Err(err) => {
            tracing::warn!(error = %err, "LLM key mapping skipped");
            warnings.push(format!("LLM 키 매핑을 건너뛰었습니다: {err}"));
            return Vec::new();
        }
    };
    let pairs = match parse_llm_mapping(&answer) {
        Ok(pairs) => pairs,
        Err(reason) => {
            tracing::warn!(%reason, "LLM key mapping unusable");
            warnings.push(format!(
                "LLM 키 매핑 응답을 해석하지 못해 건너뛰었습니다: {reason}"
            ));
            return Vec::new();
        }
    };

    // Accept only known keys and fields, each at most once.
    let mut used_fields: Vec<usize> = Vec::new();
    let mut accepted: Vec<Match> = Vec::new();
    for (key, field_name) in pairs {
        let known_key = remaining_keys.iter().any(|(k, _)| **k == key);
        let target = remaining_fields
            .iter()
            .find(|(i, f)| f.name == field_name && !used_fields.contains(i))
            .map(|(i, _)| *i);
        match (known_key, target) {
            (true, Some(index)) => {
                used_fields.push(index);
                accepted.push(Match {
                    key,
                    field: index,
                    method: Method::Llm,
                });
            }
            _ => warnings.push(format!(
                "LLM 이 제안한 매핑을 무시했습니다: {key} -> {field_name}"
            )),
        }
    }
    accepted
}

async fn write_report(ctx: &NodeContext, report: &FillReport) -> Result<(), NodeError> {
    let work = work_dir(&ctx.out_dir);
    tokio::fs::create_dir_all(&work).await.map_err(|e| {
        NodeError::Other(anyhow::Error::new(e).context("failed to create work dir"))
    })?;
    let json = serde_json::to_vec_pretty(report).map_err(anyhow::Error::from)?;
    tokio::fs::write(work.join("fill_report.json"), json)
        .await
        .map_err(|e| NodeError::Other(anyhow::Error::new(e).context("failed to save fill report")))
}
