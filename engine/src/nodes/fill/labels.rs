//! Labels printed next to form fields.
//!
//! Many real forms name their fields `Text1`, `Check3`, `Group1` ... and only the page text says what a
//! field means ("성명", "개인정보 수집에 동의합니다"), and radio buttons are exported as `Choice1`/`Choice2`
//! while the printed labels beside them read "남"/"여". This module finds that text: it replays the text
//! operators of each page to get every text run with its position, then picks, for each widget, the nearest
//! run on its left (or above it) — and, for checkboxes and radio widgets, to its right. The field tooltip
//! (`/TU`) is collected too. Only short label strings (never values) end up in the LLM prompt.

use std::collections::{BTreeMap, HashMap};

use lopdf::content::Operation;
use lopdf::{Dictionary, Document, Object, ObjectId};

use super::form::{FieldKind, FormField, decode_pdf_text};

/// Longest label kept (characters).
const MAX_LABEL_CHARS: usize = 80;
/// How far a label may be from its widget (points).
const MAX_LEFT_GAP: f32 = 250.0;
const MAX_RIGHT_GAP: f32 = 40.0;
const MAX_ABOVE_GAP: f32 = 36.0;
/// Text runs on one line closer than this (in em) are one label (writers split labels into pieces).
const MERGE_GAP_EM: f32 = 0.6;

/// What the page and the field dictionary tell about a field besides its name.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Hints {
    /// The printed label next to the field (for a radio group: next to its first widget).
    pub label: Option<String>,
    /// The `/TU` tooltip (alternate field name) when it differs from the label.
    pub tooltip: Option<String>,
    /// For radio buttons: the printed label beside each widget, in the order of the options.
    pub option_labels: Vec<Option<String>>,
}

#[derive(Debug, Clone)]
struct Run {
    x: f32,
    end: f32,
    y: f32,
    size: f32,
    text: String,
}

type Matrix = [f32; 6];
const IDENTITY: Matrix = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

/// `m` followed by `n` (PDF row-vector convention).
fn mul(m: Matrix, n: Matrix) -> Matrix {
    [
        m[0] * n[0] + m[1] * n[2],
        m[0] * n[1] + m[1] * n[3],
        m[2] * n[0] + m[3] * n[2],
        m[2] * n[1] + m[3] * n[3],
        m[4] * n[0] + m[5] * n[2] + n[4],
        m[4] * n[1] + m[5] * n[3] + n[5],
    ]
}

fn translate(tx: f32, ty: f32) -> Matrix {
    [1.0, 0.0, 0.0, 1.0, tx, ty]
}

fn numbers(operands: &[Object]) -> Vec<f32> {
    operands.iter().filter_map(|o| o.as_float().ok()).collect()
}

/// Rough advance width of `text` at `size`: full-width for CJK and other non-ASCII, half for ASCII.
fn estimate_width(text: &str, size: f32) -> f32 {
    text.chars()
        .map(|c| if c.is_ascii() { 0.5 } else { 1.0 })
        .sum::<f32>()
        * size
}

/// The text runs of one page, with the position of their start (user space).
fn page_runs(doc: &Document, page_id: ObjectId) -> Vec<Run> {
    let Ok(fonts) = doc.get_page_fonts(page_id) else {
        return Vec::new();
    };
    let encodings: BTreeMap<Vec<u8>, lopdf::Encoding> = fonts
        .into_iter()
        .filter_map(|(name, font)| Some((name, font.get_font_encoding(doc).ok()?)))
        .collect();
    let Ok(content) = doc.get_and_decode_page_content(page_id) else {
        return Vec::new();
    };

    let mut runs: Vec<Run> = Vec::new();
    let mut ctm = IDENTITY;
    let mut stack: Vec<Matrix> = Vec::new();
    let (mut tm, mut tlm) = (IDENTITY, IDENTITY);
    let mut leading = 0.0f32;
    let mut size = 12.0f32;
    let mut font: Option<&lopdf::Encoding> = None;

    let mut show = |text: String, tm: &mut Matrix, ctm: &Matrix, size: f32| {
        let text_clean: String = text.chars().filter(|c| !c.is_control()).collect();
        let origin = mul(*tm, *ctm);
        let scale = (origin[0] * origin[0] + origin[1] * origin[1])
            .sqrt()
            .max(0.01);
        let effective = size * scale;
        let width = estimate_width(&text_clean, size);
        if !text_clean.trim().is_empty() {
            runs.push(Run {
                x: origin[4],
                end: origin[4] + width * scale,
                y: origin[5],
                size: effective,
                text: text_clean,
            });
        }
        *tm = mul(translate(width, 0.0), *tm);
    };

    for Operation { operator, operands } in &content.operations {
        match operator.as_str() {
            "q" => stack.push(ctm),
            "Q" => ctm = stack.pop().unwrap_or(IDENTITY),
            "cm" => {
                if let [a, b, c, d, e, f] = numbers(operands)[..] {
                    ctm = mul([a, b, c, d, e, f], ctm);
                }
            }
            "BT" => {
                tm = IDENTITY;
                tlm = IDENTITY;
            }
            "Tm" => {
                if let [a, b, c, d, e, f] = numbers(operands)[..] {
                    tm = [a, b, c, d, e, f];
                    tlm = tm;
                }
            }
            "Td" | "TD" => {
                if let [tx, ty] = numbers(operands)[..] {
                    if operator == "TD" {
                        leading = -ty;
                    }
                    tlm = mul(translate(tx, ty), tlm);
                    tm = tlm;
                }
            }
            "TL" => leading = numbers(operands).first().copied().unwrap_or(leading),
            "T*" => {
                tlm = mul(translate(0.0, -leading), tlm);
                tm = tlm;
            }
            "Tf" => {
                font = operands
                    .first()
                    .and_then(|o| o.as_name().ok())
                    .and_then(|name| encodings.get(name));
                size = operands
                    .get(1)
                    .and_then(|o| o.as_float().ok())
                    .unwrap_or(size);
            }
            "Tj" | "TJ" | "'" | "\"" => {
                if matches!(operator.as_str(), "'" | "\"") {
                    tlm = mul(translate(0.0, -leading), tlm);
                    tm = tlm;
                }
                let Some(encoding) = font else { continue };
                let mut text = String::new();
                for operand in operands {
                    match operand {
                        Object::String(bytes, _) => {
                            if let Ok(t) = Document::decode_text(encoding, bytes) {
                                text.push_str(&t);
                            }
                        }
                        Object::Array(items) => {
                            for item in items {
                                if let Object::String(bytes, _) = item
                                    && let Ok(t) = Document::decode_text(encoding, bytes)
                                {
                                    text.push_str(&t);
                                }
                            }
                        }
                        _ => {}
                    }
                }
                show(text, &mut tm, &ctm, size);
            }
            _ => {}
        }
    }
    merge_runs(runs)
}

/// Joins pieces of one label that a writer split into several text-showing operations.
fn merge_runs(mut runs: Vec<Run>) -> Vec<Run> {
    runs.sort_by(|a, b| {
        b.y.partial_cmp(&a.y)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal))
    });
    let mut merged: Vec<Run> = Vec::new();
    for run in runs {
        if let Some(last) = merged.last_mut()
            && (last.y - run.y).abs() < 1.0
            && run.x >= last.x
            && run.x - last.end <= MERGE_GAP_EM * last.size.max(run.size)
        {
            last.text.push_str(&run.text);
            last.end = last.end.max(run.end);
            continue;
        }
        merged.push(run);
    }
    merged
}

fn number(object: &Object) -> Option<f32> {
    object.as_float().ok()
}

/// `[left, bottom, right, top]` of a widget.
fn widget_rect(doc: &Document, widget: &Dictionary) -> Option<[f32; 4]> {
    let rect = widget.get(b"Rect").ok()?;
    let array = doc.dereference(rect).ok()?.1.as_array().ok()?;
    let [a, b, c, d] = array.as_slice() else {
        return None;
    };
    let (x1, y1, x2, y2) = (
        number(doc.dereference(a).ok()?.1)?,
        number(doc.dereference(b).ok()?.1)?,
        number(doc.dereference(c).ok()?.1)?,
        number(doc.dereference(d).ok()?.1)?,
    );
    Some([x1.min(x2), y1.min(y2), x1.max(x2), y1.max(y2)])
}

#[derive(Clone, Copy)]
enum Side {
    Left,
    Above,
    Right,
}

fn same_line(run: &Run, rect: &[f32; 4]) -> bool {
    run.y >= rect[1] - 0.5 * run.size && run.y <= rect[3]
}

/// The nearest run on one side of the widget.
fn nearest(runs: &[Run], rect: &[f32; 4], side: Side) -> Option<String> {
    let candidate = runs
        .iter()
        .filter_map(|run| {
            let distance = match side {
                Side::Left if same_line(run, rect) && run.end <= rect[0] + 3.0 => rect[0] - run.end,
                Side::Right if same_line(run, rect) && run.x >= rect[2] - 3.0 => run.x - rect[2],
                Side::Above
                    if run.y > rect[3]
                        && run.y - rect[3] <= MAX_ABOVE_GAP
                        && run.x <= rect[2]
                        && run.end >= rect[0] - 5.0 =>
                {
                    run.y - rect[3]
                }
                _ => return None,
            };
            let limit = match side {
                Side::Left => MAX_LEFT_GAP,
                Side::Right => MAX_RIGHT_GAP,
                Side::Above => MAX_ABOVE_GAP,
            };
            (distance <= limit).then_some((distance, run))
        })
        .min_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal))?;
    let text: String = candidate
        .1
        .text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    (!text.is_empty()).then(|| text.chars().take(MAX_LABEL_CHARS).collect())
}

fn first_of(runs: &[Run], rect: &[f32; 4], order: &[Side]) -> Option<String> {
    order.iter().find_map(|side| nearest(runs, rect, *side))
}

/// Hints for every field (same order as `fields`).
pub fn collect(doc: &Document, fields: &[FormField]) -> Vec<Hints> {
    // widget object -> page
    let mut page_of: HashMap<ObjectId, ObjectId> = HashMap::new();
    let mut runs_of: HashMap<ObjectId, Vec<Run>> = HashMap::new();
    for page_id in doc.get_pages().into_values() {
        let annots = doc
            .get_dictionary(page_id)
            .ok()
            .and_then(|page| page.get(b"Annots").ok())
            .and_then(|o| doc.dereference(o).ok())
            .and_then(|(_, o)| o.as_array().ok());
        for annot in annots.into_iter().flatten() {
            if let Ok(id) = annot.as_reference() {
                page_of.insert(id, page_id);
            }
        }
        runs_of.insert(page_id, page_runs(doc, page_id));
    }

    fields
        .iter()
        .map(|field| {
            let widget_ids: Vec<ObjectId> = if field.widgets.is_empty() {
                vec![field.id]
            } else {
                field.widgets.clone()
            };
            let placed: Vec<([f32; 4], &Vec<Run>)> = widget_ids
                .iter()
                .filter_map(|id| {
                    let rect = widget_rect(doc, doc.get_dictionary(*id).ok()?)?;
                    Some((rect, runs_of.get(page_of.get(id)?)?))
                })
                .collect();

            let tooltip = doc
                .get_dictionary(field.id)
                .ok()
                .and_then(|d| d.get(b"TU").ok())
                .and_then(|o| doc.dereference(o).ok())
                .and_then(|(_, o)| match o {
                    Object::String(bytes, _) => Some(decode_pdf_text(bytes)),
                    _ => None,
                })
                .map(|t| t.split_whitespace().collect::<Vec<_>>().join(" "))
                .filter(|t| !t.is_empty())
                .map(|t| t.chars().take(MAX_LABEL_CHARS).collect::<String>());

            let (label, option_labels) = match (&field.kind, placed.first()) {
                (_, None) => (None, Vec::new()),
                (FieldKind::Checkbox { .. }, Some((rect, runs))) => (
                    first_of(runs, rect, &[Side::Right, Side::Left, Side::Above]),
                    Vec::new(),
                ),
                (FieldKind::Radio { .. }, Some((rect, runs))) => (
                    first_of(runs, rect, &[Side::Left, Side::Above]),
                    placed
                        .iter()
                        .map(|(rect, runs)| nearest(runs, rect, Side::Right))
                        .collect(),
                ),
                (_, Some((rect, runs))) => {
                    (first_of(runs, rect, &[Side::Left, Side::Above]), Vec::new())
                }
            };
            let tooltip = tooltip.filter(|t| Some(t) != label.as_ref());
            Hints {
                label,
                tooltip,
                option_labels,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(x: f32, y: f32, text: &str) -> Run {
        Run {
            x,
            end: x + estimate_width(text, 12.0),
            y,
            size: 12.0,
            text: text.to_owned(),
        }
    }

    const FIELD: [f32; 4] = [170.0, 730.0, 410.0, 752.0];

    #[test]
    fn matrices_compose_in_pdf_order() {
        // Translate by (10, 20), then scale by 2: the origin ends at (20, 40).
        let m = mul(translate(10.0, 20.0), [2.0, 0.0, 0.0, 2.0, 0.0, 0.0]);
        assert_eq!((m[4], m[5]), (20.0, 40.0));
    }

    #[test]
    fn the_nearest_run_to_the_left_on_the_same_line_is_the_label() {
        let runs = vec![
            run(50.0, 736.0, "성명"),
            run(50.0, 700.0, "연락처"), // another line
            run(420.0, 736.0, "오른쪽"),
        ];
        assert_eq!(nearest(&runs, &FIELD, Side::Left).as_deref(), Some("성명"));
        assert_eq!(
            nearest(&runs, &FIELD, Side::Right).as_deref(),
            Some("오른쪽")
        );
    }

    #[test]
    fn a_label_above_the_field_is_found_when_nothing_is_on_its_left() {
        let runs = vec![run(172.0, 760.0, "신청인 성명"), run(172.0, 790.0, "제목")];
        assert_eq!(nearest(&runs, &FIELD, Side::Left), None);
        // Only the closer one is within reach, and it is chosen.
        assert_eq!(
            nearest(&runs, &FIELD, Side::Above).as_deref(),
            Some("신청인 성명")
        );
        assert_eq!(
            first_of(&runs, &FIELD, &[Side::Left, Side::Above]).as_deref(),
            Some("신청인 성명")
        );
    }

    #[test]
    fn far_text_is_not_a_label() {
        let runs = vec![
            run(10.0, 736.0, "멀리 있는 글"),
            run(172.0, 800.0, "너무 위"),
        ];
        assert_eq!(
            first_of(
                &runs,
                &[400.0, 730.0, 450.0, 752.0],
                &[Side::Left, Side::Above]
            ),
            None
        );
    }

    #[test]
    fn pieces_of_one_label_are_merged_but_separate_labels_are_not() {
        let pieces = vec![
            run(50.0, 736.0, "생년"),
            run(50.0 + estimate_width("생년", 12.0) + 1.0, 736.0, "월일"),
            run(300.0, 736.0, "연락처"),
        ];
        let merged = merge_runs(pieces);
        let texts: Vec<&str> = merged.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(texts, ["생년월일", "연락처"]);
    }
}
