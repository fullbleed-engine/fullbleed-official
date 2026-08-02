//! Bounded native PDF object, stream, content, and document support.
//!
//! This module owns the PDF parsing boundary used by inspection, composition, raster input, and
//! Python diagnostics. It supports classic and stream cross-reference data, incremental updates,
//! object streams, the standard lossless stream filters, page-tree traversal, and deterministic
//! classic-xref serialization.

use std::collections::{BTreeMap, BTreeSet, HashSet, VecDeque};
use std::fmt;
use std::io::Write;
use std::path::Path;

pub(crate) type ObjectId = (u32, u16);

const MAX_STREAM_OUTPUT: usize = 512 * 1024 * 1024;
const MAX_OBJECT_DEPTH: usize = 256;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Error {
    Io(String),
    Parse(String),
    MissingObject(ObjectId),
    Type(&'static str),
    UnsupportedFilter(String),
    Stream(String),
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(message) => write!(formatter, "I/O error: {message}"),
            Self::Parse(message) => write!(formatter, "PDF parse error: {message}"),
            Self::MissingObject((number, generation)) => {
                write!(formatter, "missing PDF object {number} {generation}")
            }
            Self::Type(expected) => write!(formatter, "PDF object is not {expected}"),
            Self::UnsupportedFilter(filter) => write!(formatter, "unsupported PDF filter {filter}"),
            Self::Stream(message) => write!(formatter, "PDF stream error: {message}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error.to_string())
    }
}

pub(crate) type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StringFormat {
    Literal,
    Hexadecimal,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Object {
    Null,
    Boolean(bool),
    Integer(i64),
    Real(f32),
    Name(Vec<u8>),
    String(Vec<u8>, StringFormat),
    Array(Vec<Object>),
    Dictionary(Dictionary),
    Stream(Stream),
    Reference(ObjectId),
}

impl Object {
    pub(crate) fn as_array(&self) -> Result<&Vec<Object>> {
        match self {
            Self::Array(value) => Ok(value),
            _ => Err(Error::Type("an array")),
        }
    }

    pub(crate) fn as_dict(&self) -> Result<&Dictionary> {
        match self {
            Self::Dictionary(value) => Ok(value),
            _ => Err(Error::Type("a dictionary")),
        }
    }

    pub(crate) fn as_dict_mut(&mut self) -> Result<&mut Dictionary> {
        match self {
            Self::Dictionary(value) => Ok(value),
            _ => Err(Error::Type("a dictionary")),
        }
    }

    pub(crate) fn as_stream(&self) -> Result<&Stream> {
        match self {
            Self::Stream(value) => Ok(value),
            _ => Err(Error::Type("a stream")),
        }
    }

    pub(crate) fn as_name(&self) -> Result<&[u8]> {
        match self {
            Self::Name(value) => Ok(value),
            _ => Err(Error::Type("a name")),
        }
    }

    pub(crate) fn as_str(&self) -> Result<&[u8]> {
        match self {
            Self::String(value, _) => Ok(value),
            _ => Err(Error::Type("a string")),
        }
    }

    pub(crate) fn as_reference(&self) -> Result<ObjectId> {
        match self {
            Self::Reference(value) => Ok(*value),
            _ => Err(Error::Type("a reference")),
        }
    }

    pub(crate) fn as_i64(&self) -> Result<i64> {
        match self {
            Self::Integer(value) => Ok(*value),
            _ => Err(Error::Type("an integer")),
        }
    }

    pub(crate) fn as_float(&self) -> Result<f32> {
        match self {
            Self::Integer(value) => Ok(*value as f32),
            Self::Real(value) => Ok(*value),
            _ => Err(Error::Type("a number")),
        }
    }
}

impl From<bool> for Object {
    fn from(value: bool) -> Self {
        Self::Boolean(value)
    }
}

macro_rules! integer_from {
    ($($type:ty),+ $(,)?) => {$(
        impl From<$type> for Object {
            fn from(value: $type) -> Self {
                Self::Integer(value as i64)
            }
        }
    )+};
}

integer_from!(i8, i16, i32, i64, isize, u8, u16, u32, usize);

impl From<f32> for Object {
    fn from(value: f32) -> Self {
        Self::Real(value)
    }
}

impl From<f64> for Object {
    fn from(value: f64) -> Self {
        Self::Real(value as f32)
    }
}

impl From<&str> for Object {
    fn from(value: &str) -> Self {
        Self::Name(value.as_bytes().to_vec())
    }
}

impl From<String> for Object {
    fn from(value: String) -> Self {
        Self::Name(value.into_bytes())
    }
}

impl From<ObjectId> for Object {
    fn from(value: ObjectId) -> Self {
        Self::Reference(value)
    }
}

impl From<Vec<Object>> for Object {
    fn from(value: Vec<Object>) -> Self {
        Self::Array(value)
    }
}

impl From<Dictionary> for Object {
    fn from(value: Dictionary) -> Self {
        Self::Dictionary(value)
    }
}

impl From<Stream> for Object {
    fn from(value: Stream) -> Self {
        Self::Stream(value)
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Dictionary(BTreeMap<Vec<u8>, Object>);

impl Dictionary {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn get<K: AsRef<[u8]> + ?Sized>(&self, key: &K) -> Result<&Object> {
        self.0
            .get(key.as_ref())
            .ok_or(Error::Type("a dictionary key"))
    }

    pub(crate) fn set<K: AsRef<[u8]>, V: Into<Object>>(&mut self, key: K, value: V) {
        self.0.insert(key.as_ref().to_vec(), value.into());
    }

    pub(crate) fn remove<K: AsRef<[u8]> + ?Sized>(&mut self, key: &K) -> Option<Object> {
        self.0.remove(key.as_ref())
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = (&Vec<u8>, &Object)> {
        self.0.iter()
    }

    fn values(&self) -> impl Iterator<Item = &Object> {
        self.0.values()
    }
}

macro_rules! dictionary {
    () => {
        $crate::pdf_native::Dictionary::new()
    };
    ($($key:expr => $value:expr),+ $(,)?) => {{
        let mut dictionary = $crate::pdf_native::Dictionary::new();
        $(dictionary.set($key, $value);)+
        dictionary
    }};
}

pub(crate) use dictionary;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Stream {
    pub(crate) dict: Dictionary,
    pub(crate) content: Vec<u8>,
}

impl Stream {
    pub(crate) fn new(dict: Dictionary, content: Vec<u8>) -> Self {
        Self { dict, content }
    }

    pub(crate) fn get_plain_content(&self) -> Result<Vec<u8>> {
        decode_stream_filters(&self.dict, &self.content)
    }

    pub(crate) fn filters(&self) -> Result<Vec<&[u8]>> {
        let Ok(filter) = self.dict.get(b"Filter") else {
            return Ok(Vec::new());
        };
        match filter {
            Object::Name(name) => Ok(vec![name]),
            Object::Array(filters) => filters.iter().map(Object::as_name).collect(),
            _ => Err(Error::Stream("invalid Filter entry".to_string())),
        }
    }
}

fn filter_names(dict: &Dictionary) -> Result<Vec<Vec<u8>>> {
    let Ok(filter) = dict.get(b"Filter") else {
        return Ok(Vec::new());
    };
    match filter {
        Object::Name(name) => Ok(vec![name.clone()]),
        Object::Array(filters) => filters
            .iter()
            .map(|filter| filter.as_name().map(Vec::from))
            .collect(),
        _ => Err(Error::Stream("invalid Filter entry".to_string())),
    }
}

fn decode_parameters(dict: &Dictionary, count: usize) -> Vec<Option<Dictionary>> {
    let Ok(parameters) = dict.get(b"DecodeParms") else {
        return vec![None; count];
    };
    match parameters {
        Object::Dictionary(value) => {
            let mut output = vec![None; count];
            if let Some(first) = output.first_mut() {
                *first = Some(value.clone());
            }
            output
        }
        Object::Array(values) => (0..count)
            .map(|index| match values.get(index) {
                Some(Object::Dictionary(value)) => Some(value.clone()),
                _ => None,
            })
            .collect(),
        _ => vec![None; count],
    }
}

fn decode_stream_filters(dict: &Dictionary, content: &[u8]) -> Result<Vec<u8>> {
    let filters = filter_names(dict)?;
    let parameters = decode_parameters(dict, filters.len());
    let mut output = content.to_vec();
    for (index, filter) in filters.iter().enumerate() {
        output = match filter.as_slice() {
            b"FlateDecode" | b"Fl" => {
                let inflated = crate::flate_native::zlib_inflate(&output, MAX_STREAM_OUTPUT)
                    .map_err(|error| Error::Stream(error.to_string()))?;
                apply_predictor(inflated, parameters.get(index).and_then(Option::as_ref))?
            }
            b"ASCIIHexDecode" | b"AHx" => decode_ascii_hex(&output)?,
            b"ASCII85Decode" | b"A85" => decode_ascii85(&output)?,
            b"RunLengthDecode" | b"RL" => decode_run_length(&output)?,
            b"LZWDecode" | b"LZW" => {
                let early_change = parameters
                    .get(index)
                    .and_then(Option::as_ref)
                    .and_then(|dict| dict.get(b"EarlyChange").ok())
                    .and_then(|value| value.as_i64().ok())
                    .unwrap_or(1)
                    != 0;
                let decoded = decode_lzw(&output, early_change)?;
                apply_predictor(decoded, parameters.get(index).and_then(Option::as_ref))?
            }
            b"Crypt" => output,
            other => {
                return Err(Error::UnsupportedFilter(
                    String::from_utf8_lossy(other).into_owned(),
                ));
            }
        };
        if output.len() > MAX_STREAM_OUTPUT {
            return Err(Error::Stream(
                "decoded stream exceeds safety limit".to_string(),
            ));
        }
    }
    Ok(output)
}

fn decode_ascii_hex(data: &[u8]) -> Result<Vec<u8>> {
    let mut digits = Vec::new();
    for &byte in data {
        if byte == b'>' {
            break;
        }
        if byte.is_ascii_whitespace() {
            continue;
        }
        let value =
            hex_value(byte).ok_or_else(|| Error::Stream("invalid ASCIIHex digit".to_string()))?;
        digits.push(value);
    }
    let mut output = Vec::with_capacity(digits.len().div_ceil(2));
    for pair in digits.chunks(2) {
        output.push((pair[0] << 4) | pair.get(1).copied().unwrap_or(0));
    }
    Ok(output)
}

fn decode_ascii85(data: &[u8]) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    let mut group = Vec::with_capacity(5);
    let mut index = 0;
    while index < data.len() {
        let byte = data[index];
        index += 1;
        if byte.is_ascii_whitespace() || (byte == b'<' && data.get(index) == Some(&b'~')) {
            if byte == b'<' {
                index += 1;
            }
            continue;
        }
        if byte == b'~' && data.get(index) == Some(&b'>') {
            break;
        }
        if byte == b'z' {
            if !group.is_empty() {
                return Err(Error::Stream("ASCII85 z inside a group".to_string()));
            }
            output.extend_from_slice(&[0; 4]);
            if output.len() > MAX_STREAM_OUTPUT {
                return Err(Error::Stream(
                    "ASCII85 output exceeds safety limit".to_string(),
                ));
            }
            continue;
        }
        if !(b'!'..=b'u').contains(&byte) {
            return Err(Error::Stream("invalid ASCII85 digit".to_string()));
        }
        group.push(u32::from(byte - b'!'));
        if group.len() == 5 {
            let value = ascii85_group_value(&group)?;
            output.extend_from_slice(&value.to_be_bytes());
            if output.len() > MAX_STREAM_OUTPUT {
                return Err(Error::Stream(
                    "ASCII85 output exceeds safety limit".to_string(),
                ));
            }
            group.clear();
        }
    }
    if group.len() == 1 {
        return Err(Error::Stream("invalid one-digit ASCII85 tail".to_string()));
    }
    if !group.is_empty() {
        let keep = group.len() - 1;
        group.resize(5, 84);
        let value = ascii85_group_value(&group)?;
        output.extend_from_slice(&value.to_be_bytes()[..keep]);
    }
    Ok(output)
}

fn ascii85_group_value(group: &[u32]) -> Result<u32> {
    let value = group.iter().try_fold(0u64, |value, digit| {
        value
            .checked_mul(85)
            .and_then(|value| value.checked_add(u64::from(*digit)))
            .ok_or_else(|| Error::Stream("ASCII85 group overflow".to_string()))
    })?;
    u32::try_from(value).map_err(|_| Error::Stream("ASCII85 group overflow".to_string()))
}

fn decode_run_length(data: &[u8]) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    let mut index = 0;
    while index < data.len() {
        let length = data[index];
        index += 1;
        match length {
            128 => break,
            0..=127 => {
                let count = usize::from(length) + 1;
                let bytes = data
                    .get(index..index + count)
                    .ok_or_else(|| Error::Stream("truncated RunLength literal".to_string()))?;
                output.extend_from_slice(bytes);
                index += count;
            }
            129..=255 => {
                let byte = *data
                    .get(index)
                    .ok_or_else(|| Error::Stream("truncated RunLength repeat".to_string()))?;
                index += 1;
                output.extend(std::iter::repeat_n(byte, 257 - usize::from(length)));
            }
        }
        if output.len() > MAX_STREAM_OUTPUT {
            return Err(Error::Stream(
                "RunLength output exceeds safety limit".to_string(),
            ));
        }
    }
    Ok(output)
}

fn decode_lzw(data: &[u8], early_change: bool) -> Result<Vec<u8>> {
    let mut reader = BitReader::new(data);
    let mut dictionary: Vec<Vec<u8>> = (0u16..=255).map(|value| vec![value as u8]).collect();
    dictionary.resize(258, Vec::new());
    let mut code_width = 9usize;
    let mut previous: Option<Vec<u8>> = None;
    let mut output = Vec::new();
    while let Some(code) = reader.read(code_width) {
        match code {
            256 => {
                dictionary.truncate(258);
                code_width = 9;
                previous = None;
            }
            257 => break,
            _ => {
                let entry =
                    if let Some(entry) = dictionary.get(code).filter(|entry| !entry.is_empty()) {
                        entry.clone()
                    } else if code == dictionary.len() {
                        let mut value = previous
                            .clone()
                            .ok_or_else(|| Error::Stream("invalid initial LZW code".to_string()))?;
                        let first = value[0];
                        value.push(first);
                        value
                    } else {
                        return Err(Error::Stream("invalid LZW code".to_string()));
                    };
                output.extend_from_slice(&entry);
                if output.len() > MAX_STREAM_OUTPUT {
                    return Err(Error::Stream("LZW output exceeds safety limit".to_string()));
                }
                if let Some(previous) = previous.as_ref() {
                    if dictionary.len() < 4096 {
                        let mut next = previous.clone();
                        next.push(entry[0]);
                        dictionary.push(next);
                        let threshold = (1usize << code_width) - usize::from(early_change);
                        if dictionary.len() == threshold && code_width < 12 {
                            code_width += 1;
                        }
                    }
                }
                previous = Some(entry);
            }
        }
    }
    Ok(output)
}

struct BitReader<'a> {
    data: &'a [u8],
    bit: usize,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, bit: 0 }
    }

    fn read(&mut self, width: usize) -> Option<usize> {
        if self.bit.checked_add(width)? > self.data.len().checked_mul(8)? {
            return None;
        }
        let mut value = 0usize;
        for _ in 0..width {
            let byte = self.data[self.bit / 8];
            let shift = 7 - self.bit % 8;
            value = (value << 1) | usize::from((byte >> shift) & 1);
            self.bit += 1;
        }
        Some(value)
    }
}

fn apply_predictor(data: Vec<u8>, parameters: Option<&Dictionary>) -> Result<Vec<u8>> {
    let predictor = parameters
        .and_then(|dict| dict.get(b"Predictor").ok())
        .and_then(|value| value.as_i64().ok())
        .unwrap_or(1);
    if predictor <= 1 {
        return Ok(data);
    }
    let colors = parameter_usize(parameters, b"Colors", 1).max(1);
    let bits = parameter_usize(parameters, b"BitsPerComponent", 8).max(1);
    let columns = parameter_usize(parameters, b"Columns", 1).max(1);
    let row_bytes = colors
        .checked_mul(columns)
        .and_then(|value| value.checked_mul(bits))
        .and_then(|value| value.checked_add(7))
        .map(|value| value / 8)
        .ok_or_else(|| Error::Stream("predictor row overflow".to_string()))?;
    let bytes_per_pixel = colors
        .checked_mul(bits)
        .map(|value| value.div_ceil(8).max(1))
        .ok_or_else(|| Error::Stream("predictor pixel overflow".to_string()))?;
    match predictor {
        2 => decode_tiff_predictor(data, row_bytes, colors, bits, columns),
        10..=15 => decode_png_predictor(data, row_bytes, bytes_per_pixel),
        _ => Err(Error::Stream(format!("unsupported predictor {predictor}"))),
    }
}

fn parameter_usize(parameters: Option<&Dictionary>, key: &[u8], default: usize) -> usize {
    parameters
        .and_then(|dict| dict.get(key).ok())
        .and_then(|value| value.as_i64().ok())
        .and_then(|value| usize::try_from(value).ok())
        .unwrap_or(default)
}

fn decode_tiff_predictor(
    mut data: Vec<u8>,
    row_bytes: usize,
    colors: usize,
    bits: usize,
    columns: usize,
) -> Result<Vec<u8>> {
    if row_bytes == 0 || data.len() % row_bytes != 0 {
        return Err(Error::Stream("invalid TIFF predictor rows".to_string()));
    }
    if !matches!(bits, 1 | 2 | 4 | 8 | 16) {
        return Err(Error::Stream(format!(
            "unsupported TIFF predictor bit depth {bits}"
        )));
    }
    let samples = colors
        .checked_mul(columns)
        .ok_or_else(|| Error::Stream("TIFF predictor sample overflow".to_string()))?;
    for row in data.chunks_exact_mut(row_bytes) {
        match bits {
            8 => {
                for index in colors..samples {
                    row[index] = row[index].wrapping_add(row[index - colors]);
                }
            }
            16 => {
                for index in colors..samples {
                    let byte_index = index * 2;
                    let previous_index = (index - colors) * 2;
                    let encoded = u16::from_be_bytes([row[byte_index], row[byte_index + 1]]);
                    let previous =
                        u16::from_be_bytes([row[previous_index], row[previous_index + 1]]);
                    let decoded = encoded.wrapping_add(previous).to_be_bytes();
                    row[byte_index..byte_index + 2].copy_from_slice(&decoded);
                }
            }
            _ => {
                let mask = (1u16 << bits) - 1;
                for index in colors..samples {
                    let encoded = read_packed_sample(row, index, bits);
                    let previous = read_packed_sample(row, index - colors, bits);
                    write_packed_sample(row, index, bits, (encoded + previous) & mask);
                }
            }
        }
    }
    Ok(data)
}

fn read_packed_sample(row: &[u8], index: usize, bits: usize) -> u16 {
    let bit_offset = index * bits;
    let byte_index = bit_offset / 8;
    let shift = 8 - bits - (bit_offset % 8);
    u16::from((row[byte_index] >> shift) & ((1u8 << bits) - 1))
}

fn write_packed_sample(row: &mut [u8], index: usize, bits: usize, value: u16) {
    let bit_offset = index * bits;
    let byte_index = bit_offset / 8;
    let shift = 8 - bits - (bit_offset % 8);
    let mask = ((1u8 << bits) - 1) << shift;
    row[byte_index] = (row[byte_index] & !mask) | (((value as u8) << shift) & mask);
}

fn decode_png_predictor(
    data: Vec<u8>,
    row_bytes: usize,
    bytes_per_pixel: usize,
) -> Result<Vec<u8>> {
    let encoded_row = row_bytes
        .checked_add(1)
        .ok_or_else(|| Error::Stream("PNG predictor row overflow".to_string()))?;
    if encoded_row == 0 || data.len() % encoded_row != 0 {
        return Err(Error::Stream("invalid PNG predictor rows".to_string()));
    }
    let row_count = data.len() / encoded_row;
    let output_length = row_count
        .checked_mul(row_bytes)
        .ok_or_else(|| Error::Stream("PNG predictor output overflow".to_string()))?;
    let mut output = vec![0u8; output_length];
    for row_index in 0..row_count {
        let filter = data[row_index * encoded_row];
        let source = &data[row_index * encoded_row + 1..(row_index + 1) * encoded_row];
        let (before, current_and_after) = output.split_at_mut(row_index * row_bytes);
        let current = &mut current_and_after[..row_bytes];
        let previous = if row_index == 0 {
            None
        } else {
            Some(&before[(row_index - 1) * row_bytes..row_index * row_bytes])
        };
        for index in 0..row_bytes {
            let left = index
                .checked_sub(bytes_per_pixel)
                .map(|offset| current[offset])
                .unwrap_or(0);
            let up = previous.map(|row| row[index]).unwrap_or(0);
            let upper_left = previous
                .and_then(|row| index.checked_sub(bytes_per_pixel).map(|offset| row[offset]))
                .unwrap_or(0);
            let predictor = match filter {
                0 => 0,
                1 => left,
                2 => up,
                3 => ((u16::from(left) + u16::from(up)) / 2) as u8,
                4 => paeth(left, up, upper_left),
                _ => return Err(Error::Stream("invalid PNG predictor filter".to_string())),
            };
            current[index] = source[index].wrapping_add(predictor);
        }
    }
    Ok(output)
}

fn paeth(left: u8, up: u8, upper_left: u8) -> u8 {
    let left = i32::from(left);
    let up = i32::from(up);
    let upper_left = i32::from(upper_left);
    let estimate = left + up - upper_left;
    let left_distance = (estimate - left).abs();
    let up_distance = (estimate - up).abs();
    let diagonal_distance = (estimate - upper_left).abs();
    if left_distance <= up_distance && left_distance <= diagonal_distance {
        left as u8
    } else if up_distance <= diagonal_distance {
        up as u8
    } else {
        upper_left as u8
    }
}

struct Parser<'a> {
    data: &'a [u8],
    position: usize,
}

impl<'a> Parser<'a> {
    fn new(data: &'a [u8], position: usize) -> Self {
        Self { data, position }
    }

    fn skip_space(&mut self) {
        loop {
            while self
                .data
                .get(self.position)
                .is_some_and(|byte| is_pdf_whitespace(*byte))
            {
                self.position += 1;
            }
            if self.data.get(self.position) != Some(&b'%') {
                break;
            }
            while self
                .data
                .get(self.position)
                .is_some_and(|byte| !matches!(*byte, b'\r' | b'\n'))
            {
                self.position += 1;
            }
        }
    }

    fn parse_object(&mut self, depth: usize) -> Result<Object> {
        if depth > MAX_OBJECT_DEPTH {
            return Err(Error::Parse(
                "PDF object nesting limit exceeded".to_string(),
            ));
        }
        self.skip_space();
        let byte = *self
            .data
            .get(self.position)
            .ok_or_else(|| Error::Parse("unexpected end of PDF object".to_string()))?;
        match byte {
            b'/' => self.parse_name().map(Object::Name),
            b'(' => self.parse_literal_string(),
            b'[' => self.parse_array(depth + 1),
            b'<' if self.data.get(self.position + 1) == Some(&b'<') => {
                self.parse_dictionary(depth + 1).map(Object::Dictionary)
            }
            b'<' => self.parse_hex_string(),
            b'+' | b'-' | b'.' | b'0'..=b'9' => self.parse_number_or_reference(),
            b't' if self.consume_keyword(b"true") => Ok(Object::Boolean(true)),
            b'f' if self.consume_keyword(b"false") => Ok(Object::Boolean(false)),
            b'n' if self.consume_keyword(b"null") => Ok(Object::Null),
            _ => Err(Error::Parse(format!(
                "unexpected PDF object token at byte {}",
                self.position
            ))),
        }
    }

    fn parse_name(&mut self) -> Result<Vec<u8>> {
        if self.data.get(self.position) != Some(&b'/') {
            return Err(Error::Parse("expected PDF name".to_string()));
        }
        self.position += 1;
        let mut output = Vec::new();
        while let Some(&byte) = self.data.get(self.position) {
            if is_pdf_whitespace(byte) || is_pdf_delimiter(byte) {
                break;
            }
            self.position += 1;
            if byte == b'#' {
                if let (Some(&first), Some(&second)) = (
                    self.data.get(self.position),
                    self.data.get(self.position + 1),
                ) {
                    if let (Some(high), Some(low)) = (hex_value(first), hex_value(second)) {
                        output.push((high << 4) | low);
                        self.position += 2;
                        continue;
                    }
                }
            }
            output.push(byte);
        }
        Ok(output)
    }

    fn parse_literal_string(&mut self) -> Result<Object> {
        self.position += 1;
        let mut output = Vec::new();
        let mut nesting = 1usize;
        while let Some(&byte) = self.data.get(self.position) {
            self.position += 1;
            match byte {
                b'(' => {
                    nesting += 1;
                    output.push(byte);
                }
                b')' => {
                    nesting -= 1;
                    if nesting == 0 {
                        return Ok(Object::String(output, StringFormat::Literal));
                    }
                    output.push(byte);
                }
                b'\\' => {
                    let Some(&escaped) = self.data.get(self.position) else {
                        break;
                    };
                    self.position += 1;
                    match escaped {
                        b'n' => output.push(b'\n'),
                        b'r' => output.push(b'\r'),
                        b't' => output.push(b'\t'),
                        b'b' => output.push(0x08),
                        b'f' => output.push(0x0c),
                        b'(' | b')' | b'\\' => output.push(escaped),
                        b'\r' => {
                            if self.data.get(self.position) == Some(&b'\n') {
                                self.position += 1;
                            }
                        }
                        b'\n' => {}
                        b'0'..=b'7' => {
                            let mut value = u16::from(escaped - b'0');
                            for _ in 0..2 {
                                let Some(&next) = self.data.get(self.position) else {
                                    break;
                                };
                                if !(b'0'..=b'7').contains(&next) {
                                    break;
                                }
                                self.position += 1;
                                value = value * 8 + u16::from(next - b'0');
                            }
                            output.push((value & 0xff) as u8);
                        }
                        _ => output.push(escaped),
                    }
                }
                _ => output.push(byte),
            }
        }
        Err(Error::Parse("unterminated PDF literal string".to_string()))
    }

    fn parse_hex_string(&mut self) -> Result<Object> {
        self.position += 1;
        let mut digits = Vec::new();
        loop {
            let byte = *self
                .data
                .get(self.position)
                .ok_or_else(|| Error::Parse("unterminated PDF hex string".to_string()))?;
            self.position += 1;
            if byte == b'>' {
                break;
            }
            if is_pdf_whitespace(byte) {
                continue;
            }
            digits.push(
                hex_value(byte)
                    .ok_or_else(|| Error::Parse("invalid PDF hex string".to_string()))?,
            );
        }
        let mut output = Vec::with_capacity(digits.len().div_ceil(2));
        for pair in digits.chunks(2) {
            output.push((pair[0] << 4) | pair.get(1).copied().unwrap_or(0));
        }
        Ok(Object::String(output, StringFormat::Hexadecimal))
    }

    fn parse_array(&mut self, depth: usize) -> Result<Object> {
        self.position += 1;
        let mut output = Vec::new();
        loop {
            self.skip_space();
            if self.data.get(self.position) == Some(&b']') {
                self.position += 1;
                return Ok(Object::Array(output));
            }
            output.push(self.parse_object(depth)?);
        }
    }

    fn parse_dictionary(&mut self, depth: usize) -> Result<Dictionary> {
        if self.data.get(self.position..self.position + 2) != Some(b"<<".as_slice()) {
            return Err(Error::Parse("expected PDF dictionary".to_string()));
        }
        self.position += 2;
        let mut output = Dictionary::new();
        loop {
            self.skip_space();
            if self.data.get(self.position..self.position + 2) == Some(b">>".as_slice()) {
                self.position += 2;
                return Ok(output);
            }
            let key = self.parse_name()?;
            let value = self.parse_object(depth)?;
            output.set(key, value);
        }
    }

    fn parse_number_or_reference(&mut self) -> Result<Object> {
        let start = self.position;
        let token = self.read_regular_token();
        let token_text = std::str::from_utf8(token)
            .map_err(|_| Error::Parse("invalid PDF number".to_string()))?;
        let is_integer = !token.contains(&b'.') && !token.contains(&b'e') && !token.contains(&b'E');
        if is_integer {
            if let Ok(first) = token_text.parse::<i64>() {
                if first >= 0 {
                    let after_first = self.position;
                    self.skip_space();
                    let second_token = self.read_regular_token();
                    let second = if !second_token.is_empty() && !second_token.contains(&b'.') {
                        std::str::from_utf8(second_token)
                            .ok()
                            .and_then(|text| text.parse::<i64>().ok())
                            .filter(|value| *value >= 0)
                    } else {
                        None
                    };
                    if let Some(second) = second {
                        self.skip_space();
                        if self.consume_keyword(b"R") {
                            if let (Ok(number), Ok(generation)) =
                                (u32::try_from(first), u16::try_from(second))
                            {
                                return Ok(Object::Reference((number, generation)));
                            }
                        }
                    }
                    self.position = after_first;
                    let value = token_text
                        .parse::<i64>()
                        .map_err(|_| Error::Parse("integer outside supported range".to_string()))?;
                    return Ok(Object::Integer(value));
                }
            }
        }
        self.position = start + token.len();
        let value = token_text
            .parse::<f32>()
            .map_err(|_| Error::Parse("invalid PDF real number".to_string()))?;
        if !value.is_finite() {
            return Err(Error::Parse("non-finite PDF real number".to_string()));
        }
        Ok(Object::Real(value))
    }

    fn read_regular_token(&mut self) -> &'a [u8] {
        let start = self.position;
        while self
            .data
            .get(self.position)
            .is_some_and(|byte| !is_pdf_whitespace(*byte) && !is_pdf_delimiter(*byte))
        {
            self.position += 1;
        }
        &self.data[start..self.position]
    }

    fn consume_keyword(&mut self, keyword: &[u8]) -> bool {
        let Some(candidate) = self.data.get(self.position..self.position + keyword.len()) else {
            return false;
        };
        if candidate != keyword {
            return false;
        }
        let end = self.position + keyword.len();
        if self
            .data
            .get(end)
            .is_some_and(|byte| !is_pdf_whitespace(*byte) && !is_pdf_delimiter(*byte))
        {
            return false;
        }
        self.position = end;
        true
    }
}

fn parse_indirect_object(data: &[u8], offset: usize) -> Result<(ObjectId, Object, usize)> {
    parse_indirect_object_with_length(data, offset, None)
}

fn parse_indirect_object_with_length(
    data: &[u8],
    offset: usize,
    length_override: Option<usize>,
) -> Result<(ObjectId, Object, usize)> {
    let mut parser = Parser::new(data, offset);
    parser.skip_space();
    let number = parse_unsigned_token(&mut parser)?;
    parser.skip_space();
    let generation = parse_unsigned_token(&mut parser)?;
    parser.skip_space();
    if !parser.consume_keyword(b"obj") {
        return Err(Error::Parse(format!("expected obj at byte {offset}")));
    }
    let number = u32::try_from(number)
        .map_err(|_| Error::Parse("object number outside range".to_string()))?;
    let generation = u16::try_from(generation)
        .map_err(|_| Error::Parse("object generation outside range".to_string()))?;
    let mut object = parser.parse_object(0)?;
    if let Object::Dictionary(dictionary) = object {
        let after_dictionary = parser.position;
        parser.skip_space();
        if parser.consume_keyword(b"stream") {
            consume_stream_eol(&mut parser);
            let content_start = parser.position;
            let direct_length = length_override.or_else(|| {
                dictionary
                    .get(b"Length")
                    .ok()
                    .and_then(|value| value.as_i64().ok())
                    .and_then(|value| usize::try_from(value).ok())
            });
            let (content, after_stream) = if let Some(length) = direct_length {
                let end = content_start
                    .checked_add(length)
                    .ok_or_else(|| Error::Parse("stream length overflow".to_string()))?;
                let content = data
                    .get(content_start..end)
                    .ok_or_else(|| Error::Parse("truncated PDF stream".to_string()))?
                    .to_vec();
                let mut end_parser = Parser::new(data, end);
                end_parser.skip_space();
                if !end_parser.consume_keyword(b"endstream") {
                    return Err(Error::Parse(
                        "stream length does not reach endstream".to_string(),
                    ));
                }
                (content, end_parser.position)
            } else {
                let end_marker = find_stream_end(data, content_start)
                    .ok_or_else(|| Error::Parse("unterminated PDF stream".to_string()))?;
                let mut content_end = end_marker;
                if data.get(content_end.wrapping_sub(2)..content_end) == Some(b"\r\n".as_slice()) {
                    content_end -= 2;
                } else if content_end > content_start
                    && matches!(data[content_end - 1], b'\r' | b'\n')
                {
                    content_end -= 1;
                }
                (
                    data[content_start..content_end].to_vec(),
                    end_marker + b"endstream".len(),
                )
            };
            object = Object::Stream(Stream::new(dictionary, content));
            parser.position = after_stream;
        } else {
            parser.position = after_dictionary;
            object = Object::Dictionary(dictionary);
        }
    }
    parser.skip_space();
    if parser.consume_keyword(b"endobj") {
        Ok(((number, generation), object, parser.position))
    } else {
        Err(Error::Parse(format!(
            "object {number} {generation} is missing endobj"
        )))
    }
}

fn parse_unsigned_token(parser: &mut Parser<'_>) -> Result<u64> {
    let token = parser.read_regular_token();
    std::str::from_utf8(token)
        .ok()
        .and_then(|text| text.parse::<u64>().ok())
        .ok_or_else(|| Error::Parse("expected unsigned PDF integer".to_string()))
}

fn consume_stream_eol(parser: &mut Parser<'_>) {
    if parser.data.get(parser.position) == Some(&b'\r') {
        parser.position += 1;
        if parser.data.get(parser.position) == Some(&b'\n') {
            parser.position += 1;
        }
    } else if parser.data.get(parser.position) == Some(&b'\n') {
        parser.position += 1;
    }
}

fn find_keyword(data: &[u8], start: usize, keyword: &[u8]) -> Option<usize> {
    data.get(start..)?
        .windows(keyword.len())
        .position(|window| window == keyword)
        .map(|offset| start + offset)
}

fn find_stream_end(data: &[u8], start: usize) -> Option<usize> {
    let keyword = b"endstream";
    let mut cursor = start;
    while let Some(position) = find_keyword(data, cursor, keyword) {
        let has_start_boundary = position == start
            || data
                .get(position.wrapping_sub(1))
                .is_some_and(|byte| is_pdf_whitespace(*byte) || is_pdf_delimiter(*byte));
        let mut parser = Parser::new(data, position + keyword.len());
        parser.skip_space();
        if has_start_boundary && parser.consume_keyword(b"endobj") {
            return Some(position);
        }
        cursor = position.checked_add(1)?;
    }
    None
}

fn is_pdf_whitespace(byte: u8) -> bool {
    matches!(byte, 0 | b'\t' | b'\n' | 0x0c | b'\r' | b' ')
}

fn is_pdf_delimiter(byte: u8) -> bool {
    matches!(
        byte,
        b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%'
    )
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug)]
enum XrefEntry {
    Free,
    Normal { offset: usize, generation: u16 },
    Compressed { stream: u32, index: u32 },
}

#[derive(Clone, Debug)]
pub(crate) struct Document {
    pub(crate) version: String,
    pub(crate) trailer: Dictionary,
    pub(crate) objects: BTreeMap<ObjectId, Object>,
    pub(crate) max_id: u32,
}

impl Document {
    pub(crate) fn with_version(version: &str) -> Self {
        Self {
            version: version.to_string(),
            trailer: Dictionary::new(),
            objects: BTreeMap::new(),
            max_id: 0,
        }
    }

    pub(crate) fn load(path: impl AsRef<Path>) -> Result<Self> {
        let data = std::fs::read(path)?;
        Self::load_mem(&data)
    }

    pub(crate) fn load_mem(data: &[u8]) -> Result<Self> {
        let version = parse_pdf_version(data)?;
        let mut xref = BTreeMap::new();
        let mut trailer = Dictionary::new();
        let xref_result = find_startxref(data)
            .ok_or_else(|| Error::Parse("missing startxref".to_string()))
            .and_then(|offset| {
                let mut visited = HashSet::new();
                parse_xref_chain(data, offset, &mut xref, &mut trailer, &mut visited)
            });

        let mut objects = BTreeMap::new();
        if xref_result.is_ok() {
            let entries: Vec<(u32, XrefEntry)> = xref
                .iter()
                .map(|(number, entry)| (*number, *entry))
                .collect();
            let mut unresolved = Vec::new();
            for (number, entry) in entries {
                let XrefEntry::Normal { offset, generation } = entry else {
                    continue;
                };
                let expected_id = (number, generation);
                match parse_indirect_object(data, offset) {
                    Ok((parsed_id, object, _)) if parsed_id == expected_id => {
                        objects.insert(expected_id, object);
                    }
                    Ok((parsed_id, _, _)) => unresolved.push((
                        expected_id,
                        Error::Parse(format!(
                            "xref expected object {number} {generation} at byte {offset}, found {} {}",
                            parsed_id.0, parsed_id.1
                        )),
                    )),
                    Err(error) => unresolved.push((
                        expected_id,
                        Error::Parse(format!(
                            "cannot read xref object {number} {generation} at byte {offset}: {error}"
                        )),
                    )),
                }
            }
            if !unresolved.is_empty() {
                let (scanned, _) = scan_indirect_objects(data)?;
                for (id, original_error) in unresolved {
                    let Some(object) = scanned.get(&id).cloned() else {
                        return Err(original_error);
                    };
                    objects.insert(id, object);
                }
            }
            reparse_indirect_length_streams(data, &xref, &mut objects)?;
            parse_object_streams(&mut objects, Some(&xref))?;
            reparse_indirect_length_streams(data, &xref, &mut objects)?;
        }

        if objects.is_empty() {
            let (scanned, scanned_trailer) = scan_indirect_objects(data)?;
            objects = scanned;
            if trailer.0.is_empty() {
                trailer = scanned_trailer;
            }
            parse_object_streams(&mut objects, None)?;
        }

        if trailer.0.is_empty() {
            if let Some(found) = scan_last_trailer(data) {
                trailer = found;
            }
        }
        if objects.is_empty() {
            return Err(Error::Parse("PDF contains no indirect objects".to_string()));
        }
        let max_id = objects.keys().map(|id| id.0).max().unwrap_or(0);
        Ok(Self {
            version,
            trailer,
            objects,
            max_id,
        })
    }

    pub(crate) fn is_encrypted(&self) -> bool {
        self.trailer.get(b"Encrypt").is_ok()
    }

    pub(crate) fn new_object_id(&mut self) -> ObjectId {
        self.max_id = self.max_id.saturating_add(1);
        (self.max_id, 0)
    }

    pub(crate) fn add_object<T: Into<Object>>(&mut self, object: T) -> ObjectId {
        let id = self.new_object_id();
        self.objects.insert(id, object.into());
        id
    }

    pub(crate) fn get_object(&self, id: ObjectId) -> Result<&Object> {
        self.objects.get(&id).ok_or(Error::MissingObject(id))
    }

    pub(crate) fn get_object_mut(&mut self, id: ObjectId) -> Result<&mut Object> {
        self.objects.get_mut(&id).ok_or(Error::MissingObject(id))
    }

    pub(crate) fn get_pages(&self) -> BTreeMap<u32, ObjectId> {
        let mut page_ids = Vec::new();
        if let Some(root_id) = self
            .trailer
            .get(b"Root")
            .ok()
            .and_then(|object| object.as_reference().ok())
        {
            if let Some(catalog) = self
                .resolve_object_id(root_id)
                .and_then(|object| object.as_dict().ok())
            {
                if let Some(pages) = catalog
                    .get(b"Pages")
                    .ok()
                    .and_then(|object| object.as_reference().ok())
                {
                    let mut visited = HashSet::new();
                    self.collect_pages(pages, &mut page_ids, &mut visited);
                }
            }
        }
        if page_ids.is_empty() {
            for (id, object) in &self.objects {
                if object_type_name(object) == Some(b"Page".as_slice()) {
                    page_ids.push(*id);
                }
            }
        }
        page_ids
            .into_iter()
            .enumerate()
            .map(|(index, id)| ((index + 1) as u32, id))
            .collect()
    }

    pub(crate) fn get_page_attribute<K: AsRef<[u8]> + ?Sized>(
        &self,
        page_id: ObjectId,
        key: &K,
    ) -> Result<&Object> {
        let mut current = page_id;
        let mut visited = HashSet::new();
        for _ in 0..=MAX_OBJECT_DEPTH {
            if !visited.insert(current) {
                return Err(Error::Parse("cycle in PDF page tree".to_string()));
            }
            let dictionary = self.get_object(current)?.as_dict()?;
            if let Ok(value) = dictionary.get(key) {
                return Ok(value);
            }
            current = dictionary.get(b"Parent")?.as_reference()?;
        }
        Err(Error::Parse(
            "PDF page-tree inheritance limit exceeded".to_string(),
        ))
    }

    fn collect_pages(
        &self,
        id: ObjectId,
        output: &mut Vec<ObjectId>,
        visited: &mut HashSet<ObjectId>,
    ) {
        if !visited.insert(id) {
            return;
        }
        let Some(object) = self.resolve_object_id(id) else {
            return;
        };
        let Some(dictionary) = object_dictionary(object) else {
            return;
        };
        if dictionary
            .get(b"Type")
            .ok()
            .and_then(|object| object.as_name().ok())
            == Some(b"Page".as_slice())
        {
            output.push(id);
            return;
        }
        if let Ok(kids) = dictionary.get(b"Kids").and_then(Object::as_array) {
            for kid in kids {
                if let Ok(kid_id) = kid.as_reference() {
                    self.collect_pages(kid_id, output, visited);
                }
            }
        }
    }

    pub(crate) fn get_page_content(&self, page_id: ObjectId) -> Result<Vec<u8>> {
        let page = self.get_object(page_id)?.as_dict()?;
        let Ok(contents) = page.get(b"Contents") else {
            return Ok(Vec::new());
        };
        let mut output = Vec::new();
        self.append_content_object(contents, &mut output, 0)?;
        Ok(output)
    }

    fn append_content_object(
        &self,
        object: &Object,
        output: &mut Vec<u8>,
        depth: usize,
    ) -> Result<()> {
        if depth > MAX_OBJECT_DEPTH {
            return Err(Error::Parse(
                "content reference nesting limit exceeded".to_string(),
            ));
        }
        match object {
            Object::Reference(id) => {
                self.append_content_object(self.get_object(*id)?, output, depth + 1)
            }
            Object::Stream(stream) => {
                if !output.is_empty() && !output.ends_with(b"\n") {
                    output.push(b'\n');
                }
                output.extend_from_slice(&stream.get_plain_content()?);
                if !output.ends_with(b"\n") {
                    output.push(b'\n');
                }
                Ok(())
            }
            Object::Array(items) => {
                for item in items {
                    self.append_content_object(item, output, depth + 1)?;
                }
                Ok(())
            }
            _ => Err(Error::Type("a content stream")),
        }
    }

    pub(crate) fn add_page_contents(&mut self, page_id: ObjectId, content: Vec<u8>) -> Result<()> {
        let content_id = self.add_object(Stream::new(Dictionary::new(), content));
        let page = self.get_object_mut(page_id)?.as_dict_mut()?;
        let next = Object::Reference(content_id);
        match page.remove(b"Contents") {
            None | Some(Object::Null) => page.set("Contents", next),
            Some(Object::Array(mut items)) => {
                items.push(next);
                page.set("Contents", Object::Array(items));
            }
            Some(existing) => page.set("Contents", Object::Array(vec![existing, next])),
        }
        Ok(())
    }

    pub(crate) fn renumber_objects_with(&mut self, start_at: u32) {
        let mut mapping = BTreeMap::new();
        let mut next = start_at;
        for id in self.objects.keys() {
            mapping.insert(*id, (next, 0));
            next = next.saturating_add(1);
        }
        let mut objects = BTreeMap::new();
        for (old_id, mut object) in std::mem::take(&mut self.objects) {
            remap_references(&mut object, &mapping);
            objects.insert(mapping[&old_id], object);
        }
        remap_dictionary_references(&mut self.trailer, &mapping);
        self.objects = objects;
        self.max_id = next.saturating_sub(1);
    }

    pub(crate) fn renumber_objects(&mut self) {
        self.renumber_objects_with(1);
    }

    pub(crate) fn prune_objects(&mut self) {
        let mut reachable = BTreeSet::new();
        let mut queue = VecDeque::new();
        collect_dictionary_references(&self.trailer, &mut queue);
        while let Some(id) = queue.pop_front() {
            if !reachable.insert(id) {
                continue;
            }
            if let Some(object) = self.objects.get(&id) {
                collect_object_references(object, &mut queue);
            }
        }
        if !reachable.is_empty() {
            self.objects.retain(|id, _| reachable.contains(id));
            self.max_id = self.objects.keys().map(|id| id.0).max().unwrap_or(0);
        }
    }

    pub(crate) fn compress(&mut self) {
        for object in self.objects.values_mut() {
            let Object::Stream(stream) = object else {
                continue;
            };
            if stream.dict.get(b"Filter").is_ok() || stream.content.is_empty() {
                continue;
            }
            let compressed = crate::flate_native::zlib_deflate_parallel(&stream.content);
            if compressed.len() < stream.content.len() {
                stream.content = compressed;
                stream.dict.set("Filter", "FlateDecode");
            }
        }
    }

    pub(crate) fn save(&mut self, path: impl AsRef<Path>) -> Result<()> {
        let mut data = Vec::new();
        self.save_to(&mut data)?;
        std::fs::write(path, data)?;
        Ok(())
    }

    pub(crate) fn save_to(&mut self, output: &mut impl Write) -> Result<()> {
        let bytes = self.serialize()?;
        output.write_all(&bytes)?;
        Ok(())
    }

    fn serialize(&mut self) -> Result<Vec<u8>> {
        self.max_id = self.objects.keys().map(|id| id.0).max().unwrap_or(0);
        // This is a full rewrite, not an incremental update. Byte offsets into the source file and
        // xref-stream implementation keys would be stale or illegal in the new classic trailer.
        for key in [
            b"Prev".as_slice(),
            b"XRefStm",
            b"Type",
            b"W",
            b"Index",
            b"Length",
            b"Filter",
            b"DecodeParms",
            b"F",
            b"FFilter",
            b"FDecodeParms",
            b"DL",
        ] {
            self.trailer.remove(key);
        }
        let mut output = format!("%PDF-{}\n%", self.version).into_bytes();
        output.extend_from_slice(&[0xe2, 0xe3, 0xcf, 0xd3, b'\n']);
        let mut offsets: BTreeMap<u32, (u16, usize)> = BTreeMap::new();
        for (id, object) in &self.objects {
            if offsets.insert(id.0, (id.1, output.len())).is_some() {
                return Err(Error::Parse(format!(
                    "multiple generations of object {} cannot be written in one xref section",
                    id.0
                )));
            }
            output.extend_from_slice(format!("{} {} obj\n", id.0, id.1).as_bytes());
            write_object(&mut output, object)?;
            output.extend_from_slice(b"\nendobj\n");
        }
        let xref_offset = output.len();
        let size = self.max_id.saturating_add(1);
        output.extend_from_slice(b"xref\n0 1\n");
        output.extend_from_slice(b"0000000000 65535 f \n");
        let entries: Vec<(u32, u16, usize)> = offsets
            .into_iter()
            .filter(|(number, _)| *number != 0)
            .map(|(number, (generation, offset))| (number, generation, offset))
            .collect();
        let mut group_start = 0usize;
        while group_start < entries.len() {
            let mut group_end = group_start + 1;
            while group_end < entries.len()
                && entries[group_end].0 == entries[group_end - 1].0.saturating_add(1)
            {
                group_end += 1;
            }
            output.extend_from_slice(
                format!("{} {}\n", entries[group_start].0, group_end - group_start).as_bytes(),
            );
            for &(_, generation, offset) in &entries[group_start..group_end] {
                output.extend_from_slice(format!("{offset:010} {generation:05} n \n").as_bytes());
            }
            group_start = group_end;
        }
        self.trailer.set("Size", size);
        output.extend_from_slice(b"trailer\n");
        write_dictionary(&mut output, &self.trailer)?;
        output.extend_from_slice(format!("\nstartxref\n{xref_offset}\n%%EOF\n").as_bytes());
        Ok(output)
    }

    #[cfg_attr(not(feature = "python"), allow(dead_code))]
    pub(crate) fn extract_text_chunks(&self, pages: &[u32]) -> Vec<Result<String>> {
        crate::pdf_raster::extract_text_chunks(self, pages)
    }

    fn resolve_object_id(&self, id: ObjectId) -> Option<&Object> {
        let mut object = self.objects.get(&id)?;
        let mut remaining = MAX_OBJECT_DEPTH;
        while let Object::Reference(next) = object {
            if remaining == 0 {
                return None;
            }
            object = self.objects.get(next)?;
            remaining -= 1;
        }
        Some(object)
    }
}

fn parse_pdf_version(data: &[u8]) -> Result<String> {
    let marker = data
        .windows(5)
        .position(|window| window == b"%PDF-")
        .ok_or_else(|| Error::Parse("missing PDF header".to_string()))?;
    let start = marker + 5;
    let end = data[start..]
        .iter()
        .position(|byte| is_pdf_whitespace(*byte))
        .map(|offset| start + offset)
        .unwrap_or(data.len());
    let version = std::str::from_utf8(&data[start..end])
        .map_err(|_| Error::Parse("invalid PDF version".to_string()))?;
    if version.is_empty() {
        Err(Error::Parse("empty PDF version".to_string()))
    } else {
        Ok(version.to_string())
    }
}

fn find_startxref(data: &[u8]) -> Option<usize> {
    let marker = data
        .windows(b"startxref".len())
        .rposition(|window| window == b"startxref")?;
    let mut parser = Parser::new(data, marker + b"startxref".len());
    parser.skip_space();
    usize::try_from(parse_unsigned_token(&mut parser).ok()?).ok()
}

fn parse_xref_chain(
    data: &[u8],
    offset: usize,
    entries: &mut BTreeMap<u32, XrefEntry>,
    trailer: &mut Dictionary,
    visited: &mut HashSet<usize>,
) -> Result<()> {
    if !visited.insert(offset) {
        return Ok(());
    }
    let mut parser = Parser::new(data, offset);
    parser.skip_space();
    let mut section_entries = BTreeMap::new();
    let is_classic = parser.consume_keyword(b"xref");
    let section_trailer = if is_classic {
        parse_classic_xref(&mut parser, &mut section_entries)
            .map_err(|error| Error::Parse(format!("classic xref at byte {offset}: {error}")))?
    } else {
        parse_xref_stream(data, offset, &mut section_entries)
            .map_err(|error| Error::Parse(format!("xref stream at byte {offset}: {error}")))?
    };

    // A hybrid-reference stream supplements the table in the same revision. PDF 1.5+
    // readers must let its entries replace table entries with the same object number. The
    // combined revision is merged into the document-wide map only afterwards so entries from a
    // newer incremental revision still retain precedence.
    let mut supplemental_trailer = None;
    if is_classic {
        if let Some(xref_stream_offset) = section_trailer
            .get(b"XRefStm")
            .ok()
            .and_then(|object| object.as_i64().ok())
            .and_then(|value| usize::try_from(value).ok())
        {
            if xref_stream_offset == offset {
                return Err(Error::Parse("XRefStm points to its own table".to_string()));
            }
            let mut supplemental_entries = BTreeMap::new();
            let stream_trailer =
                parse_xref_stream(data, xref_stream_offset, &mut supplemental_entries).map_err(
                    |error| {
                        Error::Parse(format!(
                            "supplemental xref stream at byte {xref_stream_offset}: {error}"
                        ))
                    },
                )?;
            section_entries.extend(supplemental_entries);
            supplemental_trailer = Some(stream_trailer);
        }
    }

    for (number, entry) in section_entries {
        entries.entry(number).or_insert(entry);
    }
    if is_classic {
        merge_trailer_missing(trailer, &section_trailer);
    } else {
        merge_xref_stream_trailer_missing(trailer, &section_trailer);
    }
    if let Some(stream_trailer) = supplemental_trailer.as_ref() {
        merge_xref_stream_trailer_missing(trailer, stream_trailer);
    }

    // For a hybrid revision, Prev belongs to the classic trailer. A Prev in its supplemental
    // stream is deliberately ignored; for a pure xref-stream revision, section_trailer is that
    // stream and its Prev is followed normally.
    if let Some(previous) = section_trailer
        .get(b"Prev")
        .ok()
        .and_then(|object| object.as_i64().ok())
        .and_then(|value| usize::try_from(value).ok())
    {
        parse_xref_chain(data, previous, entries, trailer, visited)?;
    }
    Ok(())
}

fn parse_classic_xref(
    parser: &mut Parser<'_>,
    entries: &mut BTreeMap<u32, XrefEntry>,
) -> Result<Dictionary> {
    loop {
        parser.skip_space();
        if parser.consume_keyword(b"trailer") {
            parser.skip_space();
            return parser.parse_dictionary(0);
        }
        let first = u32::try_from(parse_unsigned_token(parser)?)
            .map_err(|_| Error::Parse("xref object number outside range".to_string()))?;
        parser.skip_space();
        let count = usize::try_from(parse_unsigned_token(parser)?)
            .map_err(|_| Error::Parse("xref count outside range".to_string()))?;
        for index in 0..count {
            parser.skip_space();
            let offset = usize::try_from(parse_unsigned_token(parser)?)
                .map_err(|_| Error::Parse("xref offset outside range".to_string()))?;
            parser.skip_space();
            let generation = u16::try_from(parse_unsigned_token(parser)?)
                .map_err(|_| Error::Parse("xref generation outside range".to_string()))?;
            parser.skip_space();
            let status = parser.read_regular_token();
            let index = u32::try_from(index)
                .map_err(|_| Error::Parse("xref subsection exceeds object range".to_string()))?;
            let number = first
                .checked_add(index)
                .ok_or_else(|| Error::Parse("xref object number overflow".to_string()))?;
            let entry = match status {
                b"n" => XrefEntry::Normal { offset, generation },
                b"f" => XrefEntry::Free,
                _ => return Err(Error::Parse("invalid xref entry status".to_string())),
            };
            entries.insert(number, entry);
        }
    }
}

fn parse_xref_stream(
    data: &[u8],
    offset: usize,
    entries: &mut BTreeMap<u32, XrefEntry>,
) -> Result<Dictionary> {
    let (_, object, _) = parse_indirect_object(data, offset)?;
    let stream = object.as_stream()?;
    if stream.dict.get(b"Type").and_then(Object::as_name)? != b"XRef" {
        return Err(Error::Parse(
            "cross-reference stream has the wrong Type".to_string(),
        ));
    }
    let widths = stream
        .dict
        .get(b"W")?
        .as_array()?
        .iter()
        .map(|object| {
            object
                .as_i64()
                .ok()
                .and_then(|value| usize::try_from(value).ok())
                .ok_or_else(|| Error::Parse("invalid xref W entry".to_string()))
        })
        .collect::<Result<Vec<_>>>()?;
    if widths.len() != 3 {
        return Err(Error::Parse("xref W must contain three widths".to_string()));
    }
    if widths
        .iter()
        .any(|width| *width > std::mem::size_of::<u64>())
    {
        return Err(Error::Parse("xref field width exceeds 64 bits".to_string()));
    }
    let size = stream
        .dict
        .get(b"Size")?
        .as_i64()
        .ok()
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| Error::Parse("invalid xref Size".to_string()))?;
    let ranges = match stream.dict.get(b"Index") {
        Ok(Object::Array(values)) => values
            .iter()
            .map(|object| {
                object
                    .as_i64()
                    .ok()
                    .and_then(|value| u32::try_from(value).ok())
                    .ok_or_else(|| Error::Parse("invalid xref Index".to_string()))
            })
            .collect::<Result<Vec<_>>>()?,
        _ => vec![0, size],
    };
    if ranges.len() % 2 != 0 {
        return Err(Error::Parse("xref Index has odd length".to_string()));
    }
    let bytes = stream.get_plain_content()?;
    let entry_width = widths.iter().try_fold(0usize, |total, width| {
        total
            .checked_add(*width)
            .ok_or_else(|| Error::Parse("xref entry width overflow".to_string()))
    })?;
    if entry_width == 0 {
        return Err(Error::Parse("zero-width xref entries".to_string()));
    }
    let mut cursor = 0usize;
    for range in ranges.chunks_exact(2) {
        let end = range[0]
            .checked_add(range[1])
            .ok_or_else(|| Error::Parse("xref Index range overflow".to_string()))?;
        for number in range[0]..end {
            let row_end = cursor
                .checked_add(entry_width)
                .ok_or_else(|| Error::Parse("xref stream cursor overflow".to_string()))?;
            let row = bytes
                .get(cursor..row_end)
                .ok_or_else(|| Error::Parse("truncated xref stream".to_string()))?;
            cursor = row_end;
            let kind = if widths[0] == 0 {
                1
            } else {
                read_big_endian(&row[..widths[0]])
            };
            let second_start = widths[0];
            let third_start = second_start + widths[1];
            let second = read_big_endian(&row[second_start..third_start]);
            let third = read_big_endian(&row[third_start..]);
            let entry = match kind {
                0 => XrefEntry::Free,
                1 => XrefEntry::Normal {
                    offset: usize::try_from(second)
                        .map_err(|_| Error::Parse("xref offset outside range".to_string()))?,
                    generation: u16::try_from(third)
                        .map_err(|_| Error::Parse("xref generation outside range".to_string()))?,
                },
                2 => XrefEntry::Compressed {
                    stream: u32::try_from(second).map_err(|_| {
                        Error::Parse("object stream number outside range".to_string())
                    })?,
                    index: u32::try_from(third).map_err(|_| {
                        Error::Parse("object stream index outside range".to_string())
                    })?,
                },
                _ => continue,
            };
            entries.insert(number, entry);
        }
    }
    Ok(stream.dict.clone())
}

fn read_big_endian(bytes: &[u8]) -> u64 {
    bytes
        .iter()
        .fold(0u64, |value, byte| (value << 8) | u64::from(*byte))
}

fn merge_trailer_missing(destination: &mut Dictionary, source: &Dictionary) {
    for (key, value) in source.iter() {
        destination
            .0
            .entry(key.clone())
            .or_insert_with(|| value.clone());
    }
}

fn merge_xref_stream_trailer_missing(destination: &mut Dictionary, source: &Dictionary) {
    const STREAM_ONLY_KEYS: [&[u8]; 10] = [
        b"Type",
        b"W",
        b"Index",
        b"Length",
        b"Filter",
        b"DecodeParms",
        b"F",
        b"FFilter",
        b"FDecodeParms",
        b"DL",
    ];
    for (key, value) in source.iter() {
        if STREAM_ONLY_KEYS.contains(&key.as_slice()) {
            continue;
        }
        destination
            .0
            .entry(key.clone())
            .or_insert_with(|| value.clone());
    }
}

fn scan_indirect_objects(data: &[u8]) -> Result<(BTreeMap<ObjectId, Object>, Dictionary)> {
    let mut objects = BTreeMap::new();
    let mut position = 0usize;
    while position < data.len() {
        let byte = data[position];
        let boundary = position == 0
            || is_pdf_whitespace(data[position - 1])
            || is_pdf_delimiter(data[position - 1]);
        if boundary && byte.is_ascii_digit() {
            if let Ok((id, object, end)) = parse_indirect_object(data, position) {
                if end > position {
                    objects.insert(id, object);
                    position = end;
                    continue;
                }
            }
        }
        position += 1;
    }
    Ok((objects, scan_last_trailer(data).unwrap_or_default()))
}

fn scan_last_trailer(data: &[u8]) -> Option<Dictionary> {
    let mut search_end = data.len();
    while let Some(position) = data[..search_end]
        .windows(b"trailer".len())
        .rposition(|window| window == b"trailer")
    {
        let mut parser = Parser::new(data, position + b"trailer".len());
        parser.skip_space();
        if let Ok(dictionary) = parser.parse_dictionary(0) {
            return Some(dictionary);
        }
        if position == 0 {
            break;
        }
        search_end = position;
    }
    None
}

fn reparse_indirect_length_streams(
    data: &[u8],
    xref: &BTreeMap<u32, XrefEntry>,
    objects: &mut BTreeMap<ObjectId, Object>,
) -> Result<()> {
    let mut reparses = Vec::new();
    for (&number, &entry) in xref {
        let XrefEntry::Normal { offset, generation } = entry else {
            continue;
        };
        let id = (number, generation);
        let Some(Object::Stream(stream)) = objects.get(&id) else {
            continue;
        };
        let Ok(Object::Reference(length_id)) = stream.dict.get(b"Length") else {
            continue;
        };
        let Some(length) = resolve_indirect_stream_length(objects, *length_id) else {
            continue;
        };
        reparses.push((id, offset, length));
    }

    for (expected_id, offset, length) in reparses {
        let (parsed_id, object, _) = parse_indirect_object_with_length(data, offset, Some(length))?;
        if parsed_id != expected_id {
            return Err(Error::Parse(format!(
                "xref expected object {} {}, found {} {}",
                expected_id.0, expected_id.1, parsed_id.0, parsed_id.1
            )));
        }
        if !matches!(object, Object::Stream(_)) {
            return Err(Error::Parse(format!(
                "object {} {} with an indirect Length is not a stream",
                expected_id.0, expected_id.1
            )));
        }
        objects.insert(expected_id, object);
    }
    Ok(())
}

fn resolve_indirect_stream_length(
    objects: &BTreeMap<ObjectId, Object>,
    mut id: ObjectId,
) -> Option<usize> {
    let mut visited = HashSet::new();
    for _ in 0..=MAX_OBJECT_DEPTH {
        if !visited.insert(id) {
            return None;
        }
        match objects.get(&id)? {
            Object::Integer(value) => return usize::try_from(*value).ok(),
            Object::Reference(next) => id = *next,
            _ => return None,
        }
    }
    None
}

fn parse_object_streams(
    objects: &mut BTreeMap<ObjectId, Object>,
    xref: Option<&BTreeMap<u32, XrefEntry>>,
) -> Result<()> {
    if let Some(xref) = xref {
        let mut targets: BTreeMap<u32, Vec<(u32, u32)>> = BTreeMap::new();
        for (&number, &entry) in xref {
            if let XrefEntry::Compressed { stream, index } = entry {
                targets.entry(stream).or_default().push((number, index));
            }
        }
        for (stream_number, compressed_objects) in targets {
            let stream = objects
                .get(&(stream_number, 0))
                .and_then(|object| match object {
                    Object::Stream(stream)
                        if stream
                            .dict
                            .get(b"Type")
                            .ok()
                            .and_then(|object| object.as_name().ok())
                            == Some(b"ObjStm".as_slice()) =>
                    {
                        Some(stream.clone())
                    }
                    _ => None,
                })
                .ok_or_else(|| {
                    Error::Parse(format!(
                        "xref references missing object stream {stream_number}"
                    ))
                })?;
            let decoded = decode_object_stream(&stream)?;
            for (number, index) in compressed_objects {
                let index = usize::try_from(index).map_err(|_| {
                    Error::Parse(format!(
                        "object {number} has an invalid object-stream index"
                    ))
                })?;
                let Some((header_number, object)) = decoded.get(index) else {
                    return Err(Error::Parse(format!(
                        "object {number} has out-of-range index {index} in object stream {stream_number}"
                    )));
                };
                if *header_number != number {
                    return Err(Error::Parse(format!(
                        "xref maps object {number} to object stream {stream_number} index {index}, whose header names object {header_number}"
                    )));
                }
                objects.insert((number, 0), object.clone());
            }
        }
        return Ok(());
    }

    // Xref reconstruction is a repair path. With no authoritative compressed-entry map, recover
    // every object advertised by a syntactically valid object stream, while preserving any
    // directly scanned object with the same identifier.
    let streams: Vec<Stream> = objects
        .values()
        .filter_map(|object| {
            let Object::Stream(stream) = object else {
                return None;
            };
            (stream
                .dict
                .get(b"Type")
                .ok()
                .and_then(|object| object.as_name().ok())
                == Some(b"ObjStm".as_slice()))
            .then_some(stream.clone())
        })
        .collect();
    for stream in streams {
        for (number, object) in decode_object_stream(&stream)? {
            objects.entry((number, 0)).or_insert(object);
        }
    }
    Ok(())
}

fn decode_object_stream(stream: &Stream) -> Result<Vec<(u32, Object)>> {
    let count = stream
        .dict
        .get(b"N")?
        .as_i64()
        .ok()
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| Error::Parse("invalid object stream N".to_string()))?;
    let first = stream
        .dict
        .get(b"First")?
        .as_i64()
        .ok()
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| Error::Parse("invalid object stream First".to_string()))?;
    let bytes = stream.get_plain_content()?;
    if first > bytes.len() {
        return Err(Error::Parse("object stream First outside data".to_string()));
    }
    // Each index entry requires at least two one-byte integers and a separator. This check keeps
    // a malicious /N from causing an allocation unrelated to the bounded stream payload.
    if count > first.saturating_add(1).div_ceil(3) {
        return Err(Error::Parse(
            "object stream N exceeds its header capacity".to_string(),
        ));
    }
    let mut header = Parser::new(&bytes, 0);
    let mut index = Vec::with_capacity(count);
    for _ in 0..count {
        header.skip_space();
        let number = u32::try_from(parse_unsigned_token(&mut header)?)
            .map_err(|_| Error::Parse("object stream number outside range".to_string()))?;
        header.skip_space();
        let offset = usize::try_from(parse_unsigned_token(&mut header)?)
            .map_err(|_| Error::Parse("object stream offset outside range".to_string()))?;
        index.push((number, offset));
    }
    header.skip_space();
    if header.position > first {
        return Err(Error::Parse(
            "object stream index overlaps object data".to_string(),
        ));
    }
    let mut decoded = Vec::with_capacity(count);
    for (number, offset) in index {
        let start = first
            .checked_add(offset)
            .filter(|start| *start < bytes.len())
            .ok_or_else(|| Error::Parse("object stream offset outside data".to_string()))?;
        let mut parser = Parser::new(&bytes, start);
        decoded.push((number, parser.parse_object(0)?));
    }
    Ok(decoded)
}

fn object_dictionary(object: &Object) -> Option<&Dictionary> {
    match object {
        Object::Dictionary(dictionary) => Some(dictionary),
        Object::Stream(stream) => Some(&stream.dict),
        _ => None,
    }
}

fn object_type_name(object: &Object) -> Option<&[u8]> {
    object_dictionary(object)?.get(b"Type").ok()?.as_name().ok()
}

fn remap_references(object: &mut Object, mapping: &BTreeMap<ObjectId, ObjectId>) {
    match object {
        Object::Reference(id) => {
            if let Some(mapped) = mapping.get(id) {
                *id = *mapped;
            }
        }
        Object::Array(items) => {
            for item in items {
                remap_references(item, mapping);
            }
        }
        Object::Dictionary(dictionary) => remap_dictionary_references(dictionary, mapping),
        Object::Stream(stream) => remap_dictionary_references(&mut stream.dict, mapping),
        _ => {}
    }
}

fn remap_dictionary_references(
    dictionary: &mut Dictionary,
    mapping: &BTreeMap<ObjectId, ObjectId>,
) {
    for object in dictionary.0.values_mut() {
        remap_references(object, mapping);
    }
}

fn collect_object_references(object: &Object, queue: &mut VecDeque<ObjectId>) {
    match object {
        Object::Reference(id) => queue.push_back(*id),
        Object::Array(items) => {
            for item in items {
                collect_object_references(item, queue);
            }
        }
        Object::Dictionary(dictionary) => collect_dictionary_references(dictionary, queue),
        Object::Stream(stream) => collect_dictionary_references(&stream.dict, queue),
        _ => {}
    }
}

fn collect_dictionary_references(dictionary: &Dictionary, queue: &mut VecDeque<ObjectId>) {
    for object in dictionary.values() {
        collect_object_references(object, queue);
    }
}

fn write_object(output: &mut Vec<u8>, object: &Object) -> Result<()> {
    write_object_at_depth(output, object, 0)
}

fn write_object_at_depth(output: &mut Vec<u8>, object: &Object, depth: usize) -> Result<()> {
    if depth > MAX_OBJECT_DEPTH {
        return Err(Error::Parse(
            "PDF serialization nesting limit exceeded".to_string(),
        ));
    }
    match object {
        Object::Null => output.extend_from_slice(b"null"),
        Object::Boolean(value) => output.extend_from_slice(if *value { b"true" } else { b"false" }),
        Object::Integer(value) => output.extend_from_slice(value.to_string().as_bytes()),
        Object::Real(value) => output.extend_from_slice(format_pdf_real(*value).as_bytes()),
        Object::Name(name) => write_name(output, name),
        Object::String(bytes, StringFormat::Literal) => write_literal_string(output, bytes),
        Object::String(bytes, StringFormat::Hexadecimal) => {
            output.push(b'<');
            for byte in bytes {
                output.extend_from_slice(format!("{byte:02X}").as_bytes());
            }
            output.push(b'>');
        }
        Object::Array(items) => {
            output.push(b'[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    output.push(b' ');
                }
                write_object_at_depth(output, item, depth + 1)?;
            }
            output.push(b']');
        }
        Object::Dictionary(dictionary) => write_dictionary_at_depth(output, dictionary, depth + 1)?,
        Object::Stream(stream) => {
            let mut dictionary = stream.dict.clone();
            dictionary.set("Length", stream.content.len());
            write_dictionary_at_depth(output, &dictionary, depth + 1)?;
            output.extend_from_slice(b"\nstream\n");
            output.extend_from_slice(&stream.content);
            output.extend_from_slice(b"\nendstream");
        }
        Object::Reference((number, generation)) => {
            output.extend_from_slice(format!("{number} {generation} R").as_bytes());
        }
    }
    Ok(())
}

fn write_dictionary(output: &mut Vec<u8>, dictionary: &Dictionary) -> Result<()> {
    write_dictionary_at_depth(output, dictionary, 0)
}

fn write_dictionary_at_depth(
    output: &mut Vec<u8>,
    dictionary: &Dictionary,
    depth: usize,
) -> Result<()> {
    output.extend_from_slice(b"<<");
    for (key, value) in dictionary.iter() {
        output.push(b' ');
        write_name(output, key);
        output.push(b' ');
        write_object_at_depth(output, value, depth + 1)?;
    }
    if !dictionary.0.is_empty() {
        output.push(b' ');
    }
    output.extend_from_slice(b">>");
    Ok(())
}

fn write_name(output: &mut Vec<u8>, name: &[u8]) {
    output.push(b'/');
    for byte in name {
        if (33..=126).contains(byte) && !is_pdf_delimiter(*byte) && *byte != b'#' {
            output.push(*byte);
        } else {
            output.extend_from_slice(format!("#{byte:02X}").as_bytes());
        }
    }
}

fn write_literal_string(output: &mut Vec<u8>, bytes: &[u8]) {
    output.push(b'(');
    for byte in bytes {
        match *byte {
            b'(' | b')' | b'\\' => {
                output.push(b'\\');
                output.push(*byte);
            }
            b'\n' => output.extend_from_slice(b"\\n"),
            b'\r' => output.extend_from_slice(b"\\r"),
            b'\t' => output.extend_from_slice(b"\\t"),
            0x08 => output.extend_from_slice(b"\\b"),
            0x0c => output.extend_from_slice(b"\\f"),
            32..=126 => output.push(*byte),
            value => output.extend_from_slice(format!("\\{value:03o}").as_bytes()),
        }
    }
    output.push(b')');
}

fn format_pdf_real(value: f32) -> String {
    if !value.is_finite() {
        return "0".to_string();
    }
    let mut output = format!("{value:.7}");
    while output.contains('.') && output.ends_with('0') {
        output.pop();
    }
    if output.ends_with('.') {
        output.pop();
    }
    if output == "-0" {
        "0".to_string()
    } else {
        output
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Operation {
    pub(crate) operator: String,
    pub(crate) operands: Vec<Object>,
}

impl Operation {
    #[allow(dead_code)]
    pub(crate) fn new(operator: impl Into<String>, operands: Vec<Object>) -> Self {
        Self {
            operator: operator.into(),
            operands,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Content {
    pub(crate) operations: Vec<Operation>,
}

impl Content {
    pub(crate) fn decode(data: &[u8]) -> Result<Self> {
        let mut parser = Parser::new(data, 0);
        let mut operands = Vec::new();
        let mut operations = Vec::new();
        while parser.position < data.len() {
            parser.skip_space();
            if parser.position >= data.len() {
                break;
            }
            let byte = data[parser.position];
            let object_start = matches!(
                byte,
                b'/' | b'(' | b'[' | b'<' | b'+' | b'-' | b'.' | b'0'..=b'9'
            ) || starts_keyword(data, parser.position, b"true")
                || starts_keyword(data, parser.position, b"false")
                || starts_keyword(data, parser.position, b"null");
            if object_start {
                operands.push(parser.parse_object(0)?);
                continue;
            }
            let token = parser.read_regular_token();
            if token.is_empty() {
                return Err(Error::Parse(format!(
                    "invalid content token at byte {}",
                    parser.position
                )));
            }
            let operator = std::str::from_utf8(token)
                .map_err(|_| Error::Parse("non-UTF8 PDF content operator".to_string()))?
                .to_string();
            operations.push(Operation {
                operator,
                operands: std::mem::take(&mut operands),
            });
        }
        if !operands.is_empty() {
            return Err(Error::Parse("trailing PDF content operands".to_string()));
        }
        Ok(Self { operations })
    }
}

fn starts_keyword(data: &[u8], position: usize, keyword: &[u8]) -> bool {
    data.get(position..position + keyword.len()) == Some(keyword)
        && data
            .get(position + keyword.len())
            .is_none_or(|byte| is_pdf_whitespace(*byte) || is_pdf_delimiter(*byte))
}

pub(crate) mod content {
    pub(crate) use super::{Content, Operation};
}

pub(crate) fn decode_text_string(object: &Object) -> Result<String> {
    let bytes = object.as_str()?;
    if bytes.starts_with(&[0xfe, 0xff]) {
        let units: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
            .collect();
        return String::from_utf16(&units)
            .map_err(|_| Error::Parse("invalid UTF-16BE PDF string".to_string()));
    }
    if bytes.starts_with(&[0xff, 0xfe]) {
        let units: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        return String::from_utf16(&units)
            .map_err(|_| Error::Parse("invalid UTF-16LE PDF string".to_string()));
    }
    if bytes.starts_with(&[0xef, 0xbb, 0xbf]) {
        return std::str::from_utf8(&bytes[3..])
            .map(str::to_string)
            .map_err(|_| Error::Parse("invalid UTF-8 PDF string".to_string()));
    }
    Ok(bytes.iter().map(|byte| pdf_doc_char(*byte)).collect())
}

fn pdf_doc_char(byte: u8) -> char {
    crate::pdf_encodings::pdf_doc_encoding_byte(byte).unwrap_or('\u{fffd}')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn literal_lzw_stream(data: &[u8], early_change: bool) -> Vec<u8> {
        fn write_code(output: &mut Vec<u8>, bit: &mut usize, code: usize, width: usize) {
            for shift in (0..width).rev() {
                let byte_index = *bit / 8;
                if byte_index == output.len() {
                    output.push(0);
                }
                output[byte_index] |= (((code >> shift) & 1) as u8) << (7 - (*bit % 8));
                *bit += 1;
            }
        }

        let mut output = Vec::new();
        let mut bit = 0usize;
        let mut width = 9usize;
        let mut dictionary_len = 258usize;
        let mut has_previous = false;
        write_code(&mut output, &mut bit, 256, width);
        for byte in data {
            write_code(&mut output, &mut bit, usize::from(*byte), width);
            if has_previous && dictionary_len < 4096 {
                dictionary_len += 1;
                if dictionary_len == (1usize << width) - usize::from(early_change) && width < 12 {
                    width += 1;
                }
            }
            has_previous = true;
        }
        write_code(&mut output, &mut bit, 257, width);
        output
    }

    fn single_page_document() -> Document {
        let mut document = Document::with_version("1.7");
        let pages_id = document.new_object_id();
        let content_id = document.add_object(Stream::new(
            Dictionary::new(),
            b"BT /F1 12 Tf 10 20 Td (Hello \\(PDF\\)) Tj ET".to_vec(),
        ));
        let page_id = document.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "MediaBox" => vec![0.into(), 0.into(), 200.into(), 100.into()],
            "Contents" => content_id,
        });
        document.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => vec![page_id.into()],
                "Count" => 1,
            }),
        );
        let catalog_id = document.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
        });
        document.trailer.set("Root", catalog_id);
        document
    }

    #[test]
    fn object_parser_handles_names_references_strings_and_nested_collections() {
        let bytes =
            br#"<< /Escaped#20Name [12 -3.5 4 0 R true null] /Text (a\n\050b\051) /Hex <4142F> >>"#;
        let mut parser = Parser::new(bytes, 0);
        let dictionary = parser.parse_object(0).unwrap();
        let dictionary = dictionary.as_dict().unwrap();
        assert!(dictionary.get(b"Escaped Name").is_ok());
        let array = dictionary.get(b"Escaped Name").unwrap().as_array().unwrap();
        assert_eq!(array[0], Object::Integer(12));
        assert_eq!(array[1], Object::Real(-3.5));
        assert_eq!(array[2], Object::Reference((4, 0)));
        assert_eq!(
            dictionary.get(b"Text").unwrap().as_str().unwrap(),
            b"a\n(b)"
        );
        assert_eq!(
            dictionary.get(b"Hex").unwrap().as_str().unwrap(),
            &[0x41, 0x42, 0xf0]
        );
    }

    #[test]
    fn content_parser_preserves_text_arrays_and_operators() {
        let content = Content::decode(b"BT /F1 12 Tf [(A) -20 <0042>] TJ ET").unwrap();
        assert_eq!(content.operations.len(), 4);
        assert_eq!(content.operations[0].operator, "BT");
        assert_eq!(content.operations[1].operator, "Tf");
        assert_eq!(content.operations[2].operator, "TJ");
        let array = content.operations[2].operands[0].as_array().unwrap();
        assert_eq!(array.len(), 3);
    }

    #[test]
    fn native_document_roundtrips_pages_streams_and_text() {
        let mut document = single_page_document();
        document.compress();
        let mut bytes = Vec::new();
        document.save_to(&mut bytes).unwrap();
        let loaded = Document::load_mem(&bytes).unwrap();
        let pages = loaded.get_pages();
        assert_eq!(pages.len(), 1);
        let page_id = pages[&1];
        let content = loaded.get_page_content(page_id).unwrap();
        assert!(content.windows(5).any(|window| window == b"Hello"));
        let chunks = loaded.extract_text_chunks(&[1]);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].as_ref().unwrap(), "Hello (PDF)\n");
    }

    #[test]
    fn text_extraction_uses_inherited_fonts_and_to_unicode() {
        let mut document = Document::with_version("1.7");
        let pages_id = document.new_object_id();
        let cmap_id = document.add_object(Stream::new(
            Dictionary::new(),
            b"2 beginbfchar\n<0001> <0050>\n<0002> <006B>\nendbfchar\n".to_vec(),
        ));
        let font_id = document.add_object(dictionary! {
            "Type" => "Font",
            "Subtype" => "Type0",
            "BaseFont" => "Synthetic",
            "Encoding" => "Identity-H",
            "ToUnicode" => cmap_id,
        });
        let content_id = document.add_object(Stream::new(
            Dictionary::new(),
            b"BT /F1 12 Tf <00010002> Tj ET".to_vec(),
        ));
        let page_id = document.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "Contents" => content_id,
        });
        document.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => vec![page_id.into()],
                "Count" => 1,
                "MediaBox" => vec![0.into(), 0.into(), 200.into(), 100.into()],
                "Resources" => dictionary! {
                    "Font" => dictionary! { "F1" => font_id },
                },
            }),
        );
        let catalog_id = document.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
        });
        document.trailer.set("Root", catalog_id);

        assert!(document.get_page_attribute(page_id, b"Resources").is_ok());
        let chunks = document.extract_text_chunks(&[1]);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].as_ref().unwrap(), "Pk\n");
    }

    #[test]
    fn lossless_stream_filters_decode_canonical_vectors() {
        assert_eq!(decode_ascii_hex(b"61 62 6>").unwrap(), b"ab`");
        assert_eq!(decode_ascii85(b"<~@:E_WAS,Rg~>").unwrap(), b"abcdefgh");
        assert_eq!(
            decode_run_length(&[2, b'a', b'b', b'c', 255, b'z', 128]).unwrap(),
            b"abczz"
        );
        let original = b"predictable predictable predictable";
        let compressed = crate::flate_native::zlib_deflate_parallel(original);
        let stream = Stream::new(dictionary! { "Filter" => "FlateDecode" }, compressed);
        assert_eq!(stream.get_plain_content().unwrap(), original);
        assert!(decode_ascii85(b"uuuuu~>").is_err());
    }

    #[test]
    fn tiff_and_png_predictors_cover_packed_and_wide_samples() {
        let packed_parameters = dictionary! {
            "Predictor" => 2,
            "Colors" => 3,
            "BitsPerComponent" => 4,
            "Columns" => 2,
        };
        assert_eq!(
            apply_predictor(vec![0x12, 0x33, 0x45], Some(&packed_parameters)).unwrap(),
            vec![0x12, 0x34, 0x68]
        );

        let wide_parameters = dictionary! {
            "Predictor" => 2,
            "Colors" => 1,
            "BitsPerComponent" => 16,
            "Columns" => 2,
        };
        assert_eq!(
            apply_predictor(vec![0x00, 0xff, 0x00, 0x02], Some(&wide_parameters)).unwrap(),
            vec![0x00, 0xff, 0x01, 0x01]
        );

        let png_parameters = dictionary! {
            "Predictor" => 15,
            "Colors" => 1,
            "BitsPerComponent" => 8,
            "Columns" => 3,
        };
        assert_eq!(
            apply_predictor(vec![1, 10, 10, 10, 2, 5, 5, 5], Some(&png_parameters)).unwrap(),
            vec![10, 20, 30, 15, 25, 35]
        );
    }

    #[test]
    fn lzw_decoder_handles_clear_special_case_and_width_transitions() {
        // Clear, "A", the not-yet-created code 258 ("AA"), and EOD.
        assert_eq!(
            decode_lzw(&[0x80, 0x10, 0x60, 0x50, 0x10], true).unwrap(),
            b"AAA"
        );

        let expected: Vec<u8> = (0..900).map(|index| (index % 251) as u8).collect();
        for early_change in [false, true] {
            let encoded = literal_lzw_stream(&expected, early_change);
            assert_eq!(decode_lzw(&encoded, early_change).unwrap(), expected);
        }
    }

    #[test]
    fn xref_stream_and_object_stream_are_loaded() {
        let object_two = b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>";
        let object_three = b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] >>";
        let second_offset = object_two.len() + 1;
        let header = format!("2 0 3 {second_offset} ");
        let first = header.len();
        let mut object_stream_data = header.into_bytes();
        object_stream_data.extend_from_slice(object_two);
        object_stream_data.push(b' ');
        object_stream_data.extend_from_slice(object_three);

        let mut pdf = b"%PDF-1.7\n".to_vec();
        let object_one_offset = pdf.len();
        pdf.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");
        let object_five_offset = pdf.len();
        pdf.extend_from_slice(
            format!(
                "5 0 obj\n<< /Type /ObjStm /N 2 /First {first} /Length {} >>\nstream\n",
                object_stream_data.len()
            )
            .as_bytes(),
        );
        pdf.extend_from_slice(&object_stream_data);
        pdf.extend_from_slice(b"\nendstream\nendobj\n");
        let xref_offset = pdf.len();
        let mut xref_data = Vec::new();
        for (kind, second, third) in [
            (0u8, 0u32, 65_535u16),
            (1, object_one_offset as u32, 0),
            (2, 5, 0),
            (2, 5, 1),
            (1, xref_offset as u32, 0),
            (1, object_five_offset as u32, 0),
        ] {
            xref_data.push(kind);
            xref_data.extend_from_slice(&second.to_be_bytes());
            xref_data.extend_from_slice(&third.to_be_bytes());
        }
        pdf.extend_from_slice(
            format!(
                "4 0 obj\n<< /Type /XRef /Size 6 /Root 1 0 R /W [1 4 2] /Length {} >>\nstream\n",
                xref_data.len()
            )
            .as_bytes(),
        );
        pdf.extend_from_slice(&xref_data);
        pdf.extend_from_slice(b"\nendstream\nendobj\n");
        pdf.extend_from_slice(format!("startxref\n{xref_offset}\n%%EOF\n").as_bytes());

        let document = Document::load_mem(&pdf).unwrap();
        assert!(document.get_object((2, 0)).unwrap().as_dict().is_ok());
        assert!(document.get_object((3, 0)).unwrap().as_dict().is_ok());
        assert_eq!(document.get_pages().len(), 1);
    }

    fn hybrid_object_stream_pdf() -> (Vec<u8>, usize) {
        let object_two = b"<< /Marker /FromObjectStream >>";
        let header = b"2 0 ";
        let mut object_stream_data = header.to_vec();
        object_stream_data.extend_from_slice(object_two);

        let mut pdf = b"%PDF-1.7\n".to_vec();
        let object_one_offset = pdf.len();
        pdf.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Marker 2 0 R >>\nendobj\n");
        let object_five_offset = pdf.len();
        pdf.extend_from_slice(
            format!(
                "5 0 obj\n<< /Type /ObjStm /N 1 /First {} /Length {} >>\nstream\n",
                header.len(),
                object_stream_data.len()
            )
            .as_bytes(),
        );
        pdf.extend_from_slice(&object_stream_data);
        pdf.extend_from_slice(b"\nendstream\nendobj\n");

        let xref_stream_offset = pdf.len();
        let mut xref_data = vec![2u8];
        xref_data.extend_from_slice(&5u32.to_be_bytes());
        xref_data.extend_from_slice(&0u16.to_be_bytes());
        pdf.extend_from_slice(
            format!(
                "4 0 obj\n<< /Type /XRef /Size 6 /Index [2 1] /W [1 4 2] /Length {} >>\nstream\n",
                xref_data.len()
            )
            .as_bytes(),
        );
        pdf.extend_from_slice(&xref_data);
        pdf.extend_from_slice(b"\nendstream\nendobj\n");

        let classic_xref_offset = pdf.len();
        pdf.extend_from_slice(b"xref\n0 6\n");
        pdf.extend_from_slice(b"0000000000 65535 f \n");
        pdf.extend_from_slice(format!("{object_one_offset:010} 00000 n \n").as_bytes());
        // A PDF 1.4 reader sees object 2 as free. The supplemental xref stream replaces this
        // entry for PDF 1.5+ readers and exposes the compressed object.
        pdf.extend_from_slice(b"0000000000 00000 f \n");
        pdf.extend_from_slice(b"0000000000 00000 f \n");
        pdf.extend_from_slice(format!("{xref_stream_offset:010} 00000 n \n").as_bytes());
        pdf.extend_from_slice(format!("{object_five_offset:010} 00000 n \n").as_bytes());
        pdf.extend_from_slice(
            format!(
                "trailer\n<< /Size 6 /Root 1 0 R /XRefStm {xref_stream_offset} >>\nstartxref\n{classic_xref_offset}\n%%EOF\n"
            )
            .as_bytes(),
        );
        (pdf, classic_xref_offset)
    }

    #[test]
    fn hybrid_xref_stream_replaces_same_revision_table_entry() {
        let (pdf, _) = hybrid_object_stream_pdf();
        let mut document = Document::load_mem(&pdf).unwrap();
        let marker = document
            .get_object((2, 0))
            .unwrap()
            .as_dict()
            .unwrap()
            .get(b"Marker")
            .unwrap()
            .as_name()
            .unwrap();
        assert_eq!(marker, b"FromObjectStream");

        let mut rewritten = Vec::new();
        document.save_to(&mut rewritten).unwrap();
        let rewritten_document = Document::load_mem(&rewritten).unwrap();
        assert!(rewritten_document.trailer.get(b"Prev").is_err());
        assert!(rewritten_document.trailer.get(b"XRefStm").is_err());
        assert!(rewritten_document.trailer.get(b"W").is_err());
        assert!(rewritten_document.get_object((2, 0)).is_ok());
    }

    #[test]
    fn indirect_stream_length_disambiguates_embedded_end_markers() {
        let payload = b"abc\nendstream\nendobj\nxyz";
        let mut pdf = b"%PDF-1.7\n".to_vec();
        let catalog_offset = pdf.len();
        pdf.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Payload 2 0 R >>\nendobj\n");
        let stream_offset = pdf.len();
        pdf.extend_from_slice(b"2 0 obj\n<< /Length 3 0 R >>\nstream\n");
        pdf.extend_from_slice(payload);
        pdf.extend_from_slice(b"\nendstream\nendobj\n");
        let length_offset = pdf.len();
        pdf.extend_from_slice(format!("3 0 obj\n{}\nendobj\n", payload.len()).as_bytes());
        let xref_offset = pdf.len();
        pdf.extend_from_slice(b"xref\n0 4\n0000000000 65535 f \n");
        pdf.extend_from_slice(format!("{catalog_offset:010} 00000 n \n").as_bytes());
        pdf.extend_from_slice(format!("{stream_offset:010} 00000 n \n").as_bytes());
        pdf.extend_from_slice(format!("{length_offset:010} 00000 n \n").as_bytes());
        pdf.extend_from_slice(
            format!("trailer\n<< /Size 4 /Root 1 0 R >>\nstartxref\n{xref_offset}\n%%EOF\n")
                .as_bytes(),
        );

        let document = Document::load_mem(&pdf).unwrap();
        let stream = document.get_object((2, 0)).unwrap().as_stream().unwrap();
        assert_eq!(stream.content, payload);
    }

    #[test]
    fn newer_free_entry_suppresses_stale_compressed_object() {
        let (mut pdf, previous_xref) = hybrid_object_stream_pdf();
        let update_xref = pdf.len();
        pdf.extend_from_slice(b"xref\n2 1\n0000000000 00001 f \n");
        pdf.extend_from_slice(
            format!(
                "trailer\n<< /Size 6 /Root 1 0 R /Prev {previous_xref} >>\nstartxref\n{update_xref}\n%%EOF\n"
            )
            .as_bytes(),
        );
        let mut xref = BTreeMap::new();
        let mut trailer = Dictionary::new();
        let mut visited = HashSet::new();
        parse_xref_chain(
            &pdf,
            previous_xref,
            &mut BTreeMap::new(),
            &mut Dictionary::new(),
            &mut HashSet::new(),
        )
        .expect("previous hybrid xref remains parseable after append");
        parse_xref_chain(&pdf, update_xref, &mut xref, &mut trailer, &mut visited).unwrap();
        assert!(matches!(xref.get(&2), Some(XrefEntry::Free)));
        let document = Document::load_mem(&pdf).unwrap();
        assert!(matches!(
            document.get_object((2, 0)),
            Err(Error::MissingObject((2, 0)))
        ));
    }

    #[test]
    fn sparse_object_ids_serialize_without_materializing_the_gap() {
        let mut document = single_page_document();
        document.objects.insert((1_000_000, 0), Object::Integer(7));
        document.max_id = 1_000_000;
        let mut bytes = Vec::new();
        document.save_to(&mut bytes).unwrap();
        assert!(bytes.len() < 10_000);
        let loaded = Document::load_mem(&bytes).unwrap();
        assert_eq!(
            loaded.get_object((1_000_000, 0)).unwrap(),
            &Object::Integer(7)
        );
    }

    #[test]
    fn text_strings_support_pdfdoc_and_utf16() {
        assert_eq!(
            decode_text_string(&Object::String(
                vec![b'A', 0x80, 0xa0],
                StringFormat::Literal
            ))
            .unwrap(),
            "A•€"
        );
        assert_eq!(
            decode_text_string(&Object::String(vec![0], StringFormat::Literal)).unwrap(),
            "\u{fffd}"
        );
        assert_eq!(
            decode_text_string(&Object::String(
                vec![0xfe, 0xff, 0x00, 0x41, 0x03, 0xa9],
                StringFormat::Hexadecimal
            ))
            .unwrap(),
            "AΩ"
        );
    }
}
