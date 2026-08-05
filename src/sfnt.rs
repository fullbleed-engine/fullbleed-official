//! Bounded, dependency-free SFNT/OpenType table parsing.
//!
//! This module owns the font-file primitives shared by metrics, PDF embedding, rasterization,
//! and the native shaping work. Every offset and count is checked before use so untrusted font
//! bytes cannot escape their table or file bounds.

use std::fmt;

const TTC_TAG: [u8; 4] = *b"ttcf";
const TRUE_TYPE_TAG: [u8; 4] = [0x00, 0x01, 0x00, 0x00];
const CFF_TAG: [u8; 4] = *b"OTTO";
const APPLE_TRUE_TAG: [u8; 4] = *b"true";
const APPLE_TYPE1_TAG: [u8; 4] = *b"typ1";
const HEAD_MAGIC: u32 = 0x5f0f_3cf5;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ParseError {
    InvalidIndex,
    InvalidSignature,
    InvalidDirectory,
    MissingRequiredTable,
    InvalidRequiredTable,
}

impl fmt::Display for ParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidIndex => "font collection index is out of range",
            Self::InvalidSignature => "unsupported SFNT signature",
            Self::InvalidDirectory => "invalid SFNT table directory",
            Self::MissingRequiredTable => "required SFNT table is missing",
            Self::InvalidRequiredTable => "required SFNT table is invalid",
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct GlyphId(pub(crate) u16);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Rect {
    pub(crate) x_min: i16,
    pub(crate) y_min: i16,
    pub(crate) x_max: i16,
    pub(crate) y_max: i16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct LineMetrics {
    pub(crate) position: i16,
    pub(crate) thickness: i16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PlatformId {
    Unicode,
    Macintosh,
    Iso,
    Windows,
    Custom,
}

impl PlatformId {
    fn parse(value: u16) -> Option<Self> {
        match value {
            0 => Some(Self::Unicode),
            1 => Some(Self::Macintosh),
            2 => Some(Self::Iso),
            3 => Some(Self::Windows),
            4 => Some(Self::Custom),
            _ => None,
        }
    }
}

pub(crate) mod name_id {
    pub(crate) const FAMILY: u16 = 1;
    pub(crate) const FULL_NAME: u16 = 4;
    pub(crate) const POST_SCRIPT_NAME: u16 = 6;
    pub(crate) const TYPOGRAPHIC_FAMILY: u16 = 16;
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct NameRecord<'a> {
    pub(crate) platform_id: PlatformId,
    pub(crate) encoding_id: u16,
    // Retained because language-aware name selection will consume it as the native stack grows.
    #[allow(dead_code)]
    pub(crate) language_id: u16,
    pub(crate) name_id: u16,
    pub(crate) data: &'a [u8],
}

impl NameRecord<'_> {
    pub(crate) fn is_unicode(self) -> bool {
        matches!(self.platform_id, PlatformId::Unicode)
            || (self.platform_id == PlatformId::Windows && matches!(self.encoding_id, 0 | 1))
    }

    pub(crate) fn to_unicode_string(self) -> Option<String> {
        if !self.is_unicode() || self.data.len() % 2 != 0 {
            return None;
        }
        let units: Vec<u16> = self
            .data
            .chunks_exact(2)
            .map(|bytes| u16::from_be_bytes([bytes[0], bytes[1]]))
            .collect();
        String::from_utf16(&units).ok()
    }
}

#[derive(Clone, Copy, Debug)]
struct TableRecord<'a> {
    tag: [u8; 4],
    data: &'a [u8],
}

#[derive(Clone, Debug)]
pub(crate) struct Face<'a> {
    tables: Vec<TableRecord<'a>>,
}

impl<'a> Face<'a> {
    pub(crate) fn parse(data: &'a [u8], index: u32) -> Result<Self, ParseError> {
        let sfnt_offset = collection_face_offset(data, index)?;
        let signature = read_tag(data, sfnt_offset).ok_or(ParseError::InvalidSignature)?;
        if !matches!(
            signature,
            TRUE_TYPE_TAG | CFF_TAG | APPLE_TRUE_TAG | APPLE_TYPE1_TAG
        ) {
            return Err(ParseError::InvalidSignature);
        }

        let table_count =
            read_u16(data, sfnt_offset + 4).ok_or(ParseError::InvalidDirectory)? as usize;
        let directory_start = sfnt_offset
            .checked_add(12)
            .ok_or(ParseError::InvalidDirectory)?;
        let directory_len = table_count
            .checked_mul(16)
            .ok_or(ParseError::InvalidDirectory)?;
        checked_slice(data, directory_start, directory_len).ok_or(ParseError::InvalidDirectory)?;

        let mut tables = Vec::with_capacity(table_count);
        for table_index in 0..table_count {
            let record = directory_start + table_index * 16;
            let tag = read_tag(data, record).ok_or(ParseError::InvalidDirectory)?;
            let offset =
                usize::try_from(read_u32(data, record + 8).ok_or(ParseError::InvalidDirectory)?)
                    .map_err(|_| ParseError::InvalidDirectory)?;
            let length =
                usize::try_from(read_u32(data, record + 12).ok_or(ParseError::InvalidDirectory)?)
                    .map_err(|_| ParseError::InvalidDirectory)?;
            let table_data =
                checked_slice(data, offset, length).ok_or(ParseError::InvalidDirectory)?;
            if tables
                .iter()
                .any(|table: &TableRecord<'_>| table.tag == tag)
            {
                return Err(ParseError::InvalidDirectory);
            }
            tables.push(TableRecord {
                tag,
                data: table_data,
            });
        }

        let face = Self { tables };
        face.validate_required_tables()?;
        Ok(face)
    }

    fn validate_required_tables(&self) -> Result<(), ParseError> {
        let head = self
            .table(*b"head")
            .ok_or(ParseError::MissingRequiredTable)?;
        let hhea = self
            .table(*b"hhea")
            .ok_or(ParseError::MissingRequiredTable)?;
        let maxp = self
            .table(*b"maxp")
            .ok_or(ParseError::MissingRequiredTable)?;
        if head.len() < 54
            || hhea.len() < 36
            || maxp.len() < 6
            || read_u32(head, 12) != Some(HEAD_MAGIC)
            || !matches!(read_u16(head, 18), Some(16..=16_384))
            || read_u16(maxp, 4) == Some(0)
            || read_u16(maxp, 4).is_none()
        {
            return Err(ParseError::InvalidRequiredTable);
        }
        Ok(())
    }

    pub(crate) fn table(&self, tag: [u8; 4]) -> Option<&'a [u8]> {
        self.tables
            .iter()
            .find(|table| table.tag == tag)
            .map(|table| table.data)
    }

    pub(crate) fn units_per_em(&self) -> u16 {
        self.table(*b"head")
            .and_then(|table| read_u16(table, 18))
            .unwrap_or(1000)
    }

    pub(crate) fn number_of_glyphs(&self) -> u16 {
        self.table(*b"maxp")
            .and_then(|table| read_u16(table, 4))
            .unwrap_or(0)
    }

    pub(crate) fn weight_class(&self) -> u16 {
        self.table(*b"OS/2")
            .and_then(|table| read_u16(table, 4))
            .filter(|weight| (1..=1000).contains(weight))
            .unwrap_or(400)
    }

    pub(crate) fn global_bounding_box(&self) -> Rect {
        let head = self.table(*b"head").unwrap_or_default();
        Rect {
            x_min: read_i16(head, 36).unwrap_or(0),
            y_min: read_i16(head, 38).unwrap_or(0),
            x_max: read_i16(head, 40).unwrap_or(0),
            y_max: read_i16(head, 42).unwrap_or(0),
        }
    }

    pub(crate) fn ascender(&self) -> i16 {
        let hhea = self.table(*b"hhea").unwrap_or_default();
        let os2 = self.table(*b"OS/2");
        if os2.is_some_and(use_typographic_metrics) {
            return os2.and_then(|table| read_i16(table, 68)).unwrap_or(0);
        }
        let mut value = read_i16(hhea, 4).unwrap_or(0);
        if value == 0 {
            if let Some(os2) = os2 {
                value = read_i16(os2, 68).unwrap_or(0);
                if value == 0 {
                    value = read_i16(os2, 74).unwrap_or(0);
                }
            }
        }
        value
    }

    pub(crate) fn descender(&self) -> i16 {
        let hhea = self.table(*b"hhea").unwrap_or_default();
        let os2 = self.table(*b"OS/2");
        if os2.is_some_and(use_typographic_metrics) {
            return os2.and_then(|table| read_i16(table, 70)).unwrap_or(0);
        }
        let mut value = read_i16(hhea, 6).unwrap_or(0);
        if value == 0 {
            if let Some(os2) = os2 {
                value = read_i16(os2, 70).unwrap_or(0);
                if value == 0 {
                    value = read_u16(os2, 76)
                        .map(|value| -(i32::from(value).min(i32::from(i16::MAX)) as i16))
                        .unwrap_or(0);
                }
            }
        }
        value
    }

    pub(crate) fn line_gap(&self) -> i16 {
        let hhea = self.table(*b"hhea").unwrap_or_default();
        let os2 = self.table(*b"OS/2");
        if os2.is_some_and(use_typographic_metrics) {
            return os2.and_then(|table| read_i16(table, 72)).unwrap_or(0);
        }
        let ascender = read_i16(hhea, 4).unwrap_or(0);
        let descender = read_i16(hhea, 6).unwrap_or(0);
        if ascender == 0 || descender == 0 {
            if let Some(os2) = os2 {
                if read_i16(os2, 68).unwrap_or(0) != 0 || read_i16(os2, 70).unwrap_or(0) != 0 {
                    return read_i16(os2, 72).unwrap_or(0);
                }
                return 0;
            }
        }
        read_i16(hhea, 8).unwrap_or(0)
    }

    pub(crate) fn capital_height(&self) -> Option<i16> {
        let os2 = self.table(*b"OS/2")?;
        if read_u16(os2, 0)? < 2 {
            return None;
        }
        read_i16(os2, 88)
    }

    pub(crate) fn x_height(&self) -> Option<i16> {
        let os2 = self.table(*b"OS/2")?;
        if read_u16(os2, 0)? < 2 {
            return None;
        }
        read_i16(os2, 86)
    }

    pub(crate) fn underline_metrics(&self) -> Option<LineMetrics> {
        let post = self.table(*b"post")?;
        Some(LineMetrics {
            position: read_i16(post, 8)?,
            thickness: read_i16(post, 10)?,
        })
    }

    pub(crate) fn strikeout_metrics(&self) -> Option<LineMetrics> {
        let os2 = self.table(*b"OS/2")?;
        Some(LineMetrics {
            position: read_i16(os2, 28).unwrap_or(0),
            thickness: read_i16(os2, 26).unwrap_or(0),
        })
    }

    pub(crate) fn italic_angle(&self) -> Option<f32> {
        let post = self.table(*b"post")?;
        let raw = read_i32(post, 4)?;
        Some(raw as f32 / 65_536.0)
    }

    pub(crate) fn is_monospaced(&self) -> bool {
        self.table(*b"post")
            .and_then(|table| read_u32(table, 12))
            .is_some_and(|value| value != 0)
    }

    pub(crate) fn has_cff_outlines(&self) -> bool {
        self.has_cff1_outlines() || self.has_cff2_outlines()
    }

    pub(crate) fn has_cff1_outlines(&self) -> bool {
        self.table(*b"CFF ").is_some()
    }

    pub(crate) fn has_cff2_outlines(&self) -> bool {
        self.table(*b"CFF2").is_some()
    }

    pub(crate) fn has_true_type_outlines(&self) -> bool {
        self.table(*b"glyf").is_some() && self.table(*b"loca").is_some()
    }

    pub(crate) fn glyph_hor_advance(&self, glyph: GlyphId) -> Option<u16> {
        if glyph.0 >= self.number_of_glyphs() {
            return None;
        }
        let hhea = self.table(*b"hhea")?;
        let hmtx = self.table(*b"hmtx")?;
        let metric_count = read_u16(hhea, 34)?;
        if metric_count == 0 {
            return None;
        }
        let metric_index = u32::from(glyph.0).min(u32::from(metric_count - 1));
        let offset = usize::try_from(metric_index).ok()?.checked_mul(4)?;
        read_u16(hmtx, offset)
    }

    pub(crate) fn names(&self) -> Vec<NameRecord<'a>> {
        let Some(name) = self.table(*b"name") else {
            return Vec::new();
        };
        let Some(count) = read_u16(name, 2).map(usize::from) else {
            return Vec::new();
        };
        let Some(storage_offset) = read_u16(name, 4).map(usize::from) else {
            return Vec::new();
        };
        let Some(records_len) = count.checked_mul(12) else {
            return Vec::new();
        };
        if checked_slice(name, 6, records_len).is_none() || storage_offset > name.len() {
            return Vec::new();
        }

        let mut records = Vec::with_capacity(count);
        for index in 0..count {
            let offset = 6 + index * 12;
            let Some(platform_id) = read_u16(name, offset).and_then(PlatformId::parse) else {
                continue;
            };
            let Some(length) = read_u16(name, offset + 8).map(usize::from) else {
                continue;
            };
            let Some(relative) = read_u16(name, offset + 10).map(usize::from) else {
                continue;
            };
            let Some(start) = storage_offset.checked_add(relative) else {
                continue;
            };
            let Some(data) = checked_slice(name, start, length) else {
                continue;
            };
            records.push(NameRecord {
                platform_id,
                encoding_id: read_u16(name, offset + 2).unwrap_or(0),
                language_id: read_u16(name, offset + 4).unwrap_or(0),
                name_id: read_u16(name, offset + 6).unwrap_or(0),
                data,
            });
        }
        records
    }

    pub(crate) fn cmap_subtables(&self) -> Vec<CmapSubtable<'a>> {
        let Some(cmap) = self.table(*b"cmap") else {
            return Vec::new();
        };
        let Some(count) = read_u16(cmap, 2).map(usize::from) else {
            return Vec::new();
        };
        let Some(records_len) = count.checked_mul(8) else {
            return Vec::new();
        };
        if checked_slice(cmap, 4, records_len).is_none() {
            return Vec::new();
        }
        let mut subtables = Vec::with_capacity(count);
        for index in 0..count {
            let record = 4 + index * 8;
            let Some(platform_id) = read_u16(cmap, record).and_then(PlatformId::parse) else {
                continue;
            };
            let Some(offset) =
                read_u32(cmap, record + 4).and_then(|value| usize::try_from(value).ok())
            else {
                continue;
            };
            let Some(tail) = cmap.get(offset..) else {
                continue;
            };
            let Some(length) = cmap_subtable_length(tail) else {
                continue;
            };
            let Some(data) = checked_slice(cmap, offset, length) else {
                continue;
            };
            subtables.push(CmapSubtable {
                platform_id,
                encoding_id: read_u16(cmap, record + 2).unwrap_or(0),
                data,
            });
        }
        subtables
    }

    pub(crate) fn glyph_index(&self, codepoint: u32) -> Option<GlyphId> {
        self.cmap_subtables()
            .into_iter()
            .filter(CmapSubtable::is_unicode)
            .find_map(|subtable| subtable.glyph_index(codepoint))
    }

    pub(crate) fn glyph_variation_index(
        &self,
        codepoint: u32,
        variation_selector: u32,
    ) -> Option<GlyphId> {
        for subtable in self
            .cmap_subtables()
            .into_iter()
            .filter(|subtable| subtable.is_unicode() && subtable.format() == 14)
        {
            match cmap_format_14(subtable.data, codepoint, variation_selector) {
                Some(VariationMapping::Glyph(glyph)) if glyph != 0 => {
                    return Some(GlyphId(glyph));
                }
                Some(VariationMapping::Default) => return self.glyph_index(codepoint),
                _ => {}
            }
        }
        None
    }

    pub(crate) fn legacy_kerning(&self, left: GlyphId, right: GlyphId) -> i16 {
        let Some(kern) = self.table(*b"kern") else {
            return 0;
        };
        if read_u16(kern, 0) == Some(0) {
            legacy_kerning_opentype(kern, left, right)
        } else {
            legacy_kerning_aat(kern, left, right)
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct CmapSubtable<'a> {
    pub(crate) platform_id: PlatformId,
    pub(crate) encoding_id: u16,
    data: &'a [u8],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum VariationMapping {
    Default,
    Glyph(u16),
}

impl CmapSubtable<'_> {
    pub(crate) fn format(self) -> u16 {
        read_u16(self.data, 0).unwrap_or(u16::MAX)
    }

    pub(crate) fn is_unicode(&self) -> bool {
        match self.platform_id {
            PlatformId::Unicode => true,
            PlatformId::Windows if self.encoding_id == 1 => true,
            PlatformId::Windows if self.encoding_id == 10 => {
                matches!(self.format(), 12 | 13)
            }
            _ => false,
        }
    }

    pub(crate) fn glyph_index(self, codepoint: u32) -> Option<GlyphId> {
        let value = match self.format() {
            0 => cmap_format_0(self.data, codepoint),
            2 => cmap_format_2(self.data, codepoint),
            4 => cmap_format_4(self.data, codepoint),
            6 => cmap_format_6(self.data, codepoint),
            10 => cmap_format_10(self.data, codepoint),
            12 => cmap_format_12_or_13(self.data, codepoint, false),
            13 => cmap_format_12_or_13(self.data, codepoint, true),
            _ => None,
        }?;
        if value == 0 || value > u32::from(u16::MAX) {
            None
        } else {
            Some(GlyphId(value as u16))
        }
    }
}

fn collection_face_offset(data: &[u8], index: u32) -> Result<usize, ParseError> {
    if read_tag(data, 0) != Some(TTC_TAG) {
        return if index == 0 {
            Ok(0)
        } else {
            Err(ParseError::InvalidIndex)
        };
    }
    let count = read_u32(data, 8).ok_or(ParseError::InvalidDirectory)?;
    if index >= count {
        return Err(ParseError::InvalidIndex);
    }
    let record = usize::try_from(index)
        .ok()
        .and_then(|value| value.checked_mul(4))
        .and_then(|value| 12usize.checked_add(value))
        .ok_or(ParseError::InvalidDirectory)?;
    read_u32(data, record)
        .and_then(|value| usize::try_from(value).ok())
        .ok_or(ParseError::InvalidDirectory)
}

fn use_typographic_metrics(os2: &[u8]) -> bool {
    read_u16(os2, 0).is_some_and(|version| version >= 4)
        && read_u16(os2, 62).is_some_and(|flags| flags & (1 << 7) != 0)
}

fn cmap_subtable_length(data: &[u8]) -> Option<usize> {
    match read_u16(data, 0)? {
        0 | 2 | 4 | 6 => read_u16(data, 2).map(usize::from),
        8 | 10 | 12 | 13 => read_u32(data, 4).and_then(|value| usize::try_from(value).ok()),
        14 => read_u32(data, 2).and_then(|value| usize::try_from(value).ok()),
        _ => None,
    }
}

fn cmap_format_0(data: &[u8], codepoint: u32) -> Option<u32> {
    let index = usize::try_from(codepoint).ok()?;
    data.get(6 + index).copied().map(u32::from)
}

fn cmap_format_2(data: &[u8], codepoint: u32) -> Option<u32> {
    let codepoint = u16::try_from(codepoint).ok()?;
    let high = usize::from(codepoint >> 8);
    let low = codepoint & 0x00ff;
    let key = read_u16(data, 6 + high * 2)?;
    if key % 8 != 0 {
        return None;
    }
    let subheader = 518usize.checked_add(usize::from(key))?;
    let first = read_u16(data, subheader)?;
    let count = read_u16(data, subheader + 2)?;
    let index = low.checked_sub(first)?;
    if index >= count {
        return None;
    }
    let delta = read_i16(data, subheader + 4)? as u16;
    let range_word = subheader + 6;
    let range_offset = usize::from(read_u16(data, range_word)?);
    let glyph_offset = range_word
        .checked_add(range_offset)?
        .checked_add(usize::from(index) * 2)?;
    let glyph = read_u16(data, glyph_offset)?;
    if glyph == 0 {
        Some(0)
    } else {
        Some(u32::from(glyph.wrapping_add(delta)))
    }
}

fn cmap_format_4(data: &[u8], codepoint: u32) -> Option<u32> {
    let codepoint = u16::try_from(codepoint).ok()?;
    let segment_count = usize::from(read_u16(data, 6)? / 2);
    if segment_count == 0 {
        return None;
    }
    let end_codes = 14usize;
    let start_codes = end_codes
        .checked_add(segment_count.checked_mul(2)?)?
        .checked_add(2)?;
    let deltas = start_codes.checked_add(segment_count.checked_mul(2)?)?;
    let range_offsets = deltas.checked_add(segment_count.checked_mul(2)?)?;
    checked_slice(data, range_offsets, segment_count.checked_mul(2)?)?;

    let mut low = 0usize;
    let mut high = segment_count;
    while low < high {
        let middle = low + (high - low) / 2;
        if read_u16(data, end_codes + middle * 2)? < codepoint {
            low = middle + 1;
        } else {
            high = middle;
        }
    }
    if low >= segment_count {
        return None;
    }
    let start = read_u16(data, start_codes + low * 2)?;
    if codepoint < start {
        return None;
    }
    let delta = read_i16(data, deltas + low * 2)? as u16;
    let range_word = range_offsets + low * 2;
    let range_offset = read_u16(data, range_word)?;
    if range_offset == 0 {
        return Some(u32::from(codepoint.wrapping_add(delta)));
    }
    let glyph_offset = range_word
        .checked_add(usize::from(range_offset))?
        .checked_add(usize::from(codepoint - start) * 2)?;
    let glyph = read_u16(data, glyph_offset)?;
    if glyph == 0 {
        Some(0)
    } else {
        Some(u32::from(glyph.wrapping_add(delta)))
    }
}

fn cmap_format_6(data: &[u8], codepoint: u32) -> Option<u32> {
    let codepoint = u16::try_from(codepoint).ok()?;
    let first = read_u16(data, 6)?;
    let count = read_u16(data, 8)?;
    let index = codepoint.checked_sub(first)?;
    if index >= count {
        return None;
    }
    read_u16(data, 10 + usize::from(index) * 2).map(u32::from)
}

fn cmap_format_10(data: &[u8], codepoint: u32) -> Option<u32> {
    let first = read_u32(data, 12)?;
    let count = read_u32(data, 16)?;
    let index = codepoint.checked_sub(first)?;
    if index >= count {
        return None;
    }
    let offset = usize::try_from(index)
        .ok()?
        .checked_mul(2)?
        .checked_add(20)?;
    read_u16(data, offset).map(u32::from)
}

fn cmap_format_12_or_13(data: &[u8], codepoint: u32, constant: bool) -> Option<u32> {
    let count = usize::try_from(read_u32(data, 12)?).ok()?;
    checked_slice(data, 16, count.checked_mul(12)?)?;
    let mut low = 0usize;
    let mut high = count;
    while low < high {
        let middle = low + (high - low) / 2;
        let record = 16 + middle * 12;
        let start = read_u32(data, record)?;
        let end = read_u32(data, record + 4)?;
        if codepoint < start {
            high = middle;
        } else if codepoint > end {
            low = middle + 1;
        } else {
            let glyph = read_u32(data, record + 8)?;
            return if constant {
                Some(glyph)
            } else {
                glyph.checked_add(codepoint - start)
            };
        }
    }
    None
}

fn cmap_format_14(
    data: &[u8],
    codepoint: u32,
    variation_selector: u32,
) -> Option<VariationMapping> {
    if read_u16(data, 0)? != 14 || codepoint > 0x10ffff || variation_selector > 0x10ffff {
        return None;
    }
    let count = usize::try_from(read_u32(data, 6)?).ok()?;
    checked_slice(data, 10, count.checked_mul(11)?)?;
    let mut low = 0usize;
    let mut high = count;
    while low < high {
        let middle = low + (high - low) / 2;
        let selector = read_u24(data, 10 + middle * 11)?;
        if selector < variation_selector {
            low = middle + 1;
        } else {
            high = middle;
        }
    }
    if low >= count || read_u24(data, 10 + low * 11)? != variation_selector {
        return None;
    }
    let record = 10 + low * 11;
    let default_offset = usize::try_from(read_u32(data, record + 3)?).ok()?;
    let non_default_offset = usize::try_from(read_u32(data, record + 7)?).ok()?;

    if non_default_offset != 0 {
        let mapping_count = usize::try_from(read_u32(data, non_default_offset)?).ok()?;
        checked_slice(data, non_default_offset + 4, mapping_count.checked_mul(5)?)?;
        let mut low = 0usize;
        let mut high = mapping_count;
        while low < high {
            let middle = low + (high - low) / 2;
            let mapped = read_u24(data, non_default_offset + 4 + middle * 5)?;
            if mapped < codepoint {
                low = middle + 1;
            } else {
                high = middle;
            }
        }
        if low < mapping_count {
            let mapping = non_default_offset + 4 + low * 5;
            if read_u24(data, mapping)? == codepoint {
                return read_u16(data, mapping + 3).map(VariationMapping::Glyph);
            }
        }
    }

    if default_offset != 0 {
        let range_count = usize::try_from(read_u32(data, default_offset)?).ok()?;
        checked_slice(data, default_offset + 4, range_count.checked_mul(4)?)?;
        let mut low = 0usize;
        let mut high = range_count;
        while low < high {
            let middle = low + (high - low) / 2;
            let range = default_offset + 4 + middle * 4;
            let start = read_u24(data, range)?;
            let end = start.checked_add(u32::from(*data.get(range + 3)?))?;
            if codepoint < start {
                high = middle;
            } else if codepoint > end {
                low = middle + 1;
            } else {
                return Some(VariationMapping::Default);
            }
        }
    }
    None
}

fn legacy_kerning_opentype(data: &[u8], left: GlyphId, right: GlyphId) -> i16 {
    let count = read_u16(data, 2).map(usize::from).unwrap_or(0);
    let mut cursor = 4usize;
    let mut total = 0i32;
    for index in 0..count {
        let Some(declared_length) = read_u16(data, cursor + 2).map(usize::from) else {
            break;
        };
        // Some otherwise valid fonts use the rest of the `kern` table for their only
        // subtable so format 0 can exceed the historical u16 length field.
        let length = if count == 1 && index == 0 {
            data.len().saturating_sub(cursor)
        } else {
            declared_length
        };
        if length < 6 || checked_slice(data, cursor, length).is_none() {
            break;
        }
        let subtable = &data[cursor..cursor + length];
        let format = data.get(cursor + 4).copied().unwrap_or(u8::MAX);
        let coverage = data.get(cursor + 5).copied().unwrap_or(0);
        if coverage & 0x01 != 0 && coverage & 0x04 == 0 {
            let value = match format {
                0 => kern_format_0(&subtable[6..], left, right),
                2 => kern_format_2(subtable, 6, left, right),
                _ => None,
            };
            total = total.saturating_add(i32::from(value.unwrap_or(0)));
        }
        cursor = match cursor.checked_add(length) {
            Some(next) => next,
            None => break,
        };
    }
    total.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16
}

fn legacy_kerning_aat(data: &[u8], left: GlyphId, right: GlyphId) -> i16 {
    let count = read_u32(data, 4)
        .and_then(|value| usize::try_from(value).ok())
        .unwrap_or(0);
    let mut cursor = 8usize;
    let mut total = 0i32;
    for _ in 0..count {
        let Some(length) = read_u32(data, cursor).and_then(|value| usize::try_from(value).ok())
        else {
            break;
        };
        if length < 8 || checked_slice(data, cursor, length).is_none() {
            break;
        }
        let subtable = &data[cursor..cursor + length];
        let coverage = data.get(cursor + 4).copied().unwrap_or(0);
        let format = data.get(cursor + 5).copied().unwrap_or(u8::MAX);
        if coverage & 0x80 == 0 && coverage & 0x40 == 0 {
            let value = match format {
                0 => kern_format_0(&subtable[8..], left, right),
                2 => kern_format_2(subtable, 8, left, right),
                3 => kern_format_3(&subtable[8..], left, right),
                _ => None,
            };
            total = total.saturating_add(i32::from(value.unwrap_or(0)));
        }
        cursor = match cursor.checked_add(length) {
            Some(next) => next,
            None => break,
        };
    }
    total.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16
}

fn kern_format_0(data: &[u8], left: GlyphId, right: GlyphId) -> Option<i16> {
    let count = usize::from(read_u16(data, 0)?);
    let pairs = 8usize;
    checked_slice(data, pairs, count.checked_mul(6)?)?;
    let key = (u32::from(left.0) << 16) | u32::from(right.0);
    let mut low = 0usize;
    let mut high = count;
    while low < high {
        let middle = low + (high - low) / 2;
        let record = pairs + middle * 6;
        let candidate =
            (u32::from(read_u16(data, record)?) << 16) | u32::from(read_u16(data, record + 2)?);
        match candidate.cmp(&key) {
            std::cmp::Ordering::Less => low = middle + 1,
            std::cmp::Ordering::Greater => high = middle,
            std::cmp::Ordering::Equal => return read_i16(data, record + 4),
        }
    }
    None
}

fn kern_format_2(subtable: &[u8], header_len: usize, left: GlyphId, right: GlyphId) -> Option<i16> {
    let body = subtable.get(header_len..)?;
    let _row_width = read_u16(body, 0)?;
    let left_classes = usize::from(read_u16(body, 2)?);
    let right_classes = usize::from(read_u16(body, 4)?);
    let array_offset = read_u16(body, 6)?;

    let left_class = kern_format_2_class(subtable, left_classes, left.0).unwrap_or(0);
    let right_class = kern_format_2_class(subtable, right_classes, right.0).unwrap_or(0);

    // Left-hand entries are premultiplied row offsets from the beginning of the subtable;
    // right-hand entries are byte offsets within that row.
    if left_class < array_offset {
        return None;
    }
    let offset = usize::from(left_class).checked_add(usize::from(right_class))?;
    read_i16(subtable, offset)
}

fn kern_format_2_class(data: &[u8], offset: usize, glyph: u16) -> Option<u16> {
    let first = read_u16(data, offset)?;
    let count = read_u16(data, offset + 2)?;
    let index = glyph.checked_sub(first)?;
    if index >= count {
        return None;
    }
    read_u16(data, offset.checked_add(4 + usize::from(index) * 2)?)
}

fn kern_format_3(data: &[u8], left: GlyphId, right: GlyphId) -> Option<i16> {
    let glyph_count = read_u16(data, 0)?;
    if left.0 >= glyph_count || right.0 >= glyph_count {
        return None;
    }
    let value_count = usize::from(*data.get(2)?);
    let left_class_count = usize::from(*data.get(3)?);
    let right_class_count = usize::from(*data.get(4)?);
    if value_count == 0 || left_class_count == 0 || right_class_count == 0 {
        return None;
    }

    let values = 6usize;
    let left_classes = values.checked_add(value_count.checked_mul(2)?)?;
    let right_classes = left_classes.checked_add(usize::from(glyph_count))?;
    let indices = right_classes.checked_add(usize::from(glyph_count))?;
    let index_count = left_class_count.checked_mul(right_class_count)?;
    checked_slice(data, values, value_count.checked_mul(2)?)?;
    checked_slice(data, left_classes, usize::from(glyph_count))?;
    checked_slice(data, right_classes, usize::from(glyph_count))?;
    checked_slice(data, indices, index_count)?;

    let left_class = usize::from(*data.get(left_classes + usize::from(left.0))?);
    let right_class = usize::from(*data.get(right_classes + usize::from(right.0))?);
    if left_class >= left_class_count || right_class >= right_class_count {
        return None;
    }
    let class_pair = left_class
        .checked_mul(right_class_count)?
        .checked_add(right_class)?;
    let value_index = usize::from(*data.get(indices + class_pair)?);
    if value_index >= value_count {
        return None;
    }
    read_i16(data, values + value_index * 2)
}

fn checked_slice(data: &[u8], offset: usize, length: usize) -> Option<&[u8]> {
    let end = offset.checked_add(length)?;
    data.get(offset..end)
}

fn read_tag(data: &[u8], offset: usize) -> Option<[u8; 4]> {
    let bytes = checked_slice(data, offset, 4)?;
    Some([bytes[0], bytes[1], bytes[2], bytes[3]])
}

fn read_u16(data: &[u8], offset: usize) -> Option<u16> {
    let bytes = checked_slice(data, offset, 2)?;
    Some(u16::from_be_bytes([bytes[0], bytes[1]]))
}

fn read_u24(data: &[u8], offset: usize) -> Option<u32> {
    let bytes = checked_slice(data, offset, 3)?;
    Some((u32::from(bytes[0]) << 16) | (u32::from(bytes[1]) << 8) | u32::from(bytes[2]))
}

fn read_i16(data: &[u8], offset: usize) -> Option<i16> {
    read_u16(data, offset).map(|value| value as i16)
}

fn read_u32(data: &[u8], offset: usize) -> Option<u32> {
    let bytes = checked_slice(data, offset, 4)?;
    Some(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn read_i32(data: &[u8], offset: usize) -> Option<i32> {
    read_u32(data, offset).map(|value| value as i32)
}

#[cfg(test)]
mod tests {
    use super::{
        Face, GlyphId, PlatformId, VariationMapping, cmap_format_14, legacy_kerning_aat,
        legacy_kerning_opentype,
    };
    use fullbleed_audit_contract::sha256::Sha256;

    const INTER: &[u8] = include_bytes!("../python/fullbleed_assets/fonts/Inter-Variable.ttf");
    const NOTO: &[u8] = include_bytes!("../python/fullbleed_assets/fonts/NotoSans-Regular.ttf");
    const MATH: &[u8] = include_bytes!("../python/fullbleed_assets/fonts/NotoSansMath-Regular.ttf");
    const SYMBOLS: &[u8] =
        include_bytes!("../python/fullbleed_assets/fonts/NotoSansSymbols-Regular.ttf");
    const SYMBOLS2: &[u8] =
        include_bytes!("../python/fullbleed_assets/fonts/NotoSansSymbols2-Regular.ttf");

    fn push_u16(bytes: &mut Vec<u8>, value: u16) {
        bytes.extend_from_slice(&value.to_be_bytes());
    }

    fn push_i16(bytes: &mut Vec<u8>, value: i16) {
        bytes.extend_from_slice(&value.to_be_bytes());
    }

    fn push_u32(bytes: &mut Vec<u8>, value: u32) {
        bytes.extend_from_slice(&value.to_be_bytes());
    }

    fn push_u24(bytes: &mut Vec<u8>, value: u32) {
        bytes.extend_from_slice(&value.to_be_bytes()[1..]);
    }

    fn opentype_format_2_kern() -> Vec<u8> {
        let mut bytes = Vec::new();
        push_u16(&mut bytes, 0); // table version
        push_u16(&mut bytes, 1); // subtable count
        push_u16(&mut bytes, 0); // subtable version
        push_u16(&mut bytes, 38); // subtable length
        bytes.extend_from_slice(&[2, 1]); // format 2, horizontal
        push_u16(&mut bytes, 4); // row width
        push_u16(&mut bytes, 14); // left class table
        push_u16(&mut bytes, 22); // right class table
        push_u16(&mut bytes, 30); // kerning array
        push_u16(&mut bytes, 1); // left first glyph
        push_u16(&mut bytes, 2); // left glyph count
        push_u16(&mut bytes, 30); // left class 0 row
        push_u16(&mut bytes, 34); // left class 1 row
        push_u16(&mut bytes, 3); // right first glyph
        push_u16(&mut bytes, 2); // right glyph count
        push_u16(&mut bytes, 0); // right class 0 column
        push_u16(&mut bytes, 2); // right class 1 column
        for value in [0, -40, -20, -60] {
            push_i16(&mut bytes, value);
        }
        bytes
    }

    fn aat_format_3_kern() -> Vec<u8> {
        let mut bytes = Vec::new();
        push_u32(&mut bytes, 0x0001_0000); // table version 1.0
        push_u32(&mut bytes, 1); // subtable count
        push_u32(&mut bytes, 36); // subtable length
        bytes.extend_from_slice(&[0, 3]); // horizontal, format 3
        push_u16(&mut bytes, 0); // variation tuple index
        push_u16(&mut bytes, 5); // glyph count
        bytes.extend_from_slice(&[4, 2, 2, 0]); // value/left/right counts, reserved
        for value in [0, -10, -20, -60] {
            push_i16(&mut bytes, value);
        }
        bytes.extend_from_slice(&[0, 1, 1, 0, 0]); // left classes
        bytes.extend_from_slice(&[0, 0, 0, 1, 1]); // right classes
        bytes.extend_from_slice(&[0, 1, 2, 3]); // class-pair value indices
        bytes
    }

    fn hex_digest(hasher: Sha256) -> String {
        hasher
            .finalize()
            .into_iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    fn update_option_i16(hasher: &mut Sha256, value: Option<i16>) {
        match value {
            Some(value) => {
                hasher.update(&[1]);
                hasher.update(&value.to_be_bytes());
            }
            None => hasher.update(&[0]),
        }
    }

    fn update_option_u16(hasher: &mut Sha256, value: Option<u16>) {
        match value {
            Some(value) => {
                hasher.update(&[1]);
                hasher.update(&value.to_be_bytes());
            }
            None => hasher.update(&[0]),
        }
    }

    fn update_option_u32(hasher: &mut Sha256, value: Option<u32>) {
        match value {
            Some(value) => {
                hasher.update(&[1]);
                hasher.update(&value.to_be_bytes());
            }
            None => hasher.update(&[0]),
        }
    }

    #[test]
    fn cmap_format_14_resolves_default_and_non_default_variants() {
        let mut bytes = Vec::new();
        push_u16(&mut bytes, 14);
        push_u32(&mut bytes, 38);
        push_u32(&mut bytes, 1);
        push_u24(&mut bytes, 0xfe0f);
        push_u32(&mut bytes, 21);
        push_u32(&mut bytes, 29);
        push_u32(&mut bytes, 1);
        push_u24(&mut bytes, 0x30);
        bytes.push(9);
        push_u32(&mut bytes, 1);
        push_u24(&mut bytes, 0x2764);
        push_u16(&mut bytes, 42);

        assert_eq!(
            cmap_format_14(&bytes, 0x31, 0xfe0f),
            Some(VariationMapping::Default)
        );
        assert_eq!(
            cmap_format_14(&bytes, 0x2764, 0xfe0f),
            Some(VariationMapping::Glyph(42))
        );
        assert_eq!(cmap_format_14(&bytes, 0x41, 0xfe0f), None);
        assert_eq!(cmap_format_14(&bytes, 0x31, 0xfe0e), None);
    }

    #[test]
    fn native_legacy_class_kerning_matches_frozen_contract() {
        let opentype = opentype_format_2_kern();
        let aat = aat_format_3_kern();
        for left in 0..7 {
            for right in 0..7 {
                let expected_opentype = match (left, right) {
                    (1, 4) => -40,
                    (2, 4) => -60,
                    (2, _) => -20,
                    _ => 0,
                };
                let left = GlyphId(left);
                let right = GlyphId(right);
                assert_eq!(
                    legacy_kerning_opentype(&opentype, left, right),
                    expected_opentype,
                    "OpenType format 2 pair ({}, {})",
                    left.0,
                    right.0
                );

                let expected_aat = if left.0 >= 5 || right.0 >= 5 {
                    0
                } else {
                    let left_class = u16::from(matches!(left.0, 1 | 2));
                    let right_class = u16::from(matches!(right.0, 3 | 4));
                    match (left_class, right_class) {
                        (0, 0) => 0,
                        (0, 1) => -10,
                        (1, 0) => -20,
                        (1, 1) => -60,
                        _ => unreachable!(),
                    }
                };
                assert_eq!(
                    legacy_kerning_aat(&aat, left, right),
                    expected_aat,
                    "AAT format 3 pair ({}, {})",
                    left.0,
                    right.0
                );
            }
        }
    }

    #[test]
    fn native_sfnt_metrics_cmaps_names_and_advances_match_frozen_contracts() {
        let codepoints = [
            0x20, 0x41, 0x56, 0x66, 0xe9, 0x3a9, 0x416, 0x915, 0x94d, 0x221e, 0x1f600,
        ];
        for (label, data) in [
            ("inter", INTER),
            ("noto sans", NOTO),
            ("noto math", MATH),
            ("noto symbols", SYMBOLS),
            ("noto symbols 2", SYMBOLS2),
        ] {
            let native = Face::parse(data, 0).expect("native face");
            let mut contract = Sha256::new();
            contract.update(label.as_bytes());
            contract.update(&native.units_per_em().to_be_bytes());
            contract.update(&native.number_of_glyphs().to_be_bytes());
            contract.update(&native.ascender().to_be_bytes());
            contract.update(&native.descender().to_be_bytes());
            contract.update(&native.line_gap().to_be_bytes());
            update_option_i16(&mut contract, native.capital_height());
            if let Some(metrics) = native.underline_metrics() {
                contract.update(&[1]);
                contract.update(&metrics.position.to_be_bytes());
                contract.update(&metrics.thickness.to_be_bytes());
            } else {
                contract.update(&[0]);
            }
            if let Some(metrics) = native.strikeout_metrics() {
                contract.update(&[1]);
                contract.update(&metrics.position.to_be_bytes());
                contract.update(&metrics.thickness.to_be_bytes());
            } else {
                contract.update(&[0]);
            }
            contract.update(&[u8::from(native.is_monospaced())]);
            contract.update(&[u8::from(native.has_cff_outlines())]);
            update_option_u32(&mut contract, native.italic_angle().map(f32::to_bits));
            let native_bbox = native.global_bounding_box();
            contract.update(&native_bbox.x_min.to_be_bytes());
            contract.update(&native_bbox.y_min.to_be_bytes());
            contract.update(&native_bbox.x_max.to_be_bytes());
            contract.update(&native_bbox.y_max.to_be_bytes());

            for codepoint in codepoints {
                let native_gid = native.glyph_index(codepoint).map(|value| value.0);
                contract.update(&codepoint.to_be_bytes());
                update_option_u16(&mut contract, native_gid);
                update_option_u16(
                    &mut contract,
                    native_gid.and_then(|glyph| native.glyph_hor_advance(GlyphId(glyph))),
                );
            }

            let native_names: Vec<_> = native
                .names()
                .into_iter()
                .filter_map(|record| {
                    record
                        .to_unicode_string()
                        .map(|value| (record.name_id, record.language_id, value))
                })
                .collect();
            contract.update(&(native_names.len() as u32).to_be_bytes());
            for (name_id, language_id, value) in native_names {
                contract.update(&name_id.to_be_bytes());
                contract.update(&language_id.to_be_bytes());
                contract.update(&(value.len() as u32).to_be_bytes());
                contract.update(value.as_bytes());
            }

            let native_subtables = native.cmap_subtables();
            let has_symbol = native_subtables.iter().any(|subtable| {
                subtable.platform_id == PlatformId::Windows && subtable.encoding_id == 0
            });
            contract.update(&[u8::from(has_symbol)]);
            let expected = match label {
                "inter" => "9e0186d7722a3152f35bdc65c4906ed1926e98a790dcdf01f4140626576427c6",
                "noto sans" => "d262902910bedec3f1ea9c40ab157ee85663e1049f431e3aa2dadefd6b9c3194",
                "noto math" => "86e6524953e375336e42d13c8070405ee49995e11102d85a0570d17f8ddb19bb",
                "noto symbols" => {
                    "a16cab103d325271b3a564f04aae4973356fa2b92f0b0d1cfcad07dd42a1cbfd"
                }
                "noto symbols 2" => {
                    "1eb8dacd8708b810e93bcd6885ef759feb39d319b6c2aa07657c76cbbf6a431d"
                }
                _ => unreachable!("known bundled font"),
            };
            assert_eq!(hex_digest(contract), expected, "{label} SFNT contract");
        }
    }
}
