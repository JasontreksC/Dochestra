//! `json_input` node: writes the user-supplied JSON value as the `json` output.

use crate::executor::runner::{Artifact, NodeContext, NodeError, NodeOutputs};
use crate::storage::single_output_path;
use crate::types::{DataType, ScalarType};

pub async fn run(ctx: &NodeContext) -> Result<NodeOutputs, NodeError> {
    let value = ctx
        .params
        .get("value")
        .ok_or_else(|| NodeError::Failed("value 가 필요합니다".to_owned()))?;
    let bytes = serde_json::to_vec_pretty(value).map_err(anyhow::Error::from)?;

    let path = single_output_path(&ctx.out_dir, "json", ScalarType::Json);
    tokio::fs::write(&path, bytes).await.map_err(|e| {
        NodeError::Other(anyhow::Error::new(e).context("failed to write json output"))
    })?;

    Ok(NodeOutputs::from([(
        "json".to_owned(),
        Artifact {
            data_type: DataType::single(ScalarType::Json),
            paths: vec![path],
        },
    )]))
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use uuid::Uuid;

    use super::*;
    use crate::types::NodeKind;

    fn ctx(out_dir: std::path::PathBuf, params: serde_json::Value) -> NodeContext {
        NodeContext {
            run_id: Uuid::nil(),
            node_id: "j".to_owned(),
            kind: NodeKind::JsonInput,
            params,
            out_dir,
        }
    }

    #[tokio::test]
    async fn writes_the_value_as_pretty_json() {
        let dir = std::env::temp_dir().join(format!("dochestra-json-node-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let value = json!({"targetLanguage": "ko", "nested": {"list": [1, 2, 3]}, "text": "한글"});

        let outputs = run(&ctx(dir.clone(), json!({"value": value})))
            .await
            .unwrap();
        let artifact = &outputs["json"];
        assert_eq!(artifact.data_type, DataType::single(ScalarType::Json));
        assert_eq!(artifact.paths, vec![dir.join("json.json")]);

        let text = std::fs::read_to_string(&artifact.paths[0]).unwrap();
        assert!(text.contains('\n'), "expected pretty-printed output");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&text).unwrap(),
            value
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn scalar_values_are_allowed_and_missing_value_fails() {
        let dir = std::env::temp_dir().join(format!("dochestra-json-node-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let outputs = run(&ctx(dir.clone(), json!({"value": 42}))).await.unwrap();
        assert_eq!(
            std::fs::read_to_string(&outputs["json"].paths[0])
                .unwrap()
                .trim(),
            "42"
        );

        let err = run(&ctx(dir.clone(), json!({}))).await.unwrap_err();
        assert!(matches!(err, NodeError::Failed(m) if m == "value 가 필요합니다"));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
