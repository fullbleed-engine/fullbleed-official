//! Standard 14 font metrics for PDF text positions and document layout.
//!
//! This module supplies advances and character mappings, not font programs.
//! Paragraphs and inline fragments share advances with the PDF writer.

use crate::types::Pt;

#[path = "base14_metrics_data.rs"]
mod data;

pub(crate) struct GlyphMetric {
    name: &'static str,
    pub(crate) text: &'static str,
    pub(crate) width: u16,
}

#[derive(Clone, Copy)]
pub(crate) struct Base14Font {
    glyphs: &'static [GlyphMetric],
    unicode: &'static [(u32, u16)],
    builtin: &'static [u16; 256],
}

impl Base14Font {
    pub(crate) fn glyph_by_name(self, name: &[u8]) -> Option<&'static GlyphMetric> {
        let index = self
            .glyphs
            .binary_search_by(|glyph| glyph.name.as_bytes().cmp(name))
            .ok()?;
        self.glyphs.get(index)
    }

    pub(crate) fn glyph_by_unicode(self, ch: char) -> Option<&'static GlyphMetric> {
        // These WinAnsi aliases use the ordinary space and hyphen glyphs.
        let ch = match ch {
            '\u{00a0}' => ' ',
            '\u{00ad}' => '-',
            other => other,
        };
        let index = self
            .unicode
            .binary_search_by_key(&(ch as u32), |pair| pair.0)
            .ok()?;
        self.glyphs.get(usize::from(self.unicode[index].1))
    }

    pub(crate) fn builtin_glyph(self, code: u8) -> Option<&'static GlyphMetric> {
        let index = self.builtin[usize::from(code)].checked_sub(1)?;
        self.glyphs.get(usize::from(index))
    }
}

pub(crate) fn font(name: &str) -> Option<Base14Font> {
    data::font(name).or_else(|| data::font(canonical_name(name)?))
}

pub(crate) fn canonical_name(name: &str) -> Option<&'static str> {
    let name = name.trim().trim_matches('"').trim_matches('\'');
    [
        "Courier",
        "Courier-Bold",
        "Courier-Oblique",
        "Courier-BoldOblique",
        "Helvetica",
        "Helvetica-Bold",
        "Helvetica-Oblique",
        "Helvetica-BoldOblique",
        "Times-Roman",
        "Times-Bold",
        "Times-Italic",
        "Times-BoldItalic",
        "Symbol",
        "ZapfDingbats",
    ]
    .into_iter()
    .find(|candidate| candidate.eq_ignore_ascii_case(name))
}

pub(crate) fn measure_text_width(name: &str, size: Pt, text: &str) -> Option<Pt> {
    let name = canonical_name(name)?;
    let font = font(name)?;
    let symbolic = matches!(name, "Symbol" | "ZapfDingbats");
    let mut width = Pt::ZERO;
    for ch in text.chars() {
        let encoded = crate::pdf_encodings::encode_win_ansi_char(ch);
        for &byte in encoded.bytes() {
            // Symbol and ZapfDingbats retain their built-in code-to-glyph
            // mapping. The other Standard 14 fonts use WinAnsi glyph names.
            let glyph = if symbolic {
                font.builtin_glyph(byte)
            } else {
                crate::pdf_encodings::win_ansi_encoding_byte(byte)
                    .and_then(|ch| font.glyph_by_unicode(ch))
            };
            if let Some(glyph) = glyph {
                // Match the engine's per-glyph fixed-point precision. Summing
                // first and rounding later changes inline fragment positions.
                width += size.mul_ratio(i32::from(glyph.width), 1000);
            }
        }
    }
    Some(width)
}
