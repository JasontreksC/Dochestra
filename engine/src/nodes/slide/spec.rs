//! The slide deck description produced by the LLM, with parsing, validation and correction.

use serde::{Deserialize, Serialize};

pub const MAX_SLIDES: usize = 15;
pub const MAX_BULLETS: usize = 7;
pub const MAX_ITEM_CHARS: usize = 120;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlideDeck {
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
    pub slides: Vec<SlideContent>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlideContent {
    pub title: String,
    pub bullets: Vec<String>,
}

/// Extracts the JSON object from an LLM answer (first `{` to last `}`), parses it against the
/// deck schema, and validates it.
///
/// Over-limit content is corrected: more than 15 slides, more than 7 bullets per slide, and
/// texts above 120 characters are truncated. Structural problems — no JSON object, wrong shape,
/// an empty deck title, no slides, a slide without title or bullets — are errors whose message is
/// meant to be shown to the LLM on the retry and to the user.
pub fn parse_deck(answer: &str) -> Result<SlideDeck, String> {
    let (start, end) = match (answer.find('{'), answer.rfind('}')) {
        (Some(s), Some(e)) if s < e => (s, e),
        _ => return Err("응답에서 JSON 객체를 찾을 수 없습니다".to_owned()),
    };
    let mut deck: SlideDeck = serde_json::from_str(&answer[start..=end])
        .map_err(|e| format!("JSON 형식이 스키마와 맞지 않습니다: {e}"))?;

    deck.title = clip(&deck.title);
    if deck.title.is_empty() {
        return Err("title 이 비어 있습니다".to_owned());
    }
    deck.subtitle = deck.subtitle.map(|s| clip(&s)).filter(|s| !s.is_empty());

    deck.slides.truncate(MAX_SLIDES);
    if deck.slides.is_empty() {
        return Err("slides 가 비어 있습니다 (1~15개 필요)".to_owned());
    }
    for (i, slide) in deck.slides.iter_mut().enumerate() {
        slide.title = clip(&slide.title);
        if slide.title.is_empty() {
            return Err(format!("slides[{i}].title 이 비어 있습니다"));
        }
        slide.bullets = slide
            .bullets
            .iter()
            .map(|b| clip(b))
            .filter(|b| !b.is_empty())
            .take(MAX_BULLETS)
            .collect();
        if slide.bullets.is_empty() {
            return Err(format!("slides[{i}].bullets 가 비어 있습니다 (1~7개 필요)"));
        }
    }
    Ok(deck)
}

/// Trims and truncates to 120 characters.
fn clip(text: &str) -> String {
    text.trim().chars().take(MAX_ITEM_CHARS).collect()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn deck_json(slides: usize, bullets: usize) -> String {
        let slides: Vec<_> = (0..slides)
            .map(|i| json!({"title": format!("Slide {i}"), "bullets": (0..bullets).map(|b| format!("Point {b}")).collect::<Vec<_>>()}))
            .collect();
        json!({"title": "Deck", "subtitle": "Sub", "slides": slides}).to_string()
    }

    #[test]
    fn a_valid_deck_is_parsed_as_is() {
        let deck = parse_deck(&deck_json(3, 2)).unwrap();
        assert_eq!(deck.title, "Deck");
        assert_eq!(deck.subtitle.as_deref(), Some("Sub"));
        assert_eq!(deck.slides.len(), 3);
        assert_eq!(
            deck.slides[1],
            SlideContent {
                title: "Slide 1".into(),
                bullets: vec!["Point 0".into(), "Point 1".into()]
            }
        );
    }

    #[test]
    fn json_is_extracted_from_surrounding_text_and_code_fences() {
        let answer = format!(
            "물론이죠! 구성은 다음과 같습니다.\n```json\n{}\n```\n도움이 되길 바랍니다.",
            deck_json(1, 1)
        );
        assert_eq!(parse_deck(&answer).unwrap().slides.len(), 1);
        // Braces inside strings do not confuse the extraction (first `{` .. last `}`).
        let tricky = r#"{"title":"A {b} c","slides":[{"title":"x","bullets":["y } z"]}]}"#;
        assert_eq!(parse_deck(tricky).unwrap().title, "A {b} c");
    }

    #[test]
    fn over_limit_content_is_truncated() {
        let deck = parse_deck(&deck_json(20, 10)).unwrap();
        assert_eq!(deck.slides.len(), MAX_SLIDES);
        assert!(deck.slides.iter().all(|s| s.bullets.len() == MAX_BULLETS));

        let long = "가".repeat(300);
        let answer = json!({"title": long, "subtitle": long, "slides": [{"title": long, "bullets": [long]}]}).to_string();
        let deck = parse_deck(&answer).unwrap();
        assert_eq!(deck.title.chars().count(), MAX_ITEM_CHARS);
        assert_eq!(deck.subtitle.unwrap().chars().count(), MAX_ITEM_CHARS);
        assert_eq!(deck.slides[0].title.chars().count(), MAX_ITEM_CHARS);
        assert_eq!(deck.slides[0].bullets[0].chars().count(), MAX_ITEM_CHARS);
    }

    #[test]
    fn blank_bullets_are_dropped_and_texts_trimmed() {
        let answer = r#"{"title":"  T  ","subtitle":"   ","slides":[{"title":" S ","bullets":["  a  ","","   ","b"]}]}"#;
        let deck = parse_deck(answer).unwrap();
        assert_eq!(deck.title, "T");
        assert_eq!(deck.subtitle, None);
        assert_eq!(deck.slides[0].title, "S");
        assert_eq!(deck.slides[0].bullets, ["a", "b"]);
    }

    #[test]
    fn structural_problems_are_errors_with_a_reason() {
        for (answer, needle) in [
            ("no json here", "JSON 객체"),
            ("{not json}", "스키마"),
            (r#"{"slides":[]}"#, "스키마"),
            (r#"{"title":"T","slides":[]}"#, "slides 가 비어"),
            (
                r#"{"title":" ","slides":[{"title":"s","bullets":["b"]}]}"#,
                "title 이 비어",
            ),
            (
                r#"{"title":"T","slides":[{"title":"","bullets":["b"]}]}"#,
                "slides[0].title",
            ),
            (
                r#"{"title":"T","slides":[{"title":"s","bullets":[]}]}"#,
                "slides[0].bullets",
            ),
            (
                r#"{"title":"T","slides":[{"title":"s","bullets":["ok"]},{"title":"t","bullets":[" "]}]}"#,
                "slides[1].bullets",
            ),
            (r#"{"title":"T","slides":"nope"}"#, "스키마"),
        ] {
            let err = parse_deck(answer).unwrap_err();
            assert!(err.contains(needle), "{answer}: {err}");
        }
    }
}
