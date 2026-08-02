//! Dependency-free JSON reader for FullBleed's embedded contract registries.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Value {
    Null,
    Bool(bool),
    Number(Number),
    String(String),
    Array(Vec<Value>),
    Object(BTreeMap<String, Value>),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Number {
    Unsigned(u64),
    Signed(i64),
    Float(f64),
}

impl Value {
    pub(crate) fn get(&self, key: &str) -> Option<&Value> {
        self.as_object()?.get(key)
    }

    pub(crate) fn as_array(&self) -> Option<&Vec<Value>> {
        match self {
            Self::Array(value) => Some(value),
            _ => None,
        }
    }

    pub(crate) fn as_object(&self) -> Option<&BTreeMap<String, Value>> {
        match self {
            Self::Object(value) => Some(value),
            _ => None,
        }
    }

    pub(crate) fn is_object(&self) -> bool {
        matches!(self, Self::Object(_))
    }

    pub(crate) fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(value) => Some(value),
            _ => None,
        }
    }

    pub(crate) fn as_u64(&self) -> Option<u64> {
        match self {
            Self::Number(Number::Unsigned(value)) => Some(*value),
            Self::Number(Number::Signed(value)) => (*value).try_into().ok(),
            Self::Number(Number::Float(value))
                if value.is_finite()
                    && *value >= 0.0
                    && value.fract() == 0.0
                    && *value < u64::MAX as f64 =>
            {
                Some(*value as u64)
            }
            _ => None,
        }
    }

    #[allow(dead_code)]
    pub(crate) fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Number(Number::Unsigned(value)) => Some(*value as f64),
            Self::Number(Number::Signed(value)) => Some(*value as f64),
            Self::Number(Number::Float(value)) => Some(*value),
            _ => None,
        }
    }

    #[allow(dead_code)]
    pub(crate) fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(value) => Some(*value),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ParseError {
    offset: usize,
    message: &'static str,
}

impl fmt::Display for ParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "invalid JSON at byte {}: {}",
            self.offset, self.message
        )
    }
}

impl Error for ParseError {}

pub(crate) fn parse(input: &str) -> Result<Value, ParseError> {
    let mut parser = Parser {
        input,
        bytes: input.as_bytes(),
        offset: 0,
    };
    parser.skip_whitespace();
    let value = parser.parse_value()?;
    parser.skip_whitespace();
    if parser.offset != parser.bytes.len() {
        return Err(parser.error("trailing data after root value"));
    }
    Ok(value)
}

struct Parser<'a> {
    input: &'a str,
    bytes: &'a [u8],
    offset: usize,
}

impl Parser<'_> {
    fn parse_value(&mut self) -> Result<Value, ParseError> {
        match self.peek() {
            Some(b'n') => {
                self.consume_literal(b"null")?;
                Ok(Value::Null)
            }
            Some(b't') => {
                self.consume_literal(b"true")?;
                Ok(Value::Bool(true))
            }
            Some(b'f') => {
                self.consume_literal(b"false")?;
                Ok(Value::Bool(false))
            }
            Some(b'"') => self.parse_string().map(Value::String),
            Some(b'[') => self.parse_array(),
            Some(b'{') => self.parse_object(),
            Some(b'-' | b'0'..=b'9') => self.parse_number().map(Value::Number),
            Some(_) => Err(self.error("expected a JSON value")),
            None => Err(self.error("unexpected end of input")),
        }
    }

    fn parse_array(&mut self) -> Result<Value, ParseError> {
        self.offset += 1;
        self.skip_whitespace();
        let mut values = Vec::new();
        if self.take(b']') {
            return Ok(Value::Array(values));
        }
        loop {
            self.skip_whitespace();
            values.push(self.parse_value()?);
            self.skip_whitespace();
            if self.take(b']') {
                return Ok(Value::Array(values));
            }
            self.expect(b',', "expected ',' or ']' in array")?;
            self.skip_whitespace();
        }
    }

    fn parse_object(&mut self) -> Result<Value, ParseError> {
        self.offset += 1;
        self.skip_whitespace();
        let mut values = BTreeMap::new();
        if self.take(b'}') {
            return Ok(Value::Object(values));
        }
        loop {
            self.skip_whitespace();
            if self.peek() != Some(b'"') {
                return Err(self.error("expected a quoted object key"));
            }
            let key = self.parse_string()?;
            self.skip_whitespace();
            self.expect(b':', "expected ':' after object key")?;
            self.skip_whitespace();
            let value = self.parse_value()?;
            values.insert(key, value);
            self.skip_whitespace();
            if self.take(b'}') {
                return Ok(Value::Object(values));
            }
            self.expect(b',', "expected ',' or '}' in object")?;
            self.skip_whitespace();
        }
    }

    fn parse_string(&mut self) -> Result<String, ParseError> {
        self.offset += 1;
        let mut value = String::new();
        loop {
            let Some(byte) = self.peek() else {
                return Err(self.error("unterminated string"));
            };
            match byte {
                b'"' => {
                    self.offset += 1;
                    return Ok(value);
                }
                b'\\' => {
                    self.offset += 1;
                    self.parse_escape(&mut value)?;
                }
                0x00..=0x1f => return Err(self.error("unescaped control byte in string")),
                0x20..=0x7f => {
                    value.push(byte as char);
                    self.offset += 1;
                }
                _ => {
                    let character = self.input[self.offset..]
                        .chars()
                        .next()
                        .ok_or_else(|| self.error("invalid UTF-8 in string"))?;
                    value.push(character);
                    self.offset += character.len_utf8();
                }
            }
        }
    }

    fn parse_escape(&mut self, value: &mut String) -> Result<(), ParseError> {
        let escaped = self
            .peek()
            .ok_or_else(|| self.error("unterminated escape sequence"))?;
        self.offset += 1;
        match escaped {
            b'"' => value.push('"'),
            b'\\' => value.push('\\'),
            b'/' => value.push('/'),
            b'b' => value.push('\u{0008}'),
            b'f' => value.push('\u{000c}'),
            b'n' => value.push('\n'),
            b'r' => value.push('\r'),
            b't' => value.push('\t'),
            b'u' => {
                let first = self.parse_hex_quad()?;
                let scalar = if (0xd800..=0xdbff).contains(&first) {
                    if !self.take(b'\\') || !self.take(b'u') {
                        return Err(
                            self.error("high surrogate must be followed by a low surrogate")
                        );
                    }
                    let second = self.parse_hex_quad()?;
                    if !(0xdc00..=0xdfff).contains(&second) {
                        return Err(self.error("invalid low surrogate"));
                    }
                    0x10000 + (((first as u32 - 0xd800) << 10) | (second as u32 - 0xdc00))
                } else if (0xdc00..=0xdfff).contains(&first) {
                    return Err(self.error("unexpected low surrogate"));
                } else {
                    first as u32
                };
                value.push(
                    char::from_u32(scalar).ok_or_else(|| self.error("invalid Unicode scalar"))?,
                );
            }
            _ => return Err(self.error("unknown escape sequence")),
        }
        Ok(())
    }

    fn parse_hex_quad(&mut self) -> Result<u16, ParseError> {
        let mut value = 0u16;
        for _ in 0..4 {
            let digit = match self.peek() {
                Some(b'0'..=b'9') => self.bytes[self.offset] - b'0',
                Some(b'a'..=b'f') => self.bytes[self.offset] - b'a' + 10,
                Some(b'A'..=b'F') => self.bytes[self.offset] - b'A' + 10,
                _ => return Err(self.error("expected four hexadecimal Unicode digits")),
            };
            self.offset += 1;
            value = (value << 4) | digit as u16;
        }
        Ok(value)
    }

    fn parse_number(&mut self) -> Result<Number, ParseError> {
        let start = self.offset;
        let negative = self.take(b'-');
        match self.peek() {
            Some(b'0') => {
                self.offset += 1;
                if matches!(self.peek(), Some(b'0'..=b'9')) {
                    return Err(self.error("leading zero in number"));
                }
            }
            Some(b'1'..=b'9') => {
                self.offset += 1;
                while matches!(self.peek(), Some(b'0'..=b'9')) {
                    self.offset += 1;
                }
            }
            _ => return Err(self.error("expected digits after sign")),
        }

        let mut fractional = false;
        if self.take(b'.') {
            fractional = true;
            self.consume_digits("expected digits after decimal point")?;
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            fractional = true;
            self.offset += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.offset += 1;
            }
            self.consume_digits("expected exponent digits")?;
        }

        let text = &self.input[start..self.offset];
        if !fractional {
            if negative {
                if let Ok(value) = text.parse::<i64>() {
                    return Ok(Number::Signed(value));
                }
            } else if let Ok(value) = text.parse::<u64>() {
                return Ok(Number::Unsigned(value));
            }
        }
        let value = text
            .parse::<f64>()
            .map_err(|_| self.error("number is outside the supported range"))?;
        if !value.is_finite() {
            return Err(self.error("number is outside the supported range"));
        }
        Ok(Number::Float(value))
    }

    fn consume_digits(&mut self, message: &'static str) -> Result<(), ParseError> {
        let start = self.offset;
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.offset += 1;
        }
        if self.offset == start {
            Err(self.error(message))
        } else {
            Ok(())
        }
    }

    fn consume_literal(&mut self, literal: &[u8]) -> Result<(), ParseError> {
        if self.bytes[self.offset..].starts_with(literal) {
            self.offset += literal.len();
            Ok(())
        } else {
            Err(self.error("invalid literal"))
        }
    }

    fn skip_whitespace(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\n' | b'\r' | b'\t')) {
            self.offset += 1;
        }
    }

    fn expect(&mut self, expected: u8, message: &'static str) -> Result<(), ParseError> {
        if self.take(expected) {
            Ok(())
        } else {
            Err(self.error(message))
        }
    }

    fn take(&mut self, expected: u8) -> bool {
        if self.peek() == Some(expected) {
            self.offset += 1;
            true
        } else {
            false
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.offset).copied()
    }

    fn error(&self, message: &'static str) -> ParseError {
        ParseError {
            offset: self.offset,
            message,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_nested_values_and_numbers() {
        let root =
            parse(r#"{"name":"FullBleed","values":[null,true,-2,3.5,6e2]}"#).expect("valid JSON");
        assert_eq!(root.get("name").and_then(Value::as_str), Some("FullBleed"));
        let values = root
            .get("values")
            .and_then(Value::as_array)
            .expect("values array");
        assert_eq!(values[1].as_bool(), Some(true));
        assert_eq!(values[2].as_f64(), Some(-2.0));
        assert_eq!(values[3].as_f64(), Some(3.5));
        assert_eq!(values[4].as_f64(), Some(600.0));
    }

    #[test]
    fn decodes_unicode_escapes_and_surrogate_pairs() {
        assert_eq!(
            parse(r#""line\n\u03bb \ud83d\ude80""#)
                .expect("valid string")
                .as_str(),
            Some("line\nλ 🚀")
        );
    }

    #[test]
    fn rejects_non_json_extensions_and_trailing_input() {
        for invalid in ["{name: 1}", "[1,]", "01", "true false", r#""\ud800""#] {
            assert!(
                parse(invalid).is_err(),
                "input should be rejected: {invalid}"
            );
        }
    }
}
