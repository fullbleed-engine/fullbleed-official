//! Dependency-free RFC 4648 Base64 codecs.

use std::error::Error;
use std::fmt;

const STANDARD_ALPHABET: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
const URL_SAFE_ALPHABET: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Alphabet {
    Standard,
    UrlSafe,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DecodeError {
    offset: usize,
    reason: &'static str,
}

impl fmt::Display for DecodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "invalid Base64 at byte {}: {}",
            self.offset, self.reason
        )
    }
}

impl Error for DecodeError {}

pub(crate) fn encode_standard(input: impl AsRef<[u8]>) -> String {
    encode(input.as_ref(), STANDARD_ALPHABET, true)
}

pub(crate) fn decode_standard(input: impl AsRef<[u8]>) -> Result<Vec<u8>, DecodeError> {
    decode_padded(input.as_ref(), Alphabet::Standard)
}

pub(crate) fn encode_url_safe_no_pad(input: impl AsRef<[u8]>) -> String {
    encode(input.as_ref(), URL_SAFE_ALPHABET, false)
}

#[cfg_attr(not(feature = "python"), allow(dead_code))]
pub(crate) fn decode_url_safe_no_pad(input: impl AsRef<[u8]>) -> Result<Vec<u8>, DecodeError> {
    decode_unpadded(input.as_ref(), Alphabet::UrlSafe)
}

fn encode(input: &[u8], alphabet: &[u8; 64], padded: bool) -> String {
    let complete_len = input.len() / 3 * 4;
    let tail_len = match (input.len() % 3, padded) {
        (0, _) => 0,
        (_, true) => 4,
        (1, false) => 2,
        (2, false) => 3,
        _ => unreachable!(),
    };
    let mut output = String::with_capacity(complete_len.saturating_add(tail_len));

    let mut chunks = input.chunks_exact(3);
    for chunk in &mut chunks {
        let bits = (u32::from(chunk[0]) << 16) | (u32::from(chunk[1]) << 8) | u32::from(chunk[2]);
        output.push(alphabet[((bits >> 18) & 0x3f) as usize] as char);
        output.push(alphabet[((bits >> 12) & 0x3f) as usize] as char);
        output.push(alphabet[((bits >> 6) & 0x3f) as usize] as char);
        output.push(alphabet[(bits & 0x3f) as usize] as char);
    }

    match chunks.remainder() {
        [] => {}
        [first] => {
            output.push(alphabet[(first >> 2) as usize] as char);
            output.push(alphabet[((first & 0x03) << 4) as usize] as char);
            if padded {
                output.push_str("==");
            }
        }
        [first, second] => {
            output.push(alphabet[(first >> 2) as usize] as char);
            output.push(alphabet[(((first & 0x03) << 4) | (second >> 4)) as usize] as char);
            output.push(alphabet[((second & 0x0f) << 2) as usize] as char);
            if padded {
                output.push('=');
            }
        }
        _ => unreachable!(),
    }
    output
}

fn decode_padded(input: &[u8], alphabet: Alphabet) -> Result<Vec<u8>, DecodeError> {
    if input.len() % 4 != 0 {
        return Err(decode_error(
            input.len(),
            "padded input length is not a multiple of four",
        ));
    }
    let mut output = Vec::with_capacity(input.len() / 4 * 3);
    let group_count = input.len() / 4;
    for (group_index, group) in input.chunks_exact(4).enumerate() {
        let offset = group_index * 4;
        let first = sextet(group[0], alphabet, offset)?;
        let second = sextet(group[1], alphabet, offset + 1)?;
        let is_last = group_index + 1 == group_count;

        if group[2] == b'=' {
            if !is_last || group[3] != b'=' {
                return Err(decode_error(offset + 2, "padding is only valid at the end"));
            }
            if second & 0x0f != 0 {
                return Err(decode_error(offset + 1, "non-zero trailing bits"));
            }
            output.push((first << 2) | (second >> 4));
            continue;
        }

        let third = sextet(group[2], alphabet, offset + 2)?;
        output.push((first << 2) | (second >> 4));
        output.push((second << 4) | (third >> 2));
        if group[3] == b'=' {
            if !is_last {
                return Err(decode_error(offset + 3, "padding is only valid at the end"));
            }
            if third & 0x03 != 0 {
                return Err(decode_error(offset + 2, "non-zero trailing bits"));
            }
        } else {
            let fourth = sextet(group[3], alphabet, offset + 3)?;
            output.push((third << 6) | fourth);
        }
    }
    Ok(output)
}

#[cfg_attr(not(feature = "python"), allow(dead_code))]
fn decode_unpadded(input: &[u8], alphabet: Alphabet) -> Result<Vec<u8>, DecodeError> {
    if input.len() % 4 == 1 {
        return Err(decode_error(
            input.len(),
            "unpadded input has an impossible length",
        ));
    }
    let complete_len = input.len() / 4 * 4;
    let mut output = Vec::with_capacity(input.len() / 4 * 3 + 2);
    for (group_index, group) in input[..complete_len].chunks_exact(4).enumerate() {
        let offset = group_index * 4;
        let first = sextet(group[0], alphabet, offset)?;
        let second = sextet(group[1], alphabet, offset + 1)?;
        let third = sextet(group[2], alphabet, offset + 2)?;
        let fourth = sextet(group[3], alphabet, offset + 3)?;
        output.extend_from_slice(&[
            (first << 2) | (second >> 4),
            (second << 4) | (third >> 2),
            (third << 6) | fourth,
        ]);
    }

    let tail = &input[complete_len..];
    if tail.len() >= 2 {
        let first = sextet(tail[0], alphabet, complete_len)?;
        let second = sextet(tail[1], alphabet, complete_len + 1)?;
        if tail.len() == 2 && second & 0x0f != 0 {
            return Err(decode_error(complete_len + 1, "non-zero trailing bits"));
        }
        output.push((first << 2) | (second >> 4));
        if tail.len() == 3 {
            let third = sextet(tail[2], alphabet, complete_len + 2)?;
            if third & 0x03 != 0 {
                return Err(decode_error(complete_len + 2, "non-zero trailing bits"));
            }
            output.push((second << 4) | (third >> 2));
        }
    }
    Ok(output)
}

fn sextet(byte: u8, alphabet: Alphabet, offset: usize) -> Result<u8, DecodeError> {
    let value = match byte {
        b'A'..=b'Z' => byte - b'A',
        b'a'..=b'z' => byte - b'a' + 26,
        b'0'..=b'9' => byte - b'0' + 52,
        b'+' if alphabet == Alphabet::Standard => 62,
        b'/' if alphabet == Alphabet::Standard => 63,
        b'-' if alphabet == Alphabet::UrlSafe => 62,
        b'_' if alphabet == Alphabet::UrlSafe => 63,
        b'=' => return Err(decode_error(offset, "unexpected padding")),
        _ => {
            return Err(decode_error(
                offset,
                "character is outside the selected alphabet",
            ));
        }
    };
    Ok(value)
}

fn decode_error(offset: usize, reason: &'static str) -> DecodeError {
    DecodeError { offset, reason }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_codec_matches_rfc_4648_vectors() {
        for (plain, encoded) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(encode_standard(plain), encoded);
            assert_eq!(
                decode_standard(encoded).expect("valid Base64"),
                plain.as_bytes()
            );
        }
    }

    #[test]
    fn url_safe_codec_uses_url_alphabet_without_padding() {
        let input = [0xfb, 0xff];
        assert_eq!(encode_url_safe_no_pad(input), "-_8");
        assert_eq!(decode_url_safe_no_pad("-_8").expect("valid Base64"), input);
    }

    #[test]
    fn decoder_rejects_bad_padding_alphabets_and_trailing_bits() {
        for invalid in ["Zg=", "Zg===", "Zh==", "Zm9=", "Zm 9v", "-_8="] {
            assert!(
                decode_standard(invalid).is_err(),
                "should reject {invalid:?}"
            );
        }
        for invalid in ["A", "Zh", "Zm9", "+/8", "-_8="] {
            assert!(
                decode_url_safe_no_pad(invalid).is_err(),
                "should reject {invalid:?}"
            );
        }
    }
}
