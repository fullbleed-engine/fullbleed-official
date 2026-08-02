use crate::error::FullBleedError;
use crate::glyph_report::GlyphCoverageReport;
use crate::sfnt::{self, CmapSubtable, Face as SfntFace, GlyphId, PlatformId};
use crate::text_shape;
use crate::types::Pt;
use std::collections::{HashMap, VecDeque};
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
struct TextWidthKey {
    font_index: usize,
    size_milli: i64,
    text: String,
}

#[derive(Debug)]
struct TextWidthCache {
    map: HashMap<TextWidthKey, Pt>,
    order: VecDeque<TextWidthKey>,
    max_entries: usize,
}

impl TextWidthCache {
    fn new(max_entries: usize) -> Self {
        Self {
            map: HashMap::new(),
            order: VecDeque::new(),
            max_entries,
        }
    }

    fn get(&mut self, key: &TextWidthKey) -> Option<Pt> {
        self.map.get(key).copied()
    }

    fn insert(&mut self, key: TextWidthKey, value: Pt) {
        if self.map.contains_key(&key) {
            return;
        }
        self.map.insert(key.clone(), value);
        self.order.push_back(key);
        while self.map.len() > self.max_entries {
            if let Some(old) = self.order.pop_front() {
                self.map.remove(&old);
            } else {
                break;
            }
        }
    }
}

#[derive(Debug)]
pub(crate) struct FontRegistry {
    fonts: Vec<RegisteredFont>,
    lookup: HashMap<String, usize>,
    use_full_unicode_metrics: bool,
    text_width_cache: Mutex<TextWidthCache>,
}

#[derive(Debug, Clone)]
pub(crate) struct FontRun {
    pub font_name: Arc<str>,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RegisteredFontSourceKind {
    Directory,
    File,
    BundleAsset,
    Bytes,
}

impl RegisteredFontSourceKind {
    #[cfg(feature = "python")]
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Directory => "directory",
            Self::File => "file",
            Self::BundleAsset => "bundle",
            Self::Bytes => "bytes",
        }
    }
}

#[derive(Debug, Clone)]
#[cfg_attr(not(feature = "python"), allow(dead_code))]
pub(crate) struct RegisteredFontSourceInfo {
    pub(crate) kind: RegisteredFontSourceKind,
    pub(crate) identifier: String,
}

#[derive(Debug)]
pub(crate) struct RegisteredFont {
    pub(crate) name: String,
    pub(crate) data: Vec<u8>,
    pub(crate) metrics: FontMetrics,
    pub(crate) program_kind: FontProgramKind,
    #[cfg_attr(not(feature = "python"), allow(dead_code))]
    pub(crate) source: RegisteredFontSourceInfo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FontProgramKind {
    TrueType,
    OpenTypeCff,
}

impl FontProgramKind {
    #[cfg(feature = "python")]
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::TrueType => "truetype",
            Self::OpenTypeCff => "opentype_cff",
        }
    }
}

#[cfg(feature = "python")]
#[derive(Debug, Clone)]
pub(crate) struct RegisteredFontTrace {
    pub(crate) resolved_name: String,
    pub(crate) source_kind: RegisteredFontSourceKind,
    pub(crate) source_identifier: String,
    pub(crate) source_file_name: Option<String>,
    pub(crate) program_kind: FontProgramKind,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct DecorationMetrics {
    pub(crate) position: i16,
    pub(crate) thickness: i16,
}

#[derive(Debug)]
pub(crate) struct FontMetrics {
    pub(crate) first_char: u8,
    pub(crate) last_char: u8,
    pub(crate) widths: Vec<u16>,
    pub(crate) glyph_ids: Vec<u16>,
    pub(crate) ascent: i16,
    pub(crate) descent: i16,
    pub(crate) line_gap: i16,
    pub(crate) cap_height: i16,
    pub(crate) italic_angle: i16,
    pub(crate) stem_v: i16,
    pub(crate) bbox: (i16, i16, i16, i16),
    pub(crate) underline_metrics: Option<DecorationMetrics>,
    pub(crate) strikeout_metrics: Option<DecorationMetrics>,
    pub(crate) missing_width: u16,
    pub(crate) is_fixed_pitch: bool,
    pub(crate) kerning: HashMap<(u16, u16), i16>,
    symbolic: bool,
}

impl FontRegistry {
    pub(crate) fn new() -> Self {
        Self {
            fonts: Vec::new(),
            lookup: HashMap::new(),
            use_full_unicode_metrics: true,
            text_width_cache: Mutex::new(TextWidthCache::new(20_000)),
        }
    }

    pub(crate) fn set_use_full_unicode_metrics(&mut self, enabled: bool) {
        self.use_full_unicode_metrics = enabled;
    }

    pub(crate) fn register_dir(&mut self, path: impl AsRef<Path>) {
        let path = path.as_ref();
        let Ok(entries) = fs::read_dir(path) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                self.register_file_with_source(path.as_path(), RegisteredFontSourceKind::Directory);
            }
        }
    }

    pub(crate) fn register_file(&mut self, path: impl AsRef<Path>) {
        let path = path.as_ref();
        self.register_file_with_source(path, RegisteredFontSourceKind::File);
    }

    fn register_file_with_source(&mut self, path: &Path, source_kind: RegisteredFontSourceKind) {
        let Some(ext) = path.extension().and_then(|v| v.to_str()) else {
            return;
        };
        let ext = ext.to_ascii_lowercase();
        if ext != "ttf" && ext != "otf" {
            return;
        }
        let Ok(data) = fs::read(path) else {
            return;
        };
        let Ok(face) = SfntFace::parse(&data, 0) else {
            return;
        };

        let (name, aliases) = font_names(&face, path);
        let (metrics, program_kind) = FontMetrics::from_face(&face);
        let index = self.fonts.len();
        self.fonts.push(RegisteredFont {
            name: name.clone(),
            data,
            metrics,
            program_kind,
            source: RegisteredFontSourceInfo {
                kind: source_kind,
                identifier: path.to_string_lossy().to_string(),
            },
        });

        let mut all_aliases = Vec::new();
        all_aliases.push(name);
        all_aliases.extend(aliases);
        for alias in all_aliases {
            let key = normalize_name(&alias);
            if key.is_empty() || self.lookup.contains_key(&key) {
                continue;
            }
            self.lookup.insert(key, index);
        }
    }

    pub(crate) fn register_bytes(
        &mut self,
        data: Vec<u8>,
        source_name: Option<&str>,
    ) -> Result<String, FullBleedError> {
        self.register_bytes_with_source_kind(data, source_name, RegisteredFontSourceKind::Bytes)
    }

    pub(crate) fn register_bundle_font_bytes(
        &mut self,
        data: Vec<u8>,
        source_name: Option<&str>,
    ) -> Result<String, FullBleedError> {
        self.register_bytes_with_source_kind(
            data,
            source_name,
            RegisteredFontSourceKind::BundleAsset,
        )
    }

    fn register_bytes_with_source_kind(
        &mut self,
        data: Vec<u8>,
        source_name: Option<&str>,
        source_kind: RegisteredFontSourceKind,
    ) -> Result<String, FullBleedError> {
        let source = source_name.unwrap_or("EmbeddedFont");
        let Ok(face) = SfntFace::parse(&data, 0) else {
            return Err(FullBleedError::Asset(format!(
                "invalid font data for {source}"
            )));
        };

        let (name, aliases) = font_names(&face, Path::new(source));
        let (metrics, program_kind) = FontMetrics::from_face(&face);
        let index = self.fonts.len();
        self.fonts.push(RegisteredFont {
            name: name.clone(),
            data,
            metrics,
            program_kind,
            source: RegisteredFontSourceInfo {
                kind: source_kind,
                identifier: source.to_string(),
            },
        });

        let mut all_aliases = Vec::new();
        all_aliases.push(name.clone());
        all_aliases.extend(aliases);
        for alias in all_aliases {
            let key = normalize_name(&alias);
            if key.is_empty() || self.lookup.contains_key(&key) {
                continue;
            }
            self.lookup.insert(key, index);
        }

        Ok(name)
    }

    pub(crate) fn resolve(&self, name: &str) -> Option<&RegisteredFont> {
        let key = normalize_name(name);
        self.lookup
            .get(&key)
            .and_then(|index| self.fonts.get(*index))
    }

    #[cfg(feature = "python")]
    pub(crate) fn resolve_trace(&self, name: &str) -> Option<RegisteredFontTrace> {
        let font = self.resolve(name)?;
        let source_path = Path::new(&font.source.identifier);
        let source_file_name = source_path
            .file_name()
            .and_then(|v| v.to_str())
            .map(|v| v.to_string())
            .or_else(|| {
                let trimmed = font.source.identifier.trim();
                if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed.to_string())
                }
            });
        Some(RegisteredFontTrace {
            resolved_name: font.name.clone(),
            source_kind: font.source.kind,
            source_identifier: font.source.identifier.clone(),
            source_file_name,
            program_kind: font.program_kind,
        })
    }

    pub(crate) fn measure_text_width(&self, name: &str, font_size: Pt, text: &str) -> Pt {
        let key = normalize_name(name);
        let Some(index) = self.lookup.get(&key).copied() else {
            let char_width = (font_size * 0.6).max(Pt::from_f32(1.0));
            return char_width * (text.chars().count() as i32);
        };
        let cache_key = TextWidthKey {
            font_index: index,
            size_milli: font_size.to_milli_i64(),
            text: text.to_string(),
        };
        if let Ok(mut cache) = self.text_width_cache.lock() {
            if let Some(value) = cache.get(&cache_key) {
                return value;
            }
        }
        let Some(font) = self.fonts.get(index) else {
            let char_width = (font_size * 0.6).max(Pt::from_f32(1.0));
            return char_width * (text.chars().count() as i32);
        };
        if !self.use_full_unicode_metrics {
            let value = font.metrics.measure_text_width(font_size, text);
            if let Ok(mut cache) = self.text_width_cache.lock() {
                cache.insert(cache_key, value);
            }
            return value;
        }
        if font.metrics.is_within_basic_latin(text) {
            let value = font.metrics.measure_text_width(font_size, text);
            if let Ok(mut cache) = self.text_width_cache.lock() {
                cache.insert(cache_key, value);
            }
            return value;
        }
        let value = measure_text_width_full(font, font_size, text)
            .unwrap_or_else(|| font.metrics.measure_text_width(font_size, text));
        if let Ok(mut cache) = self.text_width_cache.lock() {
            cache.insert(cache_key, value);
        }
        value
    }

    pub(crate) fn line_height(&self, name: &str, font_size: Pt, fallback: Pt) -> Pt {
        let Some(font) = self.resolve(name) else {
            return fallback;
        };
        font.metrics.line_height(font_size).max(fallback)
    }

    pub(crate) fn map_glyph_id_for_char(&self, name: &str, ch: char) -> u16 {
        let Some(font) = self.resolve(name) else {
            return 0;
        };
        if let Ok(face) = SfntFace::parse(&font.data, 0) {
            let (_symbolic, symbol_subtable) = select_symbol_subtable(&face);
            if let Some(gid) = glyph_index_for_codepoint(&face, ch as u32, symbol_subtable) {
                return gid.0;
            }
        }
        0
    }

    pub(crate) fn font_supports_char(&self, name: &str, ch: char) -> bool {
        let Some(font) = self.resolve(name) else {
            return false;
        };
        if let Ok(face) = SfntFace::parse(&font.data, 0) {
            let (_symbolic, symbol_subtable) = select_symbol_subtable(&face);
            return glyph_index_for_codepoint(&face, ch as u32, symbol_subtable).is_some();
        }
        false
    }

    pub(crate) fn split_text_by_fallbacks(
        &self,
        primary: &Arc<str>,
        fallbacks: &[Arc<str>],
        text: &str,
    ) -> Vec<FontRun> {
        let mut stack: Vec<Arc<str>> = Vec::with_capacity(1 + fallbacks.len());
        stack.push(primary.clone());
        stack.extend(fallbacks.iter().cloned());
        if stack.is_empty() {
            return vec![FontRun {
                font_name: Arc::<str>::from("Helvetica"),
                text: text.to_string(),
            }];
        }

        let mut runs: Vec<FontRun> = Vec::new();
        let mut current_font: Option<Arc<str>> = None;
        let mut buf = String::new();

        // Cache glyph support decisions per font index + char to avoid repeated lookups.
        let mut support_cache: HashMap<(usize, char), bool> = HashMap::new();

        for ch in text.chars() {
            let mut chosen: Option<Arc<str>> = None;
            for (idx, font_name) in stack.iter().enumerate() {
                let supported = support_cache
                    .entry((idx, ch))
                    .or_insert_with(|| self.font_supports_char(font_name, ch));
                if *supported {
                    chosen = Some(font_name.clone());
                    break;
                }
            }
            let chosen = chosen.unwrap_or_else(|| stack[0].clone());

            if current_font.as_ref() != Some(&chosen) {
                if !buf.is_empty() {
                    runs.push(FontRun {
                        font_name: current_font.take().unwrap(),
                        text: std::mem::take(&mut buf),
                    });
                }
                current_font = Some(chosen.clone());
            }
            buf.push(ch);
        }

        if !buf.is_empty() {
            runs.push(FontRun {
                font_name: current_font.unwrap_or_else(|| stack[0].clone()),
                text: buf,
            });
        }

        runs
    }

    pub(crate) fn measure_text_width_with_fallbacks(
        &self,
        primary: &Arc<str>,
        fallbacks: &[Arc<str>],
        font_size: Pt,
        text: &str,
    ) -> Pt {
        let runs = self.split_text_by_fallbacks(primary, fallbacks, text);
        let mut total = Pt::ZERO;
        for run in runs {
            total = total + self.measure_text_width(&run.font_name, font_size, &run.text);
        }
        total
    }

    pub(crate) fn report_missing_glyphs(
        &self,
        primary: &Arc<str>,
        fallbacks: &[Arc<str>],
        text: &str,
        report: &mut GlyphCoverageReport,
    ) {
        let mut stack: Vec<Arc<str>> = Vec::with_capacity(1 + fallbacks.len());
        stack.push(primary.clone());
        stack.extend(fallbacks.iter().cloned());
        if stack.is_empty() {
            return;
        }

        let mut resolved: Vec<Arc<str>> = Vec::new();
        for font_name in stack {
            if self.resolve(&font_name).is_some() {
                resolved.push(font_name);
            }
        }
        if resolved.is_empty() {
            // No registered fonts to validate against; skip reporting to avoid false positives.
            return;
        }

        let mut support_cache: HashMap<(usize, char), bool> = HashMap::new();

        for ch in text.chars() {
            if ch.is_ascii() {
                continue;
            }
            let mut supported = false;
            for (idx, font_name) in resolved.iter().enumerate() {
                let ok = support_cache
                    .entry((idx, ch))
                    .or_insert_with(|| self.font_supports_char(font_name, ch));
                if *ok {
                    supported = true;
                    break;
                }
            }
            if !supported {
                let fonts_tried = resolved.iter().map(|s| s.to_string()).collect::<Vec<_>>();
                report.record_missing(ch, fonts_tried);
            }
        }
    }

    pub(crate) fn glyph_advance(&self, name: &str, gid: u16) -> u16 {
        let Some(font) = self.resolve(name) else {
            return 0;
        };
        if let Ok(face) = SfntFace::parse(&font.data, 0) {
            let advance = face.glyph_hor_advance(GlyphId(gid)).unwrap_or(0);
            let units = face.units_per_em().max(1) as i64;
            let scaled = ((advance as i64) * 1000 + (units / 2)) / units;
            return scaled.clamp(0, u16::MAX as i64) as u16;
        }
        0
    }
}

impl FontMetrics {
    fn from_face(face: &SfntFace<'_>) -> (Self, FontProgramKind) {
        let units_per_em = face.units_per_em().max(1);
        let scale = 1000.0 / units_per_em as f32;
        let first_char = 32u8;
        let last_char = 255u8;
        let (symbolic, symbol_subtable) = select_symbol_subtable(face);
        let glyph_ids = build_glyph_ids(face, first_char, last_char, symbol_subtable);
        let widths = build_widths(face, scale, first_char, last_char, symbol_subtable);
        let missing_width = widths
            .get((b' ' - first_char) as usize)
            .copied()
            .unwrap_or(0);

        let ascent = scale_i16(face.ascender(), scale);
        let descent = scale_i16(face.descender(), scale);
        let line_gap = scale_i16(face.line_gap(), scale);
        let cap_height = face
            .capital_height()
            .map(|value| scale_i16(value, scale))
            .unwrap_or(ascent);
        let underline_metrics = face.underline_metrics().map(|metrics| DecorationMetrics {
            position: scale_i16(metrics.position, scale),
            thickness: scale_i16(metrics.thickness, scale),
        });
        let strikeout_metrics = face.strikeout_metrics().map(|metrics| DecorationMetrics {
            position: scale_i16(metrics.position, scale),
            thickness: scale_i16(metrics.thickness, scale),
        });
        let bbox = face.global_bounding_box();
        let bbox = (
            scale_i16(bbox.x_min, scale),
            scale_i16(bbox.y_min, scale),
            scale_i16(bbox.x_max, scale),
            scale_i16(bbox.y_max, scale),
        );

        let italic_angle = face
            .italic_angle()
            .map(|value| value.round() as i16)
            .unwrap_or(0);

        let program_kind = if face.has_cff_outlines() {
            FontProgramKind::OpenTypeCff
        } else {
            FontProgramKind::TrueType
        };

        let kerning = build_kerning_pairs(face, &glyph_ids, scale);

        (
            Self {
                first_char,
                last_char,
                widths,
                glyph_ids,
                ascent,
                descent,
                line_gap,
                cap_height,
                italic_angle,
                stem_v: 80,
                bbox,
                underline_metrics,
                strikeout_metrics,
                missing_width,
                is_fixed_pitch: face.is_monospaced(),
                kerning,
                symbolic,
            },
            program_kind,
        )
    }
}

impl FontMetrics {
    pub(crate) fn is_symbolic(&self) -> bool {
        self.symbolic
    }

    fn glyph_id_for_char(&self, ch: char) -> u16 {
        let code = ch as u32;
        let first = self.first_char as u32;
        let last = self.last_char as u32;
        if code < first || code > last {
            return 0;
        }
        let idx = (code - first) as usize;
        self.glyph_ids.get(idx).copied().unwrap_or(0)
    }

    fn advance_for_char(&self, ch: char) -> u16 {
        let code = ch as u32;
        let first = self.first_char as u32;
        let last = self.last_char as u32;
        if code < first || code > last {
            return self.missing_width;
        }
        let idx = (code - first) as usize;
        self.widths.get(idx).copied().unwrap_or(self.missing_width)
    }

    fn measure_text_width(&self, font_size: Pt, text: &str) -> Pt {
        let mut total_units: i32 = 0;
        let mut prev: Option<u16> = None;
        for ch in text.chars() {
            let gid = self.glyph_id_for_char(ch);
            let adv = self.advance_for_char(ch) as i32;
            total_units = total_units.saturating_add(adv);
            if let Some(prev_gid) = prev {
                if let Some(k) = self.kerning.get(&(prev_gid, gid)) {
                    total_units = total_units.saturating_add(*k as i32);
                }
            }
            prev = Some(gid);
        }
        if total_units <= 0 {
            return Pt::ZERO;
        }
        font_size.mul_ratio(total_units, 1000)
    }

    fn is_within_basic_latin(&self, text: &str) -> bool {
        let first = self.first_char as u32;
        let last = self.last_char as u32;
        text.chars().all(|ch| {
            let code = ch as u32;
            code >= first && code <= last
        })
    }

    fn line_height(&self, font_size: Pt) -> Pt {
        let height_1000 = self.ascent as i32 - self.descent as i32 + self.line_gap as i32;
        if height_1000 <= 0 {
            return Pt::ZERO;
        }
        font_size.mul_ratio(height_1000, 1000)
    }
}

fn select_symbol_subtable<'a>(face: &'a SfntFace<'a>) -> (bool, Option<CmapSubtable<'a>>) {
    let mut first = None;
    let mut symbol = None;
    let mut has_unicode = false;
    for subtable in face.cmap_subtables() {
        if first.is_none() {
            first = Some(subtable);
        }
        if subtable.platform_id == PlatformId::Windows && subtable.encoding_id == 0 {
            symbol = Some(subtable);
        }
        if subtable.is_unicode() {
            has_unicode = true;
        }
    }
    if has_unicode {
        (false, None)
    } else {
        (symbol.is_some(), symbol.or(first))
    }
}

fn build_glyph_ids(
    face: &SfntFace<'_>,
    first: u8,
    last: u8,
    fallback: Option<CmapSubtable<'_>>,
) -> Vec<u16> {
    let mut glyphs = Vec::with_capacity((last - first + 1) as usize);
    for code in first..=last {
        let gid = glyph_index_for_codepoint(face, code as u32, fallback)
            .map(|g| g.0)
            .unwrap_or(0);
        glyphs.push(gid);
    }
    glyphs
}

fn glyph_index_for_codepoint<'a>(
    face: &'a SfntFace<'a>,
    codepoint: u32,
    fallback: Option<CmapSubtable<'a>>,
) -> Option<GlyphId> {
    if char::from_u32(codepoint).is_some() {
        if let Some(id) = face.glyph_index(codepoint) {
            return Some(id);
        }
    }
    if let Some(subtable) = fallback {
        if let Some(id) = subtable.glyph_index(codepoint) {
            return Some(id);
        }
        let symbol_codepoint = codepoint + 0xF000;
        return subtable.glyph_index(symbol_codepoint);
    }
    None
}

fn build_widths(
    face: &SfntFace<'_>,
    scale: f32,
    first: u8,
    last: u8,
    fallback: Option<CmapSubtable<'_>>,
) -> Vec<u16> {
    let mut widths = Vec::with_capacity((last - first + 1) as usize);
    for code in first..=last {
        let width = glyph_index_for_codepoint(face, code as u32, fallback)
            .and_then(|id| face.glyph_hor_advance(id))
            .unwrap_or(0);
        let scaled = (width as f32 * scale).round() as i32;
        widths.push(scaled.clamp(0, u16::MAX as i32) as u16);
    }
    widths
}

fn build_kerning_pairs(
    face: &SfntFace<'_>,
    glyph_ids: &[u16],
    scale: f32,
) -> HashMap<(u16, u16), i16> {
    let mut out = HashMap::new();

    for &left in glyph_ids {
        if left == 0 {
            continue;
        }
        for &right in glyph_ids {
            if right == 0 {
                continue;
            }
            let left_id = GlyphId(left);
            let right_id = GlyphId(right);
            let total = i32::from(face.legacy_kerning(left_id, right_id));
            if total != 0 {
                let clamped = total.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
                let scaled = scale_i16(clamped, scale);
                if scaled != 0 {
                    out.insert((left, right), scaled);
                }
            }
        }
    }
    out
}

fn measure_text_width_full(font: &RegisteredFont, font_size: Pt, text: &str) -> Option<Pt> {
    let shaped = text_shape::shape(&font.data, text)?;
    if shaped.glyphs.is_empty() {
        return None;
    }
    let units_per_em = i64::from(shaped.units_per_em);
    let mut total_units: i32 = 0;
    for glyph in shaped.glyphs {
        let adv =
            (((i64::from(glyph.x_advance)) * 1000 + (units_per_em / 2)) / units_per_em) as i32;
        total_units = total_units.saturating_add(adv);
    }
    if total_units <= 0 {
        return Some(Pt::ZERO);
    }
    Some(font_size.mul_ratio(total_units, 1000))
}

fn scale_i16(value: i16, scale: f32) -> i16 {
    let scaled = (value as f32 * scale).round() as i32;
    scaled.clamp(i16::MIN as i32, i16::MAX as i32) as i16
}

fn font_names(face: &SfntFace<'_>, path: &Path) -> (String, Vec<String>) {
    use sfnt::name_id;

    let mut family = None;
    let mut full = None;
    let mut post = None;

    for entry in face.names() {
        let Some(name) = decode_font_name(entry) else {
            continue;
        };
        match entry.name_id {
            name_id::TYPOGRAPHIC_FAMILY | name_id::FAMILY => {
                if family.is_none() {
                    family = Some(name);
                }
            }
            name_id::FULL_NAME => {
                if full.is_none() {
                    full = Some(name);
                }
            }
            name_id::POST_SCRIPT_NAME => {
                if post.is_none() {
                    post = Some(name);
                }
            }
            _ => {}
        }
    }

    let stem = path
        .file_stem()
        .and_then(|v| v.to_str())
        .map(|v| v.to_string());
    let primary = post
        .clone()
        .or_else(|| full.clone())
        .or_else(|| family.clone())
        .or_else(|| stem.clone())
        .unwrap_or_else(|| "EmbeddedFont".to_string());

    let mut aliases = Vec::new();
    for candidate in [family, full, post, stem].into_iter().flatten() {
        if candidate != primary {
            aliases.push(candidate);
        }
    }

    (primary, aliases)
}

fn decode_font_name(entry: sfnt::NameRecord<'_>) -> Option<String> {
    entry.to_unicode_string()
}

#[cfg(feature = "python")]
pub(crate) fn font_primary_name_from_bytes(
    data: &[u8],
    source_name: Option<&str>,
) -> Option<String> {
    let Ok(face) = SfntFace::parse(data, 0) else {
        return None;
    };
    let source = source_name.unwrap_or("EmbeddedFont");
    let (primary, _) = font_names(&face, Path::new(source));
    Some(primary)
}

#[cfg(test)]
mod tests {
    use super::decode_font_name;
    use crate::sfnt::{NameRecord, PlatformId};

    fn name(platform_id: PlatformId, encoding_id: u16, bytes: &[u8]) -> NameRecord<'_> {
        NameRecord {
            platform_id,
            encoding_id,
            language_id: 0,
            name_id: 1,
            data: bytes,
        }
    }

    #[test]
    fn font_name_decoder_matches_unicode_name_table_contract() {
        let unicode = [
            0x00, 0x46, 0x00, 0x75, 0x00, 0x6c, 0x00, 0x6c, 0x00, 0x42, 0x00, 0x6c, 0x00, 0x65,
            0x00, 0x65, 0x00, 0x64, 0xd8, 0x3d, 0xde, 0x00,
        ];
        assert_eq!(
            decode_font_name(name(PlatformId::Unicode, 4, &unicode)).as_deref(),
            Some("FullBleed😀")
        );
        assert!(decode_font_name(name(PlatformId::Unicode, 4, &[0, 65, 0])).is_none());
        assert!(decode_font_name(name(PlatformId::Unicode, 4, &[0xd8, 0x00])).is_none());
        assert!(decode_font_name(name(PlatformId::Macintosh, 0, b"FullBleed")).is_none());
    }
}

fn normalize_name(name: &str) -> String {
    name.trim()
        .trim_matches('"')
        .trim_matches('\'')
        .to_ascii_lowercase()
}
