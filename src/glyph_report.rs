use std::collections::BTreeMap;

#[derive(Debug, Clone, Default)]
pub struct GlyphCoverageReport {
    missing: BTreeMap<u32, MissingGlyph>,
}

#[derive(Debug, Clone)]
pub struct MissingGlyph {
    pub codepoint: u32,
    pub ch: char,
    pub fonts_tried: Vec<String>,
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

    pub fn missing(&self) -> Vec<MissingGlyph> {
        self.missing.values().cloned().collect()
    }

    pub fn is_empty(&self) -> bool {
        self.missing.is_empty()
    }
}
