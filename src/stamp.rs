//! Text overlays: watermarks and page numbers / headers / footers.

use crate::pdf::{self, page_box};
use anyhow::{Result, bail};
use lopdf::{Document, ObjectId, dictionary};

/// Where a page-number stamp sits on the page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Position {
    TopLeft,
    TopCenter,
    TopRight,
    BottomLeft,
    BottomCenter,
    BottomRight,
}

impl Position {
    pub fn parse(value: &str) -> Result<Self> {
        Ok(match value.to_ascii_lowercase().replace('_', "-").as_str() {
            "" | "bottom-center" => Self::BottomCenter,
            "bottom-left" => Self::BottomLeft,
            "bottom-right" => Self::BottomRight,
            "top-center" => Self::TopCenter,
            "top-left" => Self::TopLeft,
            "top-right" => Self::TopRight,
            other => bail!("unknown position '{other}' (use top-left, top-center, top-right, bottom-left, bottom-center or bottom-right)"),
        })
    }
}

/// Approximate width of Helvetica text, good enough for alignment.
fn text_width(text: &str, size: f32) -> f32 {
    text.chars().count() as f32 * size * 0.55
}

/// Escape text for a PDF literal string. Non-ASCII characters become `?` (Helvetica is WinAnsi only).
fn pdf_escape(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_ascii() { c } else { '?' })
        .flat_map(|c| match c {
            '\\' => vec!['\\', '\\'],
            '(' => vec!['\\', '('],
            ')' => vec!['\\', ')'],
            other => vec![other],
        })
        .collect()
}

/// Register a shared Helvetica font and a transparency state on the document, returning their ids.
fn shared_resources(doc: &mut Document, opacity: f32) -> (ObjectId, ObjectId) {
    let font = doc.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => "Helvetica",
        "Encoding" => "WinAnsiEncoding",
    });
    let gs = doc.add_object(dictionary! {
        "Type" => "ExtGState",
        "CA" => opacity,
        "ca" => opacity,
    });
    (font, gs)
}

/// Draw one text string onto a page with a single content-stream operator sequence.
fn draw_text(doc: &mut Document, page: ObjectId, font: ObjectId, gs: ObjectId, text: &str, size: f32, angle_deg: f32, x: f32, y: f32, grey: f32) -> Result<()> {
    let radians = angle_deg.to_radians();
    let (sin, cos) = radians.sin_cos();
    let content = format!(
        "q /PVGs gs BT /PVFont {size:.2} Tf {grey:.3} g {cos:.5} {sin:.5} {:.5} {cos:.5} {x:.2} {y:.2} Tm ({}) Tj ET Q\n",
        -sin,
        pdf_escape(text)
    );
    pdf::add_resource(doc, page, b"Font", "PVFont", font)?;
    pdf::add_resource(doc, page, b"ExtGState", "PVGs", gs)?;
    doc.add_page_contents(page, content.into_bytes())?;
    Ok(())
}

/// Diagonal (or horizontal) text watermark across the centre of each selected page.
pub fn watermark(bytes: &[u8], text: &str, size: f32, opacity: f32, angle: f32, selected: Option<&[u32]>) -> Result<Vec<u8>> {
    if text.trim().is_empty() {
        bail!("watermark text is required");
    }
    let mut doc = pdf::load(bytes)?;
    let (font, gs) = shared_resources(&mut doc, opacity.clamp(0.0, 1.0));
    for (index, page) in pdf::page_ids(&doc).into_iter().enumerate() {
        if selected.is_some_and(|s| !s.contains(&(index as u32 + 1))) {
            continue;
        }
        let [x0, y0, x1, y1] = page_box(&doc, page);
        let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
        // Start the text half its width back from the centre so it is centred on the page.
        let half = text_width(text, size) / 2.0;
        let radians = angle.to_radians();
        let x = cx - half * radians.cos();
        let y = cy - half * radians.sin();
        draw_text(&mut doc, page, font, gs, text, size, angle, x, y, 0.5)?;
    }
    pdf::save(&mut doc)
}

/// Stamp page numbers. `template` supports `{n}` (this page), `{total}` (page count) and
/// `{start}`-based numbering via `start`.
pub fn page_numbers(bytes: &[u8], template: &str, position: Position, start: u32, size: f32, selected: Option<&[u32]>) -> Result<Vec<u8>> {
    let mut doc = pdf::load(bytes)?;
    let pages = pdf::page_ids(&doc);
    let total = pages.len() as u32;
    let (font, gs) = shared_resources(&mut doc, 1.0);
    let template = if template.trim().is_empty() { "{n}" } else { template };
    for (index, page) in pages.into_iter().enumerate() {
        let number = index as u32 + 1;
        if selected.is_some_and(|s| !s.contains(&number)) {
            continue;
        }
        let shown = (start + index as u32).to_string();
        let text = template.replace("{n}", &shown).replace("{total}", &(start + total - 1).to_string());
        let [x0, y0, x1, y1] = page_box(&doc, page);
        let width = text_width(&text, size);
        let margin = 36.0;
        let x = match position {
            Position::TopLeft | Position::BottomLeft => x0 + margin,
            Position::TopCenter | Position::BottomCenter => (x0 + x1 - width) / 2.0,
            Position::TopRight | Position::BottomRight => x1 - margin - width,
        };
        let y = match position {
            Position::TopLeft | Position::TopCenter | Position::TopRight => y1 - margin,
            _ => y0 + margin / 2.0,
        };
        draw_text(&mut doc, page, font, gs, &text, size, 0.0, x, y, 0.2)?;
    }
    pdf::save(&mut doc)
}
