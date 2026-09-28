//! Bounded structural inspection of vendorable ICC v2/v4 output-intent inputs.
//!
//! No color transform is performed, and this is not an ICC conformance verifier.
//! Header/tag layout follows ICC.1:2022 section 7 (https://www.color.org/specification/ICC.1-2022-05.pdf).
//! Legacy OutputIntent::new remains permissive for existing callers.

use std::{collections::BTreeSet, fmt};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IccProfileInfo {
    pub version: String,
    pub device_class: String,
    pub color_space: String,
    pub connection_space: String,
    pub components: u8,
    pub tag_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IccProfileError(pub &'static str);

impl fmt::Display for IccProfileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ICC_PROFILE_INVALID: {}", self.0)
    }
}
impl std::error::Error for IccProfileError {}

/// Checks declared size, header, supported device color spaces, and every tag
/// range before deriving the PDF /N count. Unknown/private tag types are retained.
/// DeviceLink/abstract/named-color profiles cannot describe an output device.
/// Required transforms, color accuracy and standards-specific suitability must
/// be established independently for the intended output condition.
pub fn inspect_output_intent_icc(bytes: &[u8]) -> Result<IccProfileInfo, IccProfileError> {
    let invalid = |message| IccProfileError(message);
    if bytes.len() < 132 {
        return Err(invalid("truncated header or tag table"));
    }
    let u32_at = |at| u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap());
    if u32_at(0) as usize != bytes.len() {
        return Err(invalid("declared size does not match exact input bytes"));
    }
    if &bytes[36..40] != b"acsp" {
        return Err(invalid("missing acsp signature"));
    }
    if !matches!(bytes[8], 2 | 4)
        || bytes[9] >> 4 > 9
        || bytes[9] & 15 > 9
        || bytes[10..12] != [0, 0]
    {
        return Err(invalid("only ICC v2/v4 profile headers are supported"));
    }
    if bytes[100..128].iter().any(|b| *b != 0) {
        return Err(invalid("nonzero reserved header bytes"));
    }
    if !matches!(&bytes[12..16], b"scnr" | b"mntr" | b"prtr" | b"spac") {
        return Err(invalid(
            "profile class cannot be used as a device output intent",
        ));
    }
    let components = match &bytes[16..20] {
        b"GRAY" => 1,
        b"RGB " => 3,
        b"CMYK" => 4,
        _ => return Err(invalid("output intent must declare Gray, RGB or CMYK data")),
    };
    if !matches!(&bytes[20..24], b"XYZ " | b"Lab ") {
        return Err(invalid("invalid profile connection space"));
    }
    if u32_at(64) > 3 {
        return Err(invalid("invalid rendering intent"));
    }
    let count = u32_at(128);
    let table_end = (count as usize)
        .checked_mul(12)
        .and_then(|n| n.checked_add(132))
        .filter(|end| *end <= bytes.len())
        .ok_or_else(|| invalid("truncated tag table"))?;
    if count == 0 {
        return Err(invalid("profile contains no tags"));
    }
    let mut signatures = BTreeSet::new();
    let mut ranges = BTreeSet::new();
    for entry in (132..table_end).step_by(12) {
        if !signatures.insert(u32_at(entry)) {
            return Err(invalid("duplicate tag signature"));
        }
        let start = u32_at(entry + 4) as usize;
        let length = u32_at(entry + 8) as usize;
        let end = start
            .checked_add(length)
            .filter(|end| *end <= bytes.len())
            .ok_or_else(|| invalid("tag extends beyond input bytes"))?;
        if start < table_end || start % 4 != 0 || length < 8 {
            return Err(invalid("invalid tag offset, alignment or length"));
        }
        if bytes[start + 4..start + 8] != [0, 0, 0, 0] {
            return Err(invalid("nonzero reserved tag bytes"));
        }
        ranges.insert((start, end)); // Multiple signatures may share the identical range.
    }
    let mut previous_end = table_end;
    for (start, end) in ranges {
        if start < previous_end {
            return Err(invalid("partially overlapping tag data"));
        }
        previous_end = end;
    }
    Ok(IccProfileInfo {
        version: format!("{}.{}.{}", bytes[8], bytes[9] >> 4, bytes[9] & 15),
        device_class: String::from_utf8_lossy(&bytes[12..16]).trim().to_owned(),
        color_space: String::from_utf8_lossy(&bytes[16..20]).trim().to_owned(),
        connection_space: String::from_utf8_lossy(&bytes[20..24]).trim().to_owned(),
        components,
        tag_count: count,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn structural_fixture() -> Vec<u8> {
        let mut bytes = vec![0; 168];
        bytes[0..4].copy_from_slice(&168_u32.to_be_bytes());
        bytes[8] = 4;
        bytes[12..16].copy_from_slice(b"mntr");
        bytes[16..20].copy_from_slice(b"RGB ");
        bytes[20..24].copy_from_slice(b"XYZ ");
        bytes[36..40].copy_from_slice(b"acsp");
        bytes[128..132].copy_from_slice(&2_u32.to_be_bytes());
        for (index, signature) in [b"rTRC", b"gTRC"].into_iter().enumerate() {
            let at = 132 + index * 12;
            bytes[at..at + 4].copy_from_slice(signature);
            bytes[at + 4..at + 8].copy_from_slice(&156_u32.to_be_bytes());
            bytes[at + 8..at + 12].copy_from_slice(&12_u32.to_be_bytes());
        }
        bytes[156..160].copy_from_slice(b"curv");
        bytes
    }

    #[test]
    fn icc_inspector_derives_components_and_allows_shared_tag_ranges() {
        for (space, expected) in [(b"RGB ", 3), (b"GRAY", 1), (b"CMYK", 4)] {
            let mut bytes = structural_fixture();
            bytes[16..20].copy_from_slice(space);
            assert_eq!(
                inspect_output_intent_icc(&bytes).unwrap().components,
                expected
            );
            bytes[8] = 2;
            assert_eq!(inspect_output_intent_icc(&bytes).unwrap().version, "2.0.0");
        }
    }

    #[test]
    fn icc_inspector_rejects_malformed_or_unsupported_inputs() {
        let source = structural_fixture();
        for at in 0..source.len() {
            assert!(inspect_output_intent_icc(&source[..at]).is_err());
        }
        for (at, value) in [
            (0, 1),
            (8, 5),
            (9, 255),
            (36, 0),
            (64, 1),
            (100, 1),
            (128, 255),
            (143, 255),
            (148, 1),
            (151, 160),
            (160, 1),
        ] {
            let mut bytes = source.clone();
            bytes[at] = value;
            assert!(
                inspect_output_intent_icc(&bytes).is_err(),
                "accepted mutation at {at}"
            );
        }
        for class in [b"link", b"abst", b"nmcl"] {
            let mut bytes = source.clone();
            bytes[12..16].copy_from_slice(class);
            assert!(inspect_output_intent_icc(&bytes).is_err());
        }
    }
}
