//! Compact Font Format 1 parsing and Type 2 charstring interpretation.

use crate::sfnt::{Face, GlyphId, Rect};
use crate::sfnt_outline::OutlineBuilder;

const MAX_ARGUMENT_STACK: usize = 48;
const MAX_SUBROUTINE_DEPTH: usize = 10;

#[derive(Clone, Debug)]
struct CffIndex<'a> {
    objects: Vec<&'a [u8]>,
}

impl<'a> CffIndex<'a> {
    fn empty() -> Self {
        Self {
            objects: Vec::new(),
        }
    }

    fn parse(data: &'a [u8], start: usize) -> Option<(Self, usize)> {
        Self::parse_with_count_size(data, start, 2)
    }

    fn parse32(data: &'a [u8], start: usize) -> Option<(Self, usize)> {
        Self::parse_with_count_size(data, start, 4)
    }

    fn parse_with_count_size(
        data: &'a [u8],
        start: usize,
        count_size: usize,
    ) -> Option<(Self, usize)> {
        let count = if count_size == 2 {
            usize::from(read_u16(data, start)?)
        } else {
            usize::try_from(read_u32(data, start)?).ok()?
        };
        if count == 0 {
            return Some((Self::empty(), start.checked_add(count_size)?));
        }
        let off_size_offset = start.checked_add(count_size)?;
        let off_size = usize::from(*data.get(off_size_offset)?);
        if !(1..=4).contains(&off_size) {
            return None;
        }
        let offsets_start = off_size_offset.checked_add(1)?;
        let offsets_count = count.checked_add(1)?;
        let offsets_len = offsets_count.checked_mul(off_size)?;
        checked_slice(data, offsets_start, offsets_len)?;
        let objects_start = offsets_start.checked_add(offsets_len)?;

        let mut offsets = Vec::with_capacity(offsets_count);
        for index in 0..offsets_count {
            let offset = read_offset(data, offsets_start + index * off_size, off_size)?;
            if offset == 0 || offsets.last().is_some_and(|previous| offset < *previous) {
                return None;
            }
            offsets.push(offset);
        }
        if offsets.first().copied() != Some(1) {
            return None;
        }
        let end = objects_start.checked_add(offsets.last()?.checked_sub(1)?)?;
        if end > data.len() {
            return None;
        }

        let mut objects = Vec::with_capacity(count);
        for pair in offsets.windows(2) {
            let object_start = objects_start.checked_add(pair[0].checked_sub(1)?)?;
            let object_end = objects_start.checked_add(pair[1].checked_sub(1)?)?;
            objects.push(data.get(object_start..object_end)?);
        }
        Some((Self { objects }, end))
    }

    fn get(&self, index: usize) -> Option<&'a [u8]> {
        self.objects.get(index).copied()
    }

    fn len(&self) -> usize {
        self.objects.len()
    }
}

#[derive(Clone, Debug, Default)]
struct Dict {
    entries: Vec<(u16, Vec<f32>)>,
}

impl Dict {
    fn parse(data: &[u8]) -> Option<Self> {
        let mut cursor = 0usize;
        let mut operands = Vec::new();
        let mut entries = Vec::new();
        while cursor < data.len() {
            let byte = *data.get(cursor)?;
            if is_dict_number(byte) {
                operands.push(parse_dict_number(data, &mut cursor)?);
                continue;
            }
            cursor += 1;
            let operator = if byte == 12 {
                0x0c00 | u16::from(*data.get(cursor)?)
            } else {
                u16::from(byte)
            };
            if byte == 12 {
                cursor += 1;
            }
            entries.push((operator, std::mem::take(&mut operands)));
        }
        if !operands.is_empty() {
            return None;
        }
        Some(Self { entries })
    }

    fn operands(&self, operator: u16) -> Option<&[f32]> {
        self.entries
            .iter()
            .rev()
            .find(|(candidate, _)| *candidate == operator)
            .map(|(_, operands)| operands.as_slice())
    }

    fn offset(&self, operator: u16) -> Option<usize> {
        let values = self.operands(operator)?;
        number_to_usize(*values.last()?)
    }

    fn private_range(&self) -> Option<(usize, usize)> {
        let values = self.operands(18)?;
        if values.len() != 2 {
            return None;
        }
        Some((number_to_usize(values[1])?, number_to_usize(values[0])?))
    }
}

#[derive(Clone, Debug)]
struct CffFont<'a> {
    char_strings: CffIndex<'a>,
    global_subrs: CffIndex<'a>,
    local_subrs: Vec<Option<CffIndex<'a>>>,
    fd_by_glyph: Option<Vec<usize>>,
    charset: Vec<u16>,
}

impl<'a> CffFont<'a> {
    fn parse(data: &'a [u8]) -> Option<Self> {
        if data.get(0).copied()? != 1 {
            return None;
        }
        let header_size = usize::from(*data.get(2)?);
        if header_size < 4 || header_size > data.len() {
            return None;
        }
        let (_names, cursor) = CffIndex::parse(data, header_size)?;
        let (top_dicts, cursor) = CffIndex::parse(data, cursor)?;
        if top_dicts.len() != 1 {
            return None;
        }
        let (_strings, cursor) = CffIndex::parse(data, cursor)?;
        let (global_subrs, _) = CffIndex::parse(data, cursor)?;
        let top = Dict::parse(top_dicts.get(0)?)?;
        if top
            .operands(0x0c06)
            .and_then(|values| values.last())
            .is_some_and(|value| *value != 2.0)
        {
            return None;
        }

        let char_strings_offset = top.offset(17)?;
        let (char_strings, _) = CffIndex::parse(data, char_strings_offset)?;
        if char_strings.len() == 0 || char_strings.len() > usize::from(u16::MAX) {
            return None;
        }
        let glyph_count = char_strings.len();
        let charset = parse_charset(data, top.offset(15).unwrap_or(0), glyph_count)?;
        let is_cid = top.operands(0x0c1e).is_some();

        let (local_subrs, fd_by_glyph) = if is_cid {
            let fd_array_offset = top.offset(0x0c24)?;
            let fd_select_offset = top.offset(0x0c25)?;
            let (fd_array, _) = CffIndex::parse(data, fd_array_offset)?;
            if fd_array.len() == 0 {
                return None;
            }
            let mut local_subrs = Vec::with_capacity(fd_array.len());
            for index in 0..fd_array.len() {
                let dict = Dict::parse(fd_array.get(index)?)?;
                local_subrs.push(parse_local_subrs(data, dict.private_range())?);
            }
            let fd_by_glyph = parse_fd_select(data, fd_select_offset, glyph_count, fd_array.len())?;
            (local_subrs, Some(fd_by_glyph))
        } else {
            (vec![parse_local_subrs(data, top.private_range())?], None)
        };

        Some(Self {
            char_strings,
            global_subrs,
            local_subrs,
            fd_by_glyph,
            charset,
        })
    }

    fn local_subrs(&self, glyph: GlyphId) -> Option<&CffIndex<'a>> {
        let fd = self
            .fd_by_glyph
            .as_ref()
            .and_then(|map| map.get(usize::from(glyph.0)).copied())
            .unwrap_or(0);
        self.local_subrs.get(fd)?.as_ref()
    }

    fn glyph_for_standard_code(&self, code: u8) -> Option<GlyphId> {
        let sid = standard_encoding_sid(code)?;
        self.charset
            .iter()
            .position(|candidate| *candidate == sid)
            .and_then(|index| u16::try_from(index).ok())
            .map(GlyphId)
    }
}

#[derive(Clone, Debug, Default)]
struct VariationStore {
    region_scalars: Vec<f32>,
    item_regions: Vec<Vec<usize>>,
}

impl VariationStore {
    fn parse(data: &[u8], offset: usize) -> Option<Self> {
        let declared_length = usize::from(read_u16(data, offset)?);
        let store_start = offset.checked_add(2)?;
        let store = if declared_length == 0 {
            data.get(store_start..)?
        } else {
            checked_slice(data, store_start, declared_length)?
        };
        if read_u16(store, 0)? != 1 {
            return None;
        }
        let region_list_offset = usize::try_from(read_u32(store, 2)?).ok()?;
        let data_count = usize::from(read_u16(store, 6)?);
        checked_slice(store, 8, data_count.checked_mul(4)?)?;

        let axis_count = usize::from(read_u16(store, region_list_offset)?);
        let region_count = usize::from(read_u16(store, region_list_offset + 2)?);
        let region_records = region_list_offset.checked_add(4)?;
        checked_slice(
            store,
            region_records,
            region_count.checked_mul(axis_count)?.checked_mul(6)?,
        )?;
        let mut region_scalars = Vec::with_capacity(region_count);
        for region in 0..region_count {
            let mut scalar = 1.0f32;
            for axis in 0..axis_count {
                let record = region_records + (region * axis_count + axis) * 6;
                scalar *= evaluate_default_region_axis(
                    read_i16(store, record)?,
                    read_i16(store, record + 2)?,
                    read_i16(store, record + 4)?,
                );
            }
            region_scalars.push(scalar);
        }

        let mut item_regions = Vec::with_capacity(data_count);
        for index in 0..data_count {
            let item_offset = usize::try_from(read_u32(store, 8 + index * 4)?).ok()?;
            let region_index_count = usize::from(read_u16(store, item_offset + 4)?);
            checked_slice(store, item_offset + 6, region_index_count.checked_mul(2)?)?;
            let mut indices = Vec::with_capacity(region_index_count);
            for region in 0..region_index_count {
                let value = usize::from(read_u16(store, item_offset + 6 + region * 2)?);
                if value >= region_count {
                    return None;
                }
                indices.push(value);
            }
            item_regions.push(indices);
        }
        Some(Self {
            region_scalars,
            item_regions,
        })
    }

    fn scalars(&self, index: usize) -> Option<Vec<f32>> {
        if self.item_regions.is_empty() && index == 0 {
            return Some(Vec::new());
        }
        self.item_regions.get(index).map(|regions| {
            regions
                .iter()
                .map(|region| self.region_scalars[*region])
                .collect()
        })
    }
}

fn evaluate_default_region_axis(start: i16, peak: i16, end: i16) -> f32 {
    if start > peak || peak > end || (start < 0 && end > 0 && peak != 0) || peak == 0 {
        return 1.0;
    }
    if 0 <= start || end <= 0 {
        return 0.0;
    }
    if 0 < peak {
        f32::from(-start) / f32::from(peak - start)
    } else {
        f32::from(end) / f32::from(end - peak)
    }
}

#[derive(Clone, Debug)]
struct Cff2Font<'a> {
    char_strings: CffIndex<'a>,
    global_subrs: CffIndex<'a>,
    local_subrs: Vec<Option<CffIndex<'a>>>,
    fd_by_glyph: Option<Vec<usize>>,
    variation_store: VariationStore,
}

impl<'a> Cff2Font<'a> {
    fn parse(data: &'a [u8]) -> Option<Self> {
        if data.get(0).copied()? != 2 {
            return None;
        }
        let header_size = usize::from(*data.get(2)?);
        let top_dict_length = usize::from(read_u16(data, 3)?);
        if header_size < 5 {
            return None;
        }
        let top_dict = checked_slice(data, header_size, top_dict_length)?;
        let top = Dict::parse(top_dict)?;
        let globals_start = header_size.checked_add(top_dict_length)?;
        let (global_subrs, _) = CffIndex::parse32(data, globals_start)?;
        let (char_strings, _) = CffIndex::parse32(data, top.offset(17)?)?;
        if char_strings.len() == 0 || char_strings.len() > usize::from(u16::MAX) {
            return None;
        }

        let variation_store = match top.offset(24) {
            Some(offset) => VariationStore::parse(data, offset)?,
            None => VariationStore::default(),
        };
        let (local_subrs, fd_by_glyph) = if let Some(fd_array_offset) = top.offset(0x0c24) {
            let (font_dicts, _) = CffIndex::parse32(data, fd_array_offset)?;
            if font_dicts.len() == 0 {
                return None;
            }
            let mut local_subrs = Vec::with_capacity(font_dicts.len());
            for index in 0..font_dicts.len() {
                let dict = Dict::parse(font_dicts.get(index)?)?;
                local_subrs.push(parse_local_subrs32(data, dict.private_range())?);
            }
            let fd_by_glyph = match top.offset(0x0c25) {
                Some(offset) => Some(parse_fd_select(
                    data,
                    offset,
                    char_strings.len(),
                    font_dicts.len(),
                )?),
                None if font_dicts.len() == 1 => None,
                None => return None,
            };
            (local_subrs, fd_by_glyph)
        } else {
            (vec![None], None)
        };

        Some(Self {
            char_strings,
            global_subrs,
            local_subrs,
            fd_by_glyph,
            variation_store,
        })
    }

    fn local_subrs(&self, glyph: GlyphId) -> Option<&CffIndex<'a>> {
        let fd = self
            .fd_by_glyph
            .as_ref()
            .and_then(|map| map.get(usize::from(glyph.0)).copied())
            .unwrap_or(0);
        self.local_subrs.get(fd)?.as_ref()
    }
}

#[derive(Clone, Debug)]
pub(crate) struct CffOutlines<'a> {
    font: CffFont<'a>,
}

#[derive(Clone, Debug)]
pub(crate) struct Cff2Outlines<'a> {
    font: Cff2Font<'a>,
}

impl<'a> Cff2Outlines<'a> {
    pub(crate) fn parse(face: &Face<'a>) -> Option<Self> {
        Some(Self {
            font: Cff2Font::parse(face.table(*b"CFF2")?)?,
        })
    }

    pub(crate) fn outline(
        &self,
        glyph: GlyphId,
        builder: &mut impl OutlineBuilder,
    ) -> Option<Rect> {
        let program = self.font.char_strings.get(usize::from(glyph.0))?;
        let mut tracking = TrackingBuilder::new(builder);
        let mut interpreter =
            Interpreter::new_cff2(&self.font, self.font.local_subrs(glyph), &mut tracking)?;
        interpreter.execute(program, 0)?;
        tracking.bounds.to_rect()
    }
}

impl<'a> CffOutlines<'a> {
    pub(crate) fn parse(face: &Face<'a>) -> Option<Self> {
        Some(Self {
            font: CffFont::parse(face.table(*b"CFF ")?)?,
        })
    }

    pub(crate) fn outline(
        &self,
        glyph: GlyphId,
        builder: &mut impl OutlineBuilder,
    ) -> Option<Rect> {
        if usize::from(glyph.0) >= self.font.char_strings.len() {
            return None;
        }
        let mut tracking = TrackingBuilder::new(builder);
        let local_subrs = self.font.local_subrs(glyph);
        let mut interpreter = Interpreter::new_cff1(&self.font, local_subrs, &mut tracking);
        let program = self.font.char_strings.get(usize::from(glyph.0))?;
        if interpreter.execute(program, 0)? != Stop::EndChar {
            return None;
        }
        tracking.bounds.to_rect()
    }
}

pub(crate) fn outline(
    face: &Face<'_>,
    glyph: GlyphId,
    builder: &mut impl OutlineBuilder,
) -> Option<Rect> {
    if face.has_cff1_outlines() {
        CffOutlines::parse(face)?.outline(glyph, builder)
    } else {
        Cff2Outlines::parse(face)?.outline(glyph, builder)
    }
}

fn parse_local_subrs<'a>(
    data: &'a [u8],
    private_range: Option<(usize, usize)>,
) -> Option<Option<CffIndex<'a>>> {
    let Some((offset, size)) = private_range else {
        return Some(None);
    };
    let private = checked_slice(data, offset, size)?;
    let dict = Dict::parse(private)?;
    let Some(relative) = dict.offset(19) else {
        return Some(None);
    };
    let start = offset.checked_add(relative)?;
    let (index, _) = CffIndex::parse(data, start)?;
    Some(Some(index))
}

fn parse_local_subrs32<'a>(
    data: &'a [u8],
    private_range: Option<(usize, usize)>,
) -> Option<Option<CffIndex<'a>>> {
    let Some((offset, size)) = private_range else {
        return Some(None);
    };
    let private = checked_slice(data, offset, size)?;
    let dict = Dict::parse(private)?;
    let Some(relative) = dict.offset(19) else {
        return Some(None);
    };
    let start = offset.checked_add(relative)?;
    let (index, _) = CffIndex::parse32(data, start)?;
    Some(Some(index))
}

fn parse_fd_select(
    data: &[u8],
    offset: usize,
    glyph_count: usize,
    fd_count: usize,
) -> Option<Vec<usize>> {
    let format = *data.get(offset)?;
    let mut out = vec![usize::MAX; glyph_count];
    match format {
        0 => {
            let values = checked_slice(data, offset.checked_add(1)?, glyph_count)?;
            for (target, &value) in out.iter_mut().zip(values) {
                *target = usize::from(value);
            }
        }
        3 => {
            let range_count = usize::from(read_u16(data, offset + 1)?);
            let ranges_start = offset.checked_add(3)?;
            checked_slice(
                data,
                ranges_start,
                range_count.checked_mul(3)?.checked_add(2)?,
            )?;
            let mut ranges = Vec::with_capacity(range_count);
            for index in 0..range_count {
                let record = ranges_start + index * 3;
                ranges.push((
                    usize::from(read_u16(data, record)?),
                    usize::from(*data.get(record + 2)?),
                ));
            }
            let sentinel = usize::from(read_u16(data, ranges_start + range_count * 3)?);
            if ranges.first().map(|range| range.0) != Some(0) || sentinel != glyph_count {
                return None;
            }
            for index in 0..ranges.len() {
                let start = ranges[index].0;
                let end = ranges
                    .get(index + 1)
                    .map(|range| range.0)
                    .unwrap_or(sentinel);
                if start > end || end > glyph_count {
                    return None;
                }
                out.get_mut(start..end)?.fill(ranges[index].1);
            }
        }
        4 => {
            let range_count = usize::try_from(read_u32(data, offset + 1)?).ok()?;
            let ranges_start = offset.checked_add(5)?;
            checked_slice(
                data,
                ranges_start,
                range_count.checked_mul(6)?.checked_add(4)?,
            )?;
            let mut ranges = Vec::with_capacity(range_count);
            for index in 0..range_count {
                let record = ranges_start + index * 6;
                ranges.push((
                    usize::try_from(read_u32(data, record)?).ok()?,
                    usize::from(read_u16(data, record + 4)?),
                ));
            }
            let sentinel = usize::try_from(read_u32(data, ranges_start + range_count * 6)?).ok()?;
            if ranges.first().map(|range| range.0) != Some(0) || sentinel != glyph_count {
                return None;
            }
            for index in 0..ranges.len() {
                let start = ranges[index].0;
                let end = ranges
                    .get(index + 1)
                    .map(|range| range.0)
                    .unwrap_or(sentinel);
                if start > end || end > glyph_count {
                    return None;
                }
                out.get_mut(start..end)?.fill(ranges[index].1);
            }
        }
        _ => return None,
    }
    if out.iter().any(|fd| *fd >= fd_count) {
        return None;
    }
    Some(out)
}

fn parse_charset(data: &[u8], offset: usize, glyph_count: usize) -> Option<Vec<u16>> {
    if glyph_count == 0 {
        return None;
    }
    if offset == 0 {
        return (glyph_count <= 229).then(|| (0..glyph_count as u16).collect());
    }
    // Predefined Expert charsets are handled by their standardized SID lists below.
    if offset == 1 {
        return predefined_expert_charset(glyph_count, false);
    }
    if offset == 2 {
        return predefined_expert_charset(glyph_count, true);
    }

    let format = *data.get(offset)?;
    let mut cursor = offset.checked_add(1)?;
    let mut out = Vec::with_capacity(glyph_count);
    out.push(0);
    match format {
        0 => {
            for _ in 1..glyph_count {
                out.push(read_u16(data, cursor)?);
                cursor = cursor.checked_add(2)?;
            }
        }
        1 | 2 => {
            while out.len() < glyph_count {
                let first = read_u16(data, cursor)?;
                cursor = cursor.checked_add(2)?;
                let left = if format == 1 {
                    let value = usize::from(*data.get(cursor)?);
                    cursor += 1;
                    value
                } else {
                    let value = usize::from(read_u16(data, cursor)?);
                    cursor += 2;
                    value
                };
                let count = left.checked_add(1)?;
                if count > glyph_count - out.len() {
                    return None;
                }
                for delta in 0..count {
                    out.push(first.checked_add(u16::try_from(delta).ok()?)?);
                }
            }
        }
        _ => return None,
    }
    Some(out)
}

fn predefined_expert_charset(glyph_count: usize, subset: bool) -> Option<Vec<u16>> {
    #[rustfmt::skip]
    const EXPERT: &[u16] = &[
          0,   1, 229, 230, 231, 232, 233, 234, 235, 236, 237, 238,  13,  14,  15,  99,
        239, 240, 241, 242, 243, 244, 245, 246, 247, 248,  27,  28, 249, 250, 251, 252,
        253, 254, 255, 256, 257, 258, 259, 260, 261, 262, 263, 264, 265, 266, 109, 110,
        267, 268, 269, 270, 271, 272, 273, 274, 275, 276, 277, 278, 279, 280, 281, 282,
        283, 284, 285, 286, 287, 288, 289, 290, 291, 292, 293, 294, 295, 296, 297, 298,
        299, 300, 301, 302, 303, 304, 305, 306, 307, 308, 309, 310, 311, 312, 313, 314,
        315, 316, 317, 318, 158, 155, 163, 319, 320, 321, 322, 323, 324, 325, 326, 150,
        164, 169, 327, 328, 329, 330, 331, 332, 333, 334, 335, 336, 337, 338, 339, 340,
        341, 342, 343, 344, 345, 346, 347, 348, 349, 350, 351, 352, 353, 354, 355, 356,
        357, 358, 359, 360, 361, 362, 363, 364, 365, 366, 367, 368, 369, 370, 371, 372,
        373, 374, 375, 376, 377, 378,
    ];
    #[rustfmt::skip]
    const EXPERT_SUBSET: &[u16] = &[
          0,   1, 231, 232, 235, 236, 237, 238,  13,  14,  15,  99, 239, 240, 241, 242,
        243, 244, 245, 246, 247, 248,  27,  28, 249, 250, 251, 253, 254, 255, 256, 257,
        258, 259, 260, 261, 262, 263, 264, 265, 266, 109, 110, 267, 268, 269, 270, 272,
        300, 301, 302, 305, 314, 315, 158, 155, 163, 320, 321, 322, 323, 324, 325, 326,
        150, 164, 169, 327, 328, 329, 330, 331, 332, 333, 334, 335, 336, 337, 338, 339,
        340, 341, 342, 343, 344, 345, 346,
    ];
    let source = if subset { EXPERT_SUBSET } else { EXPERT };
    source.get(..glyph_count).map(<[u16]>::to_vec)
}

fn standard_encoding_sid(code: u8) -> Option<u16> {
    #[rustfmt::skip]
    const STANDARD: [u8; 256] = [
          0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,
          0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,
          1,   2,   3,   4,   5,   6,   7,   8,   9,  10,  11,  12,  13,  14,  15,  16,
         17,  18,  19,  20,  21,  22,  23,  24,  25,  26,  27,  28,  29,  30,  31,  32,
         33,  34,  35,  36,  37,  38,  39,  40,  41,  42,  43,  44,  45,  46,  47,  48,
         49,  50,  51,  52,  53,  54,  55,  56,  57,  58,  59,  60,  61,  62,  63,  64,
         65,  66,  67,  68,  69,  70,  71,  72,  73,  74,  75,  76,  77,  78,  79,  80,
         81,  82,  83,  84,  85,  86,  87,  88,  89,  90,  91,  92,  93,  94,  95,   0,
          0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,
          0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,
          0,  96,  97,  98,  99, 100, 101, 102, 103, 104, 105, 106, 107, 108, 109, 110,
          0, 111, 112, 113, 114,   0, 115, 116, 117, 118, 119, 120, 121, 122,   0, 123,
          0, 124, 125, 126, 127, 128, 129, 130, 131,   0, 132, 133,   0, 134, 135, 136,
        137,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,
          0, 138,   0, 139,   0,   0,   0,   0, 140, 141, 142, 143,   0,   0,   0,   0,
          0, 144,   0,   0,   0, 145,   0,   0, 146, 147, 148, 149,   0,   0,   0,   0,
    ];
    Some(u16::from(STANDARD[usize::from(code)]))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Stop {
    Return,
    EndChar,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CharStringKind {
    Cff1,
    Cff2,
}

struct Interpreter<'a, 'state, 'output, B: OutlineBuilder> {
    cff1: Option<&'a CffFont<'a>>,
    global_subrs: &'a CffIndex<'a>,
    local_subrs: Option<&'a CffIndex<'a>>,
    variation_store: Option<&'a VariationStore>,
    scalars: Vec<f32>,
    kind: CharStringKind,
    had_vsindex: bool,
    had_blend: bool,
    builder: &'state mut TrackingBuilder<'output, B>,
    stack: Vec<f32>,
    x: f32,
    y: f32,
    stem_count: usize,
    has_move: bool,
    first_move: bool,
    width_seen: bool,
}

impl<'a, 'state, 'output, B: OutlineBuilder> Interpreter<'a, 'state, 'output, B> {
    fn new_cff1(
        font: &'a CffFont<'a>,
        local_subrs: Option<&'a CffIndex<'a>>,
        builder: &'state mut TrackingBuilder<'output, B>,
    ) -> Self {
        Self {
            cff1: Some(font),
            global_subrs: &font.global_subrs,
            local_subrs,
            variation_store: None,
            scalars: Vec::new(),
            kind: CharStringKind::Cff1,
            had_vsindex: false,
            had_blend: false,
            builder,
            stack: Vec::with_capacity(MAX_ARGUMENT_STACK),
            x: 0.0,
            y: 0.0,
            stem_count: 0,
            has_move: false,
            first_move: true,
            width_seen: false,
        }
    }

    fn new_cff2(
        font: &'a Cff2Font<'a>,
        local_subrs: Option<&'a CffIndex<'a>>,
        builder: &'state mut TrackingBuilder<'output, B>,
    ) -> Option<Self> {
        Some(Self {
            cff1: None,
            global_subrs: &font.global_subrs,
            local_subrs,
            variation_store: Some(&font.variation_store),
            scalars: font.variation_store.scalars(0)?,
            kind: CharStringKind::Cff2,
            had_vsindex: false,
            had_blend: false,
            builder,
            stack: Vec::with_capacity(64),
            x: 0.0,
            y: 0.0,
            stem_count: 0,
            has_move: false,
            first_move: true,
            width_seen: true,
        })
    }

    fn execute(&mut self, program: &'a [u8], depth: usize) -> Option<Stop> {
        if depth > MAX_SUBROUTINE_DEPTH {
            return None;
        }
        let mut cursor = 0usize;
        while cursor < program.len() {
            let operator = *program.get(cursor)?;
            cursor += 1;
            if is_charstring_number(operator) {
                let value = parse_charstring_number(operator, program, &mut cursor)?;
                let stack_limit = if self.kind == CharStringKind::Cff1 {
                    MAX_ARGUMENT_STACK
                } else {
                    513
                };
                if self.stack.len() >= stack_limit {
                    return None;
                }
                self.stack.push(value);
                continue;
            }
            match operator {
                1 | 3 | 18 | 23 => self.stems()?,
                4 => self.vmoveto()?,
                5 => self.rlineto()?,
                6 => self.hlineto()?,
                7 => self.vlineto()?,
                8 => self.rrcurveto()?,
                10 => {
                    let subroutine = self.resolve_subroutine(false)?;
                    if self.execute(subroutine, depth + 1)? == Stop::EndChar {
                        if cursor != program.len() {
                            return None;
                        }
                        return Some(Stop::EndChar);
                    }
                }
                11 if self.kind == CharStringKind::Cff1 => return Some(Stop::Return),
                12 => {
                    let escaped = *program.get(cursor)?;
                    cursor += 1;
                    match escaped {
                        34 => self.hflex()?,
                        35 => self.flex()?,
                        36 => self.hflex1()?,
                        37 => self.flex1()?,
                        _ => return None,
                    }
                }
                14 if self.kind == CharStringKind::Cff1 => {
                    self.endchar(depth)?;
                    if cursor != program.len() {
                        return None;
                    }
                    return Some(Stop::EndChar);
                }
                19 | 20 => {
                    self.hint_mask()?;
                    let bytes = self.stem_count.checked_add(7)? / 8;
                    checked_slice(program, cursor, bytes)?;
                    cursor = cursor.checked_add(bytes)?;
                }
                15 if self.kind == CharStringKind::Cff2 => self.vsindex()?,
                16 if self.kind == CharStringKind::Cff2 => self.blend()?,
                21 => self.rmoveto()?,
                22 => self.hmoveto()?,
                24 => self.rcurveline()?,
                25 => self.rlinecurve()?,
                26 => self.vvcurveto()?,
                27 => self.hhcurveto()?,
                29 => {
                    let subroutine = self.resolve_subroutine(true)?;
                    if self.execute(subroutine, depth + 1)? == Stop::EndChar {
                        if cursor != program.len() {
                            return None;
                        }
                        return Some(Stop::EndChar);
                    }
                }
                30 => self.vhcurveto()?,
                31 => self.hvcurveto()?,
                _ => return None,
            }
        }
        Some(Stop::Return)
    }

    fn resolve_subroutine(&mut self, global: bool) -> Option<&'a [u8]> {
        let operand = self.stack.pop()?;
        let index = f32_to_i32(operand)?;
        let index = index.checked_add(subroutine_bias(if global {
            self.global_subrs.len()
        } else {
            self.local_subrs?.len()
        }))?;
        let index = usize::try_from(index).ok()?;
        if global {
            self.global_subrs.get(index)
        } else {
            self.local_subrs?.get(index)
        }
    }

    fn vsindex(&mut self) -> Option<()> {
        if self.had_blend || self.had_vsindex || self.stack.len() != 1 {
            return None;
        }
        let index = usize::try_from(f32_to_i32(self.stack.pop()?)?).ok()?;
        self.scalars = self.variation_store?.scalars(index)?;
        self.had_vsindex = true;
        Some(())
    }

    fn blend(&mut self) -> Option<()> {
        self.had_blend = true;
        let count = usize::try_from(f32_to_i32(self.stack.pop()?)?).ok()?;
        let scalar_count = self.scalars.len();
        let blend_length = count.checked_mul(scalar_count.checked_add(1)?)?;
        let start = self.stack.len().checked_sub(blend_length)?;
        for value in (0..count).rev() {
            for scalar in (0..scalar_count).rev() {
                let delta = self.stack.pop()?;
                self.stack[start + value] += delta * self.scalars[scalar];
            }
        }
        Some(())
    }

    fn discard_width_for_stems(&mut self) {
        if self.kind == CharStringKind::Cff1 && self.stack.len() % 2 == 1 && !self.width_seen {
            self.stack.remove(0);
            self.width_seen = true;
        }
    }

    fn stems(&mut self) -> Option<()> {
        self.discard_width_for_stems();
        if self.stack.len() % 2 != 0 {
            return None;
        }
        self.stem_count = self.stem_count.checked_add(self.stack.len() / 2)?;
        self.stack.clear();
        Some(())
    }

    fn hint_mask(&mut self) -> Option<()> {
        self.stems()
    }

    fn prepare_move(&mut self) {
        if self.first_move {
            self.first_move = false;
        } else {
            self.builder.close();
        }
        self.has_move = true;
    }

    fn rmoveto(&mut self) -> Option<()> {
        let offset = if self.stack.len() == 3 && !self.width_seen {
            self.width_seen = true;
            1
        } else {
            0
        };
        if self.stack.len() != offset + 2 {
            return None;
        }
        self.prepare_move();
        self.x += self.stack[offset];
        self.y += self.stack[offset + 1];
        self.builder.move_to(self.x, self.y);
        self.stack.clear();
        Some(())
    }

    fn hmoveto(&mut self) -> Option<()> {
        let offset = if self.stack.len() == 2 && !self.width_seen {
            self.width_seen = true;
            1
        } else {
            0
        };
        if self.stack.len() != offset + 1 {
            return None;
        }
        self.prepare_move();
        self.x += self.stack[offset];
        self.builder.move_to(self.x, self.y);
        self.stack.clear();
        Some(())
    }

    fn vmoveto(&mut self) -> Option<()> {
        let offset = if self.stack.len() == 2 && !self.width_seen {
            self.width_seen = true;
            1
        } else {
            0
        };
        if self.stack.len() != offset + 1 {
            return None;
        }
        self.prepare_move();
        self.y += self.stack[offset];
        self.builder.move_to(self.x, self.y);
        self.stack.clear();
        Some(())
    }

    fn rlineto(&mut self) -> Option<()> {
        if !self.has_move || self.stack.len() % 2 != 0 {
            return None;
        }
        for pair in self.stack.chunks_exact(2) {
            self.x += pair[0];
            self.y += pair[1];
            self.builder.line_to(self.x, self.y);
        }
        self.stack.clear();
        Some(())
    }

    fn hlineto(&mut self) -> Option<()> {
        if !self.has_move || self.stack.is_empty() {
            return None;
        }
        for (index, value) in self.stack.iter().copied().enumerate() {
            if index % 2 == 0 {
                self.x += value;
            } else {
                self.y += value;
            }
            self.builder.line_to(self.x, self.y);
        }
        self.stack.clear();
        Some(())
    }

    fn vlineto(&mut self) -> Option<()> {
        if !self.has_move || self.stack.is_empty() {
            return None;
        }
        for (index, value) in self.stack.iter().copied().enumerate() {
            if index % 2 == 0 {
                self.y += value;
            } else {
                self.x += value;
            }
            self.builder.line_to(self.x, self.y);
        }
        self.stack.clear();
        Some(())
    }

    fn curve(&mut self, values: &[f32]) {
        let x1 = self.x + values[0];
        let y1 = self.y + values[1];
        let x2 = x1 + values[2];
        let y2 = y1 + values[3];
        self.x = x2 + values[4];
        self.y = y2 + values[5];
        self.builder.curve_to(x1, y1, x2, y2, self.x, self.y);
    }

    fn rrcurveto(&mut self) -> Option<()> {
        if !self.has_move || self.stack.len() % 6 != 0 {
            return None;
        }
        let values = std::mem::take(&mut self.stack);
        for curve in values.chunks_exact(6) {
            self.curve(curve);
        }
        self.stack = values;
        self.stack.clear();
        Some(())
    }

    fn rcurveline(&mut self) -> Option<()> {
        if !self.has_move || self.stack.len() < 8 || (self.stack.len() - 2) % 6 != 0 {
            return None;
        }
        let values = std::mem::take(&mut self.stack);
        let line = values.len() - 2;
        for curve in values[..line].chunks_exact(6) {
            self.curve(curve);
        }
        self.x += values[line];
        self.y += values[line + 1];
        self.builder.line_to(self.x, self.y);
        self.stack = values;
        self.stack.clear();
        Some(())
    }

    fn rlinecurve(&mut self) -> Option<()> {
        if !self.has_move || self.stack.len() < 8 || (self.stack.len() - 6) % 2 != 0 {
            return None;
        }
        let values = std::mem::take(&mut self.stack);
        let curve = values.len() - 6;
        for line in values[..curve].chunks_exact(2) {
            self.x += line[0];
            self.y += line[1];
            self.builder.line_to(self.x, self.y);
        }
        self.curve(&values[curve..]);
        self.stack = values;
        self.stack.clear();
        Some(())
    }

    fn hhcurveto(&mut self) -> Option<()> {
        if !self.has_move || self.stack.len() < 4 {
            return None;
        }
        let values = std::mem::take(&mut self.stack);
        let mut index = 0usize;
        if values.len() % 2 == 1 {
            self.y += values[0];
            index = 1;
        }
        if (values.len() - index) % 4 != 0 {
            return None;
        }
        while index < values.len() {
            let x1 = self.x + values[index];
            let y1 = self.y;
            let x2 = x1 + values[index + 1];
            let y2 = y1 + values[index + 2];
            self.x = x2 + values[index + 3];
            self.y = y2;
            self.builder.curve_to(x1, y1, x2, y2, self.x, self.y);
            index += 4;
        }
        self.stack = values;
        self.stack.clear();
        Some(())
    }

    fn vvcurveto(&mut self) -> Option<()> {
        if !self.has_move || self.stack.len() < 4 {
            return None;
        }
        let values = std::mem::take(&mut self.stack);
        let mut index = 0usize;
        if values.len() % 2 == 1 {
            self.x += values[0];
            index = 1;
        }
        if (values.len() - index) % 4 != 0 {
            return None;
        }
        while index < values.len() {
            let x1 = self.x;
            let y1 = self.y + values[index];
            let x2 = x1 + values[index + 1];
            let y2 = y1 + values[index + 2];
            self.x = x2;
            self.y = y2 + values[index + 3];
            self.builder.curve_to(x1, y1, x2, y2, self.x, self.y);
            index += 4;
        }
        self.stack = values;
        self.stack.clear();
        Some(())
    }

    fn alternating_curves(&mut self, horizontal_first: bool) -> Option<()> {
        if !self.has_move || self.stack.len() < 4 {
            return None;
        }
        let values = std::mem::take(&mut self.stack);
        let mut index = 0usize;
        let mut horizontal = horizontal_first;
        while index < values.len() {
            if values.len() - index < 4 {
                return None;
            }
            if horizontal {
                let x1 = self.x + values[index];
                let y1 = self.y;
                let x2 = x1 + values[index + 1];
                let y2 = y1 + values[index + 2];
                self.x = x2;
                self.y = y2 + values[index + 3];
                index += 4;
                if values.len() - index == 1 {
                    self.x += values[index];
                    index += 1;
                }
                self.builder.curve_to(x1, y1, x2, y2, self.x, self.y);
            } else {
                let x1 = self.x;
                let y1 = self.y + values[index];
                let x2 = x1 + values[index + 1];
                let y2 = y1 + values[index + 2];
                self.x = x2 + values[index + 3];
                self.y = y2;
                index += 4;
                if values.len() - index == 1 {
                    self.y += values[index];
                    index += 1;
                }
                self.builder.curve_to(x1, y1, x2, y2, self.x, self.y);
            }
            horizontal = !horizontal;
        }
        self.stack = values;
        self.stack.clear();
        Some(())
    }

    fn hvcurveto(&mut self) -> Option<()> {
        self.alternating_curves(true)
    }

    fn vhcurveto(&mut self) -> Option<()> {
        self.alternating_curves(false)
    }

    fn flex(&mut self) -> Option<()> {
        if !self.has_move || self.stack.len() != 13 {
            return None;
        }
        let values = std::mem::take(&mut self.stack);
        self.curve(&values[..6]);
        self.curve(&values[6..12]);
        self.stack = values;
        self.stack.clear();
        Some(())
    }

    fn hflex(&mut self) -> Option<()> {
        if !self.has_move || self.stack.len() != 7 {
            return None;
        }
        let v = std::mem::take(&mut self.stack);
        let x1 = self.x + v[0];
        let y1 = self.y;
        let x2 = x1 + v[1];
        let y2 = y1 + v[2];
        let x3 = x2 + v[3];
        let y3 = y2;
        let x4 = x3 + v[4];
        let y4 = y2;
        let x5 = x4 + v[5];
        let y5 = self.y;
        self.x = x5 + v[6];
        self.builder.curve_to(x1, y1, x2, y2, x3, y3);
        self.builder.curve_to(x4, y4, x5, y5, self.x, self.y);
        self.stack = v;
        self.stack.clear();
        Some(())
    }

    fn hflex1(&mut self) -> Option<()> {
        if !self.has_move || self.stack.len() != 9 {
            return None;
        }
        let v = std::mem::take(&mut self.stack);
        let x1 = self.x + v[0];
        let y1 = self.y + v[1];
        let x2 = x1 + v[2];
        let y2 = y1 + v[3];
        let x3 = x2 + v[4];
        let y3 = y2;
        let x4 = x3 + v[5];
        let y4 = y2;
        let x5 = x4 + v[6];
        let y5 = y4 + v[7];
        self.x = x5 + v[8];
        self.builder.curve_to(x1, y1, x2, y2, x3, y3);
        self.builder.curve_to(x4, y4, x5, y5, self.x, self.y);
        self.stack = v;
        self.stack.clear();
        Some(())
    }

    fn flex1(&mut self) -> Option<()> {
        if !self.has_move || self.stack.len() != 11 {
            return None;
        }
        let v = std::mem::take(&mut self.stack);
        let start_x = self.x;
        let start_y = self.y;
        let x1 = self.x + v[0];
        let y1 = self.y + v[1];
        let x2 = x1 + v[2];
        let y2 = y1 + v[3];
        let x3 = x2 + v[4];
        let y3 = y2 + v[5];
        let x4 = x3 + v[6];
        let y4 = y3 + v[7];
        let x5 = x4 + v[8];
        let y5 = y4 + v[9];
        if (x5 - start_x).abs() > (y5 - start_y).abs() {
            self.x = x5 + v[10];
            self.y = start_y;
        } else {
            self.x = start_x;
            self.y = y5 + v[10];
        }
        self.builder.curve_to(x1, y1, x2, y2, x3, y3);
        self.builder.curve_to(x4, y4, x5, y5, self.x, self.y);
        self.stack = v;
        self.stack.clear();
        Some(())
    }

    fn endchar(&mut self, depth: usize) -> Option<()> {
        let font = self.cff1?;
        let seac = self.stack.len() == 4 || (!self.width_seen && self.stack.len() == 5);
        if seac {
            let values = std::mem::take(&mut self.stack);
            let offset = usize::from(values.len() == 5);
            let dx = values[offset];
            let dy = values[offset + 1];
            let base = u8::try_from(f32_to_i32(values[offset + 2])?).ok()?;
            let accent = u8::try_from(f32_to_i32(values[offset + 3])?).ok()?;
            let base_glyph = font.glyph_for_standard_code(base)?;
            let accent_glyph = font.glyph_for_standard_code(accent)?;
            let base_program = font.char_strings.get(usize::from(base_glyph.0))?;
            if self.execute(base_program, depth + 1)? != Stop::EndChar {
                return None;
            }
            self.x = dx;
            self.y = dy;
            self.first_move = true;
            self.has_move = false;
            let accent_program = font.char_strings.get(usize::from(accent_glyph.0))?;
            if self.execute(accent_program, depth + 1)? != Stop::EndChar {
                return None;
            }
            self.stack = values;
            self.stack.clear();
            return Some(());
        }
        if self.stack.len() == 1 && !self.width_seen {
            self.width_seen = true;
            self.stack.clear();
        } else if !self.stack.is_empty() {
            return None;
        }
        if !self.first_move {
            self.first_move = true;
            self.builder.close();
        }
        Some(())
    }
}

struct TrackingBuilder<'a, B: OutlineBuilder> {
    builder: &'a mut B,
    bounds: Bounds,
}

impl<'a, B: OutlineBuilder> TrackingBuilder<'a, B> {
    fn new(builder: &'a mut B) -> Self {
        Self {
            builder,
            bounds: Bounds::default(),
        }
    }

    fn move_to(&mut self, x: f32, y: f32) {
        self.bounds.extend(x, y);
        self.builder.move_to(x, y);
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.bounds.extend(x, y);
        self.builder.line_to(x, y);
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.bounds.extend(x1, y1);
        self.bounds.extend(x2, y2);
        self.bounds.extend(x, y);
        self.builder.curve_to(x1, y1, x2, y2, x, y);
    }

    fn close(&mut self) {
        self.builder.close();
    }
}

#[derive(Default)]
struct Bounds(Option<(f32, f32, f32, f32)>);

impl Bounds {
    fn extend(&mut self, x: f32, y: f32) {
        self.0 = Some(match self.0 {
            Some((x_min, y_min, x_max, y_max)) => {
                (x_min.min(x), y_min.min(y), x_max.max(x), y_max.max(y))
            }
            None => (x, y, x, y),
        });
    }

    fn to_rect(&self) -> Option<Rect> {
        let (x_min, y_min, x_max, y_max) = self.0?;
        Some(Rect {
            x_min: float_to_i16(x_min)?,
            y_min: float_to_i16(y_min)?,
            x_max: float_to_i16(x_max)?,
            y_max: float_to_i16(y_max)?,
        })
    }
}

fn subroutine_bias(count: usize) -> i32 {
    if count < 1240 {
        107
    } else if count < 33_900 {
        1131
    } else {
        32_768
    }
}

fn is_dict_number(byte: u8) -> bool {
    matches!(byte, 28..=30 | 32..=255)
}

fn parse_dict_number(data: &[u8], cursor: &mut usize) -> Option<f32> {
    let byte = *data.get(*cursor)?;
    *cursor += 1;
    match byte {
        28 => {
            let value = read_i16(data, *cursor)?;
            *cursor += 2;
            Some(f32::from(value))
        }
        29 => {
            let value = read_i32(data, *cursor)?;
            *cursor += 4;
            Some(value as f32)
        }
        30 => parse_real_number(data, cursor),
        32..=246 => Some(f32::from(i16::from(byte) - 139)),
        247..=250 => {
            let next = i32::from(*data.get(*cursor)?);
            *cursor += 1;
            Some(((i32::from(byte) - 247) * 256 + next + 108) as f32)
        }
        251..=254 => {
            let next = i32::from(*data.get(*cursor)?);
            *cursor += 1;
            Some((-(i32::from(byte) - 251) * 256 - next - 108) as f32)
        }
        255 => {
            let value = read_i32(data, *cursor)?;
            *cursor += 4;
            Some(value as f32 / 65_536.0)
        }
        _ => None,
    }
}

fn parse_real_number(data: &[u8], cursor: &mut usize) -> Option<f32> {
    let mut text = String::new();
    loop {
        let byte = *data.get(*cursor)?;
        *cursor += 1;
        for nibble in [byte >> 4, byte & 0x0f] {
            match nibble {
                0..=9 => text.push(char::from(b'0' + nibble)),
                0x0a => text.push('.'),
                0x0b => text.push('E'),
                0x0c => text.push_str("E-"),
                0x0d => return None,
                0x0e => text.push('-'),
                0x0f => return text.parse().ok(),
                _ => return None,
            }
        }
    }
}

fn is_charstring_number(byte: u8) -> bool {
    matches!(byte, 28 | 32..=255)
}

fn parse_charstring_number(byte: u8, data: &[u8], cursor: &mut usize) -> Option<f32> {
    match byte {
        28 => {
            let value = read_i16(data, *cursor)?;
            *cursor += 2;
            Some(f32::from(value))
        }
        32..=246 => Some(f32::from(i16::from(byte) - 139)),
        247..=250 => {
            let next = i32::from(*data.get(*cursor)?);
            *cursor += 1;
            Some(((i32::from(byte) - 247) * 256 + next + 108) as f32)
        }
        251..=254 => {
            let next = i32::from(*data.get(*cursor)?);
            *cursor += 1;
            Some((-(i32::from(byte) - 251) * 256 - next - 108) as f32)
        }
        255 => {
            let value = read_i32(data, *cursor)?;
            *cursor += 4;
            Some(value as f32 / 65_536.0)
        }
        _ => None,
    }
}

fn number_to_usize(value: f32) -> Option<usize> {
    usize::try_from(f32_to_i32(value)?).ok()
}

fn f32_to_i32(value: f32) -> Option<i32> {
    if value.is_finite()
        && value.fract() == 0.0
        && value >= i32::MIN as f32
        && value <= i32::MAX as f32
    {
        Some(value as i32)
    } else {
        None
    }
}

fn float_to_i16(value: f32) -> Option<i16> {
    if value.is_finite() && value >= f32::from(i16::MIN) && value <= f32::from(i16::MAX) {
        Some(value as i16)
    } else {
        None
    }
}

fn read_offset(data: &[u8], offset: usize, size: usize) -> Option<usize> {
    let mut value = 0usize;
    for &byte in checked_slice(data, offset, size)? {
        value = value.checked_mul(256)?.checked_add(usize::from(byte))?;
    }
    Some(value)
}

fn checked_slice(data: &[u8], offset: usize, length: usize) -> Option<&[u8]> {
    data.get(offset..offset.checked_add(length)?)
}

fn read_u16(data: &[u8], offset: usize) -> Option<u16> {
    let bytes = checked_slice(data, offset, 2)?;
    Some(u16::from_be_bytes([bytes[0], bytes[1]]))
}

fn read_i16(data: &[u8], offset: usize) -> Option<i16> {
    read_u16(data, offset).map(|value| value as i16)
}

fn read_i32(data: &[u8], offset: usize) -> Option<i32> {
    let bytes = checked_slice(data, offset, 4)?;
    Some(i32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn read_u32(data: &[u8], offset: usize) -> Option<u32> {
    let bytes = checked_slice(data, offset, 4)?;
    Some(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

#[cfg(test)]
mod tests {
    use super::OutlineBuilder;
    use crate::sfnt::{Face, GlyphId};

    #[derive(Clone, Copy, Debug)]
    enum Command {
        Move(f32, f32),
        Line(f32, f32),
        Quad(f32, f32, f32, f32),
        Curve(f32, f32, f32, f32, f32, f32),
        Close,
    }

    #[derive(Default)]
    struct Recorder(Vec<Command>);

    impl OutlineBuilder for Recorder {
        fn move_to(&mut self, x: f32, y: f32) {
            self.0.push(Command::Move(x, y));
        }
        fn line_to(&mut self, x: f32, y: f32) {
            self.0.push(Command::Line(x, y));
        }
        fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
            self.0.push(Command::Quad(x1, y1, x, y));
        }
        fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
            self.0.push(Command::Curve(x1, y1, x2, y2, x, y));
        }
        fn close(&mut self) {
            self.0.push(Command::Close);
        }
    }

    fn cff_index(objects: &[&[u8]]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&(objects.len() as u16).to_be_bytes());
        if objects.is_empty() {
            return out;
        }
        out.push(1); // one-byte offsets are sufficient for this fixture
        let mut offset = 1usize;
        out.push(offset as u8);
        for object in objects {
            offset += object.len();
            out.push(offset as u8);
        }
        for object in objects {
            out.extend_from_slice(object);
        }
        out
    }

    fn cff2_index(objects: &[&[u8]]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&(objects.len() as u32).to_be_bytes());
        if objects.is_empty() {
            return out;
        }
        out.push(1);
        let mut offset = 1usize;
        out.push(offset as u8);
        for object in objects {
            offset += object.len();
            out.push(offset as u8);
        }
        for object in objects {
            out.extend_from_slice(object);
        }
        out
    }

    fn dict_long(out: &mut Vec<u8>, value: usize) {
        out.push(29);
        out.extend_from_slice(&(value as i32).to_be_bytes());
    }

    fn synthetic_cff1() -> Vec<u8> {
        let name = cff_index(&[b"Test"]);
        let strings = cff_index(&[]);
        let global_program = [239, 139, 139, 239, 5, 11]; // two rlineto segments, return
        let globals = cff_index(&[&global_program]);
        let notdef = [139, 22, 239, 139, 139, 239, 39, 139, 139, 39, 5, 14];
        let glyph = [139, 22, 32, 29, 32, 10, 14]; // hmoveto, global/local subrs, endchar
        let char_strings = cff_index(&[&notdef, &glyph]);
        let local_program = [39, 139, 139, 39, 5, 11]; // remaining square sides, return
        let local_subrs = cff_index(&[&local_program]);
        let private_size = 6usize;
        let mut private = Vec::new();
        dict_long(&mut private, private_size);
        private.push(19); // Subrs offset, relative to Private DICT
        assert_eq!(private.len(), private_size);

        // The Top DICT has fixed-width long integers, so its INDEX length is known up front.
        let top_dict_len = 17usize;
        let top_index_len = 2 + 1 + 2 + top_dict_len;
        let char_strings_offset = 4 + name.len() + top_index_len + strings.len() + globals.len();
        let private_offset = char_strings_offset + char_strings.len();
        let mut top_dict = Vec::new();
        dict_long(&mut top_dict, char_strings_offset);
        top_dict.push(17); // CharStrings
        dict_long(&mut top_dict, private_size);
        dict_long(&mut top_dict, private_offset);
        top_dict.push(18); // Private size and offset
        assert_eq!(top_dict.len(), top_dict_len);
        let top = cff_index(&[&top_dict]);

        let mut out = vec![1, 0, 4, 4];
        out.extend(name);
        out.extend(top);
        out.extend(strings);
        out.extend(globals);
        assert_eq!(out.len(), char_strings_offset);
        out.extend(char_strings);
        assert_eq!(out.len(), private_offset);
        out.extend(private);
        out.extend(local_subrs);
        out
    }

    fn synthetic_cff2() -> Vec<u8> {
        // At the default coordinate this region has scalar 1.0, so `10 20 1 blend` becomes 30.
        let mut variation_store = Vec::new();
        variation_store.extend_from_slice(&1u16.to_be_bytes()); // ItemVariationStore format
        variation_store.extend_from_slice(&12u32.to_be_bytes()); // region list offset
        variation_store.extend_from_slice(&1u16.to_be_bytes()); // item data count
        variation_store.extend_from_slice(&22u32.to_be_bytes()); // item data offset
        variation_store.extend_from_slice(&1u16.to_be_bytes()); // axis count
        variation_store.extend_from_slice(&1u16.to_be_bytes()); // region count
        variation_store.extend_from_slice(&i16::MIN.to_be_bytes());
        variation_store.extend_from_slice(&0i16.to_be_bytes());
        variation_store.extend_from_slice(&i16::MAX.to_be_bytes());
        variation_store.extend_from_slice(&0u16.to_be_bytes()); // item count (unused by blend)
        variation_store.extend_from_slice(&0u16.to_be_bytes()); // short delta count
        variation_store.extend_from_slice(&1u16.to_be_bytes()); // region index count
        variation_store.extend_from_slice(&0u16.to_be_bytes()); // region index
        let mut variation_table = Vec::new();
        variation_table.extend_from_slice(&(variation_store.len() as u16).to_be_bytes());
        variation_table.extend_from_slice(&variation_store);

        let glyph = [
            149, 159, 140, 16, 139, 21, 239, 139, 139, 239, 39, 139, 139, 39, 5,
        ];
        let char_strings = cff2_index(&[&glyph]);
        let globals = cff2_index(&[]);
        let top_length = 12usize;
        let char_strings_offset = 5 + top_length + globals.len();
        let variation_offset = char_strings_offset + char_strings.len();
        let mut top = Vec::new();
        dict_long(&mut top, char_strings_offset);
        top.push(17);
        dict_long(&mut top, variation_offset);
        top.push(24);
        assert_eq!(top.len(), top_length);

        let mut out = vec![2, 0, 5];
        out.extend_from_slice(&(top_length as u16).to_be_bytes());
        out.extend(top);
        out.extend(globals);
        assert_eq!(out.len(), char_strings_offset);
        out.extend(char_strings);
        assert_eq!(out.len(), variation_offset);
        out.extend(variation_table);
        out
    }

    #[test]
    fn synthetic_cff1_indexes_dicts_subroutines_and_paths_are_bounded() {
        let data = synthetic_cff1();
        let font = super::CffFont::parse(&data).expect("CFF font");
        let outlines = super::CffOutlines { font };
        let mut recorder = Recorder::default();
        let bbox = outlines
            .outline(GlyphId(1), &mut recorder)
            .expect("glyph outline");
        assert_eq!(
            (bbox.x_min, bbox.y_min, bbox.x_max, bbox.y_max),
            (0, 0, 100, 100)
        );
        assert!(matches!(
            recorder.0.as_slice(),
            [
                Command::Move(0.0, 0.0),
                Command::Line(100.0, 0.0),
                Command::Line(100.0, 100.0),
                Command::Line(0.0, 100.0),
                Command::Line(0.0, 0.0),
                Command::Close,
            ]
        ));

        for length in 0..data.len() {
            let _ = super::CffFont::parse(&data[..length]);
        }
    }

    #[test]
    fn synthetic_cff2_variation_blend_matches_frozen_contract() {
        let data = synthetic_cff2();
        let font = super::Cff2Font::parse(&data).expect("CFF2 font");
        let outlines = super::Cff2Outlines { font };
        let mut native = Recorder::default();
        let native_bbox = outlines
            .outline(GlyphId(0), &mut native)
            .expect("native outline");
        assert_eq!(
            (
                native_bbox.x_min,
                native_bbox.y_min,
                native_bbox.x_max,
                native_bbox.y_max
            ),
            (30, 0, 130, 100)
        );
        assert!(
            matches!(
                native.0.as_slice(),
                [
                    Command::Move(30.0, 0.0),
                    Command::Line(130.0, 0.0),
                    Command::Line(130.0, 100.0),
                    Command::Line(30.0, 100.0),
                    Command::Line(30.0, 0.0),
                ]
            ),
            "{:?}",
            native.0
        );

        for length in 0..data.len() {
            let _ = super::Cff2Font::parse(&data[..length]);
        }
    }

    fn command_is_finite(command: Command) -> bool {
        match command {
            Command::Move(x, y) | Command::Line(x, y) => x.is_finite() && y.is_finite(),
            Command::Quad(x1, y1, x, y) => [x1, y1, x, y].into_iter().all(f32::is_finite),
            Command::Curve(x1, y1, x2, y2, x, y) => {
                [x1, y1, x2, y2, x, y].into_iter().all(f32::is_finite)
            }
            Command::Close => true,
        }
    }

    #[test]
    #[ignore = "set FULLBLEED_CFF_FONT to a local CFF1 OpenType font"]
    fn external_cff1_font_outlines_are_finite_and_bounded() {
        let path = std::env::var_os("FULLBLEED_CFF_FONT")
            .or_else(|| std::env::var_os("FULLBLEED_CFF_ORACLE_FONT"))
            .expect("font path");
        let data = std::fs::read(path).expect("font data");
        let native_face = Face::parse(&data, 0).expect("native face");
        let native_cff = super::CffOutlines::parse(&native_face).expect("native CFF");
        assert!(native_face.has_cff_outlines());
        for glyph in 0..native_face.number_of_glyphs() {
            let mut native = Recorder::default();
            let native_bbox = native_cff.outline(GlyphId(glyph), &mut native);
            if std::env::var_os("FULLBLEED_CFF_TRACE_GLYPH")
                .and_then(|value| value.to_string_lossy().parse::<u16>().ok())
                == Some(glyph)
            {
                eprintln!("native {glyph}: {:?}", native.0);
            }
            if let Some(bbox) = native_bbox {
                assert!(bbox.x_min <= bbox.x_max, "glyph {glyph} x bounds");
                assert!(bbox.y_min <= bbox.y_max, "glyph {glyph} y bounds");
            }
            for (index, command) in native.0.into_iter().enumerate() {
                assert!(
                    command_is_finite(command),
                    "glyph {glyph} command {index}: {command:?}"
                );
            }
        }
    }
}
