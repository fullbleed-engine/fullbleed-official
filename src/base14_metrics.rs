//! Standard 14 font metrics for PDF text positions and inline word advances.
//!
//! This module supplies advances and character mappings, not font programs.
//! Inline word fragments also use these advances to match whole-string painting.

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
    data::font(name)
}
