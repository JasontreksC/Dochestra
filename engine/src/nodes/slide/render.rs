//! Renders a [`SlideDeck`] to a 16:9 PDF with an embedded Korean font.

use std::path::Path;

use printpdf::{
    Color, Mm, Op, PaintMode, ParsedFont, PdfDocument, PdfFontHandle, PdfPage, PdfSaveOptions,
    Point, Pt, RawImage, Rect, Rgb, TextItem, XObjectId, XObjectTransform,
};

use super::spec::SlideDeck;

pub const REGULAR_FONT_FILE: &str = "NanumGothic-Regular.ttf";
pub const BOLD_FONT_FILE: &str = "NanumGothic-Bold.ttf";

/// 16:9 page, 254 x 142.875 mm = 720 x 405 pt.
const PAGE_W: f32 = 720.0;
const PAGE_H: f32 = 405.0;
const PAGE_W_MM: f32 = 254.0;
const PAGE_H_MM: f32 = 142.875;
const MARGIN: f32 = 48.0;
const FOOTER_Y: f32 = 22.0;
const LINE_HEIGHT: f32 = 1.35;
/// Title color (the PDF type color of CONTEXT §3.2, `#E5484D`).
const TITLE_RGB: (f32, f32, f32) = (229.0 / 255.0, 72.0 / 255.0, 77.0 / 255.0);
const BODY_RGB: (f32, f32, f32) = (0.13, 0.13, 0.15);
const MUTED_RGB: (f32, f32, f32) = (0.45, 0.45, 0.5);
/// A picture is never enlarged beyond this factor (1 px = 1 pt at 1x): a 144 x 26 px chart blown
/// up 4.7x to fill the closing slide was a pixelated smear (live check), so it stays smaller and centered.
const MAX_IMAGE_UPSCALE: f32 = 2.0;

#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("폰트를 읽을 수 없습니다 ({path}): {source}")]
    FontRead {
        path: String,
        source: std::io::Error,
    },
    #[error("폰트를 해석할 수 없습니다: {0}")]
    FontParse(String),
}

/// A parsed font with the metrics needed for layout.
struct Face {
    font: ParsedFont,
}

impl Face {
    fn load(dir: &Path, file: &str) -> Result<Self, RenderError> {
        let path = dir.join(file);
        let bytes = std::fs::read(&path).map_err(|source| RenderError::FontRead {
            path: path.display().to_string(),
            source,
        })?;
        let font = ParsedFont::from_bytes(&bytes, 0, &mut Vec::new())
            .ok_or_else(|| RenderError::FontParse(path.display().to_string()))?;
        Ok(Self { font })
    }

    fn has_glyph(&self, c: char) -> bool {
        self.font.lookup_glyph_index(c as u32).is_some()
    }

    /// Advance width of one character in points at `size`.
    fn char_width(&self, c: char, size: f32) -> f32 {
        let units = f32::from(self.font.units_per_em.max(1));
        self.font
            .lookup_glyph_index(c as u32)
            .and_then(|gid| self.font.get_glyph_width(gid))
            .map_or(size * 0.5, |w| f32::from(w) * size / units)
    }

    fn text_width(&self, text: &str, size: f32) -> f32 {
        text.chars().map(|c| self.char_width(c, size)).sum()
    }

    /// Replaces control characters by spaces and drops characters the font cannot draw.
    fn sanitize(&self, text: &str) -> String {
        text.chars()
            .filter_map(|c| {
                if c.is_control() || c == '\u{a0}' {
                    Some(' ')
                } else if c == ' ' || self.has_glyph(c) {
                    Some(c)
                } else {
                    None
                }
            })
            .collect()
    }
}

/// Greedy word wrap of `text` into lines no wider than `max_width` at `size`. Words wider than a
/// line are broken between characters (needed for CJK text without spaces).
fn wrap(face: &Face, text: &str, size: f32, max_width: f32) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut line = String::new();
    let mut line_w = 0.0f32;
    let space_w = face.char_width(' ', size);

    for word in text.split_whitespace() {
        let word_w = face.text_width(word, size);
        let gap = if line.is_empty() { 0.0 } else { space_w };
        if !line.is_empty() && line_w + gap + word_w <= max_width {
            line.push(' ');
            line.push_str(word);
            line_w += gap + word_w;
            continue;
        }
        if word_w <= max_width {
            if !line.is_empty() {
                lines.push(std::mem::take(&mut line));
            }
            line = word.to_owned();
            line_w = word_w;
            continue;
        }
        // Too long for any line: continue on the current line and break between characters
        // (needed for CJK text without spaces).
        if !line.is_empty() {
            line.push(' ');
            line_w += space_w;
        }
        for c in word.chars() {
            let w = face.char_width(c, size);
            if line_w + w > max_width && !line.is_empty() {
                lines.push(std::mem::take(&mut line));
                line_w = 0.0;
            }
            line.push(c);
            line_w += w;
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

fn color((r, g, b): (f32, f32, f32)) -> Color {
    Color::Rgb(Rgb::new(r, g, b, None))
}

struct Page {
    ops: Vec<Op>,
}

impl Page {
    fn new() -> Self {
        Self { ops: Vec::new() }
    }

    fn text(
        &mut self,
        font: &PdfFontHandle,
        size: f32,
        rgb: (f32, f32, f32),
        x: f32,
        y: f32,
        text: &str,
    ) {
        if text.trim().is_empty() {
            return;
        }
        self.ops.extend([
            Op::StartTextSection,
            Op::SetFont {
                font: font.clone(),
                size: Pt(size),
            },
            Op::SetFillColor { col: color(rgb) },
            Op::SetTextCursor {
                pos: Point { x: Pt(x), y: Pt(y) },
            },
            Op::ShowText {
                items: vec![TextItem::Text(text.to_owned())],
            },
            Op::EndTextSection,
        ]);
    }

    fn rect(&mut self, rgb: (f32, f32, f32), x: f32, y: f32, w: f32, h: f32) {
        self.ops.push(Op::SetFillColor { col: color(rgb) });
        self.ops.push(Op::DrawRectangle {
            rectangle: Rect {
                x: Pt(x),
                y: Pt(y),
                width: Pt(w),
                height: Pt(h),
                mode: Some(PaintMode::Fill),
                winding_order: None,
            },
        });
    }

    /// Places `image` inside the box (x, y = bottom-left), keeping its aspect ratio, centered and
    /// enlarged at most [`MAX_IMAGE_UPSCALE`] times.
    fn image(&mut self, image: &PlacedImage, x: f32, y: f32, w: f32, h: f32) {
        let scale = (w / image.width)
            .min(h / image.height)
            .min(MAX_IMAGE_UPSCALE);
        let (draw_w, draw_h) = (image.width * scale, image.height * scale);
        self.ops.push(Op::UseXobject {
            id: image.id.clone(),
            transform: XObjectTransform {
                translate_x: Some(Pt(x + (w - draw_w) / 2.0)),
                translate_y: Some(Pt(y + (h - draw_h) / 2.0)),
                scale_x: Some(scale),
                scale_y: Some(scale),
                dpi: Some(72.0), // 1 pixel = 1 pt before scaling
                ..Default::default()
            },
        });
    }
}

struct PlacedImage {
    id: XObjectId,
    width: f32,
    height: f32,
}

/// Largest font size from `sizes` at which `text` wraps into at most `max_lines` lines; with
/// none fitting, the smallest size and its lines cut to `max_lines` (last line ends with `…`).
fn fit_lines(
    face: &Face,
    text: &str,
    sizes: &[f32],
    max_width: f32,
    max_lines: usize,
) -> (f32, Vec<String>) {
    for &size in sizes {
        let lines = wrap(face, text, size, max_width);
        if lines.len() <= max_lines {
            return (size, lines);
        }
    }
    let size = sizes.last().copied().unwrap_or(12.0);
    let mut lines = wrap(face, text, size, max_width);
    cut_with_ellipsis(face, &mut lines, size, max_width, max_lines);
    (size, lines)
}

fn cut_with_ellipsis(
    face: &Face,
    lines: &mut Vec<String>,
    size: f32,
    max_width: f32,
    max_lines: usize,
) {
    if lines.len() <= max_lines {
        return;
    }
    lines.truncate(max_lines);
    if let Some(last) = lines.last_mut() {
        let ellipsis = if face.has_glyph('…') { "…" } else { "..." };
        while !last.is_empty() && face.text_width(&format!("{last}{ellipsis}"), size) > max_width {
            last.pop();
        }
        last.push_str(ellipsis);
    }
}

/// Renders the deck. Pages: cover, one per content slide, and (with an image) a final full-page
/// image slide.
pub fn render_deck(
    deck: &SlideDeck,
    image: Option<&[u8]>,
    font_dir: &Path,
) -> Result<Vec<u8>, RenderError> {
    let regular = Face::load(font_dir, REGULAR_FONT_FILE)?;
    let bold = Face::load(font_dir, BOLD_FONT_FILE)?;

    let mut doc = PdfDocument::new(&bold.sanitize(&deck.title));
    let regular_h = PdfFontHandle::External(doc.add_font(&regular.font));
    let bold_h = PdfFontHandle::External(doc.add_font(&bold.font));

    // An undecodable picture is skipped rather than failing the whole deck.
    let placed = image.and_then(|bytes| match RawImage::decode_from_bytes(bytes, &mut Vec::new()) {
        Ok(raw) if raw.width > 0 && raw.height > 0 => {
            let (width, height) = (raw.width as f32, raw.height as f32);
            Some(PlacedImage { id: doc.add_image(&raw), width, height })
        }
        Ok(_) => None,
        Err(err) => {
            tracing::warn!(error = %err, "slide image could not be decoded; rendering without it");
            None
        }
    });

    let total = 1 + deck.slides.len() + usize::from(placed.is_some());
    let mut pages: Vec<Page> = Vec::with_capacity(total);

    // --- cover
    {
        let mut page = Page::new();
        let text_w = if placed.is_some() {
            400.0
        } else {
            PAGE_W - 2.0 * MARGIN
        };
        let title = bold.sanitize(&deck.title);
        let (size, lines) = fit_lines(&bold, &title, &[42.0, 36.0, 30.0, 26.0], text_w, 4);
        let block_h = lines.len() as f32 * size * LINE_HEIGHT;
        let mut y = PAGE_H / 2.0 + block_h / 2.0 + 20.0 - size;
        page.rect(TITLE_RGB, MARGIN, y + size * 1.2, 72.0, 5.0);
        for line in &lines {
            page.text(&bold_h, size, TITLE_RGB, MARGIN, y, line);
            y -= size * LINE_HEIGHT;
        }
        if let Some(sub) = &deck.subtitle {
            let sub = regular.sanitize(sub);
            let (sub_size, sub_lines) = fit_lines(&regular, &sub, &[22.0, 18.0, 16.0], text_w, 3);
            y -= 10.0;
            for line in &sub_lines {
                page.text(&regular_h, sub_size, MUTED_RGB, MARGIN, y, line);
                y -= sub_size * LINE_HEIGHT;
            }
        }
        if let Some(img) = &placed {
            page.image(img, 470.0, 70.0, PAGE_W - 470.0 - MARGIN, PAGE_H - 140.0);
        }
        pages.push(page);
    }

    // --- content slides
    let bullet_glyph = if regular.has_glyph('•') { "•" } else { "-" };
    for slide in &deck.slides {
        let mut page = Page::new();
        let width = PAGE_W - 2.0 * MARGIN;

        let title = bold.sanitize(&slide.title);
        let (title_size, title_lines) = fit_lines(&bold, &title, &[30.0, 26.0, 22.0], width, 2);
        let mut y = PAGE_H - MARGIN - title_size * 0.8;
        for line in &title_lines {
            page.text(&bold_h, title_size, TITLE_RGB, MARGIN, y, line);
            y -= title_size * LINE_HEIGHT;
        }
        let rule_y = y + title_size * LINE_HEIGHT - title_size * 0.45;
        page.rect(TITLE_RGB, MARGIN, rule_y, width, 2.0);

        // Bullet area: below the title rule down to the footer.
        let top = rule_y - 22.0;
        let bottom = FOOTER_Y + 26.0;
        let indent = 22.0;
        let bullets: Vec<String> = slide.bullets.iter().map(|b| regular.sanitize(b)).collect();

        let layout = |size: f32| -> Vec<Vec<String>> {
            bullets
                .iter()
                .map(|b| wrap(&regular, b, size, width - indent))
                .collect()
        };
        let height_of = |size: f32, wrapped: &[Vec<String>]| -> f32 {
            let lines: usize = wrapped.iter().map(Vec::len).sum();
            lines as f32 * size * LINE_HEIGHT + wrapped.len().saturating_sub(1) as f32 * size * 0.5
        };
        let sizes = [24.0f32, 22.0, 20.0, 18.0, 16.0, 14.0, 12.0];
        let mut chosen = (sizes[sizes.len() - 1], layout(sizes[sizes.len() - 1]));
        for &size in &sizes {
            let wrapped = layout(size);
            if height_of(size, &wrapped) <= top - bottom {
                chosen = (size, wrapped);
                break;
            }
        }
        let (size, mut wrapped) = chosen;
        // Still too tall at the smallest size: drop lines from the end (last visible gets "…").
        let max_lines = (((top - bottom) / (size * LINE_HEIGHT)).floor() as usize).max(1);
        let mut flat: Vec<(usize, String)> = Vec::new();
        for (i, lines) in wrapped.drain(..).enumerate() {
            flat.extend(lines.into_iter().map(|l| (i, l)));
        }
        if flat.len() > max_lines {
            flat.truncate(max_lines);
            if let Some((_, last)) = flat.last_mut() {
                let mut lines = vec![std::mem::take(last)];
                lines.push(String::new());
                cut_with_ellipsis(&regular, &mut lines, size, width - indent, 1);
                *last = lines.swap_remove(0);
            }
        }

        let mut y = top - size;
        let mut previous_bullet: Option<usize> = None;
        for (bullet_index, line) in &flat {
            if previous_bullet != Some(*bullet_index) {
                if previous_bullet.is_some() {
                    y -= size * 0.5;
                }
                page.text(&regular_h, size, TITLE_RGB, MARGIN, y, bullet_glyph);
                previous_bullet = Some(*bullet_index);
            }
            page.text(&regular_h, size, BODY_RGB, MARGIN + indent, y, line);
            y -= size * LINE_HEIGHT;
        }
        pages.push(page);
    }

    // --- closing image slide
    if let Some(img) = &placed {
        let mut page = Page::new();
        page.image(
            img,
            MARGIN / 2.0,
            MARGIN,
            PAGE_W - MARGIN,
            PAGE_H - 2.0 * MARGIN,
        );
        pages.push(page);
    }

    // --- page numbers
    let count = pages.len();
    let pdf_pages: Vec<PdfPage> = pages
        .into_iter()
        .enumerate()
        .map(|(i, mut page)| {
            let label = format!("{} / {count}", i + 1);
            let x = PAGE_W - MARGIN - regular.text_width(&label, 11.0);
            page.text(&regular_h, 11.0, MUTED_RGB, x, FOOTER_Y, &label);
            PdfPage::new(Mm(PAGE_W_MM), Mm(PAGE_H_MM), page.ops)
        })
        .collect();

    let mut warnings = Vec::new();
    let bytes = doc
        .with_pages(pdf_pages)
        .save(&PdfSaveOptions::default(), &mut warnings);
    for warning in &warnings {
        tracing::debug!(?warning, "pdf save warning");
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn regular() -> Face {
        let dir = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/fonts"));
        Face::load(dir, REGULAR_FONT_FILE).expect("bundled font loads")
    }

    #[test]
    fn short_text_stays_on_one_line() {
        let face = regular();
        assert_eq!(wrap(&face, "Hello world", 20.0, 600.0), ["Hello world"]);
        assert!(wrap(&face, "   ", 20.0, 600.0).is_empty());
    }

    #[test]
    fn lines_break_at_spaces_and_never_exceed_the_width() {
        let face = regular();
        let text =
            "매출은 전년 대비 크게 증가했으며 해외 시장 비중이 처음으로 삼분의 일을 넘었습니다";
        let lines = wrap(&face, text, 24.0, 300.0);
        assert!(lines.len() >= 2);
        assert!(
            lines
                .iter()
                .all(|l| face.text_width(l, 24.0) <= 300.0 + 0.01),
            "{lines:?}"
        );
        assert_eq!(lines.join(" "), text);
    }

    #[test]
    fn an_overlong_word_continues_on_the_current_line() {
        let face = regular();
        let word = "가".repeat(60);
        let lines = wrap(&face, &format!("0 {word}"), 20.0, 300.0);
        // "0" is not left alone on its own line.
        assert!(lines[0].starts_with("0 가"), "{lines:?}");
        assert!(
            lines
                .iter()
                .all(|l| face.text_width(l, 20.0) <= 300.0 + 0.01),
            "{lines:?}"
        );
        assert_eq!(lines.concat().replace(' ', ""), format!("0{word}"));
    }

    #[test]
    fn cutting_adds_an_ellipsis_within_the_width() {
        let face = regular();
        let mut lines = wrap(&face, &"가나다라마바사 ".repeat(40), 20.0, 300.0);
        assert!(lines.len() > 3);
        cut_with_ellipsis(&face, &mut lines, 20.0, 300.0, 3);
        assert_eq!(lines.len(), 3);
        assert!(lines[2].ends_with('…') || lines[2].ends_with("..."));
        assert!(face.text_width(&lines[2], 20.0) <= 300.0 + 0.01);
    }

    #[test]
    fn unsupported_characters_are_dropped_and_controls_become_spaces() {
        let face = regular();
        assert_eq!(face.sanitize("a🚀b\tc\nd"), "ab c d");
        assert!(face.has_glyph('가') && face.has_glyph('A') && face.has_glyph('•'));
    }
}
