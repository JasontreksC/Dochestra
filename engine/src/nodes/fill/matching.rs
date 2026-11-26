//! Matching JSON keys to form fields: exact, normalized, and (optionally) LLM-assisted.

use std::collections::BTreeMap;

use serde_json::Value;

use super::form::{FieldKind, FormField, normalize};
use super::labels::Hints;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Method {
    Exact,
    Normalized,
    Llm,
}

/// One key matched to a field (by index into the discovered fields).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Match {
    pub key: String,
    pub field: usize,
    pub method: Method,
}

/// Exact matches (key == full field name), then normalized matches (case-, space- and
/// `_`/`-`/`.`-insensitive, against the full name or its last segment). A normalized match is
/// only taken when it is unambiguous: one candidate, or exactly one whose *full* name matches.
/// Each key and each field is used at most once. Keys are processed in sorted order.
pub fn match_keys(keys: &BTreeMap<String, Value>, fields: &[FormField]) -> Vec<Match> {
    let mut matches: Vec<Match> = Vec::new();
    let mut used = vec![false; fields.len()];

    // Pass 1: exact.
    for key in keys.keys() {
        if let Some(i) = fields
            .iter()
            .enumerate()
            .position(|(i, f)| !used[i] && f.name == *key)
        {
            used[i] = true;
            matches.push(Match {
                key: key.clone(),
                field: i,
                method: Method::Exact,
            });
        }
    }

    // Pass 2: normalized.
    for key in keys.keys() {
        if matches.iter().any(|m| m.key == *key) {
            continue;
        }
        let wanted = normalize(key);
        if wanted.is_empty() {
            continue;
        }
        let candidates: Vec<usize> = (0..fields.len())
            .filter(|&i| !used[i])
            .filter(|&i| {
                normalize(&fields[i].name) == wanted
                    || normalize(fields[i].last_segment()) == wanted
            })
            .collect();
        let chosen = match candidates.as_slice() {
            [only] => Some(*only),
            [] => None,
            many => {
                let full: Vec<usize> = many
                    .iter()
                    .copied()
                    .filter(|&i| normalize(&fields[i].name) == wanted)
                    .collect();
                (full.len() == 1).then(|| full[0])
            }
        };
        if let Some(i) = chosen {
            used[i] = true;
            matches.push(Match {
                key: key.clone(),
                field: i,
                method: Method::Normalized,
            });
        }
    }
    matches
}

/// A key that matched a field also fills every other field with the very same name (a form that
/// repeats "name" or "phone" on several pages is meant to carry one value; the PDF spec treats
/// same-named fields as one field). Keeps each field used at most once.
pub fn expand_same_name(matches: Vec<Match>, fields: &[FormField]) -> Vec<Match> {
    let mut used: Vec<bool> = vec![false; fields.len()];
    for m in &matches {
        used[m.field] = true;
    }
    let mut out = matches.clone();
    for m in &matches {
        for (i, other) in fields.iter().enumerate() {
            if !used[i] && other.name == fields[m.field].name {
                used[i] = true;
                out.push(Match {
                    key: m.key.clone(),
                    field: i,
                    method: m.method,
                });
            }
        }
    }
    out
}

pub const LLM_SYSTEM_PROMPT: &str = "\
You map input keys to the fields of a PDF form. Output ONLY one JSON object of the form \
{\"<key>\": \"<field name>\" or null, ...} with every given key as a property.

Each field is listed with its name, its type and, when known, `label` (the text printed next to \
it on the page) and `tooltip`. Field names are often meaningless codes such as Text1 or Check3: \
then decide by the label. A radio field also shows the printed label of each of its options.

Rules:
- Choose a field only when it clearly means the same thing as the key. Names and labels may be \
in any language, abbreviated, or written in different cases or styles.
- Use the exact field names from the list; each field may be chosen at most once.
- Use null when no field fits or you are unsure.
- Do not output anything but the JSON object.";

fn json_type(value: &Value) -> &'static str {
    match value {
        Value::String(_) => "string",
        Value::Number(_) => "number",
        Value::Bool(_) => "boolean",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
        Value::Null => "null",
    }
}

/// The user prompt: remaining keys (with their JSON types — never their values) and fields.
pub fn llm_prompt(keys: &[(&String, &Value)], fields: &[(&FormField, &Hints)]) -> String {
    let mut prompt = String::from("Keys:\n");
    for (key, value) in keys {
        prompt.push_str(&format!("- {key} ({})\n", json_type(value)));
    }
    prompt.push_str("\nFields:\n");
    for (field, hints) in fields {
        let detail = match &field.kind {
            FieldKind::Text | FieldKind::Checkbox { .. } => String::new(),
            FieldKind::Radio { options } => {
                let list: Vec<String> = options
                    .iter()
                    .enumerate()
                    .map(|(i, option)| {
                        match hints.option_labels.get(i).and_then(Option::as_deref) {
                            Some(label) => format!("{option} \"{label}\""),
                            None => option.clone(),
                        }
                    })
                    .collect();
                format!(": {}", list.join(" | "))
            }
            FieldKind::Choice { options } => {
                let list: Vec<String> = options
                    .iter()
                    .map(|o| {
                        if o.export == o.display {
                            o.export.clone()
                        } else {
                            format!("{} ({})", o.export, o.display)
                        }
                    })
                    .collect();
                format!(": {}", list.join(" | "))
            }
        };
        let mut extra = String::new();
        if let Some(label) = &hints.label {
            extra.push_str(&format!(" — label: \"{label}\""));
        }
        if let Some(tooltip) = &hints.tooltip {
            extra.push_str(&format!(" — tooltip: \"{tooltip}\""));
        }
        prompt.push_str(&format!(
            "- {} ({}{detail}){extra}\n",
            field.name,
            field.kind.label()
        ));
    }
    prompt
}

/// Parses the LLM answer (`{"key": "field" | null}`, found between the first `{` and the last
/// `}`) into `(key, field name)` pairs.
pub fn parse_llm_mapping(answer: &str) -> Result<Vec<(String, String)>, String> {
    let (start, end) = match (answer.find('{'), answer.rfind('}')) {
        (Some(s), Some(e)) if s < e => (s, e),
        _ => return Err("응답에서 JSON 객체를 찾을 수 없습니다".to_owned()),
    };
    let map: BTreeMap<String, Option<String>> =
        serde_json::from_str(&answer[start..=end]).map_err(|e| e.to_string())?;
    Ok(map
        .into_iter()
        .filter_map(|(key, field)| field.map(|f| (key, f)))
        .collect())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::nodes::fill::form::ChoiceOption;

    fn text(name: &str, index: u32) -> FormField {
        FormField {
            name: name.into(),
            kind: FieldKind::Text,
            current: None,
            id: (index, 0),
            widgets: vec![],
        }
    }

    fn keys(pairs: &[(&str, Value)]) -> BTreeMap<String, Value> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), v.clone()))
            .collect()
    }

    #[test]
    fn exact_beats_normalized_and_each_field_is_used_once() {
        let fields = vec![text("name", 1), text("Name", 2)];
        let matches = match_keys(
            &keys(&[("Name", json!("a")), ("name", json!("b"))]),
            &fields,
        );
        // "Name" -> field "Name" and "name" -> field "name", both exact.
        assert_eq!(matches.len(), 2);
        assert!(matches.iter().all(|m| m.method == Method::Exact));
        let by_key: BTreeMap<_, _> = matches.iter().map(|m| (m.key.as_str(), m.field)).collect();
        assert_eq!(by_key["Name"], 1);
        assert_eq!(by_key["name"], 0);
    }

    #[test]
    fn normalized_matching_uses_full_name_or_last_segment() {
        let fields = vec![
            text("applicant.first_name", 1),
            text("korean_name", 2),
            text("Phone Number", 3),
        ];
        let matches = match_keys(
            &keys(&[
                ("firstName", json!("A")),
                ("Korean Name", json!("B")),
                ("phone-number", json!("C")),
            ]),
            &fields,
        );
        let by_key: BTreeMap<_, _> = matches
            .iter()
            .map(|m| (m.key.as_str(), (m.field, m.method)))
            .collect();
        assert_eq!(by_key["firstName"], (0, Method::Normalized)); // last segment
        assert_eq!(by_key["Korean Name"], (1, Method::Normalized));
        assert_eq!(by_key["phone-number"], (2, Method::Normalized));

        // The full name also matches loosely.
        let matches = match_keys(&keys(&[("Applicant First-Name", json!("A"))]), &fields);
        assert_eq!(
            matches,
            vec![Match {
                key: "Applicant First-Name".into(),
                field: 0,
                method: Method::Normalized
            }]
        );
    }

    #[test]
    fn a_field_takes_only_one_key() {
        let fields = vec![text("first_name", 1)];
        // Both keys normalize to the field; the first in sorted order wins, the other stays unmatched.
        let matches = match_keys(
            &keys(&[("FirstName", json!("A")), ("first name", json!("B"))]),
            &fields,
        );
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].key, "FirstName");
    }

    #[test]
    fn ambiguous_normalized_matches_are_left_unmatched() {
        // Two sections both have "name": the bare key "name" is ambiguous.
        let fields = vec![text("home.name", 1), text("work.name", 2)];
        assert!(match_keys(&keys(&[("name", json!("x"))]), &fields).is_empty());
        // ... unless exactly one full name matches.
        let fields = vec![text("name", 1), text("work.name", 2)];
        let matches = match_keys(&keys(&[("NAME", json!("x"))]), &fields);
        assert_eq!(
            matches,
            vec![Match {
                key: "NAME".into(),
                field: 0,
                method: Method::Normalized
            }]
        );
    }

    #[test]
    fn unknown_keys_do_not_match() {
        let fields = vec![text("name", 1)];
        assert!(
            match_keys(&keys(&[("address", json!("x")), ("", json!("y"))]), &fields).is_empty()
        );
    }

    #[test]
    fn prompt_lists_keys_with_types_but_never_values() {
        let radio = FormField {
            name: "gender".into(),
            kind: FieldKind::Radio {
                options: vec!["Male".into(), "Female".into()],
            },
            current: None,
            id: (5, 0),
            widgets: vec![],
        };
        let choice = FormField {
            name: "city".into(),
            kind: FieldKind::Choice {
                options: vec![
                    ChoiceOption {
                        export: "sel".into(),
                        display: "서울".into(),
                    },
                    ChoiceOption {
                        export: "KR".into(),
                        display: "KR".into(),
                    },
                ],
            },
            current: None,
            id: (6, 0),
            widgets: vec![],
        };
        let check = FormField {
            name: "agree".into(),
            kind: FieldKind::Checkbox {
                on_state: "Yes".into(),
            },
            current: None,
            id: (7, 0),
            widgets: vec![],
        };
        let (k1, v1) = ("성명".to_owned(), json!("홍길동-비밀"));
        let (k2, v2) = ("동의".to_owned(), json!(true));
        let none = Hints::default();
        let prompt = llm_prompt(
            &[(&k1, &v1), (&k2, &v2)],
            &[(&radio, &none), (&choice, &none), (&check, &none)],
        );
        assert!(prompt.contains("- 성명 (string)") && prompt.contains("- 동의 (boolean)"));
        assert!(prompt.contains("- gender (radio: Male | Female)"));
        assert!(prompt.contains("- city (choice: sel (서울) | KR)"));
        assert!(prompt.contains("- agree (checkbox)"));
        assert!(
            !prompt.contains("홍길동"),
            "values must not be sent: {prompt}"
        );
    }

    #[test]
    fn llm_answers_are_parsed_tolerantly() {
        let answer = "Here you go:\n```json\n{\"성명\": \"korean_name\", \"없음\": null, \"동의\": \"agree\"}\n```";
        assert_eq!(
            parse_llm_mapping(answer).unwrap(),
            vec![
                ("동의".to_owned(), "agree".to_owned()),
                ("성명".to_owned(), "korean_name".to_owned())
            ]
        );
        assert!(parse_llm_mapping("no json").is_err());
        assert!(parse_llm_mapping("{\"a\": 5}").is_err());
        assert!(parse_llm_mapping("[1,2]").is_err());
    }

    #[test]
    fn a_matched_name_extends_to_the_fields_repeating_it() {
        let fields = vec![
            text("name", 1),
            text("phone", 2),
            text("name", 3),
            text("name", 4),
        ];
        let matches = match_keys(&keys(&[("name", json!("x"))]), &fields);
        assert_eq!(matches.len(), 1);
        let expanded = expand_same_name(matches, &fields);
        let mut targets: Vec<usize> = expanded.iter().map(|m| m.field).collect();
        targets.sort_unstable();
        assert_eq!(targets, [0, 2, 3]);
        assert!(
            expanded
                .iter()
                .all(|m| m.key == "name" && m.method == Method::Exact)
        );

        // A field another key already took is left alone.
        let taken = vec![
            Match {
                key: "name".into(),
                field: 0,
                method: Method::Exact,
            },
            Match {
                key: "other".into(),
                field: 2,
                method: Method::Llm,
            },
        ];
        let expanded = expand_same_name(taken, &fields);
        assert_eq!(expanded.len(), 3);
        assert_eq!(expanded.iter().filter(|m| m.key == "name").count(), 2); // fields 0 and 3
    }

    #[test]
    fn the_prompt_shows_labels_option_labels_and_tooltips() {
        let radio = FormField {
            name: "Group1".into(),
            kind: FieldKind::Radio {
                options: vec!["Choice1".into(), "Choice2".into()],
            },
            current: None,
            id: (5, 0),
            widgets: vec![],
        };
        let plain = FormField {
            name: "Text1".into(),
            kind: FieldKind::Text,
            current: None,
            id: (6, 0),
            widgets: vec![],
        };
        let radio_hints = Hints {
            label: Some("성별".into()),
            tooltip: None,
            option_labels: vec![Some("남".into()), None],
        };
        let text_hints = Hints {
            label: Some("성명".into()),
            tooltip: Some("Applicant name".into()),
            option_labels: vec![],
        };
        let key = "이름".to_owned();
        let value = json!("비밀");
        let prompt = llm_prompt(
            &[(&key, &value)],
            &[(&radio, &radio_hints), (&plain, &text_hints)],
        );
        assert!(
            prompt.contains("- Group1 (radio: Choice1 \"남\" | Choice2) — label: \"성별\""),
            "{prompt}"
        );
        assert!(
            prompt.contains("- Text1 (text) — label: \"성명\" — tooltip: \"Applicant name\""),
            "{prompt}"
        );
        assert!(!prompt.contains("비밀"));
    }
}
