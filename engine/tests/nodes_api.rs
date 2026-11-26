use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use dochestra_engine::build_app;
use dochestra_engine::config::Config;
use dochestra_engine::executor::runner::UnimplementedRunner;
use dochestra_engine::state::AppState;
use http_body_util::BodyExt;
use sqlx::postgres::PgPoolOptions;
use tower::ServiceExt;

#[tokio::test]
async fn nodes_returns_catalog() {
    let config = Config::from_lookup(|k| {
        (k == "DATABASE_URL").then(|| "postgres://u:p@127.0.0.1:1/none".to_owned())
    })
    .unwrap();
    let db = PgPoolOptions::new()
        .connect_lazy(&config.database_url)
        .unwrap();
    let state = AppState::new(
        db,
        config,
        reqwest::Client::new(),
        Arc::new(UnimplementedRunner),
    );

    let resp = build_app(state)
        .oneshot(Request::get("/api/nodes").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let nodes = body.as_array().unwrap();
    assert_eq!(nodes.len(), 12);
    assert_eq!(nodes[0]["kind"], "bundle");
    assert_eq!(
        nodes[0]["outputs"][0]["dataType"],
        serde_json::json!({"scalar": "PDF", "list": true})
    );
}
