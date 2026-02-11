use crate::error::FullBleedError;
use crate::glyph_report::GlyphCoverageReport;
use crate::types::Pt;
use rustybuzz::{Direction as HbDirection, Face as HbFace, UnicodeBuffer};
use std::collections::{HashMap, VecDeque};
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};
use ttf_parser::GlyphId;

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

#[derive(Debug)]
pub(crate) struct RegisteredFont {
    pub(crate) name: String,
    pub(crate) data: Vec<u8>,
    pub(crate) metrics: FontMetrics,
    pub(crate) program_kind: FontProgramKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FontProgramKind {
    TrueType,
    OpenTypeCff,
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
                self.register_file(path);
            }
        }
    }

    pub(crate) fn register_file(&mut self, path: impl AsRef<Path>) {
        let path = path.as_ref();
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
        let Ok(face) = ttf_parser::Face::parse(&data, 0) else {
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
        let source = source_name.unwrap_or("EmbeddedFont");
        let Ok(face) = ttf_parser::Face::parse(&data, 0) else {
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
        if let Ok(face) = ttf_parser::Face::parse(&font.data, 0) {
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
        if let Ok(face) = ttf_parser::Face::parse(&font.data, 0) {
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
        if let Ok(face) = ttf_parser::Face::parse(&font.data, 0) {
            let advance = face.glyph_hor_advance(GlyphId(gid)).unwrap_or(0);
            let units = face.units_per_em().max(1) as i64;
            let scaled = ((advance as i64) * 1000 + (units / 2)) / units;
            return scaled.clamp(0, u16::MAX as i64) as u16;
        }
        0
    }
}

impl FontMetrics {
    fn from_face(face: &ttf_parser::Face<'_>) -> (Self, FontProgramKind) {
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

        let program_kind = if face.tables().cff.is_some() {
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

fn select_symbol_subtable<'a>(
    face: &'a ttf_parser::Face<'a>,
) -> (bool, Option<ttf_parser::cmap::Subtable<'a>>) {
    let Some(cmap) = face.tables().cmap else {
        return (false, None);
    };
    let mut first = None;
    let mut symbol = None;
    let mut has_unicode = false;
    for subtable in cmap.subtables {
        if first.is_none() {
            first = Some(subtable);
        }
        if subtable.platform_id == ttf_parser::name::PlatformId::Windows
            && subtable.encoding_id == 0
        {
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
    face: &ttf_parser::Face<'_>,
    first: u8,
    last: u8,
    fallback: Option<ttf_parser::cmap::Subtable<'_>>,
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
    face: &'a ttf_parser::Face<'a>,
    codepoint: u32,
    fallback: Option<ttf_parser::cmap::Subtable<'a>>,
) -> Option<ttf_parser::GlyphId> {
    if let Some(ch) = char::from_u32(codepoint) {
        if let Some(id) = face.glyph_index(ch) {
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
    face: &ttf_parser::Face<'_>,
    scale: f32,
    first: u8,
    last: u8,
    fallback: Option<ttf_parser::cmap::Subtable<'_>>,
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
    face: &ttf_parser::Face<'_>,
    glyph_ids: &[u16],
    scale: f32,
) -> HashMap<(u16, u16), i16> {
    let mut out = HashMap::new();
    let Some(kern) = face.tables().kern else {
        return out;
    };

    let subtables: Vec<_> = kern
        .subtables
        .into_iter()
        .filter(|s| s.horizontal && !s.has_cross_stream && !s.has_state_machine)
        .collect();
    if subtables.is_empty() {
        return out;
    }

    for &left in glyph_ids {
        if left == 0 {
            continue;
        }
        for &right in glyph_ids {
            if right == 0 {
                continue;
            }
            let mut total: i32 = 0;
            let left_id = GlyphId(left);
            let right_id = GlyphId(right);
            for sub in &subtables {
                if let Some(v) = sub.glyphs_kerning(left_id, right_id) {
                    total = total.saturating_add(v as i32);
                }
            }
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
    let face = HbFace::from_slice(&font.data, 0)?;
    let units_per_em = face.units_per_em().max(1) as i64;

    let mut buffer = UnicodeBuffer::new();
    buffer.set_direction(detect_direction(text));
    buffer.push_str(text);
    let output = rustybuzz::shape(&face, &[], buffer);
    let positions = output.glyph_positions();
    if positions.is_empty() {
        return None;
    }
    let mut total_units: i32 = 0;
    for pos in positions {
        let adv = (((pos.x_advance as i64) * 1000 + (units_per_em / 2)) / units_per_em) as i32;
        total_units = total_units.saturating_add(adv);
    }
    if total_units <= 0 {
        return Some(Pt::ZERO);
    }
    Some(font_size.mul_ratio(total_units, 1000))
}

fn detect_direction(text: &str) -> HbDirection {
    for ch in text.chars() {
        let code = ch as u32;
        let rtl = matches!(
            code,
            0x0590..=0x08FF
                | 0xFB1D..=0xFDFF
                | 0xFE70..=0xFEFF
                | 0x1EE00..=0x1EEFF
        );
        if rtl {
            return HbDirection::RightToLeft;
        }
    }
    HbDirection::LeftToRight
}

fn scale_i16(value: i16, scale: f32) -> i16 {
    let scaled = (value as f32 * scale).round() as i32;
    scaled.clamp(i16::MIN as i32, i16::MAX as i32) as i16
}

fn font_names(face: &ttf_parser::Face<'_>, path: &Path) -> (String, Vec<String>) {
    use ttf_parser::name::name_id;

    let mut family = None;
    let mut full = None;
    let mut post = None;

    for entry in face.names() {
        let Some(name) = entry.to_string() else {
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

#[cfg(feature = "python")]
pub(crate) fn font_primary_name_from_bytes(
    data: &[u8],
    source_name: Option<&str>,
) -> Option<String> {
    let Ok(face) = ttf_parser::Face::parse(data, 0) else {
        return None;
    };
    let source = source_name.unwrap_or("EmbeddedFont");
    let (primary, _) = font_names(&face, Path::new(source));
    Some(primary)
}

fn normalize_name(name: &str) -> String {
    name.trim()
        .trim_matches('"')
        .trim_matches('\'')
        .to_ascii_lowercase()
}
