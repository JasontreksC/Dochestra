//! Node catalog: the single definition site of all node specs (CONTEXT §3.3).

use std::sync::LazyLock;

use serde_json::{Value, json};

use crate::types::{DataType, NodeKind, NodeSpec, ScalarType, SocketSpec};

fn socket(id: &str, label: &str, data_type: DataType, required: bool) -> SocketSpec {
    SocketSpec {
        id: id.to_owned(),
        label: label.to_owned(),
        data_type,
        required,
    }
}

fn node(
    kind: NodeKind,
    label: &str,
    description: &str,
    inputs: Vec<SocketSpec>,
    outputs: Vec<SocketSpec>,
    default_params: Value,
) -> NodeSpec {
    NodeSpec {
        kind,
        label: label.to_owned(),
        description: description.to_owned(),
        inputs,
        outputs,
        default_params,
    }
}

static CATALOG: LazyLock<Vec<NodeSpec>> = LazyLock::new(build_catalog);

/// All node specs, in catalog order.
pub fn catalog() -> &'static [NodeSpec] {
    &CATALOG
}

/// Spec for a node kind.
pub fn spec(kind: NodeKind) -> &'static NodeSpec {
    // The catalog contains every `NodeKind` exactly once (enforced by tests).
    CATALOG
        .iter()
        .find(|s| s.kind == kind)
        .unwrap_or_else(|| unreachable!("catalog is missing {kind:?}"))
}

fn build_catalog() -> Vec<NodeSpec> {
    use ScalarType::{Csv, Jpg, Json, Md, Mp3, Pdf};

    let pdf = DataType::single(Pdf);
    let md = DataType::single(Md);
    let json_t = DataType::single(Json);
    let jpg = DataType::single(Jpg);
    let mp3 = DataType::single(Mp3);
    let pdf_list = DataType::list_of(Pdf);
    let jpg_list = DataType::list_of(Jpg);
    let csv_list = DataType::list_of(Csv);

    vec![
        node(
            NodeKind::Bundle,
            "PDF 번들",
            "PDF 파일 여러 개를 묶어 놓은 것으로, 파일시스템과 연동되며 입력 없이 출력만 한다.",
            vec![],
            vec![socket("files", "PDF 목록", pdf_list, true)],
            json!({ "bundleId": null }),
        ),
        node(
            NodeKind::PickPdf,
            "PDF 선택",
            "PDF 목록에서 지정한 순번의 PDF 한 개를 고른다.",
            vec![socket("files", "PDF 목록", pdf_list, true)],
            vec![socket("pdf", "PDF", pdf, true)],
            json!({ "index": 0 }),
        ),
        node(
            NodeKind::PickJpg,
            "이미지 선택",
            "이미지 목록에서 지정한 순번의 이미지 한 개를 고른다.",
            vec![socket("images", "이미지 목록", jpg_list, true)],
            vec![socket("image", "이미지", jpg, true)],
            json!({ "index": 0 }),
        ),
        node(
            NodeKind::JsonInput,
            "JSON 입력",
            "사용자가 직접 편집한 JSON 값을 출력한다.",
            vec![],
            vec![socket("json", "JSON", json_t, true)],
            json!({ "value": {} }),
        ),
        node(
            NodeKind::Extract,
            "추출",
            "PDF에서 텍스트, 이미지, 표 등을 추출한다.",
            vec![socket("pdf", "PDF", pdf, true)],
            vec![
                socket("md", "Markdown", md, true),
                socket("json", "JSON", json_t, true),
                socket("tables", "표 목록", csv_list, true),
                socket("images", "이미지 목록", jpg_list, true),
            ],
            json!({}),
        ),
        node(
            NodeKind::Translate,
            "번역",
            "추출 후 마크다운으로 정제된 PDF 내용을 번역한다. 번역 관련 설정은 JSON으로 받는다.",
            vec![
                socket("md", "Markdown", md, true),
                socket("settings", "설정(JSON)", json_t, true),
            ],
            vec![socket("md", "Markdown", md, true)],
            json!({}),
        ),
        node(
            NodeKind::Analyze,
            "이미지 분석",
            "LLM이 이미지에 대해 분석한다.",
            vec![socket("image", "이미지", jpg, true)],
            vec![socket("md", "Markdown", md, true)],
            json!({}),
        ),
        node(
            NodeKind::Summary,
            "요약",
            "추출된 PDF 내용을 바탕으로 요약한다. 추출된 이미지 목록을 넣어 요약에 보탤 수 있다.",
            vec![
                socket("md", "Markdown", md, true),
                socket("images", "이미지 목록", jpg_list, false),
            ],
            vec![socket("md", "Markdown", md, true)],
            json!({}),
        ),
        node(
            NodeKind::Slide,
            "슬라이드 생성",
            "발표 자료 슬라이드를 생성한다.",
            vec![
                socket("md", "Markdown", md, true),
                socket("image", "이미지", jpg, true),
            ],
            vec![socket("pdf", "PDF", pdf, true)],
            json!({}),
        ),
        node(
            NodeKind::Voice,
            "해설 음성 생성",
            "해설 음성을 생성한다.",
            vec![
                socket("md", "Markdown", md, true),
                socket("images", "이미지 목록", jpg_list, true),
            ],
            vec![socket("mp3", "음성", mp3, true)],
            json!({}),
        ),
        node(
            NodeKind::Fill,
            "폼 자동 입력",
            "주어진 PDF에 정의된 폼을 인식해 자동으로 채운다. 채울 값은 JSON 키-값 쌍으로 받는다.",
            vec![
                socket("pdf", "PDF", pdf, true),
                socket("values", "값(JSON)", json_t, true),
            ],
            vec![socket("pdf", "PDF", pdf, true)],
            json!({}),
        ),
        node(
            NodeKind::Rag,
            "RAG 챗봇",
            "문서를 바탕으로 RAG를 구축하고, API 호출 정보를 JSON으로 출력한다.",
            vec![socket("md", "Markdown", md, true)],
            vec![socket("api", "API 정보(JSON)", json_t, true)],
            json!({}),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    type Sock = (&'static str, &'static str, bool); // (id, "PDF"/"LIST[JPG]", required)

    fn dt(s: &str) -> DataType {
        let parse = |t: &str| match t {
            "PDF" => ScalarType::Pdf,
            "MD" => ScalarType::Md,
            "JSON" => ScalarType::Json,
            "CSV" => ScalarType::Csv,
            "JPG" => ScalarType::Jpg,
            "MP3" => ScalarType::Mp3,
            other => panic!("bad type {other}"),
        };
        match s.strip_prefix("LIST[").and_then(|r| r.strip_suffix(']')) {
            Some(inner) => DataType::list_of(parse(inner)),
            None => DataType::single(parse(s)),
        }
    }

    fn actual(sockets: &[SocketSpec]) -> Vec<(String, String, bool)> {
        sockets
            .iter()
            .map(|s| (s.id.clone(), s.data_type.to_string(), s.required))
            .collect()
    }

    fn expected(sockets: &[Sock]) -> Vec<(String, String, bool)> {
        sockets
            .iter()
            .map(|(id, t, r)| ((*id).to_owned(), dt(t).to_string(), *r))
            .collect()
    }

    #[test]
    fn catalog_matches_contract_table() {
        let table: Vec<(NodeKind, Vec<Sock>, Vec<Sock>, Value)> = vec![
            (
                NodeKind::Bundle,
                vec![],
                vec![("files", "LIST[PDF]", true)],
                json!({"bundleId": null}),
            ),
            (
                NodeKind::PickPdf,
                vec![("files", "LIST[PDF]", true)],
                vec![("pdf", "PDF", true)],
                json!({"index": 0}),
            ),
            (
                NodeKind::PickJpg,
                vec![("images", "LIST[JPG]", true)],
                vec![("image", "JPG", true)],
                json!({"index": 0}),
            ),
            (
                NodeKind::JsonInput,
                vec![],
                vec![("json", "JSON", true)],
                json!({"value": {}}),
            ),
            (
                NodeKind::Extract,
                vec![("pdf", "PDF", true)],
                vec![
                    ("md", "MD", true),
                    ("json", "JSON", true),
                    ("tables", "LIST[CSV]", true),
                    ("images", "LIST[JPG]", true),
                ],
                json!({}),
            ),
            (
                NodeKind::Translate,
                vec![("md", "MD", true), ("settings", "JSON", true)],
                vec![("md", "MD", true)],
                json!({}),
            ),
            (
                NodeKind::Analyze,
                vec![("image", "JPG", true)],
                vec![("md", "MD", true)],
                json!({}),
            ),
            (
                NodeKind::Summary,
                vec![("md", "MD", true), ("images", "LIST[JPG]", false)],
                vec![("md", "MD", true)],
                json!({}),
            ),
            (
                NodeKind::Slide,
                vec![("md", "MD", true), ("image", "JPG", true)],
                vec![("pdf", "PDF", true)],
                json!({}),
            ),
            (
                NodeKind::Voice,
                vec![("md", "MD", true), ("images", "LIST[JPG]", true)],
                vec![("mp3", "MP3", true)],
                json!({}),
            ),
            (
                NodeKind::Fill,
                vec![("pdf", "PDF", true), ("values", "JSON", true)],
                vec![("pdf", "PDF", true)],
                json!({}),
            ),
            (
                NodeKind::Rag,
                vec![("md", "MD", true)],
                vec![("api", "JSON", true)],
                json!({}),
            ),
        ];

        let cat = catalog();
        assert_eq!(cat.len(), 12);
        for (spec, (kind, ins, outs, params)) in cat.iter().zip(table) {
            assert_eq!(spec.kind, kind, "order mismatch at {kind:?}");
            assert_eq!(actual(&spec.inputs), expected(&ins), "{kind:?} inputs");
            assert_eq!(actual(&spec.outputs), expected(&outs), "{kind:?} outputs");
            assert_eq!(spec.default_params, params, "{kind:?} defaultParams");
            assert!(!spec.label.is_empty() && !spec.description.is_empty());
        }
    }

    #[test]
    fn socket_ids_unique_within_node() {
        for spec in catalog() {
            let ins: HashSet<_> = spec.inputs.iter().map(|s| &s.id).collect();
            let outs: HashSet<_> = spec.outputs.iter().map(|s| &s.id).collect();
            assert_eq!(ins.len(), spec.inputs.len(), "{:?} inputs", spec.kind);
            assert_eq!(outs.len(), spec.outputs.len(), "{:?} outputs", spec.kind);
        }
    }

    #[test]
    fn only_summary_images_is_optional() {
        let optional: Vec<(NodeKind, &str)> = catalog()
            .iter()
            .flat_map(|n| {
                n.inputs
                    .iter()
                    .chain(n.outputs.iter())
                    .filter(|s| !s.required)
                    .map(move |s| (n.kind, s.id.as_str()))
            })
            .collect();
        assert_eq!(optional, vec![(NodeKind::Summary, "images")]);
    }

    #[test]
    fn spec_lookup_works_for_every_kind() {
        for s in catalog() {
            assert_eq!(spec(s.kind).kind, s.kind);
        }
    }
}
