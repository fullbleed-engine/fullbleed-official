use std::collections::BTreeMap;

/// Missing-character diagnostics collected while a document is rendered.
///
/// Obtain this alongside PDF bytes with
/// [`crate::FullBleed::render_with_glyph_report`]. Rendering can succeed even when
/// the report is nonempty; applications decide whether to reject that output.
#[derive(Debug, Clone, Default)]
pub struct GlyphCoverageReport {
    missing: BTreeMap<u32, MissingGlyph>,
}

/// A missing Unicode character and the font fallbacks attempted for it.
#[derive(Debug, Clone)]
pub struct MissingGlyph {
    /// Unicode scalar value as an integer, suitable for formatting as `U+XXXX`.
    pub codepoint: u32,
    /// The character itself.
    pub ch: char,
    /// Font names recorded for the first missing occurrence of this character.
    pub fonts_tried: Vec<String>,
    /// Number of missing-character observations accumulated during rendering.
    pub count: usize,
}

impl GlyphCoverageReport {
    pub fn record_missing(&mut self, ch: char, fonts_tried: Vec<String>) {
        let codepoint = ch as u32;
        let entry = self.missing.entry(codepoint).or_insert(MissingGlyph {
            codepoint,
            ch,
            fonts_tried,
            count: 0,
        });
        entry.count = entry.count.saturating_add(1);
    }

    pub fn merge(&mut self, other: GlyphCoverageReport) {
        for (codepoint, missing) in other.missing {
            let entry = self.missing.entry(codepoint).or_insert(MissingGlyph {
                codepoint,
                ch: missing.ch,
                fonts_tried: missing.fonts_tried.clone(),
                count: 0,
            });
            entry.count = entry.count.saturating_add(missing.count);
        }
    }

    /// Clone the missing-character entries in ascending Unicode codepoint order.
    pub fn missing(&self) -> Vec<MissingGlyph> {
        self.missing.values().cloned().collect()
    }

    /// Whether no missing characters were recorded.
    ///
    /// This is a coverage check, not proof of the intended typeface, layout
    /// quality, accessibility, or PDF conformance.
    pub fn is_empty(&self) -> bool {
        self.missing.is_empty()
    }
}
