use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Body;
use axum::extract::State;
use axum::http::{Request, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use dochestra_engine::build_app;
use dochestra_engine::clients::llm::LlmClient;
use dochestra_engine::config::Config;
use dochestra_engine::executor::runner::{Artifact, NodeContext, NodeError, NodeInputs};
use dochestra_engine::nodes::EngineRunner;
use dochestra_engine::nodes::fill;
use dochestra_engine::nodes::fill::form::{FieldKind, FormField, discover};
use dochestra_engine::state::AppState;
use dochestra_engine::types::{DataType, NodeKind, ScalarType};
use http_body_util::BodyExt;
use lopdf::{Document, Object, Stream, StringFormat, dictionary};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

const KEY: &str = "sk-fill-secret";

// ------------------------------------------------------------------ form PDF builder

fn name(n: &str) -> Object {
    Object::Name(n.as_bytes().to_vec())
}

fn text(s: &str) -> Object {
    Object::String(s.as_bytes().to_vec(), StringFormat::Literal)
}

struct Builder {
    doc: Document,
    page: lopdf::ObjectId,
    annots: Vec<Object>,
    fields: Vec<Object>,
    y: i64,
}

impl Builder {
    fn new() -> Self {
        let mut doc = Document::with_version("1.7");
        let page = doc.new_object_id();
        Self {
            doc,
            page,
            annots: vec![],
            fields: vec![],
            y: 700,
        }
    }

    fn rect(&mut self) -> Object {
        self.y -= 30;
        Object::Array(vec![
            50.into(),
            self.y.into(),
            250.into(),
            (self.y + 20).into(),
        ])
    }

    fn widget_base(&mut self) -> lopdf::Dictionary {
        let rect = self.rect();
        dictionary! {
            "Type" => "Annot",
            "Subtype" => "Widget",
            "Rect" => rect,
            "F" => 4,
            "P" => Object::Reference(self.page),
        }
    }

    fn register(&mut self, dict: lopdf::Dictionary, top_level: bool) -> lopdf::ObjectId {
        let id = self.doc.add_object(dict);
        self.annots.push(Object::Reference(id));
        if top_level {
            self.fields.push(Object::Reference(id));
        }
        id
    }

    fn appearance(&mut self, states: &[&str]) -> Object {
        let mut normal = lopdf::Dictionary::new();
        for state in states {
            let stream = Stream::new(
                dictionary! {"Type" => "XObject", "Subtype" => "Form", "BBox" => vec![0.into(), 0.into(), 10.into(), 10.into()]},
                Vec::new(),
            );
            normal.set(*state, Object::Reference(self.doc.add_object(stream)));
        }
        Object::Dictionary(dictionary! {"N" => Object::Dictionary(normal)})
    }

    /// Merged field/widget text field.
    fn text_field(&mut self, field_name: &str, value: Option<&str>) {
        let mut dict = self.widget_base();
        dict.set("FT", name("Tx"));
        dict.set("T", text(field_name));
        if let Some(v) = value {
            dict.set("V", text(v));
        }
        self.register(dict, true);
    }

    fn checkbox(&mut self, field_name: &str, on_state: &str) {
        let mut dict = self.widget_base();
        dict.set("FT", name("Btn"));
        dict.set("T", text(field_name));
        dict.set("V", name("Off"));
        dict.set("AS", name("Off"));
        let ap = self.appearance(&[on_state, "Off"]);
        dict.set("AP", ap);
        self.register(dict, true);
    }

    fn choice(&mut self, field_name: &str, options: Vec<Object>) {
        let mut dict = self.widget_base();
        dict.set("FT", name("Ch"));
        dict.set("Ff", Object::Integer(1 << 17)); // combo
        dict.set("T", text(field_name));
        dict.set("Opt", Object::Array(options));
        self.register(dict, true);
    }

    /// Parent `Tx` field whose kids are fields without their own `FT` (inherited).
    fn group_with_inherited_type(&mut self, parent: &str, kids: &[&str]) {
        let parent_id = self.doc.new_object_id();
        let mut kid_refs = Vec::new();
        for kid in kids {
            let mut dict = self.widget_base();
            dict.set("T", text(kid));
            dict.set("Parent", Object::Reference(parent_id));
            kid_refs.push(Object::Reference(self.register(dict, false)));
        }
        self.doc.objects.insert(
            parent_id,
            Object::Dictionary(
                dictionary! {"FT" => "Tx", "T" => text(parent), "Kids" => Object::Array(kid_refs)},
            ),
        );
        self.fields.push(Object::Reference(parent_id));
    }

    /// Radio group: widgets without `T` under a parent that carries the name and flags.
    fn radio(&mut self, group: &str, options: &[&str]) {
        let parent_id = self.doc.new_object_id();
        let mut kid_refs = Vec::new();
        for option in options {
            let mut dict = self.widget_base();
            dict.set("Parent", Object::Reference(parent_id));
            dict.set("AS", name("Off"));
            let ap = self.appearance(&[option, "Off"]);
            dict.set("AP", ap);
            kid_refs.push(Object::Reference(self.register(dict, false)));
        }
        self.doc.objects.insert(
            parent_id,
            Object::Dictionary(dictionary! {
                "FT" => "Btn",
                "Ff" => Object::Integer(1 << 15),
                "T" => text(group),
                "Kids" => Object::Array(kid_refs)
            }),
        );
        self.fields.push(Object::Reference(parent_id));
    }

    fn signature_and_button(&mut self) {
        let mut sig = self.widget_base();
        sig.set("FT", name("Sig"));
        sig.set("T", text("signature"));
        self.register(sig, true);
        let mut button = self.widget_base();
        button.set("FT", name("Btn"));
        button.set("Ff", Object::Integer(1 << 16));
        button.set("T", text("submit"));
        self.register(button, true);
    }

    fn finish(mut self, with_acroform: bool, indirect_acroform: bool) -> Vec<u8> {
        let pages_id = self.doc.new_object_id();
        self.doc.objects.insert(
            self.page,
            Object::Dictionary(dictionary! {
                "Type" => "Page",
                "Parent" => Object::Reference(pages_id),
                "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
                "Resources" => Object::Dictionary(lopdf::Dictionary::new()),
                "Annots" => Object::Array(self.annots.clone())
            }),
        );
        self.doc.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {"Type" => "Pages", "Kids" => vec![Object::Reference(self.page)], "Count" => 1}),
        );
        let mut catalog = dictionary! {"Type" => "Catalog", "Pages" => Object::Reference(pages_id)};
        if with_acroform {
            let acro = dictionary! {"Fields" => Object::Array(self.fields.clone())};
            if indirect_acroform {
                let id = self.doc.add_object(acro);
                catalog.set("AcroForm", Object::Reference(id));
            } else {
                catalog.set("AcroForm", Object::Dictionary(acro));
            }
        }
        let catalog_id = self.doc.add_object(catalog);
        self.doc.trailer.set("Root", Object::Reference(catalog_id));
        let mut bytes = Vec::new();
        self.doc.save_to(&mut bytes).unwrap();
        bytes
    }
}

fn sample_form(indirect_acroform: bool) -> Vec<u8> {
    let mut b = Builder::new();
    b.text_field("name", Some("Old"));
    b.text_field("korean_name", None);
    b.group_with_inherited_type("applicant", &["first_name", "last_name"]);
    b.checkbox("agree", "Yes");
    b.checkbox("newsletter", "Accepted");
    b.radio("gender", &["Male", "Female"]);
    b.choice("country", vec![text("KR"), text("US"), text("JP")]);
    b.choice(
        "city",
        vec![
            Object::Array(vec![
                text("sel"),
                Object::String(
                    "서울".encode_utf16().flat_map(|u| u.to_be_bytes()).fold(
                        vec![0xFE, 0xFF],
                        |mut v, x| {
                            v.push(x);
                            v
                        },
                    ),
                    StringFormat::Hexadecimal,
                ),
            ]),
            Object::Array(vec![
                text("bus"),
                Object::String(
                    "부산".encode_utf16().flat_map(|u| u.to_be_bytes()).fold(
                        vec![0xFE, 0xFF],
                        |mut v, x| {
                            v.push(x);
                            v
                        },
                    ),
                    StringFormat::Hexadecimal,
                ),
            ]),
        ],
    );
    b.signature_and_button();
    b.finish(true, indirect_acroform)
}

fn many_field_form(count: usize) -> Vec<u8> {
    let mut b = Builder::new();
    for i in 0..count {
        b.text_field(&format!("field_{i:02}"), None);
    }
    b.finish(true, false)
}

// ------------------------------------------------------------------ inspection helpers

fn load(bytes: &[u8]) -> Document {
    Document::load_mem(bytes).expect("valid PDF")
}

fn fields_of(bytes: &[u8]) -> HashMap<String, FormField> {
    discover(&load(bytes))
        .into_iter()
        .map(|f| (f.name.clone(), f))
        .collect()
}

fn need_appearances(bytes: &[u8]) -> bool {
    let doc = load(bytes);
    let acro = doc.catalog().unwrap().get(b"AcroForm").unwrap();
    let dict = doc.dereference(acro).unwrap().1.as_dict().unwrap();
    dict.get(b"NeedAppearances")
        .is_ok_and(|v| v.as_bool().unwrap_or(false))
}

/// `/V` object of a field, as the raw PDF object.
fn raw_value(bytes: &[u8], field: &str) -> Object {
    let doc = load(bytes);
    let f = &fields_of(bytes)[field];
    doc.get_dictionary(f.id).unwrap().get(b"V").unwrap().clone()
}

/// `/AS` state names of a field's widgets.
fn appearance_states(bytes: &[u8], field: &str) -> Vec<String> {
    let doc = load(bytes);
    let f = &fields_of(bytes)[field];
    let ids = if f.widgets.is_empty() {
        vec![f.id]
    } else {
        f.widgets.clone()
    };
    ids.iter()
        .map(|id| {
            String::from_utf8_lossy(
                doc.get_dictionary(*id)
                    .unwrap()
                    .get(b"AS")
                    .unwrap()
                    .as_name()
                    .unwrap(),
            )
            .into_owned()
        })
        .collect()
}

// ------------------------------------------------------------------ mock Anthropic

#[derive(Clone)]
struct Reply {
    status: u16,
    body: Value,
}

fn answer(text: &str) -> Reply {
    Reply {
        status: 200,
        body: json!({"choices": [{"message": {"role": "assistant", "content": text}, "finish_reason": "stop"}]}),
    }
}

#[derive(Clone)]
struct Shared {
    replies: Arc<Mutex<VecDeque<Reply>>>,
    requests: Arc<Mutex<Vec<Value>>>,
}

struct MockLlm {
    url: String,
    shared: Shared,
}

impl MockLlm {
    fn count(&self) -> usize {
        self.shared.requests.lock().unwrap().len()
    }
    fn prompt(&self, i: usize) -> String {
        let body = self.shared.requests.lock().unwrap()[i].clone();
        body["messages"][1]["content"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|b| b["type"] == "text")
            .map(|b| b["text"].as_str().unwrap().to_owned())
            .collect()
    }
}

async fn spawn_llm(replies: Vec<Reply>) -> MockLlm {
    let shared = Shared {
        replies: Arc::new(Mutex::new(replies.into())),
        requests: Arc::new(Mutex::new(Vec::new())),
    };
    async fn handler(State(shared): State<Shared>, Json(body): Json<Value>) -> Response {
        shared.requests.lock().unwrap().push(body);
        let reply = shared.replies.lock().unwrap().pop_front().unwrap_or(Reply {
            status: 418,
            body: json!({"error": {"message": "no scripted reply"}}),
        });
        (
            StatusCode::from_u16(reply.status).unwrap(),
            Json(reply.body),
        )
            .into_response()
    }
    let app = Router::new()
        .route("/chat/completions", post(handler))
        .with_state(shared.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    MockLlm { url, shared }
}

fn llm_client(url: &str, key: Option<&str>) -> LlmClient {
    LlmClient::new(
        reqwest::Client::new(),
        url,
        key.map(str::to_owned),
        "meta/muse-ember",
        "medium",
    )
    .with_backoff_base(Duration::from_millis(5))
}

// ------------------------------------------------------------------ node harness

struct Env {
    dir: PathBuf,
    ctx: NodeContext,
}

impl Env {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("dochestra-fill-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let ctx = NodeContext {
            run_id: Uuid::new_v4(),
            node_id: "f".to_owned(),
            kind: NodeKind::Fill,
            params: json!({}),
            out_dir: dir.clone(),
        };
        Self { dir, ctx }
    }

    fn inputs(&self, pdf: &[u8], values: &str) -> NodeInputs {
        let pdf_path = self.dir.join("form.pdf");
        std::fs::write(&pdf_path, pdf).unwrap();
        let values_path = self.dir.join("values.json");
        std::fs::write(&values_path, values).unwrap();
        NodeInputs::from([
            (
                "pdf".to_owned(),
                Artifact {
                    data_type: DataType::single(ScalarType::Pdf),
                    paths: vec![pdf_path],
                },
            ),
            (
                "values".to_owned(),
                Artifact {
                    data_type: DataType::single(ScalarType::Json),
                    paths: vec![values_path],
                },
            ),
        ])
    }

    async fn fill(&self, llm: &LlmClient, pdf: &[u8], values: Value) -> Result<Vec<u8>, NodeError> {
        let inputs = self.inputs(pdf, &values.to_string());
        let out = fill::run(llm, &self.ctx, &inputs).await?;
        assert_eq!(out["pdf"].data_type, DataType::single(ScalarType::Pdf));
        Ok(std::fs::read(&out["pdf"].paths[0]).unwrap())
    }

    fn report(&self) -> Value {
        serde_json::from_slice(&std::fs::read(self.dir.join("_work/fill_report.json")).unwrap())
            .unwrap()
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn failed(result: Result<Vec<u8>, NodeError>) -> String {
    match result {
        Err(NodeError::Failed(message)) => message,
        other => panic!("expected Failed, got {:?}", other.map(|b| b.len())),
    }
}

/// A client that is never expected to be used (no key, unreachable URL).
fn no_llm() -> LlmClient {
    llm_client("http://127.0.0.1:9", None)
}

// ------------------------------------------------------------------ discovery

#[test]
fn discovery_finds_names_types_states_and_options() {
    let fields = fields_of(&sample_form(false));
    let mut names: Vec<&str> = fields.keys().map(String::as_str).collect();
    names.sort_unstable();
    assert_eq!(
        names,
        [
            "agree",
            "applicant.first_name",
            "applicant.last_name",
            "city",
            "country",
            "gender",
            "korean_name",
            "name",
            "newsletter"
        ],
        "signature and push button fields must be excluded"
    );

    // Type inherited from the parent (the kids carry no /FT).
    assert_eq!(fields["applicant.first_name"].kind, FieldKind::Text);
    assert_eq!(fields["applicant.first_name"].last_segment(), "first_name");
    assert_eq!(fields["name"].current.as_deref(), Some("Old"));
    assert_eq!(fields["korean_name"].current, None);

    assert_eq!(
        fields["agree"].kind,
        FieldKind::Checkbox {
            on_state: "Yes".into()
        }
    );
    assert_eq!(
        fields["newsletter"].kind,
        FieldKind::Checkbox {
            on_state: "Accepted".into()
        }
    );
    assert_eq!(fields["agree"].current.as_deref(), Some("Off"));

    let FieldKind::Radio { options } = &fields["gender"].kind else {
        panic!("{:?}", fields["gender"].kind)
    };
    assert_eq!(options, &["Male", "Female"]);
    assert_eq!(fields["gender"].widgets.len(), 2);

    let FieldKind::Choice { options } = &fields["country"].kind else {
        panic!()
    };
    assert_eq!(
        options
            .iter()
            .map(|o| o.export.as_str())
            .collect::<Vec<_>>(),
        ["KR", "US", "JP"]
    );
    let FieldKind::Choice { options } = &fields["city"].kind else {
        panic!()
    };
    assert_eq!(
        (options[0].export.as_str(), options[0].display.as_str()),
        ("sel", "서울")
    );
    assert_eq!(
        (options[1].export.as_str(), options[1].display.as_str()),
        ("bus", "부산")
    );
}

// ------------------------------------------------------------------ filling

#[tokio::test]
async fn exact_and_normalized_keys_fill_text_fields() {
    let env = Env::new();
    let out = env
        .fill(&no_llm(), &sample_form(false), json!({"name": "John", "Korean Name": "홍길동", "firstName": "길동", "Applicant.Last-Name": "홍", "ignored": null}))
        .await
        .unwrap();

    let fields = fields_of(&out);
    assert_eq!(fields["name"].current.as_deref(), Some("John"));
    assert_eq!(fields["korean_name"].current.as_deref(), Some("홍길동"));
    assert_eq!(
        fields["applicant.first_name"].current.as_deref(),
        Some("길동")
    );
    assert_eq!(fields["applicant.last_name"].current.as_deref(), Some("홍"));

    // ASCII is written as a plain string, Korean as UTF-16BE with a BOM.
    assert_eq!(
        raw_value(&out, "name"),
        Object::String(b"John".to_vec(), StringFormat::Literal)
    );
    let Object::String(bytes, _) = raw_value(&out, "korean_name") else {
        panic!()
    };
    assert_eq!(&bytes[..2], &[0xFE, 0xFF]);
    assert_eq!(
        bytes[2..]
            .chunks(2)
            .map(|c| u16::from_be_bytes([c[0], c[1]]))
            .collect::<Vec<_>>(),
        "홍길동".encode_utf16().collect::<Vec<_>>()
    );

    // The report lists every match with its method; null keys are simply not keys.
    let report = env.report();
    let filled: HashMap<String, (String, String)> = report["filled"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            (
                f["key"].as_str().unwrap().to_owned(),
                (
                    f["field"].as_str().unwrap().to_owned(),
                    f["method"].as_str().unwrap().to_owned(),
                ),
            )
        })
        .collect();
    assert_eq!(filled["name"], ("name".to_owned(), "exact".to_owned()));
    assert_eq!(
        filled["Korean Name"],
        ("korean_name".to_owned(), "normalized".to_owned())
    );
    assert_eq!(filled["firstName"].1, "normalized");
    assert_eq!(report["unmatchedKeys"], json!([]));
    let untouched: Vec<&str> = report["untouchedFields"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(
        untouched.contains(&"agree")
            && untouched.contains(&"country")
            && !untouched.contains(&"name")
    );
    assert_eq!(report["filled"].as_array().unwrap().len(), 4);
}

#[tokio::test]
async fn checkboxes_follow_their_on_state_names() {
    let env = Env::new();
    let out = env
        .fill(
            &no_llm(),
            &sample_form(false),
            json!({"agree": true, "newsletter": "예"}),
        )
        .await
        .unwrap();
    assert_eq!(raw_value(&out, "agree"), Object::Name(b"Yes".to_vec()));
    assert_eq!(appearance_states(&out, "agree"), ["Yes"]);
    assert_eq!(
        raw_value(&out, "newsletter"),
        Object::Name(b"Accepted".to_vec())
    );
    assert_eq!(appearance_states(&out, "newsletter"), ["Accepted"]);
    assert_eq!(env.report()["filled"][0]["value"], "on");

    let env = Env::new();
    let out = env
        .fill(
            &no_llm(),
            &sample_form(false),
            json!({"agree": false, "newsletter": 0, "name": "x"}),
        )
        .await
        .unwrap();
    assert_eq!(raw_value(&out, "agree"), Object::Name(b"Off".to_vec()));
    assert_eq!(appearance_states(&out, "agree"), ["Off"]);
    assert_eq!(appearance_states(&out, "newsletter"), ["Off"]);
}

#[tokio::test]
async fn radio_groups_select_the_matching_widget() {
    let env = Env::new();
    let out = env
        .fill(&no_llm(), &sample_form(false), json!({"gender": "female"}))
        .await
        .unwrap();
    assert_eq!(raw_value(&out, "gender"), Object::Name(b"Female".to_vec()));
    assert_eq!(appearance_states(&out, "gender"), ["Off", "Female"]);

    // An unknown option is reported and leaves the group untouched (other keys still apply).
    let env = Env::new();
    let out = env
        .fill(
            &no_llm(),
            &sample_form(false),
            json!({"gender": "other", "name": "x"}),
        )
        .await
        .unwrap();
    assert_eq!(appearance_states(&out, "gender"), ["Off", "Off"]);
    let warnings = env.report()["warnings"].to_string();
    assert!(
        warnings.contains("other") && warnings.contains("Male, Female"),
        "{warnings}"
    );
    assert!(
        env.report()["untouchedFields"]
            .to_string()
            .contains("gender")
    );
}

#[tokio::test]
async fn choice_fields_write_export_values() {
    let env = Env::new();
    let out = env
        .fill(
            &no_llm(),
            &sample_form(false),
            json!({"country": "US", "city": "부산"}),
        )
        .await
        .unwrap();
    assert_eq!(fields_of(&out)["country"].current.as_deref(), Some("US"));
    assert_eq!(fields_of(&out)["city"].current.as_deref(), Some("bus")); // display name -> export value
    assert_eq!(env.report()["warnings"], json!([]));

    // A value outside the options is written as-is, with a warning.
    let env = Env::new();
    let out = env
        .fill(&no_llm(), &sample_form(false), json!({"country": "FR"}))
        .await
        .unwrap();
    assert_eq!(fields_of(&out)["country"].current.as_deref(), Some("FR"));
    assert!(
        env.report()["warnings"]
            .to_string()
            .contains("선택지에 없는 값")
    );
}

#[tokio::test]
async fn need_appearances_is_set_for_direct_and_indirect_acroforms() {
    for indirect in [false, true] {
        let env = Env::new();
        let form = sample_form(indirect);
        assert!(!need_appearances_or_false(&form));
        let out = env
            .fill(&no_llm(), &form, json!({"name": "x"}))
            .await
            .unwrap();
        assert!(need_appearances(&out), "indirect = {indirect}");
    }
}

fn need_appearances_or_false(bytes: &[u8]) -> bool {
    let doc = load(bytes);
    let acro = doc.catalog().unwrap().get(b"AcroForm").unwrap();
    doc.dereference(acro)
        .unwrap()
        .1
        .as_dict()
        .unwrap()
        .get(b"NeedAppearances")
        .is_ok()
}

#[tokio::test]
async fn numbers_and_unsupported_values() {
    let env = Env::new();
    let out = env
        .fill(
            &no_llm(),
            &sample_form(false),
            json!({"name": 42, "korean_name": 1.5, "agree": ["x"], "country": {"a": 1}}),
        )
        .await
        .unwrap();
    assert_eq!(fields_of(&out)["name"].current.as_deref(), Some("42"));
    assert_eq!(
        fields_of(&out)["korean_name"].current.as_deref(),
        Some("1.5")
    );
    let warnings = env.report()["warnings"].to_string();
    assert!(
        warnings.contains("'agree'") && warnings.contains("'country'"),
        "{warnings}"
    );
    assert_eq!(env.report()["filled"].as_array().unwrap().len(), 2);
}

// ------------------------------------------------------------------ LLM key mapping

#[tokio::test]
async fn the_llm_maps_leftover_keys_without_seeing_their_values() {
    let env = Env::new();
    let mock = spawn_llm(vec![answer("```json\n{\"성명\": \"korean_name\", \"동의\": \"agree\", \"모르는키\": null, \"나이\": \"no_such_field\"}\n```")]).await;
    let out = env
        .fill(&llm_client(&mock.url, Some(KEY)), &sample_form(false), json!({"name": "Exact", "성명": "김철수-비밀값", "동의": true, "모르는키": "x", "나이": 30}))
        .await
        .unwrap();

    let fields = fields_of(&out);
    assert_eq!(
        fields["korean_name"].current.as_deref(),
        Some("김철수-비밀값")
    );
    assert_eq!(fields["agree"].current.as_deref(), Some("Yes"));
    assert_eq!(fields["name"].current.as_deref(), Some("Exact"));

    let report = env.report();
    let methods: HashMap<&str, &str> = report["filled"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| (f["key"].as_str().unwrap(), f["method"].as_str().unwrap()))
        .collect();
    assert_eq!(methods["name"], "exact");
    assert_eq!(methods["성명"], "llm");
    assert_eq!(methods["동의"], "llm");
    assert_eq!(report["unmatchedKeys"], json!(["나이", "모르는키"]));
    assert!(
        report["warnings"].to_string().contains("no_such_field"),
        "an invented field name is ignored with a warning"
    );

    // One request; it names the remaining keys (with types) and fields, never the values.
    assert_eq!(mock.count(), 1);
    let prompt = mock.prompt(0);
    assert!(
        prompt.contains("- 성명 (string)")
            && prompt.contains("- 동의 (boolean)")
            && prompt.contains("- 나이 (number)")
    );
    assert!(prompt.contains("- korean_name (text)") && prompt.contains("- agree (checkbox)"));
    assert!(
        prompt.contains("- gender (radio: Male | Female)")
            && prompt.contains("- city (choice: sel (서울) | bus (부산))")
    );
    assert!(
        !prompt.contains("김철수") && !prompt.contains("Exact"),
        "values must not be sent: {prompt}"
    );
    assert!(
        !prompt.contains("- name ("),
        "already matched fields are not offered again"
    );
}

#[tokio::test]
async fn the_llm_step_is_skipped_when_unusable_or_unneeded() {
    // Unusable answer: skipped with a warning, the exact match still fills.
    let env = Env::new();
    let mock = spawn_llm(vec![answer("죄송합니다, 모르겠어요")]).await;
    env.fill(
        &llm_client(&mock.url, Some(KEY)),
        &sample_form(false),
        json!({"name": "A", "성명": "B"}),
    )
    .await
    .unwrap();
    let report = env.report();
    assert_eq!(report["unmatchedKeys"], json!(["성명"]));
    assert!(
        report["warnings"]
            .to_string()
            .contains("해석하지 못해 건너뛰었습니다")
    );
    assert_eq!(mock.count(), 1);

    // No API key: skipped with a warning, no HTTP call.
    let env = Env::new();
    let mock = spawn_llm(vec![answer("{}")]).await;
    env.fill(
        &llm_client(&mock.url, None),
        &sample_form(false),
        json!({"name": "A", "성명": "B"}),
    )
    .await
    .unwrap();
    assert!(
        env.report()["warnings"]
            .to_string()
            .contains("OPENROUTER_API_KEY")
    );
    assert_eq!(mock.count(), 0);

    // LLM API error: skipped as well.
    let env = Env::new();
    let mock = spawn_llm(vec![Reply {
        status: 400,
        body: json!({"error": {"message": "bad"}}),
    }])
    .await;
    env.fill(
        &llm_client(&mock.url, Some(KEY)),
        &sample_form(false),
        json!({"name": "A", "성명": "B"}),
    )
    .await
    .unwrap();
    assert!(
        env.report()["warnings"]
            .to_string()
            .contains("건너뛰었습니다")
    );

    // Everything matched without the LLM: it is never asked.
    let env = Env::new();
    let mock = spawn_llm(vec![]).await;
    env.fill(
        &llm_client(&mock.url, Some(KEY)),
        &sample_form(false),
        json!({"name": "A", "korean_name": "B"}),
    )
    .await
    .unwrap();
    assert_eq!(mock.count(), 0);

    // The key never shows up in the report.
    assert!(!env.report().to_string().contains(KEY));
}

// ------------------------------------------------------------------ failures

#[tokio::test]
async fn invalid_inputs_fail_with_clear_messages() {
    let env = Env::new();
    let form = sample_form(false);
    for values in ["[1, 2]", "\"text\"", "42", "null"] {
        let inputs = env.inputs(&form, values);
        let message = match fill::run(&no_llm(), &env.ctx, &inputs).await {
            Err(NodeError::Failed(m)) => m,
            other => panic!("{other:?}"),
        };
        assert_eq!(message, "폼 값 JSON 은 객체여야 합니다", "{values}");
    }
    let inputs = env.inputs(&form, "{not json");
    match fill::run(&no_llm(), &env.ctx, &inputs).await {
        Err(NodeError::Failed(m)) => {
            assert!(m.starts_with("폼 값 JSON 을 읽을 수 없습니다: "), "{m}")
        }
        other => panic!("{other:?}"),
    }

    // Not a PDF at all.
    let message = failed(
        env.fill(&no_llm(), b"hello, definitely not a pdf", json!({"a": 1}))
            .await,
    );
    assert!(message.starts_with("PDF 를 읽을 수 없습니다"), "{message}");
}

#[tokio::test]
async fn a_pdf_without_form_fields_fails() {
    let env = Env::new();
    let mut plain = Builder::new();
    plain.text_field("ignored", None);
    let no_acroform = plain.finish(false, false);
    assert_eq!(
        failed(env.fill(&no_llm(), &no_acroform, json!({"a": 1})).await),
        "PDF 에 입력 가능한 폼 필드가 없습니다"
    );

    // An AcroForm whose only fields are a signature and a push button has nothing to fill either.
    let mut only_unfillable = Builder::new();
    only_unfillable.signature_and_button();
    let bytes = only_unfillable.finish(true, false);
    assert_eq!(
        failed(env.fill(&no_llm(), &bytes, json!({"a": 1})).await),
        "PDF 에 입력 가능한 폼 필드가 없습니다"
    );
}

#[tokio::test]
async fn no_match_fails_listing_up_to_twenty_fields_and_keeps_the_report() {
    let env = Env::new();
    let message = failed(
        env.fill(
            &no_llm(),
            &many_field_form(25),
            json!({"nothing": "relevant"}),
        )
        .await,
    );
    assert!(
        message.starts_with("입력 값과 일치하는 폼 필드가 없습니다 (필드: field_00, field_01"),
        "{message}"
    );
    assert!(
        message.contains("field_19") && !message.contains("field_20"),
        "{message}"
    );
    assert!(message.ends_with("외 5개)"), "{message}");
    // The report explaining the failure is still written.
    let report = env.report();
    assert_eq!(report["filled"], json!([]));
    assert_eq!(report["unmatchedKeys"], json!(["nothing"]));
    assert_eq!(report["untouchedFields"].as_array().unwrap().len(), 25);

    // Matched but not applicable (checkbox given a nonsense value) is not a success either.
    let env = Env::new();
    let message = failed(
        env.fill(&no_llm(), &sample_form(false), json!({"agree": "maybe"}))
            .await,
    );
    assert!(
        message.starts_with("입력 값과 일치하는 폼 필드가 없습니다"),
        "{message}"
    );
    assert!(env.report()["warnings"].to_string().contains("agree"));
}

// ------------------------------------------------------------------ full run through the API

struct Harness {
    app: Router,
    data: PathBuf,
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.data);
    }
}

fn harness(pool: PgPool) -> Harness {
    let data = std::env::temp_dir().join(format!("dochestra-fill-run-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&data).unwrap();
    let dir = data.to_string_lossy().into_owned();
    let config = Config::from_lookup(move |k| match k {
        "DATABASE_URL" => Some("unused".to_owned()),
        "DATA_DIR" => Some(dir.clone()),
        _ => None,
    })
    .unwrap();
    let http = reqwest::Client::new();
    let runner = EngineRunner::new(pool.clone(), Arc::new(config.clone()), http.clone());
    Harness {
        app: build_app(AppState::new(pool, config, http, Arc::new(runner))),
        data,
    }
}

async fn raw(app: &Router, req: Request<Body>) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
    let resp = app.clone().oneshot(req).await.unwrap();
    let (status, headers) = (resp.status(), resp.headers().clone());
    (
        status,
        headers,
        resp.into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
}

async fn call(app: &Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let mut builder = Request::builder().method(method).uri(uri);
    let body = match body {
        Some(v) => {
            builder = builder.header("content-type", "application/json");
            Body::from(v.to_string())
        }
        None => Body::empty(),
    };
    let (status, _, bytes) = raw(app, builder.body(body).unwrap()).await;
    (
        status,
        if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap()
        },
    )
}

async fn upload_bundle(app: &Router, pdf: &[u8]) -> String {
    let (_, bundle) = call(app, "POST", "/api/bundles", Some(json!({"name": "b"}))).await;
    let id = bundle["id"].as_str().unwrap().to_owned();
    let boundary = "fill-boundary";
    let mut body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"files\"; filename=\"form.pdf\"\r\nContent-Type: application/pdf\r\n\r\n"
    )
    .into_bytes();
    body.extend_from_slice(pdf);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    let req = Request::post(format!("/api/bundles/{id}/files"))
        .header(
            "content-type",
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(Body::from(body))
        .unwrap();
    assert_eq!(raw(app, req).await.0, StatusCode::CREATED);
    id
}

fn node(id: &str, kind: &str, params: Value, is_output: bool) -> Value {
    json!({"id": id, "kind": kind, "position": {"x": 0, "y": 0}, "params": params, "isOutput": is_output})
}

fn edge(id: &str, s: &str, sh: &str, t: &str, th: &str) -> Value {
    json!({"id": id, "source": s, "sourceHandle": sh, "target": t, "targetHandle": th})
}

#[sqlx::test(migrations = "./migrations")]
async fn bundle_pick_pdf_and_json_input_feed_the_fill_node(pool: PgPool) {
    let h = harness(pool);
    let form = sample_form(false);
    let bundle = upload_bundle(&h.app, &form).await;
    let graph = json!({
        "nodes": [
            node("b", "bundle", json!({"bundleId": bundle}), false),
            node("p", "pick_pdf", json!({"index": 0}), false),
            node("j", "json_input", json!({"value": {"name": "Jane Doe", "korean_name": "김영희", "agree": true, "country": "JP"}}), false),
            node("f", "fill", json!({}), true)
        ],
        "edges": [
            edge("e1", "b", "files", "p", "files"),
            edge("e2", "p", "pdf", "f", "pdf"),
            edge("e3", "j", "json", "f", "values")
        ]
    });
    let (_, wf) = call(&h.app, "POST", "/api/workflows", Some(json!({"name": "w"}))).await;
    let wf = wf["id"].as_str().unwrap().to_owned();
    let (status, body) = call(
        &h.app,
        "PUT",
        &format!("/api/workflows/{wf}"),
        Some(json!({"graph": graph})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (_, run) = call(&h.app, "POST", &format!("/api/workflows/{wf}/runs"), None).await;
    let run_id = run["runId"].as_str().unwrap().to_owned();
    let mut detail = Value::Null;
    for _ in 0..400 {
        let (_, d) = call(&h.app, "GET", &format!("/api/runs/{run_id}"), None).await;
        if d["status"] != "running" {
            detail = d;
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert_eq!(detail["status"], "succeeded", "{detail}");

    let (status, headers, bytes) = raw(
        &h.app,
        Request::get(format!("/api/runs/{run_id}/artifacts/f/pdf"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["content-type"], "application/pdf");
    let fields = fields_of(&bytes);
    assert_eq!(fields["name"].current.as_deref(), Some("Jane Doe"));
    assert_eq!(fields["korean_name"].current.as_deref(), Some("김영희"));
    assert_eq!(fields["agree"].current.as_deref(), Some("Yes"));
    assert_eq!(fields["country"].current.as_deref(), Some("JP"));
    assert!(need_appearances(&bytes));
    // The original upload is untouched.
    let original = std::fs::read_dir(h.data.join("bundles"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let stored = std::fs::read_dir(&original)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    assert_eq!(std::fs::read(stored).unwrap(), form);
}

// ------------------------------------------------------------------ a form written by another library

/// `tests/fixtures/form.pdf` is produced by reportlab (see `make_form.py`), not by lopdf.
fn reportlab_form() -> Vec<u8> {
    std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/form.pdf"
    ))
    .expect("fixture exists")
}

#[test]
fn a_reportlab_form_is_discovered() {
    let fields = fields_of(&reportlab_form());
    let mut names: Vec<&str> = fields.keys().map(String::as_str).collect();
    names.sort_unstable();
    assert_eq!(
        names,
        [
            "Email Address",
            "agree_terms",
            "comments",
            "country",
            "full_name",
            "plan"
        ]
    );
    assert!(matches!(
        fields["agree_terms"].kind,
        FieldKind::Checkbox { .. }
    ));
    let FieldKind::Radio { options } = &fields["plan"].kind else {
        panic!("{:?}", fields["plan"].kind)
    };
    assert_eq!(options.len(), 2);
    assert!(
        options.iter().any(|o| o == "basic") && options.iter().any(|o| o == "pro"),
        "{options:?}"
    );
    let FieldKind::Choice { options } = &fields["country"].kind else {
        panic!("{:?}", fields["country"].kind)
    };
    assert_eq!(
        options
            .iter()
            .map(|o| (o.export.as_str(), o.display.as_str()))
            .collect::<Vec<_>>(),
        [("KR", "Korea"), ("US", "United States"), ("JP", "Japan")]
    );
}

#[tokio::test]
async fn a_reportlab_form_can_be_filled() {
    let env = Env::new();
    let out = env
        .fill(
            &no_llm(),
            &reportlab_form(),
            json!({"fullName": "김철수", "email address": "chulsoo@example.com", "comments": "첫 줄\n둘째 줄", "agree terms": "yes", "plan": "PRO", "country": "Japan"}),
        )
        .await
        .unwrap();
    if let Ok(dir) = std::env::var("FILL_DUMP_DIR") {
        std::fs::write(PathBuf::from(dir).join("filled_reportlab.pdf"), &out).unwrap();
    }
    let fields = fields_of(&out);
    assert_eq!(fields["full_name"].current.as_deref(), Some("김철수"));
    assert_eq!(
        fields["Email Address"].current.as_deref(),
        Some("chulsoo@example.com")
    );
    assert_eq!(
        fields["comments"].current.as_deref(),
        Some("첫 줄\n둘째 줄")
    );
    assert_eq!(fields["country"].current.as_deref(), Some("JP"));
    assert_eq!(fields["plan"].current.as_deref(), Some("pro"));
    let FieldKind::Checkbox { on_state } = &fields["agree_terms"].kind else {
        panic!()
    };
    assert_eq!(
        fields["agree_terms"].current.as_deref(),
        Some(on_state.as_str())
    );
    assert_eq!(
        appearance_states(&out, "agree_terms"),
        vec![on_state.to_owned()]
    );
    let plan_states = appearance_states(&out, "plan");
    assert_eq!(plan_states.len(), 2);
    assert_eq!(
        plan_states.iter().filter(|s| *s == "pro").count(),
        1,
        "{plan_states:?}"
    );
    assert!(need_appearances(&out));
    assert_eq!(env.report()["filled"].as_array().unwrap().len(), 6);
}
