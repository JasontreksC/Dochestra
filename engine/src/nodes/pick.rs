//! `pick_pdf` / `pick_jpg` nodes: select one element of a list (no copying).

use crate::executor::runner::{Artifact, NodeContext, NodeError, NodeInputs, NodeOutputs};
use crate::types::{DataType, NodeKind, ScalarType};

pub fn run(ctx: &NodeContext, inputs: &NodeInputs) -> Result<NodeOutputs, NodeError> {
    let (input_socket, output_socket, scalar) = match ctx.kind {
        NodeKind::PickPdf => ("files", "pdf", ScalarType::Pdf),
        NodeKind::PickJpg => ("images", "image", ScalarType::Jpg),
        other => {
            return Err(NodeError::Failed(format!(
                "pick 노드가 아닙니다: {other:?}"
            )));
        }
    };

    let list = inputs
        .get(input_socket)
        .ok_or_else(|| NodeError::Failed(format!("입력 {input_socket} 이(가) 없습니다")))?;
    let index = ctx
        .params
        .get("index")
        .and_then(serde_json::Value::as_u64)
        .and_then(|i| usize::try_from(i).ok())
        .ok_or_else(|| NodeError::Failed("index 는 0 이상의 정수여야 합니다".to_owned()))?;
    let path = list.paths.get(index).ok_or_else(|| {
        NodeError::Failed(format!(
            "index {index} 가 범위를 벗어났습니다 (항목 {}개)",
            list.paths.len()
        ))
    })?;

    Ok(NodeOutputs::from([(
        output_socket.to_owned(),
        Artifact {
            data_type: DataType::single(scalar),
            paths: vec![path.clone()],
        },
    )]))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use serde_json::json;
    use uuid::Uuid;

    use super::*;

    fn ctx(kind: NodeKind, params: serde_json::Value) -> NodeContext {
        NodeContext {
            run_id: Uuid::nil(),
            node_id: "p".to_owned(),
            kind,
            params,
            out_dir: PathBuf::from("/unused"),
        }
    }

    fn list_input(socket: &str, scalar: ScalarType, paths: &[&str]) -> NodeInputs {
        NodeInputs::from([(
            socket.to_owned(),
            Artifact {
                data_type: DataType::list_of(scalar),
                paths: paths.iter().map(PathBuf::from).collect(),
            },
        )])
    }

    fn failure(result: Result<NodeOutputs, NodeError>) -> String {
        match result {
            Err(NodeError::Failed(message)) => message,
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[test]
    fn picks_the_indexed_pdf() {
        let inputs = list_input(
            "files",
            ScalarType::Pdf,
            &["/d/a.pdf", "/d/b.pdf", "/d/c.pdf"],
        );
        let out = run(&ctx(NodeKind::PickPdf, json!({"index": 1})), &inputs).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out["pdf"].data_type, DataType::single(ScalarType::Pdf));
        assert_eq!(out["pdf"].paths, vec![PathBuf::from("/d/b.pdf")]);
    }

    #[test]
    fn picks_the_indexed_jpg() {
        let inputs = list_input("images", ScalarType::Jpg, &["/d/0.jpg", "/d/1.jpg"]);
        let out = run(&ctx(NodeKind::PickJpg, json!({"index": 0})), &inputs).unwrap();
        assert_eq!(out["image"].data_type, DataType::single(ScalarType::Jpg));
        assert_eq!(out["image"].paths, vec![PathBuf::from("/d/0.jpg")]);
    }

    #[test]
    fn last_valid_index_works_and_next_is_out_of_range() {
        let inputs = list_input("files", ScalarType::Pdf, &["/d/a.pdf", "/d/b.pdf"]);
        assert!(run(&ctx(NodeKind::PickPdf, json!({"index": 1})), &inputs).is_ok());
        let message = failure(run(&ctx(NodeKind::PickPdf, json!({"index": 2})), &inputs));
        assert_eq!(message, "index 2 가 범위를 벗어났습니다 (항목 2개)");
    }

    #[test]
    fn empty_list_is_out_of_range() {
        let inputs = list_input("images", ScalarType::Jpg, &[]);
        let message = failure(run(&ctx(NodeKind::PickJpg, json!({"index": 0})), &inputs));
        assert_eq!(message, "index 0 가 범위를 벗어났습니다 (항목 0개)");
    }

    #[test]
    fn bad_index_and_missing_input_fail() {
        let inputs = list_input("files", ScalarType::Pdf, &["/d/a.pdf"]);
        for params in [
            json!({}),
            json!({"index": -1}),
            json!({"index": "0"}),
            json!({"index": 0.5}),
        ] {
            assert!(failure(run(&ctx(NodeKind::PickPdf, params), &inputs)).contains("index"));
        }
        let message = failure(run(
            &ctx(NodeKind::PickPdf, json!({"index": 0})),
            &NodeInputs::new(),
        ));
        assert!(message.contains("files"));
    }
}
