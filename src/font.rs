use crate::error::FullBleedError;
use crate::glyph_report::GlyphCoverageReport;
use crate::sfnt::{self, CmapSubtable, Face as SfntFace, GlyphId, PlatformId};
use crate::sfnt_outline::OutlineBuilder;
use crate::text_shape;
use crate::types::Pt;
use fullbleed_audit_contract::sha256::Sha256;
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

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

const FONT_SUBSET_CACHE_MAX_ENTRIES: usize = 128;
const FONT_SUBSET_CACHE_MAX_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
struct FontSubsetKey {
    font_fingerprint: [u8; 32],
    glyphs: Vec<u16>,
}

#[derive(Debug, Clone)]
enum FontSubsetCacheValue {
    TrueType(Arc<CachedTrueTypeSubset>),
    Cff(Arc<CachedCffSubset>),
    Unsupported,
}

#[derive(Debug)]
struct FontSubsetCache {
    map: HashMap<FontSubsetKey, FontSubsetCacheValue>,
    order: VecDeque<FontSubsetKey>,
    bytes: usize,
}

static GLOBAL_FONT_SUBSET_CACHE: OnceLock<Mutex<FontSubsetCache>> = OnceLock::new();

fn global_font_subset_cache() -> &'static Mutex<FontSubsetCache> {
    GLOBAL_FONT_SUBSET_CACHE.get_or_init(|| Mutex::new(FontSubsetCache::new()))
}

impl FontSubsetCache {
    fn new() -> Self {
        Self {
            map: HashMap::new(),
            order: VecDeque::new(),
            bytes: 0,
        }
    }

    fn get(&self, key: &FontSubsetKey) -> Option<FontSubsetCacheValue> {
        self.map.get(key).cloned()
    }

    fn insert(&mut self, key: FontSubsetKey, value: FontSubsetCacheValue) {
        if self.map.contains_key(&key) {
            return;
        }
        self.bytes = self
            .bytes
            .saturating_add(font_subset_cache_value_bytes(&value));
        self.order.push_back(key.clone());
        self.map.insert(key, value);
        while self.map.len() > FONT_SUBSET_CACHE_MAX_ENTRIES
            || self.bytes > FONT_SUBSET_CACHE_MAX_BYTES
        {
            let Some(oldest) = self.order.pop_front() else {
                break;
            };
            if let Some(removed) = self.map.remove(&oldest) {
                self.bytes = self
                    .bytes
                    .saturating_sub(font_subset_cache_value_bytes(&removed));
            }
        }
    }
}

fn font_subset_cache_value_bytes(value: &FontSubsetCacheValue) -> usize {
    match value {
        FontSubsetCacheValue::TrueType(subset) => subset
            .data
            .len()
            .saturating_add(subset.compressed.as_ref().map_or(0, |data| data.len())),
        FontSubsetCacheValue::Cff(subset) => subset
            .data
            .len()
            .saturating_add(subset.compressed.as_ref().map_or(0, |data| data.len()))
            .saturating_add(
                subset
                    .old_to_new
                    .len()
                    .saturating_mul(std::mem::size_of::<(u16, u16)>()),
            ),
        FontSubsetCacheValue::Unsupported => 0,
    }
}

#[derive(Debug)]
pub(crate) struct CachedTrueTypeSubset {
    pub(crate) data: Vec<u8>,
    pub(crate) compressed: Option<Vec<u8>>,
    pub(crate) tag: [u8; 6],
    pub(crate) glyph_count: usize,
}

#[derive(Debug)]
pub(crate) struct CachedCffSubset {
    pub(crate) data: Vec<u8>,
    pub(crate) compressed: Option<Vec<u8>>,
    pub(crate) tag: [u8; 6],
    pub(crate) glyph_count: usize,
    pub(crate) old_to_new: BTreeMap<u16, u16>,
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
    font_subset_cache: Mutex<FontSubsetCache>,
}

#[derive(Debug, Clone)]
pub(crate) struct FontRun {
    pub font_name: Arc<str>,
    pub text: String,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum GlyphOutlineCommand {
    MoveTo(f32, f32),
    LineTo(f32, f32),
    QuadTo(f32, f32, f32, f32),
    CurveTo(f32, f32, f32, f32, f32, f32),
    Close,
}

#[derive(Debug, Clone)]
pub(crate) struct RegisteredGlyphOutline {
    pub(crate) commands: Vec<GlyphOutlineCommand>,
    pub(crate) units_per_em: u16,
    pub(crate) advance: u16,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct RegisteredGlyphBounds {
    pub(crate) x_min: i16,
    pub(crate) x_max: i16,
    pub(crate) y_max: i16,
    pub(crate) units_per_em: u16,
}

#[derive(Debug, Clone)]
pub(crate) struct RegisteredPositionedGlyphOutline {
    pub(crate) commands: Vec<GlyphOutlineCommand>,
    pub(crate) units_per_em: u16,
    pub(crate) x_advance: i32,
    pub(crate) y_advance: i32,
    pub(crate) x_offset: i32,
    pub(crate) y_offset: i32,
}

#[derive(Default)]
struct GlyphOutlineCollector {
    commands: Vec<GlyphOutlineCommand>,
}

impl OutlineBuilder for GlyphOutlineCollector {
    fn move_to(&mut self, x: f32, y: f32) {
        self.commands.push(GlyphOutlineCommand::MoveTo(x, y));
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.commands.push(GlyphOutlineCommand::LineTo(x, y));
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.commands
            .push(GlyphOutlineCommand::QuadTo(x1, y1, x, y));
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.commands
            .push(GlyphOutlineCommand::CurveTo(x1, y1, x2, y2, x, y));
    }

    fn close(&mut self) {
        self.commands.push(GlyphOutlineCommand::Close);
    }
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
    fingerprint: [u8; 32],
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
    pub(crate) weight_class: u16,
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
            font_subset_cache: Mutex::new(FontSubsetCache::new()),
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
        if ext != "ttf" && ext != "otf" && ext != "ttc" && ext != "otc" {
            return;
        }
        let Ok(data) = fs::read(path) else {
            return;
        };
        let faces = if matches!(ext.as_str(), "ttc" | "otc") {
            // Collection tables are commonly shared and can be tens of megabytes.
            // Register one deterministic face without duplicating every regional
            // face into the per-engine registry. Well-known Noto CJK collections
            // select their SC face, matching the engine's deterministic generic
            // CJK fallback; other collections use a source-name match or face 0.
            let face_index = preferred_collection_face_index(&data, &path.to_string_lossy());
            extract_collection_face_at(&data, face_index)
                .map(|face| vec![(face_index, face)])
                .unwrap_or_default()
        } else {
            vec![(0, data)]
        };
        for (face_index, face_data) in faces {
            let Ok(face) = SfntFace::parse(&face_data, 0) else {
                continue;
            };

            let (name, aliases) = font_names(&face, path);
            let (metrics, program_kind) = FontMetrics::from_face(&face);
            let index = self.fonts.len();
            let identifier = if matches!(ext.as_str(), "ttc" | "otc") {
                format!("{}#{}", path.to_string_lossy(), face_index)
            } else {
                path.to_string_lossy().to_string()
            };
            self.fonts.push(RegisteredFont {
                name: name.clone(),
                fingerprint: sfnt_cache_fingerprint(&face_data),
                data: face_data,
                metrics,
                program_kind,
                source: RegisteredFontSourceInfo {
                    kind: source_kind,
                    identifier,
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
        let (data, collection_face_index) = if data.get(0..4) == Some(b"ttcf") {
            let index = preferred_collection_face_index(&data, source);
            let face = extract_collection_face_at(&data, index).ok_or_else(|| {
                FullBleedError::Asset(format!("invalid font collection data for {source}"))
            })?;
            (face, Some(index))
        } else {
            (data, None)
        };
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
            fingerprint: sfnt_cache_fingerprint(&data),
            data,
            metrics,
            program_kind,
            source: RegisteredFontSourceInfo {
                kind: source_kind,
                identifier: collection_face_index
                    .map(|index| format!("{source}#{index}"))
                    .unwrap_or_else(|| source.to_string()),
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

    pub(crate) fn cached_truetype_subset(
        &self,
        name: &str,
        glyphs: &BTreeSet<u16>,
    ) -> Option<Arc<CachedTrueTypeSubset>> {
        let normalized = normalize_name(name);
        let font_index = *self.lookup.get(&normalized)?;
        let font = self.fonts.get(font_index)?;
        if !matches!(font.program_kind, FontProgramKind::TrueType) {
            return None;
        }
        let key = FontSubsetKey {
            font_fingerprint: font.fingerprint,
            glyphs: glyphs.iter().copied().collect(),
        };
        if let Ok(cache) = self.font_subset_cache.lock() {
            if let Some(value) = cache.get(&key) {
                return match value {
                    FontSubsetCacheValue::TrueType(subset) => Some(subset),
                    FontSubsetCacheValue::Cff(_) | FontSubsetCacheValue::Unsupported => None,
                };
            }
        }

        if let Ok(cache) = global_font_subset_cache().lock() {
            if let Some(value) = cache.get(&key) {
                if let Ok(mut local_cache) = self.font_subset_cache.lock() {
                    local_cache.insert(key, value.clone());
                }
                return match value {
                    FontSubsetCacheValue::TrueType(subset) => Some(subset),
                    FontSubsetCacheValue::Cff(_) | FontSubsetCacheValue::Unsupported => None,
                };
            }
        }

        let computed = crate::font_subset::subset_truetype(&font.data, glyphs).map(|subset| {
            let compressed = crate::flate_native::zlib_deflate_parallel(&subset.data);
            let compressed = (compressed.len() < subset.data.len()).then_some(compressed);
            Arc::new(CachedTrueTypeSubset {
                data: subset.data,
                compressed,
                tag: subset.tag,
                glyph_count: subset.glyph_count,
            })
        });
        let value = computed
            .as_ref()
            .map(|subset| FontSubsetCacheValue::TrueType(subset.clone()))
            .unwrap_or(FontSubsetCacheValue::Unsupported);
        if let Ok(mut cache) = global_font_subset_cache().lock() {
            cache.insert(key.clone(), value.clone());
        }
        if let Ok(mut cache) = self.font_subset_cache.lock() {
            cache.insert(key, value);
        }
        computed
    }

    pub(crate) fn cached_cff_subset(
        &self,
        name: &str,
        glyphs: &BTreeSet<u16>,
    ) -> Option<Arc<CachedCffSubset>> {
        let normalized = normalize_name(name);
        let font_index = *self.lookup.get(&normalized)?;
        let font = self.fonts.get(font_index)?;
        if !matches!(font.program_kind, FontProgramKind::OpenTypeCff) {
            return None;
        }
        let ordered_glyphs: Vec<u16> = glyphs.iter().copied().collect();
        let key = FontSubsetKey {
            font_fingerprint: font.fingerprint,
            glyphs: ordered_glyphs.clone(),
        };
        if let Ok(cache) = self.font_subset_cache.lock() {
            if let Some(value) = cache.get(&key) {
                return match value {
                    FontSubsetCacheValue::Cff(subset) => Some(subset),
                    FontSubsetCacheValue::TrueType(_) | FontSubsetCacheValue::Unsupported => None,
                };
            }
        }

        if let Ok(cache) = global_font_subset_cache().lock() {
            if let Some(value) = cache.get(&key) {
                if let Ok(mut local_cache) = self.font_subset_cache.lock() {
                    local_cache.insert(key, value.clone());
                }
                return match value {
                    FontSubsetCacheValue::Cff(subset) => Some(subset),
                    FontSubsetCacheValue::TrueType(_) | FontSubsetCacheValue::Unsupported => None,
                };
            }
        }

        let mapper = subsetter::GlyphRemapper::new_from_glyphs_sorted(&ordered_glyphs);
        let computed = subsetter::subset(&font.data, 0, &mapper)
            .ok()
            .filter(|data| data.len() < font.data.len())
            .map(|data| {
                let compressed = crate::flate_native::zlib_deflate_parallel(&data);
                let compressed = (compressed.len() < data.len()).then_some(compressed);
                let old_to_new = mapper
                    .remapped_gids()
                    .enumerate()
                    .filter_map(|(new_gid, old_gid)| {
                        u16::try_from(new_gid)
                            .ok()
                            .map(|new_gid| (old_gid, new_gid))
                    })
                    .collect();
                Arc::new(CachedCffSubset {
                    tag: crate::font_subset::deterministic_subset_tag(&font.data, glyphs),
                    glyph_count: usize::from(mapper.num_gids()),
                    data,
                    compressed,
                    old_to_new,
                })
            });
        let value = computed
            .as_ref()
            .map(|subset| FontSubsetCacheValue::Cff(subset.clone()))
            .unwrap_or(FontSubsetCacheValue::Unsupported);
        if let Ok(mut cache) = global_font_subset_cache().lock() {
            cache.insert(key.clone(), value.clone());
        }
        if let Ok(mut cache) = self.font_subset_cache.lock() {
            cache.insert(key, value);
        }
        computed
    }

    pub(crate) fn is_opentype_cff(&self, name: &str) -> bool {
        self.resolve(name)
            .is_some_and(|font| matches!(font.program_kind, FontProgramKind::OpenTypeCff))
    }

    pub(crate) fn glyph_outline_for_id(
        &self,
        name: &str,
        glyph_id: u16,
    ) -> Option<RegisteredGlyphOutline> {
        let font = self.resolve(name)?;
        let face = SfntFace::parse(&font.data, 0).ok()?;
        let glyph = GlyphId(glyph_id);
        let mut collector = GlyphOutlineCollector::default();
        face.outline_glyph(glyph, &mut collector)?;
        Some(RegisteredGlyphOutline {
            commands: collector.commands,
            units_per_em: face.units_per_em().max(1),
            advance: face.glyph_hor_advance(glyph).unwrap_or(0),
        })
    }

    pub(crate) fn positioned_glyph_outlines(
        &self,
        name: &str,
        text: &str,
    ) -> Option<Vec<RegisteredPositionedGlyphOutline>> {
        let font = self.resolve(name)?;
        let face = SfntFace::parse(&font.data, 0).ok()?;
        let shaped = text_shape::shape(&font.data, text)?;
        if shaped.glyphs.is_empty() {
            return Some(Vec::new());
        }

        let units_per_em = shaped.units_per_em.max(1);
        let mut outlines = Vec::with_capacity(shaped.glyphs.len());
        for glyph in shaped.glyphs {
            let mut collector = GlyphOutlineCollector::default();
            // Whitespace and other advance-only glyphs legitimately have no
            // outline. Keep their shaped advances in the run.
            let _ = face.outline_glyph(GlyphId(glyph.glyph_id), &mut collector);
            outlines.push(RegisteredPositionedGlyphOutline {
                commands: collector.commands,
                units_per_em,
                x_advance: glyph.x_advance,
                y_advance: glyph.y_advance,
                x_offset: glyph.x_offset,
                y_offset: glyph.y_offset,
            });
        }
        Some(outlines)
    }

    pub(crate) fn glyph_bounds_for_char(
        &self,
        name: &str,
        ch: char,
    ) -> Option<RegisteredGlyphBounds> {
        let font = self.resolve(name)?;
        let face = SfntFace::parse(&font.data, 0).ok()?;
        let (_symbolic, symbol_subtable) = select_symbol_subtable(&face);
        let glyph = glyph_index_for_codepoint(&face, ch as u32, symbol_subtable)?;
        let mut collector = GlyphOutlineCollector::default();
        let bounds = face.outline_glyph(glyph, &mut collector)?;
        Some(RegisteredGlyphBounds {
            x_min: bounds.x_min,
            x_max: bounds.x_max,
            y_max: bounds.y_max,
            units_per_em: face.units_per_em().max(1),
        })
    }

    pub(crate) fn requires_synthetic_bold(&self, name: &str, requested_weight: u16) -> bool {
        requested_weight >= 700
            && self
                .resolve(name)
                .is_some_and(|font| font.metrics.weight_class < 700)
    }

    pub(crate) fn requires_synthetic_italic(&self, name: &str) -> bool {
        self.resolve(name)
            .is_some_and(|font| font.metrics.italic_angle == 0)
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

    pub(crate) fn vertical_metrics(&self, name: &str, font_size: Pt) -> Option<(Pt, Pt)> {
        let font = self.resolve(name)?;
        if font.metrics.ascent <= 0 {
            return None;
        }
        let ascent = font_size.mul_ratio(font.metrics.ascent as i32, 1000);
        let descent_units = (-(font.metrics.descent as i32)).max(0);
        let descent = font_size.mul_ratio(descent_units, 1000);
        Some((ascent, descent))
    }

    /// CSS font-relative used-value metrics for `ex`, `ch`, and `cap`.
    ///
    /// OpenType's OS/2 metrics are authoritative when present. CSS defines
    /// `ch` from the advance of U+0030, so it deliberately goes through the
    /// same fixed-point text measurement path as layout.
    pub(crate) fn css_font_relative_metrics(
        &self,
        name: &str,
        font_size: Pt,
    ) -> Option<(Pt, Pt, Pt)> {
        let font = self.resolve(name)?;
        let face = SfntFace::parse(&font.data, 0).ok()?;
        let units_per_em = i32::from(face.units_per_em().max(1));
        let ex = face
            .x_height()
            .filter(|value| *value > 0)
            .map(|value| font_size.mul_ratio(i32::from(value), units_per_em))
            .or_else(|| {
                self.glyph_bounds_for_char(name, 'x').map(|bounds| {
                    let raw = font_size.mul_ratio(
                        i32::from(bounds.y_max).max(0),
                        i32::from(bounds.units_per_em.max(1)),
                    );
                    ceil_font_metric_to_css_pixel(raw)
                })
            })
            .unwrap_or_else(|| font_size.mul_ratio(1, 2));
        let cap = face
            .capital_height()
            .filter(|value| *value > 0)
            .map(|value| font_size.mul_ratio(i32::from(value), units_per_em))
            .or_else(|| {
                self.glyph_bounds_for_char(name, 'H').map(|bounds| {
                    let raw = font_size.mul_ratio(
                        i32::from(bounds.y_max).max(0),
                        i32::from(bounds.units_per_em.max(1)),
                    );
                    round_font_metric_to_css_pixel(raw)
                })
            })
            .unwrap_or_else(|| {
                let value = i32::from(font.metrics.cap_height.max(0));
                if value > 0 {
                    font_size.mul_ratio(value, 1_000)
                } else {
                    font_size.mul_ratio(7, 10)
                }
            });
        let ch = self.measure_text_width(name, font_size, "0");
        Some((ex, ch, cap))
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
            return base14_font_supports_char(name, ch);
        };
        registered_font_supports_char(font, ch)
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
            if chosen.is_none() {
                // Authored families keep priority. Once every authored face is
                // missing the scalar, walk registered assets in deterministic
                // registration order so bundle-provided CJK/symbol coverage is
                // usable as a real fallback instead of viewer-side tofu.
                for (font_index, font) in self.fonts.iter().enumerate() {
                    let cache_index = stack.len() + font_index;
                    let supported = support_cache
                        .entry((cache_index, ch))
                        .or_insert_with(|| registered_font_supports_char(font, ch));
                    if *supported {
                        chosen = Some(Arc::<str>::from(font.name.as_str()));
                        break;
                    }
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
                for (font_index, font) in self.fonts.iter().enumerate() {
                    let cache_index = resolved.len() + font_index;
                    let ok = support_cache
                        .entry((cache_index, ch))
                        .or_insert_with(|| registered_font_supports_char(font, ch));
                    if *ok {
                        supported = true;
                        break;
                    }
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

fn ceil_font_metric_to_css_pixel(value: Pt) -> Pt {
    let milli = value.to_milli_i64();
    if milli <= 0 {
        return value;
    }
    Pt::from_milli_i64(((milli + 749) / 750) * 750)
}

fn round_font_metric_to_css_pixel(value: Pt) -> Pt {
    let milli = value.to_milli_i64();
    let rounded = if milli >= 0 {
        ((milli + 375) / 750) * 750
    } else {
        ((milli - 375) / 750) * 750
    };
    Pt::from_milli_i64(rounded)
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
                weight_class: face.weight_class(),
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

fn registered_font_supports_char(font: &RegisteredFont, ch: char) -> bool {
    if let Ok(face) = SfntFace::parse(&font.data, 0) {
        let (_symbolic, symbol_subtable) = select_symbol_subtable(&face);
        return glyph_index_for_codepoint(&face, ch as u32, symbol_subtable).is_some();
    }
    false
}

fn base14_font_supports_char(name: &str, ch: char) -> bool {
    let normalized = normalize_name(name);
    let is_base14 = normalized.starts_with("helvetica")
        || normalized.starts_with("times-")
        || normalized.starts_with("courier")
        || normalized == "symbol"
        || normalized == "zapfdingbats";
    if !is_base14 {
        return false;
    }
    matches!(
        ch,
        '\u{0000}'..='\u{00ff}'
            | '\u{0152}'
            | '\u{0153}'
            | '\u{0160}'
            | '\u{0161}'
            | '\u{0178}'
            | '\u{017d}'
            | '\u{017e}'
            | '\u{0192}'
            | '\u{02c6}'
            | '\u{02dc}'
            | '\u{2013}'..='\u{2014}'
            | '\u{2018}'..='\u{201a}'
            | '\u{201c}'..='\u{201e}'
            | '\u{2020}'..='\u{2022}'
            | '\u{2026}'
            | '\u{2030}'
            | '\u{2039}'..='\u{203a}'
            | '\u{20ac}'
            | '\u{2122}'
    )
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

#[cfg(test)]
fn extract_collection_faces(data: &[u8]) -> Option<Vec<Vec<u8>>> {
    if data.get(0..4)? != b"ttcf" {
        return None;
    }
    let count = usize::try_from(font_be_u32(data, 8)?).ok()?;
    if count == 0 || count > 256 {
        return None;
    }
    let offsets_end = 12usize.checked_add(count.checked_mul(4)?)?;
    data.get(0..offsets_end)?;

    let mut faces = Vec::with_capacity(count);
    for index in 0..count {
        let face_offset = usize::try_from(font_be_u32(data, 12 + index * 4)?).ok()?;
        faces.push(extract_collection_face(data, face_offset)?);
    }
    Some(faces)
}

fn preferred_collection_face_index(data: &[u8], source: &str) -> usize {
    if data.get(0..4) != Some(b"ttcf") {
        return 0;
    }
    let Some(count) = font_be_u32(data, 8).and_then(|value| usize::try_from(value).ok()) else {
        return 0;
    };
    if count == 0 || count > 256 {
        return 0;
    }

    let source_path = Path::new(source);
    let source_stem = source_path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or(source);
    let normalized_source = normalize_name(source_stem);
    let compact_source = compact_font_name(source_stem);
    let noto_cjk_collection = compact_source.contains("notosanscjk");
    let mut noto_sc = None;

    for index in 0..count {
        let Ok(face) = SfntFace::parse(data, index as u32) else {
            continue;
        };
        for entry in face.names() {
            if !matches!(
                entry.name_id,
                sfnt::name_id::TYPOGRAPHIC_FAMILY
                    | sfnt::name_id::FAMILY
                    | sfnt::name_id::FULL_NAME
                    | sfnt::name_id::POST_SCRIPT_NAME
            ) {
                continue;
            }
            let Some(name) = decode_font_name(entry) else {
                continue;
            };
            let normalized = normalize_name(&name);
            let compact = compact_font_name(&name);
            if normalized == normalized_source || compact == compact_source {
                return index;
            }
            if noto_cjk_collection && compact.contains("notosanscjksc") && !compact.contains("mono")
            {
                noto_sc.get_or_insert(index);
            }
        }
    }

    noto_sc.unwrap_or(0)
}

fn extract_collection_face_at(data: &[u8], index: usize) -> Option<Vec<u8>> {
    if data.get(0..4)? != b"ttcf" {
        return None;
    }
    let count = usize::try_from(font_be_u32(data, 8)?).ok()?;
    if index >= count || count > 256 {
        return None;
    }
    let face_offset = usize::try_from(font_be_u32(data, 12 + index.checked_mul(4)?)?).ok()?;
    extract_collection_face(data, face_offset)
}

fn extract_collection_face(data: &[u8], face_offset: usize) -> Option<Vec<u8>> {
    let header = data.get(face_offset..face_offset.checked_add(12)?)?;
    let signature = header.get(0..4)?;
    if !matches!(signature, b"\0\x01\0\0" | b"OTTO" | b"true" | b"typ1") {
        return None;
    }
    let table_count = usize::from(font_be_u16(data, face_offset + 4)?);
    let source_directory = face_offset.checked_add(12)?;
    let source_directory_end = source_directory.checked_add(table_count.checked_mul(16)?)?;
    data.get(source_directory..source_directory_end)?;

    let output_directory_end = 12usize.checked_add(table_count.checked_mul(16)?)?;
    let mut output = vec![0u8; align_font_table(output_directory_end)?];
    output.get_mut(0..12)?.copy_from_slice(header);
    let mut head_output_offset = None;

    for table_index in 0..table_count {
        let source_record = source_directory + table_index * 16;
        let tag = data.get(source_record..source_record + 4)?;
        let checksum = font_be_u32(data, source_record + 4)?;
        let source_offset = usize::try_from(font_be_u32(data, source_record + 8)?).ok()?;
        let length = usize::try_from(font_be_u32(data, source_record + 12)?).ok()?;
        let source_table = data.get(source_offset..source_offset.checked_add(length)?)?;

        let output_offset = align_font_table(output.len())?;
        if output_offset > output.len() {
            output.resize(output_offset, 0);
        }
        let output_end = output_offset.checked_add(length)?;
        if output_end > u32::MAX as usize {
            return None;
        }
        output.resize(output_end, 0);
        output
            .get_mut(output_offset..output_end)?
            .copy_from_slice(source_table);
        output.resize(align_font_table(output_end)?, 0);

        let output_record = 12 + table_index * 16;
        output
            .get_mut(output_record..output_record + 4)?
            .copy_from_slice(tag);
        font_write_u32(&mut output, output_record + 4, checksum)?;
        font_write_u32(&mut output, output_record + 8, output_offset as u32)?;
        font_write_u32(&mut output, output_record + 12, length as u32)?;
        if tag == b"head" && length >= 12 {
            head_output_offset = Some(output_offset);
        }
    }

    if let Some(head_offset) = head_output_offset {
        font_write_u32(&mut output, head_offset + 8, 0)?;
        let adjustment = 0xB1B0_AFBAu32.wrapping_sub(font_checksum(&output));
        font_write_u32(&mut output, head_offset + 8, adjustment)?;
    }
    Some(output)
}

fn align_font_table(value: usize) -> Option<usize> {
    value.checked_add(3).map(|value| value & !3)
}

fn sfnt_cache_fingerprint(data: &[u8]) -> [u8; 32] {
    // Every SFNT table record carries the checksum of its table contents. Hashing
    // the header plus (tag, checksum, length) records therefore identifies a
    // well-formed font without rescanning multi-megabyte glyph programs whenever
    // a short-lived engine registers the same asset. Offsets are intentionally
    // omitted: repacking identical tables must not defeat the process cache.
    let Some(table_count) = font_be_u16(data, 4).map(usize::from) else {
        return Sha256::digest(data);
    };
    let Some(directory_end) = table_count
        .checked_mul(16)
        .and_then(|length| 12usize.checked_add(length))
    else {
        return Sha256::digest(data);
    };
    let Some(header) = data.get(..12) else {
        return Sha256::digest(data);
    };
    let Some(directory) = data.get(12..directory_end) else {
        return Sha256::digest(data);
    };

    let mut identity = Vec::with_capacity(32 + table_count.saturating_mul(12));
    identity.extend_from_slice(b"fullbleed.sfnt-cache.v1\0");
    identity.extend_from_slice(&(data.len() as u64).to_be_bytes());
    identity.extend_from_slice(header);
    for record in directory.chunks_exact(16) {
        identity.extend_from_slice(&record[..8]);
        identity.extend_from_slice(&record[12..16]);
    }
    Sha256::digest(&identity)
}

fn font_be_u16(data: &[u8], offset: usize) -> Option<u16> {
    let bytes = data.get(offset..offset.checked_add(2)?)?;
    Some(u16::from_be_bytes([bytes[0], bytes[1]]))
}

fn font_be_u32(data: &[u8], offset: usize) -> Option<u32> {
    let bytes = data.get(offset..offset.checked_add(4)?)?;
    Some(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn font_write_u32(data: &mut [u8], offset: usize, value: u32) -> Option<()> {
    data.get_mut(offset..offset.checked_add(4)?)?
        .copy_from_slice(&value.to_be_bytes());
    Some(())
}

fn font_checksum(data: &[u8]) -> u32 {
    data.chunks(4).fold(0u32, |sum, chunk| {
        let mut bytes = [0u8; 4];
        bytes[..chunk.len()].copy_from_slice(chunk);
        sum.wrapping_add(u32::from_be_bytes(bytes))
    })
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
    let source = source_name.unwrap_or("EmbeddedFont");
    let face_index = preferred_collection_face_index(data, source) as u32;
    let Ok(face) = SfntFace::parse(data, face_index) else {
        return None;
    };
    let (primary, _) = font_names(&face, Path::new(source));
    Some(primary)
}

#[cfg(test)]
mod tests {
    use super::{
        FontRegistry, compact_font_name, decode_font_name, extract_collection_faces, font_be_u16,
        font_be_u32, font_checksum, font_write_u32, preferred_collection_face_index,
    };
    use crate::sfnt::{Face, GlyphId, NameRecord, PlatformId};
    use std::collections::BTreeSet;
    use std::sync::Arc;

    const NOTO: &[u8] = include_bytes!("../python/fullbleed_assets/fonts/NotoSans-Regular.ttf");
    const NOTO_MATH: &[u8] =
        include_bytes!("../python/fullbleed_assets/fonts/NotoSansMath-Regular.ttf");

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

    #[test]
    fn repeated_glyph_sets_reuse_compiled_font_subset() {
        let mut registry = FontRegistry::new();
        let name = registry
            .register_bytes(NOTO_MATH.to_vec(), Some("NotoSansMath-Regular.ttf"))
            .expect("register static TrueType");
        let face = Face::parse(NOTO_MATH, 0).expect("parse font");
        let glyphs = BTreeSet::from([
            face.glyph_index('A' as u32).expect("A glyph").0,
            face.glyph_index('z' as u32).expect("z glyph").0,
        ]);

        let first = registry
            .cached_truetype_subset(&name, &glyphs)
            .expect("first subset");
        let second = registry
            .cached_truetype_subset(&name, &glyphs)
            .expect("cached subset");
        assert!(Arc::ptr_eq(&first, &second));
        assert!(first.compressed.is_some());

        let mut other_registry = FontRegistry::new();
        let other_name = other_registry
            .register_bytes(NOTO_MATH.to_vec(), Some("same-font-different-engine.ttf"))
            .expect("register same font in another engine");
        let process_cached = other_registry
            .cached_truetype_subset(&other_name, &glyphs)
            .expect("process-wide cached subset");
        assert!(Arc::ptr_eq(&first, &process_cached));
    }

    #[test]
    fn collection_face_extraction_rebases_tables_and_preserves_font_data() {
        let table_count = usize::from(font_be_u16(NOTO, 4).unwrap());
        let mut collection = vec![0u8; 16 + NOTO.len()];
        collection[0..4].copy_from_slice(b"ttcf");
        font_write_u32(&mut collection, 4, 0x0001_0000).unwrap();
        font_write_u32(&mut collection, 8, 1).unwrap();
        font_write_u32(&mut collection, 12, 16).unwrap();
        collection[16..].copy_from_slice(NOTO);
        for table_index in 0..table_count {
            let record = 16 + 12 + table_index * 16;
            let old_offset = font_be_u32(&collection, record + 8).unwrap();
            font_write_u32(&mut collection, record + 8, old_offset + 16).unwrap();
        }

        let extracted = extract_collection_faces(&collection).unwrap();
        assert_eq!(extracted.len(), 1);
        let face = Face::parse(&extracted[0], 0).unwrap();
        assert!(face.glyph_index('A' as u32).is_some());
        assert_eq!(font_checksum(&extracted[0]), 0xB1B0_AFBA);

        let mut registry = FontRegistry::new();
        let registered = registry
            .register_bundle_font_bytes(collection, Some("NotoSans-Regular.ttc"))
            .expect("register collection through the bundle boundary");
        assert!(registry.resolve(&registered).is_some());

        let runs =
            registry.split_text_by_fallbacks(&Arc::<str>::from("Helvetica"), &[], "A\u{03a9}");
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].font_name.as_ref(), "Helvetica");
        assert_eq!(runs[0].text, "A");
        assert_eq!(runs[1].font_name.as_ref(), registered);
    }

    #[test]
    fn compact_font_names_match_collection_stems_and_family_names() {
        assert_eq!(
            compact_font_name("Noto Sans CJK SC"),
            compact_font_name("NotoSansCJKSC")
        );
        assert_ne!(
            compact_font_name("Noto Sans Mono CJK SC"),
            compact_font_name("Noto Sans CJK SC")
        );
    }

    #[test]
    fn collection_face_selection_uses_declared_names_not_the_source_alias() {
        let header_len = 20usize;
        let first_offset = header_len;
        let second_offset = first_offset + NOTO.len();
        let mut collection = vec![0u8; second_offset + NOTO_MATH.len()];
        collection[0..4].copy_from_slice(b"ttcf");
        font_write_u32(&mut collection, 4, 0x0001_0000).unwrap();
        font_write_u32(&mut collection, 8, 2).unwrap();
        font_write_u32(&mut collection, 12, first_offset as u32).unwrap();
        font_write_u32(&mut collection, 16, second_offset as u32).unwrap();
        collection[first_offset..second_offset].copy_from_slice(NOTO);
        collection[second_offset..].copy_from_slice(NOTO_MATH);

        for (font, face_offset) in [(NOTO, first_offset), (NOTO_MATH, second_offset)] {
            let table_count = usize::from(font_be_u16(font, 4).unwrap());
            for table_index in 0..table_count {
                let record = face_offset + 12 + table_index * 16;
                let old_offset = font_be_u32(&collection, record + 8).unwrap();
                font_write_u32(&mut collection, record + 8, old_offset + face_offset as u32)
                    .unwrap();
            }
        }

        assert_eq!(
            preferred_collection_face_index(&collection, "NotoSansMath-Regular.ttc"),
            1
        );
    }

    #[test]
    fn external_cff_collection_is_densely_subset_when_configured() {
        let Some(path) = std::env::var_os("FULLBLEED_CFF_FONT").map(std::path::PathBuf::from)
        else {
            return;
        };
        let data = std::fs::read(&path).expect("read external CFF font or collection");
        let source_name = path.file_name().and_then(|name| name.to_str());
        let mut registry = FontRegistry::new();
        let name = registry
            .register_bundle_font_bytes(data, source_name)
            .expect("register external CFF font or collection");
        let registered = registry
            .resolve(&name)
            .expect("resolve registered CFF face");
        assert!(matches!(
            registered.program_kind,
            super::FontProgramKind::OpenTypeCff
        ));
        let source_len = registered.data.len();

        let glyphs: BTreeSet<u16> = "汉字，世界。"
            .chars()
            .map(|ch| registry.map_glyph_id_for_char(&name, ch))
            .filter(|gid| *gid != 0)
            .collect();
        assert_eq!(glyphs.len(), 6);
        let subset = registry
            .cached_cff_subset(&name, &glyphs)
            .expect("subset external CFF face");
        assert!(subset.data.len() < source_len / 10);
        assert_eq!(subset.glyph_count, glyphs.len() + 1);

        let face = Face::parse(&subset.data, 0).expect("parse CFF subset");
        for old_gid in glyphs {
            let new_gid = subset.old_to_new[&old_gid];
            assert!(face.glyph_hor_advance(GlyphId(new_gid)).is_some());
        }
    }
}

fn normalize_name(name: &str) -> String {
    name.trim()
        .trim_matches('"')
        .trim_matches('\'')
        .to_ascii_lowercase()
}

fn compact_font_name(name: &str) -> String {
    normalize_name(name)
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .collect()
}
