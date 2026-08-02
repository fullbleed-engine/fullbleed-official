//! Bounded, dependency-free CSS syntax parsing.
//!
//! This module deliberately owns syntax rather than computed-value policy. It preserves declaration
//! values as normalized source text while correctly handling comments, strings, escapes, nested
//! functions, blocks, and at-rules. Consumers can therefore implement only the value grammars they
//! support without falling back to delimiter splitting that corrupts data URLs or custom properties.

use std::fmt;

const MAX_CSS_BYTES: usize = 64 * 1024 * 1024;
const MAX_RULE_DEPTH: usize = 128;
const MAX_RULES: usize = 1_000_000;
const MAX_DECLARATIONS: usize = 1_000_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Error {
    offset: usize,
    message: String,
}

impl Error {
    fn new(offset: usize, message: impl Into<String>) -> Self {
        Self {
            offset,
            message: message.into(),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "CSS parse error at byte {}: {}",
            self.offset, self.message
        )
    }
}

impl std::error::Error for Error {}

pub(crate) type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Stylesheet {
    pub(crate) rules: Vec<Rule>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Rule {
    Style(StyleRule),
    At(AtRule),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StyleRule {
    pub(crate) selectors: String,
    pub(crate) declarations: DeclarationBlock,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AtRule {
    pub(crate) name: String,
    pub(crate) prelude: String,
    pub(crate) block: Option<AtRuleBlock>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AtRuleBlock {
    Rules(Vec<Rule>),
    Declarations(DeclarationBlock),
    Raw(String),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct DeclarationBlock {
    pub(crate) declarations: Vec<Declaration>,
}

impl DeclarationBlock {
    pub(crate) fn normal(&self) -> impl Iterator<Item = &Declaration> {
        self.declarations
            .iter()
            .filter(|declaration| !declaration.important)
    }

    pub(crate) fn important(&self) -> impl Iterator<Item = &Declaration> {
        self.declarations
            .iter()
            .filter(|declaration| declaration.important)
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.declarations.is_empty()
    }

    #[cfg(test)]
    pub(crate) fn to_css_string(&self) -> String {
        let mut output = String::new();
        for declaration in &self.declarations {
            if !output.is_empty() {
                output.push(';');
            }
            output.push_str(&declaration.name);
            output.push(':');
            output.push_str(&declaration.value);
            if declaration.important {
                output.push_str("!important");
            }
        }
        output
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Declaration {
    pub(crate) name: String,
    pub(crate) value: String,
    pub(crate) important: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ComponentValue {
    Ident(String),
    String(String),
    Function { name: String, arguments: String },
    Number(String),
    Delim(char),
}

impl Declaration {
    pub(crate) fn name_eq(&self, expected: &str) -> bool {
        self.name.eq_ignore_ascii_case(expected)
    }
}

#[derive(Default)]
struct Budget {
    rules: usize,
    declarations: usize,
}

pub(crate) fn parse_stylesheet(input: &str) -> Result<Stylesheet> {
    ensure_input_bound(input)?;
    let mut budget = Budget::default();
    Ok(Stylesheet {
        rules: parse_rule_list(input, 0, 0, &mut budget)?,
    })
}

pub(crate) fn parse_declaration_block(input: &str) -> Result<DeclarationBlock> {
    ensure_input_bound(input)?;
    let mut budget = Budget::default();
    parse_declarations(input, 0, &mut budget)
}

fn ensure_input_bound(input: &str) -> Result<()> {
    if input.len() > MAX_CSS_BYTES {
        return Err(Error::new(
            MAX_CSS_BYTES,
            format!("input exceeds {MAX_CSS_BYTES} bytes"),
        ));
    }
    Ok(())
}

fn parse_rule_list(
    input: &str,
    base_offset: usize,
    depth: usize,
    budget: &mut Budget,
) -> Result<Vec<Rule>> {
    if depth > MAX_RULE_DEPTH {
        return Err(Error::new(base_offset, "at-rule nesting limit exceeded"));
    }

    let mut rules = Vec::new();
    let mut position = 0usize;
    while position < input.len() {
        position = skip_whitespace_and_comments(input, position, base_offset)?;
        if position >= input.len() {
            break;
        }
        if input.as_bytes()[position] == b'}' {
            return Err(Error::new(
                base_offset + position,
                "unexpected closing brace",
            ));
        }

        let (rule, next) = if input.as_bytes()[position] == b'@' {
            parse_at_rule(input, position, base_offset, depth, budget)?
        } else {
            parse_style_rule(input, position, base_offset, budget)?
        };
        if next <= position {
            return Err(Error::new(
                base_offset + position,
                "parser made no forward progress",
            ));
        }
        position = next;
        if let Some(rule) = rule {
            budget.rules = budget.rules.saturating_add(1);
            if budget.rules > MAX_RULES {
                return Err(Error::new(base_offset + position, "rule limit exceeded"));
            }
            rules.push(rule);
        }
    }
    Ok(rules)
}

fn parse_style_rule(
    input: &str,
    start: usize,
    base_offset: usize,
    budget: &mut Budget,
) -> Result<(Option<Rule>, usize)> {
    let Some((terminator, delimiter)) = scan_to_rule_delimiter(input, start, base_offset)? else {
        // A trailing invalid fragment is ignored in the same spirit as CSS error recovery.
        return Ok((None, input.len()));
    };
    if delimiter == b';' {
        return Ok((None, terminator + 1));
    }

    let selectors = clean_component(&input[start..terminator], base_offset + start)?;
    let (body, next) = extract_block(input, terminator, base_offset)?;
    if selectors.is_empty() {
        return Ok((None, next));
    }
    let declarations = parse_declarations(body, base_offset + terminator + 1, budget)?;
    Ok((
        Some(Rule::Style(StyleRule {
            selectors,
            declarations,
        })),
        next,
    ))
}

fn parse_at_rule(
    input: &str,
    start: usize,
    base_offset: usize,
    depth: usize,
    budget: &mut Budget,
) -> Result<(Option<Rule>, usize)> {
    let mut cursor = start + 1;
    let name_start = cursor;
    while cursor < input.len() {
        let byte = input.as_bytes()[cursor];
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_') || byte >= 0x80 {
            cursor += 1;
        } else if byte == b'\\' {
            cursor = skip_escape(input, cursor);
        } else {
            break;
        }
    }
    if cursor == name_start {
        let next = recover_invalid_rule(input, cursor, base_offset)?;
        return Ok((None, next));
    }

    let raw_name = clean_component(&input[name_start..cursor], base_offset + name_start)?;
    let name = raw_name.to_ascii_lowercase();
    let Some((terminator, delimiter)) = scan_to_rule_delimiter(input, cursor, base_offset)? else {
        // A semicolon-less at-rule at EOF is syntactically complete enough to retain.
        let prelude = clean_component(&input[cursor..], base_offset + cursor)?;
        return Ok((
            Some(Rule::At(AtRule {
                name,
                prelude,
                block: None,
            })),
            input.len(),
        ));
    };
    let prelude = clean_component(&input[cursor..terminator], base_offset + cursor)?;
    if delimiter == b';' {
        return Ok((
            Some(Rule::At(AtRule {
                name,
                prelude,
                block: None,
            })),
            terminator + 1,
        ));
    }

    let (body, next) = extract_block(input, terminator, base_offset)?;
    let body_offset = base_offset + terminator + 1;
    let block = if is_group_at_rule(&name) {
        AtRuleBlock::Rules(parse_rule_list(body, body_offset, depth + 1, budget)?)
    } else if is_declaration_at_rule(&name) {
        AtRuleBlock::Declarations(parse_declarations(body, body_offset, budget)?)
    } else {
        AtRuleBlock::Raw(clean_component(body, body_offset)?)
    };
    Ok((
        Some(Rule::At(AtRule {
            name,
            prelude,
            block: Some(block),
        })),
        next,
    ))
}

fn is_group_at_rule(name: &str) -> bool {
    matches!(
        name,
        "media" | "supports" | "layer" | "container" | "scope" | "starting-style"
    )
}

fn is_declaration_at_rule(name: &str) -> bool {
    matches!(
        name,
        "page" | "counter-style" | "property" | "view-transition" | "font-face"
    )
}

fn recover_invalid_rule(input: &str, start: usize, base_offset: usize) -> Result<usize> {
    match scan_to_rule_delimiter(input, start, base_offset)? {
        Some((position, b';')) => Ok(position + 1),
        Some((position, b'{')) => extract_block(input, position, base_offset).map(|(_, next)| next),
        _ => Ok(input.len()),
    }
}

fn parse_declarations(
    input: &str,
    base_offset: usize,
    budget: &mut Budget,
) -> Result<DeclarationBlock> {
    let mut declarations = Vec::new();
    let mut segment_start = 0usize;
    let mut cursor = 0usize;
    let mut state = ScanState::default();

    while cursor <= input.len() {
        if cursor == input.len() {
            parse_declaration_segment(
                &input[segment_start..cursor],
                base_offset + segment_start,
                &mut declarations,
                budget,
            )?;
            break;
        }
        let byte = input.as_bytes()[cursor];
        if state.quote.is_some() {
            cursor = scan_quoted_byte(input, cursor, &mut state);
            continue;
        }
        if starts_comment(input, cursor) {
            cursor = skip_comment(input, cursor, base_offset)?;
            continue;
        }
        if byte == b'\\' {
            cursor = skip_escape(input, cursor);
            continue;
        }
        match byte {
            b'\'' | b'"' => {
                state.quote = Some(byte);
                cursor += 1;
            }
            b'(' => {
                state.paren = state.paren.saturating_add(1);
                cursor += 1;
            }
            b')' => {
                state.paren = state.paren.saturating_sub(1);
                cursor += 1;
            }
            b'[' => {
                state.bracket = state.bracket.saturating_add(1);
                cursor += 1;
            }
            b']' => {
                state.bracket = state.bracket.saturating_sub(1);
                cursor += 1;
            }
            b'{' => {
                state.brace = state.brace.saturating_add(1);
                cursor += 1;
            }
            b'}' => {
                state.brace = state.brace.saturating_sub(1);
                cursor += 1;
            }
            b';' if state.is_top_level() => {
                parse_declaration_segment(
                    &input[segment_start..cursor],
                    base_offset + segment_start,
                    &mut declarations,
                    budget,
                )?;
                cursor += 1;
                segment_start = cursor;
            }
            _ => cursor += 1,
        }
    }
    if state.quote.is_some() {
        return Err(Error::new(base_offset + input.len(), "unterminated string"));
    }
    Ok(DeclarationBlock { declarations })
}

fn parse_declaration_segment(
    segment: &str,
    base_offset: usize,
    declarations: &mut Vec<Declaration>,
    budget: &mut Budget,
) -> Result<()> {
    let Some(colon) = find_top_level_colon(segment, base_offset)? else {
        return Ok(());
    };
    let name = clean_component(&segment[..colon], base_offset)?;
    if name.is_empty() || name.starts_with('@') {
        return Ok(());
    }
    let value_offset = base_offset + colon + 1;
    let value = clean_component(&segment[colon + 1..], value_offset)?;
    let (value, important) = split_important(&value);
    if value.is_empty() {
        return Ok(());
    }
    budget.declarations = budget.declarations.saturating_add(1);
    if budget.declarations > MAX_DECLARATIONS {
        return Err(Error::new(base_offset, "declaration limit exceeded"));
    }
    declarations.push(Declaration {
        name,
        value,
        important,
    });
    Ok(())
}

fn find_top_level_colon(input: &str, base_offset: usize) -> Result<Option<usize>> {
    let mut cursor = 0usize;
    let mut state = ScanState::default();
    while cursor < input.len() {
        let byte = input.as_bytes()[cursor];
        if state.quote.is_some() {
            cursor = scan_quoted_byte(input, cursor, &mut state);
            continue;
        }
        if starts_comment(input, cursor) {
            cursor = skip_comment(input, cursor, base_offset)?;
            continue;
        }
        if byte == b'\\' {
            cursor = skip_escape(input, cursor);
            continue;
        }
        match byte {
            b'\'' | b'"' => state.quote = Some(byte),
            b'(' => state.paren = state.paren.saturating_add(1),
            b')' => state.paren = state.paren.saturating_sub(1),
            b'[' => state.bracket = state.bracket.saturating_add(1),
            b']' => state.bracket = state.bracket.saturating_sub(1),
            b'{' => state.brace = state.brace.saturating_add(1),
            b'}' => state.brace = state.brace.saturating_sub(1),
            b':' if state.is_top_level() => return Ok(Some(cursor)),
            _ => {}
        }
        cursor += 1;
    }
    Ok(None)
}

fn split_important(value: &str) -> (String, bool) {
    let mut cursor = 0usize;
    let mut state = ScanState::default();
    let mut last_bang = None;
    while cursor < value.len() {
        let byte = value.as_bytes()[cursor];
        if state.quote.is_some() {
            cursor = scan_quoted_byte(value, cursor, &mut state);
            continue;
        }
        if byte == b'\\' {
            cursor = skip_escape(value, cursor);
            continue;
        }
        match byte {
            b'\'' | b'"' => state.quote = Some(byte),
            b'(' => state.paren = state.paren.saturating_add(1),
            b')' => state.paren = state.paren.saturating_sub(1),
            b'[' => state.bracket = state.bracket.saturating_add(1),
            b']' => state.bracket = state.bracket.saturating_sub(1),
            b'{' => state.brace = state.brace.saturating_add(1),
            b'}' => state.brace = state.brace.saturating_sub(1),
            b'!' if state.is_top_level() => last_bang = Some(cursor),
            _ => {}
        }
        cursor += 1;
    }
    if let Some(bang) = last_bang {
        if value[bang + 1..].trim().eq_ignore_ascii_case("important") {
            return (value[..bang].trim_end().to_string(), true);
        }
    }
    (value.trim().to_string(), false)
}

/// Split a CSS component list without breaking strings, functions, attribute selectors, or blocks.
pub(crate) fn split_top_level(input: &str, delimiter: char) -> Result<Vec<String>> {
    ensure_input_bound(input)?;
    if !delimiter.is_ascii() {
        return Err(Error::new(0, "delimiter must be ASCII"));
    }
    let delimiter = delimiter as u8;
    let mut output = Vec::new();
    let mut start = 0usize;
    let mut cursor = 0usize;
    let mut state = ScanState::default();
    while cursor < input.len() {
        let byte = input.as_bytes()[cursor];
        if state.quote.is_some() {
            cursor = scan_quoted_byte(input, cursor, &mut state);
            continue;
        }
        if starts_comment(input, cursor) {
            cursor = skip_comment(input, cursor, 0)?;
            continue;
        }
        if byte == b'\\' {
            cursor = skip_escape(input, cursor);
            continue;
        }
        match byte {
            b'\'' | b'"' => state.quote = Some(byte),
            b'(' => state.paren = state.paren.saturating_add(1),
            b')' => state.paren = state.paren.saturating_sub(1),
            b'[' => state.bracket = state.bracket.saturating_add(1),
            b']' => state.bracket = state.bracket.saturating_sub(1),
            b'{' => state.brace = state.brace.saturating_add(1),
            b'}' => state.brace = state.brace.saturating_sub(1),
            _ if byte == delimiter && state.is_top_level() => {
                output.push(clean_component(&input[start..cursor], start)?);
                start = cursor + 1;
            }
            _ => {}
        }
        cursor += 1;
    }
    if state.quote.is_some() {
        return Err(Error::new(input.len(), "unterminated string"));
    }
    output.push(clean_component(&input[start..], start)?);
    Ok(output)
}

/// Resolve a context-free CSS absolute length to CSS pixels.
///
/// Relative units, percentages, math functions, and non-zero unitless numbers intentionally return
/// `None`; their meaning depends on computed-style context. Unitless zero is accepted as CSS does.
pub(crate) fn parse_absolute_length_px(input: &str) -> Option<f32> {
    let input = input.trim();
    if input.is_empty() {
        return None;
    }
    let number_end = css_number_end(input)?;
    let number = input[..number_end].parse::<f64>().ok()?;
    if !number.is_finite() {
        return None;
    }
    let unit = input[number_end..].trim().to_ascii_lowercase();
    let pixels = match unit.as_str() {
        "" if number == 0.0 => 0.0,
        "px" => number,
        "in" => number * 96.0,
        "cm" => number * (96.0 / 2.54),
        "mm" => number * (96.0 / 25.4),
        "q" => number * (96.0 / 101.6),
        "pt" => number * (96.0 / 72.0),
        "pc" => number * 16.0,
        _ => return None,
    };
    (pixels.is_finite() && pixels >= f32::MIN as f64 && pixels <= f32::MAX as f64)
        .then_some(pixels as f32)
}

/// Parse a CSS value containing only string and identifier tokens.
///
/// This is the `<symbol>#` subset used by counter styles. Images, functions, numbers, and
/// punctuation are rejected rather than being flattened into misleading marker text.
pub(crate) fn parse_string_or_ident_list(input: &str) -> Option<Vec<String>> {
    parse_text_tokens(input, true)
}

pub(crate) fn parse_identifier(input: &str) -> Option<String> {
    let tokens = parse_text_tokens(input, false)?;
    (tokens.len() == 1).then(|| tokens[0].clone())
}

/// Tokenize the bounded component-value subset used by native property grammars.
///
/// Functions retain their argument source so consumers can apply property-specific comma and
/// slash grammars without losing whitespace or nested function structure. Identifiers and strings
/// are CSS-unescaped here, at the syntax boundary.
pub(crate) fn tokenize_component_values(input: &str) -> Option<Vec<ComponentValue>> {
    let mut values = Vec::new();
    let mut cursor = 0usize;
    while cursor < input.len() {
        cursor = skip_css_whitespace(input, cursor);
        if cursor >= input.len() {
            break;
        }
        let byte = input.as_bytes()[cursor];
        if matches!(byte, b'\'' | b'"') {
            let (value, next) = parse_css_string_at(input, cursor)?;
            values.push(ComponentValue::String(value));
            cursor = next;
            continue;
        }
        if let Some(number_end) = css_number_end(&input[cursor..]) {
            values.push(ComponentValue::Number(
                input[cursor..cursor + number_end].to_string(),
            ));
            cursor += number_end;
            continue;
        }
        if could_start_identifier(input, cursor) {
            let (name, next) = parse_identifier_at(input, cursor)?;
            cursor = next;
            if input.as_bytes().get(cursor) == Some(&b'(') {
                let (arguments, next) = parse_function_arguments_at(input, cursor)?;
                values.push(ComponentValue::Function {
                    name,
                    arguments: arguments.to_string(),
                });
                cursor = next;
            } else {
                values.push(ComponentValue::Ident(name));
            }
            continue;
        }
        let ch = input[cursor..].chars().next()?;
        values.push(ComponentValue::Delim(ch));
        cursor += ch.len_utf8();
    }
    Some(values)
}

fn parse_css_string_at(input: &str, mut cursor: usize) -> Option<(String, usize)> {
    let quote = *input.as_bytes().get(cursor)?;
    if !matches!(quote, b'\'' | b'"') {
        return None;
    }
    cursor += 1;
    let mut value = String::new();
    while cursor < input.len() {
        let byte = input.as_bytes()[cursor];
        if byte == quote {
            return Some((value, cursor + 1));
        }
        if matches!(byte, b'\n' | b'\r' | 0x0c) {
            return None;
        }
        if byte == b'\\' {
            if let Some(ch) = decode_css_escape(input, &mut cursor)? {
                value.push(ch);
            }
            continue;
        }
        let ch = input[cursor..].chars().next()?;
        value.push(ch);
        cursor += ch.len_utf8();
    }
    None
}

fn could_start_identifier(input: &str, cursor: usize) -> bool {
    let Some(ch) = input[cursor..].chars().next() else {
        return false;
    };
    if ch == '\\' || ch == '_' || ch.is_ascii_alphabetic() || !ch.is_ascii() {
        return true;
    }
    if ch != '-' {
        return false;
    }
    let next_cursor = cursor + 1;
    let Some(next) = input
        .get(next_cursor..)
        .and_then(|tail| tail.chars().next())
    else {
        return true;
    };
    next == '-' || next == '\\' || next == '_' || next.is_ascii_alphabetic() || !next.is_ascii()
}

fn parse_identifier_at(input: &str, mut cursor: usize) -> Option<(String, usize)> {
    if !could_start_identifier(input, cursor) {
        return None;
    }
    let mut value = String::new();
    while cursor < input.len() {
        let byte = input.as_bytes()[cursor];
        if byte == b'\\' {
            if let Some(ch) = decode_css_escape(input, &mut cursor)? {
                value.push(ch);
            }
            continue;
        }
        let ch = input[cursor..].chars().next()?;
        if !(ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') || !ch.is_ascii()) {
            break;
        }
        value.push(ch);
        cursor += ch.len_utf8();
    }
    (!value.is_empty()).then_some((value, cursor))
}

fn parse_function_arguments_at(input: &str, open: usize) -> Option<(&str, usize)> {
    if input.as_bytes().get(open) != Some(&b'(') {
        return None;
    }
    let mut cursor = open + 1;
    let start = cursor;
    let mut depth = 1usize;
    let mut quote: Option<u8> = None;
    while cursor < input.len() {
        let byte = input.as_bytes()[cursor];
        if let Some(active_quote) = quote {
            if byte == b'\\' {
                cursor = skip_escape(input, cursor);
            } else {
                cursor += 1;
                if byte == active_quote {
                    quote = None;
                }
            }
            continue;
        }
        if starts_comment(input, cursor) {
            cursor = skip_comment(input, cursor, 0).ok()?;
            continue;
        }
        match byte {
            b'\'' | b'"' => {
                quote = Some(byte);
                cursor += 1;
            }
            b'\\' => cursor = skip_escape(input, cursor),
            b'(' => {
                depth = depth.checked_add(1)?;
                cursor += 1;
            }
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some((&input[start..cursor], cursor + 1));
                }
                cursor += 1;
            }
            _ => cursor += input[cursor..].chars().next()?.len_utf8(),
        }
    }
    None
}

fn parse_text_tokens(input: &str, allow_strings: bool) -> Option<Vec<String>> {
    let mut tokens = Vec::new();
    let mut cursor = 0usize;
    while cursor < input.len() {
        cursor = skip_css_whitespace(input, cursor);
        if cursor >= input.len() {
            break;
        }
        let first = input.as_bytes()[cursor];
        if matches!(first, b'\'' | b'"') {
            if !allow_strings {
                return None;
            }
            let quote = first;
            cursor += 1;
            let mut value = String::new();
            let mut closed = false;
            while cursor < input.len() {
                let byte = input.as_bytes()[cursor];
                if byte == quote {
                    cursor += 1;
                    closed = true;
                    break;
                }
                if matches!(byte, b'\n' | b'\r' | 0x0c) {
                    return None;
                }
                if byte == b'\\' {
                    if let Some(ch) = decode_css_escape(input, &mut cursor)? {
                        value.push(ch);
                    }
                    continue;
                }
                let ch = input[cursor..].chars().next()?;
                value.push(ch);
                cursor += ch.len_utf8();
            }
            if !closed {
                return None;
            }
            tokens.push(value);
            continue;
        }

        let mut value = String::new();
        let mut first_char = true;
        while cursor < input.len() {
            let byte = input.as_bytes()[cursor];
            if byte.is_ascii_whitespace() {
                break;
            }
            if byte == b'\\' {
                if let Some(ch) = decode_css_escape(input, &mut cursor)? {
                    value.push(ch);
                    first_char = false;
                }
                continue;
            }
            let ch = input[cursor..].chars().next()?;
            if !(ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') || !ch.is_ascii())
                || (first_char && ch.is_ascii_digit())
                || (value == "-" && ch.is_ascii_digit())
            {
                return None;
            }
            value.push(ch);
            first_char = false;
            cursor += ch.len_utf8();
        }
        if value.is_empty() {
            return None;
        }
        tokens.push(value);
    }
    (!tokens.is_empty()).then_some(tokens)
}

fn skip_css_whitespace(input: &str, mut cursor: usize) -> usize {
    while input
        .as_bytes()
        .get(cursor)
        .is_some_and(u8::is_ascii_whitespace)
    {
        cursor += 1;
    }
    cursor
}

fn decode_css_escape(input: &str, cursor: &mut usize) -> Option<Option<char>> {
    debug_assert_eq!(input.as_bytes().get(*cursor), Some(&b'\\'));
    *cursor += 1;
    if *cursor >= input.len() {
        return Some(Some('\u{fffd}'));
    }
    match input.as_bytes()[*cursor] {
        b'\n' | 0x0c => {
            *cursor += 1;
            return Some(None);
        }
        b'\r' => {
            *cursor += 1;
            if input.as_bytes().get(*cursor) == Some(&b'\n') {
                *cursor += 1;
            }
            return Some(None);
        }
        _ => {}
    }
    let start = *cursor;
    let mut value = 0u32;
    let mut digits = 0usize;
    while digits < 6 {
        let Some(byte) = input.as_bytes().get(*cursor).copied() else {
            break;
        };
        let digit = match byte {
            b'0'..=b'9' => u32::from(byte - b'0'),
            b'a'..=b'f' => u32::from(byte - b'a' + 10),
            b'A'..=b'F' => u32::from(byte - b'A' + 10),
            _ => break,
        };
        value = value.saturating_mul(16).saturating_add(digit);
        *cursor += 1;
        digits += 1;
    }
    if *cursor > start {
        if let Some(byte) = input.as_bytes().get(*cursor).copied() {
            match byte {
                b'\r' => {
                    *cursor += 1;
                    if input.as_bytes().get(*cursor) == Some(&b'\n') {
                        *cursor += 1;
                    }
                }
                b'\n' | b'\t' | b' ' | 0x0c => *cursor += 1,
                _ => {}
            }
        }
        let decoded = char::from_u32(value)
            .filter(|ch| *ch != '\0' && !matches!(value, 0xd800..=0xdfff))
            .unwrap_or('\u{fffd}');
        return Some(Some(decoded));
    }
    let ch = input[*cursor..].chars().next()?;
    *cursor += ch.len_utf8();
    Some(Some(ch))
}

fn css_number_end(input: &str) -> Option<usize> {
    let bytes = input.as_bytes();
    let mut cursor = 0usize;
    if matches!(bytes.first(), Some(b'+') | Some(b'-')) {
        cursor += 1;
    }
    let integer_start = cursor;
    while cursor < bytes.len() && bytes[cursor].is_ascii_digit() {
        cursor += 1;
    }
    let mut digits = cursor - integer_start;
    if bytes.get(cursor) == Some(&b'.') {
        cursor += 1;
        let fraction_start = cursor;
        while cursor < bytes.len() && bytes[cursor].is_ascii_digit() {
            cursor += 1;
        }
        digits += cursor - fraction_start;
    }
    if digits == 0 {
        return None;
    }
    if matches!(bytes.get(cursor), Some(b'e') | Some(b'E')) {
        let exponent_marker = cursor;
        cursor += 1;
        if matches!(bytes.get(cursor), Some(b'+') | Some(b'-')) {
            cursor += 1;
        }
        let exponent_start = cursor;
        while cursor < bytes.len() && bytes[cursor].is_ascii_digit() {
            cursor += 1;
        }
        if cursor == exponent_start {
            cursor = exponent_marker;
        }
    }
    Some(cursor)
}

fn scan_to_rule_delimiter(
    input: &str,
    start: usize,
    base_offset: usize,
) -> Result<Option<(usize, u8)>> {
    let mut cursor = start;
    let mut state = ScanState::default();
    while cursor < input.len() {
        let byte = input.as_bytes()[cursor];
        if state.quote.is_some() {
            cursor = scan_quoted_byte(input, cursor, &mut state);
            continue;
        }
        if starts_comment(input, cursor) {
            cursor = skip_comment(input, cursor, base_offset)?;
            continue;
        }
        if byte == b'\\' {
            cursor = skip_escape(input, cursor);
            continue;
        }
        match byte {
            b'\'' | b'"' => state.quote = Some(byte),
            b'(' => state.paren = state.paren.saturating_add(1),
            b')' => state.paren = state.paren.saturating_sub(1),
            b'[' => state.bracket = state.bracket.saturating_add(1),
            b']' => state.bracket = state.bracket.saturating_sub(1),
            b'{' if state.paren == 0 && state.bracket == 0 => return Ok(Some((cursor, b'{'))),
            b';' if state.paren == 0 && state.bracket == 0 => return Ok(Some((cursor, b';'))),
            _ => {}
        }
        cursor += 1;
    }
    if state.quote.is_some() {
        return Err(Error::new(base_offset + input.len(), "unterminated string"));
    }
    Ok(None)
}

fn extract_block<'a>(input: &'a str, open: usize, base_offset: usize) -> Result<(&'a str, usize)> {
    debug_assert_eq!(input.as_bytes().get(open), Some(&b'{'));
    let mut cursor = open + 1;
    let mut depth = 1usize;
    let mut quote = None;
    while cursor < input.len() {
        let byte = input.as_bytes()[cursor];
        if quote.is_some() {
            let mut state = ScanState {
                quote,
                ..ScanState::default()
            };
            cursor = scan_quoted_byte(input, cursor, &mut state);
            quote = state.quote;
            continue;
        }
        if starts_comment(input, cursor) {
            cursor = skip_comment(input, cursor, base_offset)?;
            continue;
        }
        if byte == b'\\' {
            cursor = skip_escape(input, cursor);
            continue;
        }
        match byte {
            b'\'' | b'"' => {
                quote = Some(byte);
                cursor += 1;
            }
            b'{' => {
                depth = depth.saturating_add(1);
                cursor += 1;
            }
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Ok((&input[open + 1..cursor], cursor + 1));
                }
                cursor += 1;
            }
            _ => cursor += 1,
        }
    }
    Err(Error::new(base_offset + open, "unterminated block"))
}

fn clean_component(input: &str, base_offset: usize) -> Result<String> {
    let mut output = String::with_capacity(input.len());
    let mut cursor = 0usize;
    let mut quote = None;
    while cursor < input.len() {
        let byte = input.as_bytes()[cursor];
        if let Some(active_quote) = quote {
            let character = input[cursor..].chars().next().expect("in bounds");
            output.push(character);
            cursor += character.len_utf8();
            if byte == b'\\' && cursor < input.len() {
                let next = input[cursor..].chars().next().expect("in bounds");
                output.push(next);
                cursor += next.len_utf8();
            } else if byte == active_quote {
                quote = None;
            }
            continue;
        }
        if starts_comment(input, cursor) {
            let next = skip_comment(input, cursor, base_offset)?;
            if output.chars().last().is_some_and(|ch| !ch.is_whitespace())
                && input[next..]
                    .chars()
                    .next()
                    .is_some_and(|next_char| !next_char.is_whitespace())
            {
                output.push(' ');
            }
            cursor = next;
            continue;
        }
        let character = input[cursor..].chars().next().expect("in bounds");
        if matches!(character, '\'' | '"') {
            quote = Some(character as u8);
        }
        output.push(character);
        cursor += character.len_utf8();
    }
    if quote.is_some() {
        return Err(Error::new(base_offset + input.len(), "unterminated string"));
    }
    Ok(output.trim().to_string())
}

fn skip_whitespace_and_comments(
    input: &str,
    mut position: usize,
    base_offset: usize,
) -> Result<usize> {
    loop {
        while position < input.len() && input.as_bytes()[position].is_ascii_whitespace() {
            position += 1;
        }
        if starts_comment(input, position) {
            position = skip_comment(input, position, base_offset)?;
            continue;
        }
        return Ok(position);
    }
}

fn starts_comment(input: &str, position: usize) -> bool {
    input.as_bytes().get(position) == Some(&b'/')
        && input.as_bytes().get(position + 1) == Some(&b'*')
}

fn skip_comment(input: &str, position: usize, base_offset: usize) -> Result<usize> {
    let mut cursor = position + 2;
    while cursor + 1 < input.len() {
        if input.as_bytes()[cursor] == b'*' && input.as_bytes()[cursor + 1] == b'/' {
            return Ok(cursor + 2);
        }
        cursor += 1;
    }
    Err(Error::new(base_offset + position, "unterminated comment"))
}

fn skip_escape(input: &str, position: usize) -> usize {
    let mut cursor = position + 1;
    if cursor >= input.len() {
        return input.len();
    }
    let first = input.as_bytes()[cursor];
    if first.is_ascii_hexdigit() {
        let mut digits = 0usize;
        while cursor < input.len() && input.as_bytes()[cursor].is_ascii_hexdigit() && digits < 6 {
            cursor += 1;
            digits += 1;
        }
        if cursor < input.len() && input.as_bytes()[cursor].is_ascii_whitespace() {
            if input.as_bytes()[cursor] == b'\r' && input.as_bytes().get(cursor + 1) == Some(&b'\n')
            {
                return cursor + 2;
            }
            return cursor + 1;
        }
        return cursor;
    }
    let character = input[cursor..].chars().next().expect("in bounds");
    cursor + character.len_utf8()
}

fn scan_quoted_byte(input: &str, position: usize, state: &mut ScanState) -> usize {
    let byte = input.as_bytes()[position];
    if byte == b'\\' {
        return skip_escape(input, position);
    }
    if state.quote == Some(byte) {
        state.quote = None;
    }
    position + 1
}

#[derive(Default)]
struct ScanState {
    quote: Option<u8>,
    paren: usize,
    bracket: usize,
    brace: usize,
}

impl ScanState {
    fn is_top_level(&self) -> bool {
        self.quote.is_none() && self.paren == 0 && self.bracket == 0 && self.brace == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declarations_preserve_nested_delimiters_and_split_important() {
        let block = parse_declaration_block(
            r#"background:url("data:image/svg+xml;a:b;c");content:"x:y;z";color:red !/**/ IMPORTANT;--payload:{a:b;c:d}"#,
        )
        .expect("declarations");
        assert_eq!(block.declarations.len(), 4);
        assert_eq!(
            block.declarations[0].value,
            r#"url("data:image/svg+xml;a:b;c")"#
        );
        assert_eq!(block.declarations[1].value, r#""x:y;z""#);
        assert_eq!(block.declarations[2].value, "red");
        assert!(block.declarations[2].important);
        assert_eq!(block.declarations[3].value, "{a:b;c:d}");
    }

    #[test]
    fn stylesheet_retains_nested_group_and_descriptor_at_rules() {
        let sheet = parse_stylesheet(
            r#"
                @layer reset, theme;
                @media print and (min-width: 1px) {
                    h1, :is(.a, .b) { color: red; width: calc(100% - 2px) }
                    @supports (display: grid) { .grid { display: grid } }
                }
                @property --gap { syntax: "<length>"; inherits: false; initial-value: 1px }
            "#,
        )
        .expect("stylesheet");
        assert_eq!(sheet.rules.len(), 3);
        let Rule::At(media) = &sheet.rules[1] else {
            panic!("expected media rule");
        };
        assert_eq!(media.name, "media");
        let Some(AtRuleBlock::Rules(nested)) = &media.block else {
            panic!("expected nested rules");
        };
        assert_eq!(nested.len(), 2);
        let Rule::Style(style) = &nested[0] else {
            panic!("expected style rule");
        };
        assert_eq!(
            split_top_level(&style.selectors, ',').expect("selectors"),
            vec!["h1", ":is(.a, .b)"]
        );
        let Rule::At(property) = &sheet.rules[2] else {
            panic!("expected property rule");
        };
        let Some(AtRuleBlock::Declarations(descriptors)) = &property.block else {
            panic!("expected descriptors");
        };
        assert_eq!(descriptors.declarations.len(), 3);
    }

    #[test]
    fn comments_are_whitespace_but_strings_are_opaque() {
        let block =
            parse_declaration_block(r#"font-family: Alpha/**/Beta; content: "/*not a comment*/""#)
                .expect("declarations");
        assert_eq!(block.declarations[0].value, "Alpha Beta");
        assert_eq!(block.declarations[1].value, r#""/*not a comment*/""#);
    }

    #[test]
    fn malformed_unclosed_constructs_are_rejected() {
        assert!(parse_stylesheet("a { color: red").is_err());
        assert!(parse_stylesheet("/* never closed").is_err());
        assert!(parse_declaration_block("content: \"never closed").is_err());
    }

    #[test]
    fn declaration_serialization_is_stable() {
        let block =
            parse_declaration_block("color: red; width: 2px !important").expect("declarations");
        assert_eq!(block.normal().count(), 1);
        assert_eq!(block.important().count(), 1);
        assert_eq!(block.to_css_string(), "color:red;width:2px!important");
        assert!(block.declarations[0].name_eq("COLOR"));
    }

    #[test]
    fn absolute_lengths_follow_css_reference_pixel_ratios() {
        assert_eq!(parse_absolute_length_px("96px"), Some(96.0));
        assert_eq!(parse_absolute_length_px("1in"), Some(96.0));
        assert_eq!(parse_absolute_length_px("72pt"), Some(96.0));
        assert_eq!(parse_absolute_length_px("6pc"), Some(96.0));
        assert_eq!(parse_absolute_length_px("101.6q"), Some(96.0));
        assert_eq!(parse_absolute_length_px("0"), Some(0.0));
        assert_eq!(parse_absolute_length_px("1e2px"), Some(100.0));
        assert_eq!(parse_absolute_length_px("10%"), None);
        assert_eq!(parse_absolute_length_px("1em"), None);
        assert_eq!(parse_absolute_length_px("2"), None);
    }

    #[test]
    fn string_and_identifier_tokens_decode_css_escapes() {
        assert_eq!(
            parse_string_or_ident_list(r#""A\41" beta \67 amma"#),
            Some(vec![
                "AA".to_string(),
                "beta".to_string(),
                "gamma".to_string()
            ])
        );
        assert_eq!(
            parse_identifier(r"space\2d counter"),
            Some("space-counter".into())
        );
        assert!(parse_string_or_ident_list("url(marker.svg)").is_none());
        assert!(parse_string_or_ident_list("12").is_none());
        assert!(parse_identifier(r#""quoted""#).is_none());
    }

    #[test]
    fn component_values_preserve_nested_syntax_and_decode_tokens() {
        assert_eq!(
            tokenize_component_values(
                r#"\66 oo c\61 lc(1px, var(--x, "a,b")) "A\41" -1.25e+2px / #"#,
            ),
            Some(vec![
                ComponentValue::Ident("foo".into()),
                ComponentValue::Function {
                    name: "calc".into(),
                    arguments: r#"1px, var(--x, "a,b")"#.into(),
                },
                ComponentValue::String("AA".into()),
                ComponentValue::Number("-1.25e+2".into()),
                ComponentValue::Ident("px".into()),
                ComponentValue::Delim('/'),
                ComponentValue::Delim('#'),
            ])
        );
        assert!(tokenize_component_values("calc(1px + var(--x)").is_none());
        assert!(tokenize_component_values(r#""unterminated"#).is_none());
    }
}
