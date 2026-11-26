use std::path::PathBuf;
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use dochestra_engine::build_app;
use dochestra_engine::config::Config;
use dochestra_engine::executor::runner::UnimplementedRunner;
use dochestra_engine::state::AppState;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

const BOUNDARY: &str = "dochestra-test-boundary";

struct Harness {
    app: Router,
    pool: PgPool,
    data: PathBuf,
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.data);
    }
}

fn harness(pool: PgPool, upload_limit: Option<usize>) -> Harness {
    let data = std::env::temp_dir().join(format!("dochestra-bundles-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&data).unwrap();
    let dir = data.to_string_lossy().into_owned();
    let config = Config::from_lookup(move |k| match k {
        "DATABASE_URL" => Some("unused".to_owned()),
        "DATA_DIR" => Some(dir.clone()),
        "UPLOAD_LIMIT_BYTES" => upload_limit.map(|n| n.to_string()),
        _ => None,
    })
    .unwrap();
    let app = build_app(AppState::new(
        pool.clone(),
        config,
        reqwest::Client::new(),
        Arc::new(UnimplementedRunner),
    ));
    Harness { app, pool, data }
}

async fn send_json(
    app: &Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder().method(method).uri(uri);
    let body = match body {
        Some(v) => {
            builder = builder.header("content-type", "application/json");
            Body::from(v.to_string())
        }
        None => Body::empty(),
    };
    finish(app, builder.body(body).unwrap()).await
}

async fn finish(app: &Router, req: Request<Body>) -> (StatusCode, Value) {
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let json = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or_else(|_| panic!("non-JSON body: {bytes:?}"))
    };
    (status, json)
}

/// (field name, file name, content)
type Part<'a> = (&'a str, Option<&'a str>, &'a [u8]);

fn multipart_body(parts: &[Part<'_>]) -> Vec<u8> {
    let mut body = Vec::new();
    for (field, filename, content) in parts {
        body.extend_from_slice(format!("--{BOUNDARY}\r\n").as_bytes());
        match filename {
            Some(name) => body.extend_from_slice(
                format!(
                    "Content-Disposition: form-data; name=\"{field}\"; filename=\"{name}\"\r\n"
                )
                .as_bytes(),
            ),
            None => body.extend_from_slice(
                format!("Content-Disposition: form-data; name=\"{field}\"\r\n").as_bytes(),
            ),
        }
        body.extend_from_slice(b"Content-Type: application/octet-stream\r\n\r\n");
        body.extend_from_slice(content);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{BOUNDARY}--\r\n").as_bytes());
    body
}

async fn upload(app: &Router, bundle_id: &str, parts: &[Part<'_>]) -> (StatusCode, Value) {
    let req = Request::post(format!("/api/bundles/{bundle_id}/files"))
        .header(
            "content-type",
            format!("multipart/form-data; boundary={BOUNDARY}"),
        )
        .body(Body::from(multipart_body(parts)))
        .unwrap();
    finish(app, req).await
}

async fn create_bundle(app: &Router, name: &str) -> String {
    let (status, body) = send_json(app, "POST", "/api/bundles", Some(json!({"name": name}))).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    body["id"].as_str().unwrap().to_owned()
}

fn pdf(tag: &str) -> Vec<u8> {
    format!("%PDF-1.7\n{tag}\n%%EOF\n").into_bytes()
}

/// Files currently stored for a bundle on disk (empty if the directory does not exist).
fn disk_files(h: &Harness, bundle_id: &str) -> Vec<String> {
    let dir = h.data.join("bundles").join(bundle_id);
    match std::fs::read_dir(dir) {
        Ok(entries) => entries
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect(),
        Err(_) => vec![],
    }
}

async fn db_file_count(h: &Harness) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM bundle_files")
        .fetch_one(&h.pool)
        .await
        .unwrap()
}

fn assert_error(body: &Value, code: &str) {
    assert_eq!(body["error"]["code"], code, "body: {body}");
    assert!(body["error"]["message"].is_string());
}

#[sqlx::test(migrations = "./migrations")]
async fn bundle_crud_and_listing(pool: PgPool) {
    let h = harness(pool, None);
    let (status, created) = send_json(
        &h.app,
        "POST",
        "/api/bundles",
        Some(json!({"name": "  docs "})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(created["name"], "docs");
    assert_eq!(created["files"], json!([]));
    let id = created["id"].as_str().unwrap().to_owned();

    let second = create_bundle(&h.app, "newer").await;
    let (_, list) = send_json(&h.app, "GET", "/api/bundles", None).await;
    let ids: Vec<&str> = list
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, [second.as_str(), id.as_str()]); // newest first
    assert_eq!(list[1]["fileCount"], 0);

    let (status, fetched) = send_json(&h.app, "GET", &format!("/api/bundles/{id}"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(fetched["name"], "docs");

    for bad in ["", "   "] {
        let (status, err) =
            send_json(&h.app, "POST", "/api/bundles", Some(json!({"name": bad}))).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_error(&err, "BAD_REQUEST");
    }
    let (status, err) = send_json(
        &h.app,
        "GET",
        &format!("/api/bundles/{}", Uuid::new_v4()),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_error(&err, "NOT_FOUND");
    let (status, err) = send_json(&h.app, "GET", "/api/bundles/nope", None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_error(&err, "BAD_REQUEST");
}

#[sqlx::test(migrations = "./migrations")]
async fn uploads_store_files_with_positions(pool: PgPool) {
    let h = harness(pool, None);
    let id = create_bundle(&h.app, "b").await;
    let (a, b) = (pdf("alpha"), pdf("beta"));

    let (status, files) = upload(
        &h.app,
        &id,
        &[("files", Some("a.pdf"), &a), ("files", Some("B.PDF"), &b)],
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{files}");
    let files = files.as_array().unwrap();
    assert_eq!(files.len(), 2);
    assert_eq!(files[0]["originalName"], "a.pdf");
    assert_eq!(files[0]["position"], 0);
    assert_eq!(files[0]["sizeBytes"], a.len());
    assert_eq!(files[1]["originalName"], "B.PDF");
    assert_eq!(files[1]["position"], 1);

    // Stored as {data}/bundles/{bundle}/{file}.pdf with identical content, no leftovers.
    let on_disk = disk_files(&h, &id);
    assert_eq!(on_disk.len(), 2, "{on_disk:?}");
    for (meta, content) in files.iter().zip([&a, &b]) {
        let path = h
            .data
            .join("bundles")
            .join(&id)
            .join(format!("{}.pdf", meta["id"].as_str().unwrap()));
        assert_eq!(std::fs::read(path).unwrap(), *content);
    }

    // A later upload continues after the last position; other fields are ignored.
    let c = pdf("gamma");
    let (status, more) = upload(
        &h.app,
        &id,
        &[("note", None, b"hello"), ("files", Some("c.pdf"), &c)],
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(more[0]["position"], 2);

    let (_, bundle) = send_json(&h.app, "GET", &format!("/api/bundles/{id}"), None).await;
    let names: Vec<&str> = bundle["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["originalName"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["a.pdf", "B.PDF", "c.pdf"]);
    let (_, list) = send_json(&h.app, "GET", "/api/bundles", None).await;
    assert_eq!(list[0]["fileCount"], 3);
}

#[sqlx::test(migrations = "./migrations")]
async fn invalid_files_reject_the_whole_request_without_leftovers(pool: PgPool) {
    let h = harness(pool, None);
    let id = create_bundle(&h.app, "b").await;
    let good = pdf("ok");

    // Wrong magic bytes (valid extension) after a valid file.
    let (status, err) = upload(
        &h.app,
        &id,
        &[
            ("files", Some("good.pdf"), &good),
            ("files", Some("fake.pdf"), b"hello world"),
        ],
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_error(&err, "BAD_REQUEST");
    assert!(
        err["error"]["message"]
            .as_str()
            .unwrap()
            .contains("fake.pdf")
    );
    assert!(disk_files(&h, &id).is_empty(), "{:?}", disk_files(&h, &id));
    assert_eq!(db_file_count(&h).await, 0);

    // Wrong extension (valid PDF bytes).
    let (status, err) = upload(
        &h.app,
        &id,
        &[
            ("files", Some("good.pdf"), &good),
            ("files", Some("doc.txt"), &good),
        ],
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        err["error"]["message"]
            .as_str()
            .unwrap()
            .contains("doc.txt")
    );
    assert!(disk_files(&h, &id).is_empty());
    assert_eq!(db_file_count(&h).await, 0);

    // Shorter than the magic number.
    let (status, _) = upload(&h.app, &id, &[("files", Some("tiny.pdf"), b"%PD")]).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(disk_files(&h, &id).is_empty());
}

#[sqlx::test(migrations = "./migrations")]
async fn malformed_upload_requests_are_400(pool: PgPool) {
    let h = harness(pool, None);
    let id = create_bundle(&h.app, "b").await;

    // Multipart without any `files` field.
    let (status, err) = upload(&h.app, &id, &[("other", Some("a.pdf"), &pdf("x"))]).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_error(&err, "BAD_REQUEST");

    // Not multipart at all.
    let req = Request::post(format!("/api/bundles/{id}/files"))
        .header("content-type", "application/json")
        .body(Body::from("{}"))
        .unwrap();
    let (status, err) = finish(&h.app, req).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_error(&err, "BAD_REQUEST");
}

#[sqlx::test(migrations = "./migrations")]
async fn upload_to_unknown_bundle_is_404_and_writes_nothing(pool: PgPool) {
    let h = harness(pool, None);
    let ghost = Uuid::new_v4().to_string();
    let (status, err) = upload(&h.app, &ghost, &[("files", Some("a.pdf"), &pdf("x"))]).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_error(&err, "NOT_FOUND");
    assert!(disk_files(&h, &ghost).is_empty());
}

#[sqlx::test(migrations = "./migrations")]
async fn oversized_upload_is_413_without_leftovers(pool: PgPool) {
    let h = harness(pool, Some(1024));
    let id = create_bundle(&h.app, "b").await;
    let mut big = pdf("big");
    big.extend(std::iter::repeat_n(b'x', 8 * 1024));

    let (status, err) = upload(&h.app, &id, &[("files", Some("big.pdf"), &big)]).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_error(&err, "PAYLOAD_TOO_LARGE");
    assert!(disk_files(&h, &id).is_empty(), "{:?}", disk_files(&h, &id));
    assert_eq!(db_file_count(&h).await, 0);

    // Small uploads still work with the same limit.
    let (status, _) = upload(&h.app, &id, &[("files", Some("s.pdf"), &pdf("s"))]).await;
    assert_eq!(status, StatusCode::CREATED);
}

#[sqlx::test(migrations = "./migrations")]
async fn deleting_a_file_removes_it_from_disk(pool: PgPool) {
    let h = harness(pool, None);
    let id = create_bundle(&h.app, "b").await;
    let (_, files) = upload(
        &h.app,
        &id,
        &[
            ("files", Some("a.pdf"), &pdf("a")),
            ("files", Some("b.pdf"), &pdf("b")),
        ],
    )
    .await;
    let first = files[0]["id"].as_str().unwrap().to_owned();
    let second = files[1]["id"].as_str().unwrap().to_owned();

    let (status, _) = send_json(
        &h.app,
        "DELETE",
        &format!("/api/bundles/{id}/files/{first}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(disk_files(&h, &id), [format!("{second}.pdf")]);
    let (_, bundle) = send_json(&h.app, "GET", &format!("/api/bundles/{id}"), None).await;
    assert_eq!(bundle["files"].as_array().unwrap().len(), 1);
    assert_eq!(bundle["files"][0]["id"], second.as_str());

    let (status, err) = send_json(
        &h.app,
        "DELETE",
        &format!("/api/bundles/{id}/files/{first}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_error(&err, "NOT_FOUND");
    // A file id of another bundle does not match.
    let other = create_bundle(&h.app, "other").await;
    let (status, _) = send_json(
        &h.app,
        "DELETE",
        &format!("/api/bundles/{other}/files/{second}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(disk_files(&h, &id).len(), 1);
}

#[sqlx::test(migrations = "./migrations")]
async fn deleting_a_bundle_removes_its_directory(pool: PgPool) {
    let h = harness(pool, None);
    let id = create_bundle(&h.app, "b").await;
    upload(&h.app, &id, &[("files", Some("a.pdf"), &pdf("a"))]).await;
    assert!(h.data.join("bundles").join(&id).is_dir());

    let (status, _) = send_json(&h.app, "DELETE", &format!("/api/bundles/{id}"), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(!h.data.join("bundles").join(&id).exists());
    assert_eq!(db_file_count(&h).await, 0); // cascade
    let (status, _) = send_json(&h.app, "GET", &format!("/api/bundles/{id}"), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = send_json(&h.app, "DELETE", &format!("/api/bundles/{id}"), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrations = "./migrations")]
async fn path_components_in_file_names_are_stripped(pool: PgPool) {
    let h = harness(pool, None);
    let id = create_bundle(&h.app, "b").await;
    let (status, files) = upload(
        &h.app,
        &id,
        &[
            ("files", Some("../../x.pdf"), &pdf("x")),
            ("files", Some("C:\\temp\\y.pdf"), &pdf("y")),
        ],
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{files}");
    assert_eq!(files[0]["originalName"], "x.pdf");
    assert_eq!(files[1]["originalName"], "y.pdf");
    // Everything lives inside the bundle directory; nothing escaped to the data root.
    assert_eq!(disk_files(&h, &id).len(), 2);
    assert!(!h.data.join("x.pdf").exists());
    assert!(!h.data.parent().unwrap().join("x.pdf").exists());
}
