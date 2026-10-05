//! Prune common character maps without renumbering glyphs or changing encoding records.
//!
//! Other formats retain their original subtable bytes. Malformed input or a larger replacement
//! falls back to the source cmap, just as unsupported outlines retain the full font program.

use super::{read_u16, read_u32, write_u16, write_u32};
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn subset(source: &[u8], keep: &BTreeSet<u16>) -> Option<Vec<u8>> {
    if read_u16(source, 0)? != 0 {
        return None;
    }
    let count = usize::from(read_u16(source, 2)?);
    let header_size = 4usize.checked_add(count.checked_mul(8)?)?;
    let mut output = source.get(..header_size)?.to_vec();
    let mut offsets = BTreeMap::new();
    for index in 0..count {
        let offset = usize::try_from(read_u32(source, 4 + index * 8 + 4)?).ok()?;
        let new_offset = if let Some(offset) = offsets.get(&offset) {
            *offset
        } else {
            let tail = source.get(offset..)?;
            let format = read_u16(tail, 0)?;
            let length = match format {
                0 | 2 | 4 | 6 => usize::from(read_u16(tail, 2)?),
                8 | 10 | 12 | 13 => usize::try_from(read_u32(tail, 4)?).ok()?,
                14 => usize::try_from(read_u32(tail, 2)?).ok()?,
                _ => return None,
            };
            let original = tail.get(..length)?;
            let replacement = match format {
                4 => subset_format4(original, keep),
                12 => subset_format12(original, keep),
                _ => None,
            };
            let bytes = replacement
                .as_deref()
                .filter(|bytes| bytes.len() < original.len())
                .unwrap_or(original);
            if output.len().checked_add(bytes.len())? > source.len() {
                return None;
            }
            let new_offset = u32::try_from(output.len()).ok()?;
            output.extend_from_slice(bytes);
            offsets.insert(offset, new_offset);
            new_offset
        };
        write_u32(&mut output, 4 + index * 8 + 4, new_offset)?;
    }
    (output.len() < source.len()).then_some(output)
}

fn subset_format4(source: &[u8], keep: &BTreeSet<u16>) -> Option<Vec<u8>> {
    let count_x2 = read_u16(source, 6)?;
    if count_x2 == 0 || count_x2 % 2 != 0 {
        return None;
    }
    let count = usize::from(count_x2 / 2);
    let starts = 16 + count * 2;
    let deltas = starts + count * 2;
    let ranges = deltas + count * 2;
    source.get(..ranges + count * 2)?;
    let mut segments: Vec<(u16, u16, u16)> = Vec::new();
    let mut previous_end = None;
    for index in 0..count {
        let start = read_u16(source, starts + index * 2)?;
        let end = read_u16(source, 14 + index * 2)?;
        if start > end || previous_end.is_some_and(|prior| start <= prior) {
            return None;
        }
        previous_end = Some(end);
        let delta = read_u16(source, deltas + index * 2)?;
        let range_word = ranges + index * 2;
        let range = usize::from(read_u16(source, range_word)?);
        for code in start..=end {
            let glyph = if range == 0 {
                code.wrapping_add(delta)
            } else {
                let glyph = read_u16(source, range_word + range + usize::from(code - start) * 2)?;
                if glyph == 0 {
                    0
                } else {
                    glyph.wrapping_add(delta)
                }
            };
            if glyph == 0 || !keep.contains(&glyph) {
                continue;
            }
            let delta = glyph.wrapping_sub(code);
            if let Some(last) = segments
                .last_mut()
                .filter(|last| last.1.checked_add(1) == Some(code) && last.2 == delta)
            {
                last.1 = code;
            } else {
                segments.push((code, code, delta));
            }
        }
    }
    if !segments.last().is_some_and(|last| last.1 == u16::MAX) {
        segments.push((u16::MAX, u16::MAX, 1));
    }
    let count = u16::try_from(segments.len()).ok()?;
    let length = 16usize.checked_add(usize::from(count).checked_mul(8)?)?;
    let mut output = vec![0; usize::from(u16::try_from(length).ok()?)];
    write_u16(&mut output, 0, 4)?;
    write_u16(&mut output, 2, u16::try_from(length).ok()?)?;
    write_u16(&mut output, 4, read_u16(source, 4)?)?;
    write_u16(&mut output, 6, count.checked_mul(2)?)?;
    let mut power = 1u16;
    let mut selector = 0u16;
    while power <= count / 2 {
        power *= 2;
        selector += 1;
    }
    write_u16(&mut output, 8, power * 2)?;
    write_u16(&mut output, 10, selector)?;
    write_u16(&mut output, 12, count * 2 - power * 2)?;
    for (index, (start, end, delta)) in segments.iter().enumerate() {
        write_u16(&mut output, 14 + index * 2, *end)?;
        write_u16(&mut output, 16 + usize::from(count) * 2 + index * 2, *start)?;
        write_u16(&mut output, 16 + usize::from(count) * 4 + index * 2, *delta)?;
    }
    Some(output)
}

fn subset_format12(source: &[u8], keep: &BTreeSet<u16>) -> Option<Vec<u8>> {
    let count = usize::try_from(read_u32(source, 12)?).ok()?;
    source.get(..16usize.checked_add(count.checked_mul(12)?)?)?;
    let mut groups: Vec<(u32, u32, u32)> = Vec::new();
    let mut previous_end = None;
    for index in 0..count {
        let start = read_u32(source, 16 + index * 12)?;
        let end = read_u32(source, 20 + index * 12)?;
        let first = read_u32(source, 24 + index * 12)?;
        if start > end || end > 0x10ffff || previous_end.is_some_and(|prior| start <= prior) {
            return None;
        }
        previous_end = Some(end);
        let last = first.checked_add(end - start)?;
        if first > u32::from(u16::MAX) {
            continue;
        }
        for glyph in keep.range((first as u16)..=(last.min(u32::from(u16::MAX)) as u16)) {
            if *glyph == 0 {
                continue;
            }
            let code = start.checked_add(u32::from(*glyph) - first)?;
            if let Some(last) = groups.last_mut().filter(|last| {
                last.1.checked_add(1) == Some(code)
                    && last.2.checked_add(code - last.0) == Some(u32::from(*glyph))
            }) {
                last.1 = code;
            } else {
                groups.push((code, code, u32::from(*glyph)));
                if groups.len() > count {
                    return None;
                }
            }
        }
    }
    let mut output = source.get(..16)?.to_vec();
    for (start, end, glyph) in &groups {
        output.extend_from_slice(&start.to_be_bytes());
        output.extend_from_slice(&end.to_be_bytes());
        output.extend_from_slice(&glyph.to_be_bytes());
    }
    let length = u32::try_from(output.len()).ok()?;
    write_u32(&mut output, 4, length)?;
    write_u32(&mut output, 12, u32::try_from(groups.len()).ok()?)?;
    Some(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn array_cmap4() -> Vec<u8> {
        // A/B/C use a glyph array; U+0100/0101 use a delta. The final segment is .notdef.
        [
            4u16, 46, 0, 6, 4, 1, 2, 0x43, 0x101, 0xffff, 0, 0x41, 0x100, 0xffff, 0, 1, 1, 6, 0, 0,
            3, 7, 9,
        ]
        .into_iter()
        .flat_map(u16::to_be_bytes)
        .collect()
    }

    #[test]
    fn format4_preserves_array_and_delta_mappings_and_the_terminal_segment() {
        let result = subset_format4(&array_cmap4(), &BTreeSet::from([0, 3, 9, 0x101])).unwrap();
        let expected: Vec<u8> = [
            4u16, 48, 0, 8, 8, 2, 0, 0x41, 0x43, 0x100, 0xffff, 0, 0x41, 0x43, 0x100, 0xffff,
            0xffc2, 0xffc6, 1, 1, 0, 0, 0, 0,
        ]
        .into_iter()
        .flat_map(u16::to_be_bytes)
        .collect();
        assert_eq!(result, expected);
        // A replacement with more segment records must not inflate an existing cmap.
        let mut source = vec![0, 0, 0, 1, 0, 3, 0, 1, 0, 0, 0, 12];
        source.extend_from_slice(&array_cmap4());
        assert!(subset(&source, &BTreeSet::from([0, 3, 9, 0x101])).is_none());
    }

    #[test]
    fn format12_keeps_non_bmp_glyphs_and_merges_consecutive_survivors() {
        let source: Vec<u8> = [
            0x000c0000u32,
            52,
            0,
            3,
            0x41,
            0x43,
            3,
            0x1f600,
            0x1f605,
            40,
            0x10ffff,
            0x10ffff,
            70,
        ]
        .into_iter()
        .flat_map(u32::to_be_bytes)
        .collect();
        let result = subset_format12(&source, &BTreeSet::from([0, 4, 42, 43, 70])).unwrap();
        let expected: Vec<u8> = [
            0x000c0000u32,
            52,
            0,
            3,
            0x42,
            0x42,
            4,
            0x1f602,
            0x1f603,
            42,
            0x10ffff,
            0x10ffff,
            70,
        ]
        .into_iter()
        .flat_map(u32::to_be_bytes)
        .collect();
        assert_eq!(result, expected);
        assert_eq!(
            subset_format12(&source, &BTreeSet::from([0]))
                .unwrap()
                .len(),
            16
        );
    }

    #[test]
    fn expanding_format12_rewrite_declines_before_allocating_a_larger_map() {
        let source: Vec<u8> = [0x000c0000u32, 28, 0, 1, 0x41, 0x45, 3]
            .into_iter()
            .flat_map(u32::to_be_bytes)
            .collect();
        assert!(subset_format12(&source, &BTreeSet::from([3, 5, 7])).is_none());
        let outside_unicode: Vec<u8> = [0x000c0000u32, 28, 0, 1, 0, 0xffffffff, 0]
            .into_iter()
            .flat_map(u32::to_be_bytes)
            .collect();
        assert!(subset_format12(&outside_unicode, &BTreeSet::from([1])).is_none());
    }

    #[test]
    fn duplicate_encoding_records_share_a_compacted_subtable() {
        let mut source = vec![0, 0, 0, 2, 0, 0, 0, 3, 0, 0, 0, 20, 0, 3, 0, 1, 0, 0, 0, 20];
        source.extend_from_slice(&array_cmap4());
        let result = subset(&source, &BTreeSet::from([3])).unwrap();
        assert_eq!(read_u32(&result, 8), Some(20));
        assert_eq!(read_u32(&result, 16), Some(20));
        assert_eq!(&result[..20], &source[..20]);
        assert_eq!(result.len(), 20 + 32);
    }

    #[test]
    fn legacy_encoding_subtable_is_copied_byte_for_byte() {
        let mut source = vec![0, 0, 0, 2, 0, 0, 0, 3, 0, 0, 0, 20, 0, 1, 0, 0, 0, 0, 0, 66];
        source.extend_from_slice(&array_cmap4());
        let legacy: Vec<u8> = [6u16, 16, 18, 0x41, 3, 3, 7, 9]
            .into_iter()
            .flat_map(u16::to_be_bytes)
            .collect();
        source.extend_from_slice(&legacy);
        let result = subset(&source, &BTreeSet::from([3])).unwrap();
        assert_eq!(read_u32(&result, 16), Some(52));
        assert_eq!(&result[52..], legacy.as_slice());
        assert_eq!(&result[12..16], &source[12..16]);
    }

    #[test]
    fn format4_array_delta_wraps_and_missing_glyph_stays_missing() {
        let mut source = array_cmap4();
        write_u16(&mut source, 28, 0xfffe).unwrap();
        write_u16(&mut source, 42, 0).unwrap();
        let result = subset_format4(&source, &BTreeSet::from([0, 1, 7])).unwrap();
        // A -> 3 - 2 = 1, B remains missing, C -> 9 - 2 = 7.
        assert_eq!(read_u16(&result, 6), Some(6));
        assert_eq!(
            [read_u16(&result, 14), read_u16(&result, 16)],
            [Some(0x41), Some(0x43)]
        );
        assert_eq!(
            [read_u16(&result, 28), read_u16(&result, 30)],
            [Some(0xffc0), Some(0xffc4)]
        );
    }

    #[test]
    fn malformed_character_maps_decline_compaction_without_panicking() {
        let valid = array_cmap4();
        for end in 0..valid.len() {
            // Some prefixes omit only unused glyphs; those may still have a valid retained map.
            let _ = subset_format4(&valid[..end], &BTreeSet::from([3, 7, 9]));
        }
        let mut overlapping = valid.clone();
        write_u16(&mut overlapping, 24, 0x42).unwrap();
        assert!(subset_format4(&overlapping, &BTreeSet::from([3])).is_none());
        assert!(subset(&[0, 0, 0, 1], &BTreeSet::new()).is_none());
        let overflow: Vec<u8> = [0x000c0000u32, 28, 0, 1, 0, u32::MAX, 1]
            .into_iter()
            .flat_map(u32::to_be_bytes)
            .collect();
        assert!(subset_format12(&overflow, &BTreeSet::from([3])).is_none());
    }
}
