//! AcroForm field discovery and writing with lopdf.

use std::collections::HashSet;

use lopdf::{Dictionary, Document, Object, ObjectId, StringFormat};
use serde_json::Value;

use super::labels::Hints;

/// `Ff` bit 16 (value 1 << 15): radio button group.
const FLAG_RADIO: i64 = 1 << 15;
/// `Ff` bit 17 (value 1 << 16): push button.
const FLAG_PUSHBUTTON: i64 = 1 << 16;
/// Guards against cyclic or absurdly deep field trees.
const MAX_DEPTH: usize = 32;
const DEFAULT_ON_STATE: &str = "Yes";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChoiceOption {
    pub export: String,
    pub display: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldKind {
    Text,
    Checkbox { on_state: String },
    Radio { options: Vec<String> },
    Choice { options: Vec<ChoiceOption> },
}

impl FieldKind {
    /// Short type label used in the LLM prompt.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Checkbox { .. } => "checkbox",
            Self::Radio { .. } => "radio",
            Self::Choice { .. } => "choice",
        }
    }
}

/// A fillable terminal field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormField {
    /// Fully qualified name: parent names joined with `.`.
    pub name: String,
    pub kind: FieldKind,
    pub current: Option<String>,
    /// The terminal field object.
    pub id: ObjectId,
    /// Widget annotations of the field when they are separate objects (empty when the field
    /// object is itself the widget).
    pub widgets: Vec<ObjectId>,
}

impl FormField {
    /// The part after the last `.`.
    pub fn last_segment(&self) -> &str {
        self.name.rsplit('.').next().unwrap_or(&self.name)
    }
}

/// Attributes a field inherits from its ancestors.
#[derive(Clone, Default)]
struct Inherited {
    field_type: Option<Vec<u8>>,
    flags: Option<i64>,
    value: Option<Object>,
    options: Option<Object>,
}

/// Decodes a PDF text string (UTF-16 with BOM, UTF-8 with BOM, else Latin-1/PDFDocEncoding).
pub fn decode_pdf_text(bytes: &[u8]) -> String {
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        let units: Vec<u16> = rest
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_be_bytes(*c))
            .collect();
        String::from_utf16_lossy(&units)
    } else if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        let units: Vec<u16> = rest
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes(*c))
            .collect();
        String::from_utf16_lossy(&units)
    } else if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        String::from_utf8_lossy(rest).into_owned()
    } else {
        bytes.iter().map(|&b| char::from(b)).collect()
    }
}

/// Encodes text as a PDF string: plain bytes for printable ASCII, else UTF-16BE with a BOM.
pub fn encode_pdf_text(text: &str) -> Object {
    if text.bytes().all(|b| (0x20..0x7f).contains(&b)) {
        Object::String(text.as_bytes().to_vec(), StringFormat::Literal)
    } else {
        let mut bytes = vec![0xFE, 0xFF];
        for unit in text.encode_utf16() {
            bytes.extend_from_slice(&unit.to_be_bytes());
        }
        Object::String(bytes, StringFormat::Hexadecimal)
    }
}

fn object_text(object: &Object) -> Option<String> {
    match object {
        Object::String(bytes, _) => Some(decode_pdf_text(bytes)),
        Object::Name(bytes) => Some(String::from_utf8_lossy(bytes).into_owned()),
        _ => None,
    }
}

fn deref<'a>(doc: &'a Document, object: &'a Object) -> Option<&'a Object> {
    doc.dereference(object).ok().map(|(_, o)| o)
}

/// All fillable terminal fields of the document's AcroForm (document order).
pub fn discover(doc: &Document) -> Vec<FormField> {
    let mut out = Vec::new();
    let Some(acro) = doc
        .catalog()
        .ok()
        .and_then(|c| c.get(b"AcroForm").ok())
        .and_then(|o| deref(doc, o))
        .and_then(|o| o.as_dict().ok())
    else {
        return out;
    };
    let Some(fields) = acro
        .get(b"Fields")
        .ok()
        .and_then(|o| deref(doc, o))
        .and_then(|o| o.as_array().ok())
    else {
        return out;
    };

    let mut seen = HashSet::new();
    for entry in fields {
        if let Ok(id) = entry.as_reference() {
            walk(doc, id, "", &Inherited::default(), &mut out, &mut seen, 0);
        }
    }
    out
}

fn walk(
    doc: &Document,
    id: ObjectId,
    parent_name: &str,
    inherited: &Inherited,
    out: &mut Vec<FormField>,
    seen: &mut HashSet<ObjectId>,
    depth: usize,
) {
    if depth > MAX_DEPTH || !seen.insert(id) {
        return;
    }
    let Ok(dict) = doc.get_dictionary(id) else {
        return;
    };

    let partial = dict
        .get(b"T")
        .ok()
        .and_then(|o| deref(doc, o))
        .and_then(object_text);
    let name = match (&partial, parent_name.is_empty()) {
        (Some(t), true) => t.clone(),
        (Some(t), false) => format!("{parent_name}.{t}"),
        (None, _) => parent_name.to_owned(),
    };

    let mut attrs = inherited.clone();
    if let Some(ft) = dict
        .get(b"FT")
        .ok()
        .and_then(|o| deref(doc, o))
        .and_then(|o| o.as_name().ok())
    {
        attrs.field_type = Some(ft.to_vec());
    }
    if let Some(ff) = dict
        .get(b"Ff")
        .ok()
        .and_then(|o| deref(doc, o))
        .and_then(|o| o.as_i64().ok())
    {
        attrs.flags = Some(ff);
    }
    if let Ok(v) = dict.get(b"V") {
        attrs.value = deref(doc, v).cloned();
    }
    if let Ok(opt) = dict.get(b"Opt") {
        attrs.options = deref(doc, opt).cloned();
    }

    let kids: Vec<ObjectId> = dict
        .get(b"Kids")
        .ok()
        .and_then(|o| deref(doc, o))
        .and_then(|o| o.as_array().ok())
        .map(|a| a.iter().filter_map(|k| k.as_reference().ok()).collect())
        .unwrap_or_default();
    let field_kids: Vec<ObjectId> = kids
        .iter()
        .copied()
        .filter(|k| doc.get_dictionary(*k).is_ok_and(|d| d.has(b"T")))
        .collect();

    if !field_kids.is_empty() {
        for kid in field_kids {
            walk(doc, kid, &name, &attrs, out, seen, depth + 1);
        }
        return;
    }

    // Terminal field: its kids (if any) are widgets.
    let Some(kind) = field_kind(doc, dict, &attrs, &kids) else {
        return;
    };
    if name.is_empty() {
        return;
    }
    let current = attrs.value.as_ref().and_then(object_text);
    out.push(FormField {
        name,
        kind,
        current,
        id,
        widgets: kids,
    });
}

fn field_kind(
    doc: &Document,
    dict: &Dictionary,
    attrs: &Inherited,
    widgets: &[ObjectId],
) -> Option<FieldKind> {
    let flags = attrs.flags.unwrap_or(0);
    match attrs.field_type.as_deref()? {
        b"Tx" => Some(FieldKind::Text),
        b"Btn" => {
            if flags & FLAG_PUSHBUTTON != 0 {
                return None;
            }
            if flags & FLAG_RADIO != 0 {
                let options = widgets
                    .iter()
                    .filter_map(|w| doc.get_dictionary(*w).ok().and_then(|d| on_state(doc, d)))
                    .collect();
                return Some(FieldKind::Radio { options });
            }
            let own = on_state(doc, dict).or_else(|| {
                widgets
                    .iter()
                    .find_map(|w| doc.get_dictionary(*w).ok().and_then(|d| on_state(doc, d)))
            });
            Some(FieldKind::Checkbox {
                on_state: own.unwrap_or_else(|| DEFAULT_ON_STATE.to_owned()),
            })
        }
        b"Ch" => {
            let options = attrs
                .options
                .as_ref()
                .and_then(|o| o.as_array().ok())
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| choice_option(doc, item))
                        .collect()
                })
                .unwrap_or_default();
            Some(FieldKind::Choice { options })
        }
        _ => None, // Sig and unknown types are not fillable
    }
}

fn choice_option(doc: &Document, item: &Object) -> Option<ChoiceOption> {
    let item = deref(doc, item)?;
    if let Some(text) = object_text(item) {
        return Some(ChoiceOption {
            export: text.clone(),
            display: text,
        });
    }
    let pair = item.as_array().ok()?;
    let export = pair
        .first()
        .and_then(|o| deref(doc, o))
        .and_then(object_text)?;
    let display = pair
        .get(1)
        .and_then(|o| deref(doc, o))
        .and_then(object_text)
        .unwrap_or_else(|| export.clone());
    Some(ChoiceOption { export, display })
}

/// The "on" appearance state of a widget: the first `/AP /N` key other than `Off`.
fn on_state(doc: &Document, widget: &Dictionary) -> Option<String> {
    let normal = widget
        .get(b"AP")
        .ok()
        .and_then(|o| deref(doc, o))
        .and_then(|o| o.as_dict().ok())?
        .get(b"N")
        .ok()
        .and_then(|o| deref(doc, o))
        .and_then(|o| o.as_dict().ok())?;
    normal
        .iter()
        .map(|(key, _)| String::from_utf8_lossy(key).into_owned())
        .find(|key| key != "Off")
}

/// What to write into a field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldWrite {
    Text(String),
    Check(bool),
    Radio(String),
    Choice(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Planned {
    pub write: FieldWrite,
    /// Human-readable value for the report.
    pub shown: String,
    pub warning: Option<String>,
}

/// Normalizes for loose comparison: lowercase, without whitespace, `_`, `-` and `.`.
pub fn normalize(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_whitespace() && !matches!(c, '_' | '-' | '.'))
        .flat_map(char::to_lowercase)
        .collect()
}

fn scalar_text(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// Interprets a JSON value as a checkbox state.
pub fn to_bool(value: &Value) -> Option<bool> {
    match value {
        Value::Bool(b) => Some(*b),
        Value::Number(n) => match n.as_f64()? {
            1.0 => Some(true),
            0.0 => Some(false),
            _ => None,
        },
        Value::String(s) => match normalize(s).as_str() {
            "true" | "yes" | "y" | "on" | "1" | "checked" | "check" | "예" | "네" | "동의"
            | "o" => Some(true),
            "false" | "no" | "n" | "off" | "0" | "unchecked" | "" | "아니오" | "아니요" | "x" => {
                Some(false)
            }
            _ => None,
        },
        _ => None,
    }
}

/// Picks the radio option a value means: the option name (exact, then normalized), then the printed
/// label beside the widget (equal, then — when exactly one label qualifies — contained in the value or
/// containing it: "남자" selects the option labelled "남").
fn radio_choice<'a>(
    options: &'a [String],
    labels: &[Option<String>],
    wanted: &str,
) -> Option<&'a String> {
    if let Some(option) = options.iter().find(|o| o.as_str() == wanted) {
        return Some(option);
    }
    let wanted = normalize(wanted);
    if wanted.is_empty() {
        return None;
    }
    if let Some(option) = options.iter().find(|o| normalize(o) == wanted) {
        return Some(option);
    }
    let labels: Vec<Option<String>> = labels.iter().map(|l| l.as_deref().map(normalize)).collect();
    let unique = |test: &dyn Fn(&str) -> bool| -> Option<&'a String> {
        let hits: Vec<usize> = labels
            .iter()
            .enumerate()
            .filter(|(_, l)| l.as_deref().is_some_and(|l| !l.is_empty() && test(l)))
            .map(|(i, _)| i)
            .collect();
        match hits.as_slice() {
            [only] => options.get(*only),
            _ => None,
        }
    };
    unique(&|l| l == wanted)
        .or_else(|| unique(&|l| wanted.contains(l) || l.contains(wanted.as_str())))
}

/// Decides how `value` is written into `field`. `Err` explains why it cannot be applied.
pub fn plan_write(field: &FormField, value: &Value) -> Result<Planned, String> {
    plan_write_with(field, value, &Hints::default())
}

/// Like [`plan_write`], using what the page says about the field (printed option labels).
pub fn plan_write_with(field: &FormField, value: &Value, hints: &Hints) -> Result<Planned, String> {
    let unsupported = || format!("필드 '{}' 에 쓸 수 없는 값입니다: {value}", field.name);
    match &field.kind {
        FieldKind::Text => {
            let text = scalar_text(value).ok_or_else(unsupported)?;
            Ok(Planned {
                shown: text.clone(),
                write: FieldWrite::Text(text),
                warning: None,
            })
        }
        FieldKind::Checkbox { .. } => {
            let on = to_bool(value).ok_or_else(unsupported)?;
            Ok(Planned {
                shown: if on { "on" } else { "off" }.to_owned(),
                write: FieldWrite::Check(on),
                warning: None,
            })
        }
        FieldKind::Radio { options } => {
            let wanted = scalar_text(value)
                .filter(|_| !value.is_boolean())
                .ok_or_else(unsupported)?;
            let chosen = radio_choice(options, &hints.option_labels, &wanted).ok_or_else(|| {
                let shown: Vec<String> = options
                    .iter()
                    .enumerate()
                    .map(
                        |(i, o)| match hints.option_labels.get(i).and_then(Option::as_deref) {
                            Some(label) => format!("{o} ({label})"),
                            None => o.clone(),
                        },
                    )
                    .collect();
                format!(
                    "라디오 '{}' 에 없는 선택지입니다: {wanted} (선택지: {})",
                    field.name,
                    shown.join(", ")
                )
            })?;
            Ok(Planned {
                shown: chosen.clone(),
                write: FieldWrite::Radio(chosen.clone()),
                warning: None,
            })
        }
        FieldKind::Choice { options } => {
            let wanted = scalar_text(value).ok_or_else(unsupported)?;
            let found = options
                .iter()
                .find(|o| o.export == wanted)
                .or_else(|| options.iter().find(|o| o.display == wanted))
                .or_else(|| {
                    options.iter().find(|o| {
                        normalize(&o.export) == normalize(&wanted)
                            || normalize(&o.display) == normalize(&wanted)
                    })
                });
            match found {
                Some(option) => Ok(Planned {
                    shown: option.export.clone(),
                    write: FieldWrite::Choice(option.export.clone()),
                    warning: None,
                }),
                None => Ok(Planned {
                    shown: wanted.clone(),
                    write: FieldWrite::Choice(wanted.clone()),
                    warning: Some(format!(
                        "선택 필드 '{}' 의 선택지에 없는 값을 그대로 썼습니다: {wanted}",
                        field.name
                    )),
                }),
            }
        }
    }
}

/// Writes a planned value into the field (and its widgets' appearance states).
pub fn apply(doc: &mut Document, field: &FormField, write: &FieldWrite) -> Result<(), String> {
    let dict_err = |e: lopdf::Error| format!("필드 '{}' 를 수정할 수 없습니다: {e}", field.name);
    match write {
        FieldWrite::Text(text) | FieldWrite::Choice(text) => {
            doc.get_dictionary_mut(field.id)
                .map_err(dict_err)?
                .set("V", encode_pdf_text(text));
        }
        FieldWrite::Check(on) => {
            let FieldKind::Checkbox { on_state } = &field.kind else {
                return Err(format!("필드 '{}' 는 체크박스가 아닙니다", field.name));
            };
            let state = if *on { on_state.as_str() } else { "Off" };
            doc.get_dictionary_mut(field.id)
                .map_err(dict_err)?
                .set("V", Object::Name(state.as_bytes().to_vec()));
            for widget in widget_ids(field) {
                doc.get_dictionary_mut(widget)
                    .map_err(dict_err)?
                    .set("AS", Object::Name(state.as_bytes().to_vec()));
            }
        }
        FieldWrite::Radio(option) => {
            doc.get_dictionary_mut(field.id)
                .map_err(dict_err)?
                .set("V", Object::Name(option.as_bytes().to_vec()));
            for widget in widget_ids(field) {
                let state = doc
                    .get_dictionary(widget)
                    .ok()
                    .and_then(|d| on_state(doc, d))
                    .filter(|s| s == option)
                    .unwrap_or_else(|| "Off".to_owned());
                doc.get_dictionary_mut(widget)
                    .map_err(dict_err)?
                    .set("AS", Object::Name(state.into_bytes()));
            }
        }
    }
    Ok(())
}

/// Objects carrying the appearance state: the separate widgets, or the field itself.
fn widget_ids(field: &FormField) -> Vec<ObjectId> {
    if field.widgets.is_empty() {
        vec![field.id]
    } else {
        field.widgets.clone()
    }
}

/// Asks viewers to regenerate field appearances so the written values show up.
pub fn set_need_appearances(doc: &mut Document) -> Result<(), String> {
    let acro_ref = doc
        .catalog()
        .map_err(|e| e.to_string())?
        .get(b"AcroForm")
        .map_err(|e| e.to_string())?
        .as_reference()
        .ok();
    let dict = match acro_ref {
        Some(id) => doc.get_dictionary_mut(id).map_err(|e| e.to_string())?,
        None => doc
            .catalog_mut()
            .map_err(|e| e.to_string())?
            .get_mut(b"AcroForm")
            .map_err(|e| e.to_string())?
            .as_dict_mut()
            .map_err(|e| e.to_string())?,
    };
    dict.set("NeedAppearances", Object::Boolean(true));
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn field(kind: FieldKind) -> FormField {
        FormField {
            name: "f".into(),
            kind,
            current: None,
            id: (1, 0),
            widgets: vec![],
        }
    }

    #[test]
    fn pdf_text_round_trips_ascii_and_unicode() {
        assert_eq!(
            encode_pdf_text("Hello"),
            Object::String(b"Hello".to_vec(), StringFormat::Literal)
        );
        let korean = encode_pdf_text("홍길동");
        let Object::String(bytes, StringFormat::Hexadecimal) = &korean else {
            panic!("{korean:?}")
        };
        assert_eq!(&bytes[..2], &[0xFE, 0xFF]);
        assert_eq!(decode_pdf_text(bytes), "홍길동");
        // Newlines are not "printable ASCII": they take the UTF-16 path and survive.
        let multi = encode_pdf_text("a\nb");
        let Object::String(bytes, _) = &multi else {
            panic!()
        };
        assert_eq!(decode_pdf_text(bytes), "a\nb");
        assert_eq!(decode_pdf_text(&[0xFF, 0xFE, 0x41, 0x00]), "A");
        assert_eq!(decode_pdf_text(&[0xEF, 0xBB, 0xBF, b'h', b'i']), "hi");
        assert_eq!(decode_pdf_text(&[b'c', 0xE9]), "cé");
    }

    #[test]
    fn normalization_ignores_case_spaces_and_separators() {
        assert_eq!(normalize("First_Name"), "firstname");
        assert_eq!(normalize("first name"), "firstname");
        assert_eq!(normalize("applicant.First-Name"), "applicantfirstname");
        assert_eq!(normalize("성 명"), "성명");
    }

    #[test]
    fn checkbox_values_are_interpreted() {
        for yes in [
            json!(true),
            json!(1),
            json!("Yes"),
            json!("on"),
            json!("TRUE"),
            json!("예"),
            json!("동의"),
        ] {
            assert_eq!(to_bool(&yes), Some(true), "{yes}");
        }
        for no in [
            json!(false),
            json!(0),
            json!("no"),
            json!("Off"),
            json!(""),
            json!("아니오"),
        ] {
            assert_eq!(to_bool(&no), Some(false), "{no}");
        }
        for unknown in [
            json!("maybe"),
            json!(2),
            json!(null),
            json!([true]),
            json!({}),
        ] {
            assert_eq!(to_bool(&unknown), None, "{unknown}");
        }
    }

    #[test]
    fn text_fields_accept_scalars() {
        let f = field(FieldKind::Text);
        assert_eq!(
            plan_write(&f, &json!("홍길동")).unwrap().write,
            FieldWrite::Text("홍길동".into())
        );
        assert_eq!(
            plan_write(&f, &json!(42)).unwrap().write,
            FieldWrite::Text("42".into())
        );
        assert_eq!(plan_write(&f, &json!(1.5)).unwrap().shown, "1.5");
        assert_eq!(
            plan_write(&f, &json!(true)).unwrap().write,
            FieldWrite::Text("true".into())
        );
        assert!(plan_write(&f, &json!(["a"])).is_err());
        assert!(plan_write(&f, &json!({"a": 1})).is_err());
    }

    #[test]
    fn checkbox_and_radio_planning() {
        let check = field(FieldKind::Checkbox {
            on_state: "Yes".into(),
        });
        assert_eq!(
            plan_write(&check, &json!(true)).unwrap().write,
            FieldWrite::Check(true)
        );
        assert_eq!(plan_write(&check, &json!("no")).unwrap().shown, "off");
        assert!(plan_write(&check, &json!("maybe")).is_err());

        let radio = field(FieldKind::Radio {
            options: vec!["Male".into(), "Female".into()],
        });
        assert_eq!(
            plan_write(&radio, &json!("Male")).unwrap().write,
            FieldWrite::Radio("Male".into())
        );
        assert_eq!(
            plan_write(&radio, &json!("female")).unwrap().write,
            FieldWrite::Radio("Female".into())
        );
        let err = plan_write(&radio, &json!("Other")).unwrap_err();
        assert!(
            err.contains("Other") && err.contains("Male, Female"),
            "{err}"
        );
        assert!(plan_write(&radio, &json!(true)).is_err());
    }

    #[test]
    fn choice_values_map_to_export_values() {
        let options = vec![
            ChoiceOption {
                export: "sel".into(),
                display: "서울".into(),
            },
            ChoiceOption {
                export: "bus".into(),
                display: "부산".into(),
            },
        ];
        let choice = field(FieldKind::Choice { options });
        assert_eq!(
            plan_write(&choice, &json!("서울")).unwrap().write,
            FieldWrite::Choice("sel".into())
        );
        assert_eq!(
            plan_write(&choice, &json!("BUS")).unwrap().write,
            FieldWrite::Choice("bus".into())
        );
        let unknown = plan_write(&choice, &json!("대구")).unwrap();
        assert_eq!(unknown.write, FieldWrite::Choice("대구".into()));
        assert!(unknown.warning.unwrap().contains("선택지에 없는 값"));
    }

    #[test]
    fn radio_options_are_found_by_name_then_by_printed_label() {
        let options = vec!["male".to_owned(), "female".to_owned()];
        let labels = vec![Some("남".to_owned()), Some("여".to_owned())];
        let pick = |wanted: &str| radio_choice(&options, &labels, wanted).map(String::as_str);
        assert_eq!(pick("female"), Some("female"));
        assert_eq!(pick("FEMALE"), Some("female"));
        assert_eq!(pick("남"), Some("male"));
        assert_eq!(pick("남자"), Some("male"));
        assert_eq!(pick("여성"), Some("female"));
        assert_eq!(pick("기타"), None);
        assert_eq!(pick(""), None);
        // Without labels only names count.
        assert_eq!(radio_choice(&options, &[], "남자"), None);
    }

    #[test]
    fn an_ambiguous_label_match_is_not_guessed() {
        let options = vec!["a".to_owned(), "b".to_owned()];
        let labels = vec![Some("대".to_owned()), Some("중".to_owned())];
        // "대중" contains both labels: nothing is chosen.
        assert_eq!(radio_choice(&options, &labels, "대중"), None);
        assert_eq!(
            radio_choice(&options, &labels, "중").map(String::as_str),
            Some("b")
        );
        // Equal labels are ambiguous too.
        let same = vec![Some("예".to_owned()), Some("예".to_owned())];
        assert_eq!(radio_choice(&options, &same, "예"), None);
    }
}
