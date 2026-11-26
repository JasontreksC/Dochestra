//! Structural graph validation (CONTEXT §4.1.2, "저장" checks).

use std::collections::{HashMap, HashSet};

use crate::catalog::spec;
use uuid::Uuid;

use crate::types::{GraphNode, NodeKind, ValidationIssue, WorkflowGraph};

/// Validates graph structure: duplicate ids, unknown nodes/sockets, type mismatches and
/// multiple incoming edges on one input socket.
///
/// Returns every issue found, in a deterministic order:
/// 1. `DUPLICATE_ID` (nodes, then edges; once per duplicated id, in order of first repeat)
/// 2. per edge in order: `UNKNOWN_NODE` (source, target), `UNKNOWN_SOCKET` (source, target),
///    `TYPE_MISMATCH` (only when both sockets exist)
/// 3. `MULTIPLE_INCOMING` (once per input socket with >1 edges whose target socket resolves,
///    in order of the socket's first edge)
///
/// Cycles, unconnected inputs, output-node presence and params are *not* checked here.
pub fn validate_structure(graph: &WorkflowGraph) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();

    duplicate_ids(graph.nodes.iter().map(|n| n.id.as_str()), &mut issues);
    duplicate_ids(graph.edges.iter().map(|e| e.id.as_str()), &mut issues);

    // With duplicate node ids the first occurrence wins.
    let mut nodes: HashMap<&str, &GraphNode> = HashMap::new();
    for node in &graph.nodes {
        nodes.entry(node.id.as_str()).or_insert(node);
    }

    let mut incoming_order: Vec<(&str, &str)> = Vec::new();
    let mut incoming_count: HashMap<(&str, &str), usize> = HashMap::new();

    for edge in &graph.edges {
        let source = nodes.get(edge.source.as_str());
        let target = nodes.get(edge.target.as_str());

        if source.is_none() {
            issues.push(ValidationIssue::UnknownNode {
                edge_id: edge.id.clone(),
                node_id: edge.source.clone(),
            });
        }
        if target.is_none() {
            issues.push(ValidationIssue::UnknownNode {
                edge_id: edge.id.clone(),
                node_id: edge.target.clone(),
            });
        }

        let out_socket = source.and_then(|n| {
            spec(n.kind)
                .outputs
                .iter()
                .find(|s| s.id == edge.source_handle)
        });
        let in_socket = target.and_then(|n| {
            spec(n.kind)
                .inputs
                .iter()
                .find(|s| s.id == edge.target_handle)
        });

        if source.is_some() && out_socket.is_none() {
            issues.push(ValidationIssue::UnknownSocket {
                edge_id: edge.id.clone(),
                node_id: edge.source.clone(),
                socket_id: edge.source_handle.clone(),
            });
        }
        if target.is_some() && in_socket.is_none() {
            issues.push(ValidationIssue::UnknownSocket {
                edge_id: edge.id.clone(),
                node_id: edge.target.clone(),
                socket_id: edge.target_handle.clone(),
            });
        }

        if let (Some(out), Some(inp)) = (out_socket, in_socket)
            && out.data_type != inp.data_type
        {
            issues.push(ValidationIssue::TypeMismatch {
                edge_id: edge.id.clone(),
                expected: inp.data_type,
                actual: out.data_type,
            });
        }

        if in_socket.is_some() {
            let key = (edge.target.as_str(), edge.target_handle.as_str());
            let count = incoming_count.entry(key).or_insert(0);
            if *count == 0 {
                incoming_order.push(key);
            }
            *count += 1;
        }
    }

    for key in incoming_order {
        if incoming_count[&key] > 1 {
            issues.push(ValidationIssue::MultipleIncoming {
                node_id: key.0.to_owned(),
                socket_id: key.1.to_owned(),
            });
        }
    }

    issues
}

/// Checks a node's `params` against its kind's rules (CONTEXT §3.3 `defaultParams`).
///
/// Returns a one-line Korean message describing the first violation, or `None` if valid.
/// Rules:
/// - every kind: `params` must be a JSON object
/// - `bundle`: `bundleId` must be a string that parses as a UUID
/// - `pick_pdf` / `pick_jpg`: `index` must be a non-negative integer
/// - `json_input`: `value` must be present and not `null`
///
/// Whether a bundle actually exists is not checked here (node execution does that).
pub fn validate_params(node: &GraphNode) -> Option<String> {
    let Some(params) = node.params.as_object() else {
        return Some("params 는 JSON 객체여야 합니다".to_owned());
    };
    match node.kind {
        NodeKind::Bundle => {
            let valid = params
                .get("bundleId")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|s| Uuid::parse_str(s).is_ok());
            (!valid).then(|| "bundleId 가 올바른 UUID 가 아닙니다".to_owned())
        }
        NodeKind::PickPdf | NodeKind::PickJpg => {
            let valid = params
                .get("index")
                .and_then(serde_json::Value::as_u64)
                .is_some();
            (!valid).then(|| "index 는 0 이상의 정수여야 합니다".to_owned())
        }
        NodeKind::JsonInput => {
            let valid = params.get("value").is_some_and(|v| !v.is_null());
            (!valid).then(|| "value 가 필요합니다".to_owned())
        }
        _ => None,
    }
}

fn duplicate_ids<'a>(ids: impl Iterator<Item = &'a str>, issues: &mut Vec<ValidationIssue>) {
    let mut seen: HashSet<&str> = HashSet::new();
    let mut reported: HashSet<&str> = HashSet::new();
    for id in ids {
        if !seen.insert(id) && reported.insert(id) {
            issues.push(ValidationIssue::DuplicateId { id: id.to_owned() });
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::types::{DataType, GraphEdge, NodeKind, Position, ScalarType};

    fn node(id: &str, kind: NodeKind) -> GraphNode {
        GraphNode {
            id: id.to_owned(),
            kind,
            position: Position { x: 0.0, y: 0.0 },
            params: json!({}),
            is_output: false,
        }
    }

    fn edge(id: &str, source: &str, sh: &str, target: &str, th: &str) -> GraphEdge {
        GraphEdge {
            id: id.to_owned(),
            source: source.to_owned(),
            source_handle: sh.to_owned(),
            target: target.to_owned(),
            target_handle: th.to_owned(),
        }
    }

    fn graph(nodes: Vec<GraphNode>, edges: Vec<GraphEdge>) -> WorkflowGraph {
        WorkflowGraph { nodes, edges }
    }

    fn valid_graph() -> WorkflowGraph {
        graph(
            vec![
                node("b", NodeKind::Bundle),
                node("p", NodeKind::PickPdf),
                node("x", NodeKind::Extract),
            ],
            vec![
                edge("e1", "b", "files", "p", "files"),
                edge("e2", "p", "pdf", "x", "pdf"),
            ],
        )
    }

    #[test]
    fn empty_and_valid_graphs_have_no_issues() {
        assert!(validate_structure(&WorkflowGraph::default()).is_empty());
        assert!(validate_structure(&valid_graph()).is_empty());
    }

    #[test]
    fn duplicate_node_and_edge_ids_reported_once_each() {
        let mut g = valid_graph();
        g.nodes.push(node("b", NodeKind::Bundle));
        g.nodes.push(node("b", NodeKind::Bundle));
        g.edges.push(edge("e1", "p", "pdf", "x", "pdf"));
        let issues = validate_structure(&g);
        assert_eq!(
            issues[..2],
            [
                ValidationIssue::DuplicateId { id: "b".into() },
                ValidationIssue::DuplicateId { id: "e1".into() },
            ]
        );
    }

    #[test]
    fn unknown_node_reports_missing_side() {
        let g = graph(
            vec![node("b", NodeKind::Bundle)],
            vec![edge("e1", "b", "files", "ghost", "files")],
        );
        assert_eq!(
            validate_structure(&g),
            vec![ValidationIssue::UnknownNode {
                edge_id: "e1".into(),
                node_id: "ghost".into()
            }]
        );
    }

    #[test]
    fn unknown_node_both_sides_gives_two_issues() {
        let g = graph(vec![], vec![edge("e1", "a", "x", "b", "y")]);
        assert_eq!(
            validate_structure(&g),
            vec![
                ValidationIssue::UnknownNode {
                    edge_id: "e1".into(),
                    node_id: "a".into()
                },
                ValidationIssue::UnknownNode {
                    edge_id: "e1".into(),
                    node_id: "b".into()
                },
            ]
        );
    }

    #[test]
    fn unknown_socket_on_source_and_target() {
        let g = graph(
            vec![node("b", NodeKind::Bundle), node("p", NodeKind::PickPdf)],
            vec![edge("e1", "b", "nope", "p", "nada")],
        );
        assert_eq!(
            validate_structure(&g),
            vec![
                ValidationIssue::UnknownSocket {
                    edge_id: "e1".into(),
                    node_id: "b".into(),
                    socket_id: "nope".into()
                },
                ValidationIssue::UnknownSocket {
                    edge_id: "e1".into(),
                    node_id: "p".into(),
                    socket_id: "nada".into()
                },
            ]
        );
    }

    #[test]
    fn input_handle_used_as_source_is_unknown_socket() {
        // `extract` has an input `pdf` but no output `pdf`.
        let g = graph(
            vec![node("x", NodeKind::Extract), node("p", NodeKind::PickPdf)],
            vec![edge("e1", "x", "pdf", "p", "files")],
        );
        assert_eq!(
            validate_structure(&g),
            vec![ValidationIssue::UnknownSocket {
                edge_id: "e1".into(),
                node_id: "x".into(),
                socket_id: "pdf".into()
            }]
        );
    }

    #[test]
    fn type_mismatch_has_expected_input_and_actual_output() {
        let g = graph(
            vec![node("b", NodeKind::Bundle), node("x", NodeKind::Extract)],
            vec![edge("e1", "b", "files", "x", "pdf")],
        );
        assert_eq!(
            validate_structure(&g),
            vec![ValidationIssue::TypeMismatch {
                edge_id: "e1".into(),
                expected: DataType::single(ScalarType::Pdf),
                actual: DataType::list_of(ScalarType::Pdf),
            }]
        );
    }

    #[test]
    fn type_mismatch_not_reported_when_a_socket_is_unknown() {
        let g = graph(
            vec![node("b", NodeKind::Bundle), node("x", NodeKind::Extract)],
            vec![edge("e1", "b", "files", "x", "nope")],
        );
        let issues = validate_structure(&g);
        assert_eq!(issues.len(), 1);
        assert!(matches!(issues[0], ValidationIssue::UnknownSocket { .. }));
    }

    #[test]
    fn multiple_incoming_reported_once_per_socket() {
        let g = graph(
            vec![
                node("p1", NodeKind::PickPdf),
                node("p2", NodeKind::PickPdf),
                node("p3", NodeKind::PickPdf),
                node("x", NodeKind::Extract),
            ],
            vec![
                edge("e1", "p1", "pdf", "x", "pdf"),
                edge("e2", "p2", "pdf", "x", "pdf"),
                edge("e3", "p3", "pdf", "x", "pdf"),
            ],
        );
        assert_eq!(
            validate_structure(&g),
            vec![ValidationIssue::MultipleIncoming {
                node_id: "x".into(),
                socket_id: "pdf".into()
            }]
        );
    }

    #[test]
    fn fan_out_from_one_output_is_allowed() {
        let g = graph(
            vec![
                node("p", NodeKind::PickPdf),
                node("x1", NodeKind::Extract),
                node("x2", NodeKind::Extract),
            ],
            vec![
                edge("e1", "p", "pdf", "x1", "pdf"),
                edge("e2", "p", "pdf", "x2", "pdf"),
            ],
        );
        assert!(validate_structure(&g).is_empty());
    }

    #[test]
    fn multiple_issues_are_reported_together_in_order() {
        let g = graph(
            vec![
                node("b", NodeKind::Bundle),
                node("b", NodeKind::Bundle),
                node("p", NodeKind::PickPdf),
                node("x", NodeKind::Extract),
            ],
            vec![
                edge("e1", "b", "files", "x", "pdf"),   // type mismatch
                edge("e2", "p", "pdf", "x", "pdf"),     // second incoming on x.pdf
                edge("e3", "ghost", "pdf", "x", "pdf"), // unknown node, third incoming
            ],
        );
        let codes: Vec<String> = validate_structure(&g)
            .iter()
            .map(|i| {
                serde_json::to_value(i).unwrap()["code"]
                    .as_str()
                    .unwrap()
                    .to_owned()
            })
            .collect();
        assert_eq!(
            codes,
            [
                "DUPLICATE_ID",
                "TYPE_MISMATCH",
                "UNKNOWN_NODE",
                "MULTIPLE_INCOMING"
            ]
        );
    }
}
