//! Dependency-free TrueType font subsetting for PDF embedding.
//!
//! The PDF text path emits original glyph IDs through Identity-H. Keeping those IDs stable avoids
//! a CID remap table and lets subsetting happen at link time: unused `glyf` records become empty,
//! while used glyphs and every recursively referenced composite component retain their original
//! indices. All other tables remain byte-for-byte compatible with the original glyph namespace.

use std::collections::{BTreeSet, VecDeque};

const TRUE_TYPE_SIGNATURE: [u8; 4] = [0x00, 0x01, 0x00, 0x00];
const CHECKSUM_MAGIC: u32 = 0xB1B0_AFBA;

const REQUIRED_TABLES: [[u8; 4]; 10] = [
    *b"OS/2", *b"cmap", *b"glyf", *b"head", *b"hhea", *b"hmtx", *b"loca", *b"maxp", *b"name",
    *b"post",
];
const EMBED_TABLES: [[u8; 4]; 14] = [
    *b"OS/2", *b"cmap", *b"cvt ", *b"fpgm", *b"gasp", *b"glyf", *b"head", *b"hhea", *b"hmtx",
    *b"loca", *b"maxp", *b"name", *b"post", *b"prep",
];

const ARG_1_AND_2_ARE_WORDS: u16 = 0x0001;
const WE_HAVE_A_SCALE: u16 = 0x0008;
const MORE_COMPONENTS: u16 = 0x0020;
const WE_HAVE_AN_X_AND_Y_SCALE: u16 = 0x0040;
const WE_HAVE_A_TWO_BY_TWO: u16 = 0x0080;
const WE_HAVE_INSTRUCTIONS: u16 = 0x0100;

#[derive(Debug, Clone)]
pub(crate) struct TrueTypeSubset {
    pub(crate) data: Vec<u8>,
    pub(crate) tag: [u8; 6],
    pub(crate) glyph_count: usize,
}

#[derive(Clone, Copy)]
struct TableRecord<'a> {
    tag: [u8; 4],
    data: &'a [u8],
}

/// Build a deterministic TrueType subset while retaining original glyph IDs.
///
/// Unsupported outline formats and color/bitmap glyph technologies return `None`; the PDF linker
/// then embeds the complete source program. Variable inputs are compiled at their default instance
/// by retaining the base `glyf`/`hmtx` data and dropping variation tables, matching FullBleed's
/// current default-axis shaping contract. The result is also declined when it would not be smaller.
pub(crate) fn subset_truetype(
    source: &[u8],
    requested_glyphs: &BTreeSet<u16>,
) -> Option<TrueTypeSubset> {
    let signature: [u8; 4] = source.get(0..4)?.try_into().ok()?;
    if signature != TRUE_TYPE_SIGNATURE {
        return None;
    }

    let records = table_records(source)?;
    if REQUIRED_TABLES
        .iter()
        .any(|tag| !records.iter().any(|record| record.tag == *tag))
    {
        return None;
    }
    // SVG, bitmap, and layered-color glyph programs need table-specific dependency closure.
    // Preserve the existing full-font behavior until those subsetters exist.
    const UNSUPPORTED_GLYPH_TABLES: [[u8; 4]; 8] = [
        *b"CFF2", *b"SVG ", *b"COLR", *b"CBDT", *b"CBLC", *b"sbix", *b"EBDT", *b"EBLC",
    ];
    if records
        .iter()
        .any(|record| UNSUPPORTED_GLYPH_TABLES.contains(&record.tag))
    {
        return None;
    }

    let head = table(&records, *b"head")?;
    let maxp = table(&records, *b"maxp")?;
    let loca = table(&records, *b"loca")?;
    let glyf = table(&records, *b"glyf")?;
    if head.len() < 54 || maxp.len() < 6 {
        return None;
    }
    let glyph_total = usize::from(read_u16(maxp, 4)?);
    if glyph_total == 0 {
        return None;
    }
    let loca_is_long = match read_i16(head, 50)? {
        0 => false,
        1 => true,
        _ => return None,
    };
    let old_offsets = read_loca(loca, glyph_total, loca_is_long, glyf.len())?;

    let mut keep: BTreeSet<u16> = requested_glyphs
        .iter()
        .copied()
        .filter(|glyph| usize::from(*glyph) < glyph_total)
        .collect();
    keep.insert(0); // .notdef is required in every embedded subset.
    close_composite_glyphs(glyf, &old_offsets, glyph_total, &mut keep)?;

    let (new_glyf, new_loca) =
        rebuild_glyph_tables(glyf, &old_offsets, glyph_total, &keep, loca_is_long)?;
    let data = rebuild_sfnt(source, signature, &records, &new_glyf, &new_loca)?;
    if data.len() >= source.len() {
        return None;
    }

    Some(TrueTypeSubset {
        tag: deterministic_subset_tag(source, &keep),
        data,
        glyph_count: keep.len(),
    })
}

fn table_records(source: &[u8]) -> Option<Vec<TableRecord<'_>>> {
    let count = usize::from(read_u16(source, 4)?);
    if count == 0 || count > 4096 {
        return None;
    }
    let directory_end = 12usize.checked_add(count.checked_mul(16)?)?;
    source.get(0..directory_end)?;

    let mut records = Vec::with_capacity(count);
    for index in 0..count {
        let offset = 12 + index * 16;
        let tag = source.get(offset..offset + 4)?.try_into().ok()?;
        if records
            .iter()
            .any(|record: &TableRecord<'_>| record.tag == tag)
        {
            return None;
        }
        let table_offset = usize::try_from(read_u32(source, offset + 8)?).ok()?;
        let table_len = usize::try_from(read_u32(source, offset + 12)?).ok()?;
        let data = source.get(table_offset..table_offset.checked_add(table_len)?)?;
        records.push(TableRecord { tag, data });
    }
    Some(records)
}

fn table<'a>(records: &[TableRecord<'a>], tag: [u8; 4]) -> Option<&'a [u8]> {
    records
        .iter()
        .find(|record| record.tag == tag)
        .map(|record| record.data)
}

fn read_loca(
    loca: &[u8],
    glyph_total: usize,
    is_long: bool,
    glyf_len: usize,
) -> Option<Vec<usize>> {
    let entry_size = if is_long { 4 } else { 2 };
    let count = glyph_total.checked_add(1)?;
    loca.get(0..count.checked_mul(entry_size)?)?;
    let mut offsets = Vec::with_capacity(count);
    let mut prior = 0usize;
    for index in 0..count {
        let offset = if is_long {
            usize::try_from(read_u32(loca, index * 4)?).ok()?
        } else {
            usize::from(read_u16(loca, index * 2)?).checked_mul(2)?
        };
        if offset < prior || offset > glyf_len {
            return None;
        }
        offsets.push(offset);
        prior = offset;
    }
    Some(offsets)
}

fn close_composite_glyphs(
    glyf: &[u8],
    offsets: &[usize],
    glyph_total: usize,
    keep: &mut BTreeSet<u16>,
) -> Option<()> {
    let mut pending: VecDeque<u16> = keep.iter().copied().collect();
    let mut inspected = BTreeSet::new();
    while let Some(glyph) = pending.pop_front() {
        if !inspected.insert(glyph) {
            continue;
        }
        let index = usize::from(glyph);
        let start = *offsets.get(index)?;
        let end = *offsets.get(index + 1)?;
        let data = glyf.get(start..end)?;
        for component in composite_components(data, glyph_total)? {
            if keep.insert(component) {
                pending.push_back(component);
            }
        }
    }
    Some(())
}

fn composite_components(data: &[u8], glyph_total: usize) -> Option<Vec<u16>> {
    if data.is_empty() {
        return Some(Vec::new());
    }
    if data.len() < 10 {
        return None;
    }
    if read_i16(data, 0)? >= 0 {
        return Some(Vec::new());
    }

    let mut out = Vec::new();
    let mut cursor = 10usize;
    let final_flags = loop {
        let flags = read_u16(data, cursor)?;
        let glyph = read_u16(data, cursor + 2)?;
        if usize::from(glyph) >= glyph_total {
            return None;
        }
        out.push(glyph);
        cursor = cursor.checked_add(4)?;
        cursor = cursor.checked_add(if flags & ARG_1_AND_2_ARE_WORDS != 0 {
            4
        } else {
            2
        })?;
        cursor = cursor.checked_add(if flags & WE_HAVE_A_SCALE != 0 {
            2
        } else if flags & WE_HAVE_AN_X_AND_Y_SCALE != 0 {
            4
        } else if flags & WE_HAVE_A_TWO_BY_TWO != 0 {
            8
        } else {
            0
        })?;
        data.get(0..cursor)?;
        if out.len() > glyph_total {
            return None;
        }
        if flags & MORE_COMPONENTS == 0 {
            break flags;
        }
    };

    if final_flags & WE_HAVE_INSTRUCTIONS != 0 {
        let instruction_len = usize::from(read_u16(data, cursor)?);
        cursor = cursor.checked_add(2)?.checked_add(instruction_len)?;
        data.get(0..cursor)?;
    }
    Some(out)
}

fn rebuild_glyph_tables(
    source_glyf: &[u8],
    source_offsets: &[usize],
    glyph_total: usize,
    keep: &BTreeSet<u16>,
    loca_is_long: bool,
) -> Option<(Vec<u8>, Vec<u8>)> {
    let mut glyf = Vec::new();
    let mut offsets = Vec::with_capacity(glyph_total + 1);
    for index in 0..glyph_total {
        offsets.push(u32::try_from(glyf.len()).ok()?);
        if keep.contains(&u16::try_from(index).ok()?) {
            let start = *source_offsets.get(index)?;
            let end = *source_offsets.get(index + 1)?;
            glyf.extend_from_slice(source_glyf.get(start..end)?);
            if glyf.len() & 1 != 0 {
                glyf.push(0);
            }
        }
    }
    offsets.push(u32::try_from(glyf.len()).ok()?);

    let mut loca = Vec::with_capacity(offsets.len() * if loca_is_long { 4 } else { 2 });
    for offset in offsets {
        if loca_is_long {
            loca.extend_from_slice(&offset.to_be_bytes());
        } else {
            if offset & 1 != 0 {
                return None;
            }
            let short = u16::try_from(offset / 2).ok()?;
            loca.extend_from_slice(&short.to_be_bytes());
        }
    }
    Some((glyf, loca))
}

fn rebuild_sfnt(
    _source: &[u8],
    signature: [u8; 4],
    records: &[TableRecord<'_>],
    glyf: &[u8],
    loca: &[u8],
) -> Option<Vec<u8>> {
    let mut tables: Vec<([u8; 4], Vec<u8>)> = Vec::with_capacity(records.len());
    for record in records {
        if !EMBED_TABLES.contains(&record.tag) {
            // Layout has already been resolved to explicit GIDs and PDF positioning. Retaining
            // optional GSUB/GPOS/color/device metadata only bloats the embedded renderer program;
            // digital signatures must also be dropped after any modification.
            continue;
        }
        let mut data = match record.tag {
            tag if tag == *b"glyf" => glyf.to_vec(),
            tag if tag == *b"loca" => loca.to_vec(),
            _ => record.data.to_vec(),
        };
        if record.tag == *b"head" {
            write_u32(&mut data, 8, 0)?;
        }
        tables.push((record.tag, data));
    }
    tables.sort_unstable_by_key(|(tag, _)| *tag);

    let table_count = u16::try_from(tables.len()).ok()?;
    let directory_end = 12usize.checked_add(tables.len().checked_mul(16)?)?;
    let mut output = vec![0u8; align4(directory_end)?];
    output.get_mut(0..4)?.copy_from_slice(&signature);
    write_u16(&mut output, 4, table_count)?;
    let (search_range, entry_selector, range_shift) = sfnt_search_fields(table_count);
    write_u16(&mut output, 6, search_range)?;
    write_u16(&mut output, 8, entry_selector)?;
    write_u16(&mut output, 10, range_shift)?;

    let mut head_offset = None;
    for (index, (tag, data)) in tables.iter().enumerate() {
        let table_offset = align4(output.len())?;
        if table_offset > output.len() {
            output.resize(table_offset, 0);
        }
        let table_end = table_offset.checked_add(data.len())?;
        if table_end > u32::MAX as usize {
            return None;
        }
        output.resize(table_end, 0);
        output
            .get_mut(table_offset..table_end)?
            .copy_from_slice(data);
        output.resize(align4(table_end)?, 0);

        let record_offset = 12 + index * 16;
        output
            .get_mut(record_offset..record_offset + 4)?
            .copy_from_slice(tag);
        write_u32(&mut output, record_offset + 4, checksum(data))?;
        write_u32(&mut output, record_offset + 8, table_offset as u32)?;
        write_u32(
            &mut output,
            record_offset + 12,
            u32::try_from(data.len()).ok()?,
        )?;
        if tag == b"head" {
            head_offset = Some(table_offset);
        }
    }

    let head_offset = head_offset?;
    let adjustment = CHECKSUM_MAGIC.wrapping_sub(checksum(&output));
    write_u32(&mut output, head_offset + 8, adjustment)?;
    if checksum(&output) != CHECKSUM_MAGIC {
        return None;
    }
    Some(output)
}

fn sfnt_search_fields(table_count: u16) -> (u16, u16, u16) {
    let mut power = 1u16;
    let mut selector = 0u16;
    while power <= table_count / 2 {
        power *= 2;
        selector += 1;
    }
    let search_range = power * 16;
    let range_shift = table_count * 16 - search_range;
    (search_range, selector, range_shift)
}

fn deterministic_subset_tag(source: &[u8], glyphs: &BTreeSet<u16>) -> [u8; 6] {
    // FNV-1a over the compact SFNT identity (directory) and closed glyph set. This is a PDF
    // pseudo-unique tag, not a security boundary; avoiding a whole-font hash keeps linking cheap.
    let directory_len = read_u16(source, 4)
        .and_then(|count| 12usize.checked_add(usize::from(count).checked_mul(16)?))
        .unwrap_or(source.len())
        .min(source.len());
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in &source[..directory_len] {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01B3);
    }
    for glyph in glyphs {
        for byte in glyph.to_be_bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01B3);
        }
    }
    let mut tag = [b'A'; 6];
    for byte in tag.iter_mut().rev() {
        *byte = b'A' + (hash % 26) as u8;
        hash /= 26;
    }
    tag
}

fn align4(value: usize) -> Option<usize> {
    value.checked_add(3).map(|value| value & !3)
}

fn read_u16(data: &[u8], offset: usize) -> Option<u16> {
    let bytes = data.get(offset..offset.checked_add(2)?)?;
    Some(u16::from_be_bytes([bytes[0], bytes[1]]))
}

fn read_i16(data: &[u8], offset: usize) -> Option<i16> {
    read_u16(data, offset).map(|value| value as i16)
}

fn read_u32(data: &[u8], offset: usize) -> Option<u32> {
    let bytes = data.get(offset..offset.checked_add(4)?)?;
    Some(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn write_u16(data: &mut [u8], offset: usize, value: u16) -> Option<()> {
    data.get_mut(offset..offset.checked_add(2)?)?
        .copy_from_slice(&value.to_be_bytes());
    Some(())
}

fn write_u32(data: &mut [u8], offset: usize, value: u32) -> Option<()> {
    data.get_mut(offset..offset.checked_add(4)?)?
        .copy_from_slice(&value.to_be_bytes());
    Some(())
}

fn checksum(data: &[u8]) -> u32 {
    data.chunks(4).fold(0u32, |sum, chunk| {
        let mut word = [0u8; 4];
        word[..chunk.len()].copy_from_slice(chunk);
        sum.wrapping_add(u32::from_be_bytes(word))
    })
}

#[cfg(test)]
mod tests {
    use super::{
        CHECKSUM_MAGIC, checksum, read_i16, read_loca, subset_truetype, table, table_records,
    };
    use crate::sfnt::{Face, GlyphId};
    use crate::sfnt_outline::OutlineBuilder;
    use std::collections::BTreeSet;

    const NOTO: &[u8] = include_bytes!("../python/fullbleed_assets/fonts/NotoSansMath-Regular.ttf");
    const VARIABLE_NOTO: &[u8] =
        include_bytes!("../python/fullbleed_assets/fonts/NotoSans-Regular.ttf");

    #[derive(Default)]
    struct OutlineCounter(usize);

    impl OutlineBuilder for OutlineCounter {
        fn move_to(&mut self, _x: f32, _y: f32) {
            self.0 += 1;
        }
        fn line_to(&mut self, _x: f32, _y: f32) {
            self.0 += 1;
        }
        fn quad_to(&mut self, _x1: f32, _y1: f32, _x: f32, _y: f32) {
            self.0 += 1;
        }
        fn curve_to(&mut self, _x1: f32, _y1: f32, _x2: f32, _y2: f32, _x: f32, _y: f32) {
            self.0 += 1;
        }
        fn close(&mut self) {
            self.0 += 1;
        }
    }

    #[test]
    fn true_type_subset_is_smaller_valid_and_preserves_used_outlines() {
        let original = Face::parse(NOTO, 0).expect("parse source");
        let a = original.glyph_index('A' as u32).expect("A glyph");
        let records = table_records(NOTO).expect("table directory");
        let head = table(&records, *b"head").expect("head");
        let loca = table(&records, *b"loca").expect("loca");
        let glyf = table(&records, *b"glyf").expect("glyf");
        let offsets = read_loca(
            loca,
            usize::from(original.number_of_glyphs()),
            read_i16(head, 50) == Some(1),
            glyf.len(),
        )
        .expect("glyph locations");
        let composite = (0..original.number_of_glyphs())
            .find(|glyph| {
                let index = usize::from(*glyph);
                let data = &glyf[offsets[index]..offsets[index + 1]];
                data.len() >= 10 && read_i16(data, 0).is_some_and(|contours| contours < 0)
            })
            .map(GlyphId)
            .expect("composite glyph fixture");
        let glyphs = BTreeSet::from([a.0, composite.0]);

        let subset = subset_truetype(NOTO, &glyphs).expect("subset static TrueType");
        assert!(subset.data.len() < NOTO.len() / 2);
        assert!(subset.tag.iter().all(u8::is_ascii_uppercase));
        assert!(subset.glyph_count >= 3); // .notdef plus requested glyphs/components.
        assert_eq!(checksum(&subset.data), CHECKSUM_MAGIC);

        let parsed = Face::parse(&subset.data, 0).expect("parse subset");
        assert_eq!(parsed.number_of_glyphs(), original.number_of_glyphs());
        for glyph in [a, composite] {
            let mut outline = OutlineCounter::default();
            assert!(
                parsed
                    .outline_glyph(GlyphId(glyph.0), &mut outline)
                    .is_some()
            );
            assert!(outline.0 > 0);
        }
    }

    #[test]
    fn true_type_subset_is_byte_deterministic() {
        let face = Face::parse(NOTO, 0).expect("parse source");
        let glyphs = BTreeSet::from([
            face.glyph_index('A' as u32).expect("A").0,
            face.glyph_index('z' as u32).expect("z").0,
        ]);
        let first = subset_truetype(NOTO, &glyphs).expect("first subset");
        let second = subset_truetype(NOTO, &glyphs).expect("second subset");
        assert_eq!(first.tag, second.tag);
        assert_eq!(first.data, second.data);
    }

    #[test]
    fn variable_true_type_is_compiled_to_a_static_default_subset() {
        let face = Face::parse(VARIABLE_NOTO, 0).expect("parse variable source");
        let glyphs = BTreeSet::from([
            face.glyph_index('A' as u32).expect("A").0,
            face.glyph_index('z' as u32).expect("z").0,
        ]);
        let subset = subset_truetype(VARIABLE_NOTO, &glyphs).expect("default instance subset");
        assert!(subset.data.len() < VARIABLE_NOTO.len() / 3);
        let records = table_records(&subset.data).expect("subset directory");
        for variation_table in [*b"fvar", *b"gvar", *b"HVAR", *b"MVAR", *b"STAT", *b"avar"] {
            assert!(table(&records, variation_table).is_none());
        }
        let parsed = Face::parse(&subset.data, 0).expect("parse static subset");
        for glyph in glyphs {
            let mut outline = OutlineCounter::default();
            assert!(parsed.outline_glyph(GlyphId(glyph), &mut outline).is_some());
            assert!(outline.0 > 0);
        }
    }
}
