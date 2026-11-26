//! Shared domain/API types exported to TypeScript via ts-rs (CONTEXT §3).

pub mod api;
pub mod bundle;
pub mod data_type;
pub mod graph;
pub mod node;
pub mod run;

pub use api::*;
pub use bundle::*;
pub use data_type::*;
pub use graph::*;
pub use node::*;
pub use run::*;

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn data_type_serialization_and_display() {
        let t = DataType::list_of(ScalarType::Jpg);
        assert_eq!(
            serde_json::to_value(t).unwrap(),
            json!({"scalar": "JPG", "list": true})
        );
        assert_eq!(t.to_string(), "LIST[JPG]");
        assert_eq!(DataType::single(ScalarType::Mp3).to_string(), "MP3");
        assert_eq!(ScalarType::Csv.extension(), "csv");
        let back: DataType =
            serde_json::from_value(json!({"scalar": "PDF", "list": false})).unwrap();
        assert_eq!(back, DataType::single(ScalarType::Pdf));
    }

    #[test]
    fn node_kind_is_snake_case() {
        assert_eq!(
            serde_json::to_value(NodeKind::PickPdf).unwrap(),
            json!("pick_pdf")
        );
        assert_eq!(
            serde_json::to_value(NodeKind::JsonInput).unwrap(),
            json!("json_input")
        );
    }

    #[test]
    fn validation_issue_shapes() {
        let issue = ValidationIssue::TypeMismatch {
            edge_id: "e1".into(),
            expected: DataType::single(ScalarType::Md),
            actual: DataType::list_of(ScalarType::Jpg),
        };
        assert_eq!(
            serde_json::to_value(&issue).unwrap(),
            json!({
                "code": "TYPE_MISMATCH",
                "edgeId": "e1",
                "expected": {"scalar": "MD", "list": false},
                "actual": {"scalar": "JPG", "list": true}
            })
        );
        assert_eq!(
            serde_json::to_value(ValidationIssue::NoOutputNode).unwrap(),
            json!({"code": "NO_OUTPUT_NODE"})
        );
        assert_eq!(
            serde_json::to_value(ValidationIssue::CycleDetected {
                node_ids: vec!["a".into()]
            })
            .unwrap(),
            json!({"code": "CYCLE_DETECTED", "nodeIds": ["a"]})
        );
        let back: ValidationIssue = serde_json::from_value(
            json!({"code": "UNKNOWN_SOCKET", "edgeId": "e", "nodeId": "n", "socketId": "s"}),
        )
        .unwrap();
        assert_eq!(
            back,
            ValidationIssue::UnknownSocket {
                edge_id: "e".into(),
                node_id: "n".into(),
                socket_id: "s".into()
            }
        );
    }

    #[test]
    fn run_event_node_status_shape() {
        let ev = RunEvent::NodeStatus {
            node_id: "n1".into(),
            status: NodeStatus::Running,
            error: None,
            outputs: vec![],
        };
        assert_eq!(
            serde_json::to_value(&ev).unwrap(),
            json!({"type": "node_status", "nodeId": "n1", "status": "running", "error": null, "outputs": []})
        );
        assert_eq!(
            serde_json::to_value(RunEvent::RunFinished {
                status: RunStatus::Failed
            })
            .unwrap(),
            json!({"type": "run_finished", "status": "failed"})
        );
    }

    #[test]
    fn graph_node_defaults_on_deserialize() {
        let n: GraphNode = serde_json::from_value(json!({
            "id": "a", "kind": "extract", "position": {"x": 1.0, "y": 2.0}
        }))
        .unwrap();
        assert_eq!(n.params, json!({}));
        assert!(!n.is_output);
    }

    #[test]
    fn update_workflow_request_accepts_empty_object() {
        let r: UpdateWorkflowRequest = serde_json::from_value(json!({})).unwrap();
        assert!(r.name.is_none() && r.graph.is_none());
        assert_eq!(serde_json::to_value(&r).unwrap(), json!({}));
    }

    #[test]
    fn api_error_body_omits_absent_issues() {
        let body = ApiErrorBody {
            error: ApiErrorDetail {
                code: "NOT_FOUND".into(),
                message: "x".into(),
                issues: None,
            },
        };
        assert_eq!(
            serde_json::to_value(&body).unwrap(),
            json!({"error": {"code": "NOT_FOUND", "message": "x"}})
        );
    }
}
