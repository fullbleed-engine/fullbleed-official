//! Dependency-free CSS conditional-query parsing and evaluation.
//!
//! The stylesheet parser owns block and declaration syntax. This module handles the smaller
//! grammars embedded in conditional-rule preludes while preserving an explicit unsupported state.

use crate::css_native::{parse_absolute_length_px, split_top_level};
use crate::types::{Pt, Size};

const MAX_CONDITION_DEPTH: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Comparison {
    Equal,
    GreaterThan,
    GreaterThanEqual,
    LessThan,
    LessThanEqual,
}

impl Comparison {
    pub(crate) fn reverse(self) -> Self {
        match self {
            Self::Equal => Self::Equal,
            Self::GreaterThan => Self::LessThan,
            Self::GreaterThanEqual => Self::LessThanEqual,
            Self::LessThan => Self::GreaterThan,
            Self::LessThanEqual => Self::GreaterThanEqual,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct MediaMatch {
    pub(crate) matched: bool,
    pub(crate) matched_queries: u64,
    pub(crate) unmatched_queries: u64,
    pub(crate) unsupported_queries: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ScopePrelude {
    pub(crate) starts: Vec<String>,
    pub(crate) ends: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ContainerPrelude {
    pub(crate) name: Option<String>,
    pub(crate) condition: Option<ContainerCondition>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ContainerCondition {
    Feature(ContainerFeature),
    Not(Box<ContainerCondition>),
    And(Vec<ContainerCondition>),
    Or(Vec<ContainerCondition>),
    Unsupported,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ContainerFeature {
    Plain {
        name: ContainerFeatureName,
        value: ContainerValue,
    },
    Range {
        name: ContainerFeatureName,
        operator: Comparison,
        value: ContainerValue,
    },
    Interval {
        name: ContainerFeatureName,
        start: ContainerValue,
        start_operator: Comparison,
        end: ContainerValue,
        end_operator: Comparison,
    },
    Boolean,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ContainerFeatureName {
    Width,
    Height,
    InlineSize,
    BlockSize,
    AspectRatio,
    Orientation,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ContainerValue {
    Length(Pt),
    Ratio(f32),
    Ident(String),
}

pub(crate) fn evaluate_media_list(prelude: &str, viewport: Size, prefer_print: bool) -> MediaMatch {
    if viewport.width == Pt::ZERO && viewport.height == Pt::ZERO {
        return MediaMatch {
            matched: true,
            ..MediaMatch::default()
        };
    }
    let Ok(queries) = split_top_level(prelude, ',') else {
        return MediaMatch {
            unsupported_queries: 1,
            ..MediaMatch::default()
        };
    };
    if queries.iter().all(|query| query.trim().is_empty()) {
        return MediaMatch {
            matched: true,
            ..MediaMatch::default()
        };
    }

    let mut result = MediaMatch::default();
    for query in queries {
        if query.trim().is_empty() {
            result.unsupported_queries = result.unsupported_queries.saturating_add(1);
            continue;
        }
        match evaluate_media_query(&query, viewport, prefer_print) {
            Some(true) => {
                result.matched = true;
                result.matched_queries = result.matched_queries.saturating_add(1);
                break;
            }
            Some(false) => {
                result.unmatched_queries = result.unmatched_queries.saturating_add(1);
            }
            None => {
                result.unsupported_queries = result.unsupported_queries.saturating_add(1);
            }
        }
    }
    result
}

pub(crate) fn media_list_has_print_type(prelude: &str) -> bool {
    let Ok(queries) = split_top_level(prelude, ',') else {
        return false;
    };
    queries.into_iter().any(|query| {
        let mut raw = query.trim();
        if let Some(rest) = strip_leading_word(raw, "not") {
            if !rest.trim_start().starts_with('(') {
                raw = rest;
            }
        } else if let Some(rest) = strip_leading_word(raw, "only") {
            raw = rest;
        }
        leading_word(raw)
            .map(|word| word.eq_ignore_ascii_case("print"))
            .unwrap_or(false)
    })
}

pub(crate) fn evaluate_supports_condition<D, S>(
    condition: &str,
    mut declaration_supported: D,
    mut selector_supported: S,
) -> bool
where
    D: FnMut(&str, &str) -> bool,
    S: FnMut(&str) -> bool,
{
    evaluate_supports_inner(
        condition,
        0,
        &mut declaration_supported,
        &mut selector_supported,
    )
    .unwrap_or(false)
}

pub(crate) fn parse_scope_prelude(prelude: &str) -> Option<ScopePrelude> {
    let parts = split_top_level_keyword(prelude.trim(), "to")?;
    if parts.is_empty() || parts.len() > 2 {
        return None;
    }
    let starts_raw = strip_entire_parentheses(parts[0])?;
    let starts = split_top_level(starts_raw, ',')
        .ok()?
        .into_iter()
        .filter(|selector| !selector.trim().is_empty())
        .collect::<Vec<_>>();
    if starts.is_empty() {
        return None;
    }
    let ends = if parts.len() == 2 {
        let ends_raw = strip_entire_parentheses(parts[1])?;
        let selectors = split_top_level(ends_raw, ',')
            .ok()?
            .into_iter()
            .filter(|selector| !selector.trim().is_empty())
            .collect::<Vec<_>>();
        if selectors.is_empty() {
            return None;
        }
        selectors
    } else {
        Vec::new()
    };
    Some(ScopePrelude { starts, ends })
}

pub(crate) fn parse_container_prelude(prelude: &str) -> Option<ContainerPrelude> {
    let raw = prelude.trim();
    if raw.is_empty() {
        return None;
    }
    let (name, condition_raw) = if raw.starts_with('(') || starts_with_word(raw, "not") {
        (None, raw)
    } else {
        let name_raw = leading_word(raw)?;
        let rest = raw[name_raw.len()..].trim_start();
        (Some(name_raw.to_ascii_lowercase()), rest)
    };
    if condition_raw.is_empty() {
        return name.map(|name| ContainerPrelude {
            name: Some(name),
            condition: None,
        });
    }
    Some(ContainerPrelude {
        name,
        condition: Some(parse_container_condition(condition_raw, 0)?),
    })
}

fn parse_container_condition(raw: &str, depth: usize) -> Option<ContainerCondition> {
    if depth > MAX_CONDITION_DEPTH {
        return None;
    }
    let raw = raw.trim();
    let or_items = split_top_level_keyword(raw, "or")?;
    if or_items.len() > 1 {
        return Some(ContainerCondition::Or(
            or_items
                .into_iter()
                .map(|item| parse_container_condition(item, depth + 1))
                .collect::<Option<Vec<_>>>()?,
        ));
    }
    let and_items = split_top_level_keyword(raw, "and")?;
    if and_items.len() > 1 {
        return Some(ContainerCondition::And(
            and_items
                .into_iter()
                .map(|item| parse_container_condition(item, depth + 1))
                .collect::<Option<Vec<_>>>()?,
        ));
    }
    if let Some(rest) = strip_leading_word(raw, "not") {
        return Some(ContainerCondition::Not(Box::new(
            parse_container_condition(rest, depth + 1)?,
        )));
    }
    if strip_entire_function(raw, "style").is_some()
        || strip_entire_function(raw, "scroll-state").is_some()
    {
        return Some(ContainerCondition::Unsupported);
    }
    let inner = strip_entire_parentheses(raw)?.trim();
    if has_top_level_keyword(inner, "and")?
        || has_top_level_keyword(inner, "or")?
        || starts_with_word(inner, "not")
        || strip_entire_parentheses(inner).is_some()
    {
        return parse_container_condition(inner, depth + 1);
    }
    Some(ContainerCondition::Feature(parse_container_feature(inner)?))
}

fn parse_container_feature(raw: &str) -> Option<ContainerFeature> {
    if let Some(colon) = find_top_level_byte(raw, b':')? {
        let mut name_raw = raw[..colon].trim().to_ascii_lowercase();
        let value_raw = raw[colon + 1..].trim();
        let operator = if let Some(base) = name_raw.strip_prefix("min-") {
            name_raw = base.to_string();
            Some(Comparison::GreaterThanEqual)
        } else if let Some(base) = name_raw.strip_prefix("max-") {
            name_raw = base.to_string();
            Some(Comparison::LessThanEqual)
        } else {
            None
        };
        let name = parse_container_feature_name(&name_raw)?;
        let value = parse_container_value(name, value_raw)?;
        return Some(if let Some(operator) = operator {
            ContainerFeature::Range {
                name,
                operator,
                value,
            }
        } else {
            ContainerFeature::Plain { name, value }
        });
    }

    let comparisons = find_top_level_comparisons(raw)?;
    match comparisons.as_slice() {
        [] => {
            parse_container_feature_name(raw.trim())?;
            Some(ContainerFeature::Boolean)
        }
        [(position, operator, width)] => {
            let left = raw[..*position].trim();
            let right = raw[position + width..].trim();
            if let Some(name) = parse_container_feature_name(left) {
                Some(ContainerFeature::Range {
                    name,
                    operator: *operator,
                    value: parse_container_value(name, right)?,
                })
            } else {
                let name = parse_container_feature_name(right)?;
                Some(ContainerFeature::Range {
                    name,
                    operator: operator.reverse(),
                    value: parse_container_value(name, left)?,
                })
            }
        }
        [
            (first, first_operator, first_width),
            (second, second_operator, second_width),
        ] => {
            let start_raw = raw[..*first].trim();
            let name_raw = raw[first + first_width..*second].trim();
            let end_raw = raw[second + second_width..].trim();
            let name = parse_container_feature_name(name_raw)?;
            Some(ContainerFeature::Interval {
                name,
                start: parse_container_value(name, start_raw)?,
                start_operator: *first_operator,
                end: parse_container_value(name, end_raw)?,
                end_operator: *second_operator,
            })
        }
        _ => None,
    }
}

fn parse_container_feature_name(raw: &str) -> Option<ContainerFeatureName> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "width" => Some(ContainerFeatureName::Width),
        "height" => Some(ContainerFeatureName::Height),
        "inline-size" => Some(ContainerFeatureName::InlineSize),
        "block-size" => Some(ContainerFeatureName::BlockSize),
        "aspect-ratio" => Some(ContainerFeatureName::AspectRatio),
        "orientation" => Some(ContainerFeatureName::Orientation),
        _ => None,
    }
}

fn parse_container_value(name: ContainerFeatureName, raw: &str) -> Option<ContainerValue> {
    match name {
        ContainerFeatureName::Width
        | ContainerFeatureName::Height
        | ContainerFeatureName::InlineSize
        | ContainerFeatureName::BlockSize => {
            parse_absolute_length_px(raw).map(|px| ContainerValue::Length(Pt::from_f32(px * 0.75)))
        }
        ContainerFeatureName::AspectRatio => parse_ratio(raw).map(ContainerValue::Ratio),
        ContainerFeatureName::Orientation => {
            let value = raw.trim().to_ascii_lowercase();
            matches!(value.as_str(), "portrait" | "landscape")
                .then_some(ContainerValue::Ident(value))
        }
    }
}

fn evaluate_supports_inner<D, S>(
    raw: &str,
    depth: usize,
    declaration_supported: &mut D,
    selector_supported: &mut S,
) -> Option<bool>
where
    D: FnMut(&str, &str) -> bool,
    S: FnMut(&str) -> bool,
{
    if depth > MAX_CONDITION_DEPTH {
        return None;
    }
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }

    let or_items = split_top_level_keyword(raw, "or")?;
    if or_items.len() > 1 {
        let mut matched = false;
        for item in or_items {
            matched |= evaluate_supports_inner(
                item,
                depth + 1,
                declaration_supported,
                selector_supported,
            )?;
        }
        return Some(matched);
    }
    let and_items = split_top_level_keyword(raw, "and")?;
    if and_items.len() > 1 {
        for item in and_items {
            if !evaluate_supports_inner(item, depth + 1, declaration_supported, selector_supported)?
            {
                return Some(false);
            }
        }
        return Some(true);
    }
    if let Some(rest) = strip_leading_word(raw, "not") {
        return evaluate_supports_inner(rest, depth + 1, declaration_supported, selector_supported)
            .map(|matched| !matched);
    }
    if let Some(selector) = strip_entire_function(raw, "selector") {
        return Some(selector_supported(selector.trim()));
    }
    if let Some(inner) = strip_entire_parentheses(raw) {
        let inner = inner.trim();
        if has_top_level_keyword(inner, "and")?
            || has_top_level_keyword(inner, "or")?
            || starts_with_word(inner, "not")
            || strip_entire_parentheses(inner).is_some()
            || strip_entire_function(inner, "selector").is_some()
        {
            return evaluate_supports_inner(
                inner,
                depth + 1,
                declaration_supported,
                selector_supported,
            );
        }
        let colon = find_top_level_byte(inner, b':')??;
        let name = inner[..colon].trim();
        let value = inner[colon + 1..].trim();
        if name.is_empty() || value.is_empty() {
            return None;
        }
        return Some(declaration_supported(name, value));
    }
    None
}

fn evaluate_media_query(query: &str, viewport: Size, prefer_print: bool) -> Option<bool> {
    let mut raw = query.trim();
    let mut negate_query = false;
    let mut only = false;
    if let Some(rest) = strip_leading_word(raw, "not") {
        if !rest.trim_start().starts_with('(') {
            negate_query = true;
            raw = rest.trim_start();
        }
    } else if let Some(rest) = strip_leading_word(raw, "only") {
        only = true;
        raw = rest.trim_start();
    }

    let mut media_type = "all";
    if !raw.starts_with('(') && !starts_with_word(raw, "not") {
        let word = leading_word(raw)?;
        media_type = word;
        raw = raw[word.len()..].trim_start();
        if !raw.is_empty() {
            raw = strip_leading_word(raw, "and")?.trim_start();
        }
    } else if only {
        return None;
    }

    let type_matches = if media_type.eq_ignore_ascii_case("all") {
        true
    } else if media_type.eq_ignore_ascii_case("print") {
        true
    } else if media_type.eq_ignore_ascii_case("screen") {
        !prefer_print
    } else {
        false
    };
    if !type_matches {
        return Some(false);
    }

    let condition_matches = if raw.is_empty() {
        true
    } else {
        evaluate_media_condition(raw, viewport, 0)?
    };
    Some(if negate_query {
        !condition_matches
    } else {
        condition_matches
    })
}

fn evaluate_media_condition(raw: &str, viewport: Size, depth: usize) -> Option<bool> {
    if depth > MAX_CONDITION_DEPTH {
        return None;
    }
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }

    let or_items = split_top_level_keyword(raw, "or")?;
    if or_items.len() > 1 {
        for item in or_items {
            if evaluate_media_condition(item, viewport, depth + 1)? {
                return Some(true);
            }
        }
        return Some(false);
    }
    let and_items = split_top_level_keyword(raw, "and")?;
    if and_items.len() > 1 {
        let mut saw_item = false;
        for item in and_items {
            saw_item = true;
            if !evaluate_media_condition(item, viewport, depth + 1)? {
                return Some(false);
            }
        }
        return Some(saw_item);
    }
    if let Some(rest) = strip_leading_word(raw, "not") {
        return evaluate_media_condition(rest, viewport, depth + 1).map(|matched| !matched);
    }
    if let Some(inner) = strip_entire_parentheses(raw) {
        let inner = inner.trim();
        if has_top_level_keyword(inner, "and")?
            || has_top_level_keyword(inner, "or")?
            || starts_with_word(inner, "not")
            || strip_entire_parentheses(inner).is_some()
        {
            return evaluate_media_condition(inner, viewport, depth + 1);
        }
        return evaluate_media_feature(inner, viewport);
    }
    evaluate_media_feature(raw, viewport)
}

fn evaluate_media_feature(raw: &str, viewport: Size) -> Option<bool> {
    if let Some(colon) = find_top_level_byte(raw, b':')? {
        let mut name = raw[..colon].trim().to_ascii_lowercase();
        let value = raw[colon + 1..].trim();
        let comparison = if let Some(base) = name.strip_prefix("min-") {
            name = base.to_string();
            Comparison::GreaterThanEqual
        } else if let Some(base) = name.strip_prefix("max-") {
            name = base.to_string();
            Comparison::LessThanEqual
        } else {
            Comparison::Equal
        };
        return compare_media_feature(&name, comparison, value, viewport);
    }

    let comparisons = find_top_level_comparisons(raw)?;
    match comparisons.as_slice() {
        [] => media_boolean_feature(raw.trim()),
        [(position, operator, width)] => {
            let left = raw[..*position].trim();
            let right = raw[position + width..].trim();
            if is_feature_name(left) {
                compare_media_feature(left, *operator, right, viewport)
            } else if is_feature_name(right) {
                compare_media_feature(right, operator.reverse(), left, viewport)
            } else {
                None
            }
        }
        [
            (first, first_operator, first_width),
            (second, second_operator, second_width),
        ] => {
            let start = raw[..*first].trim();
            let name = raw[first + first_width..*second].trim();
            let end = raw[second + second_width..].trim();
            if !is_feature_name(name) {
                return None;
            }
            let lower = compare_media_feature(name, first_operator.reverse(), start, viewport)?;
            let upper = compare_media_feature(name, *second_operator, end, viewport)?;
            Some(lower && upper)
        }
        _ => None,
    }
}

fn media_boolean_feature(name: &str) -> Option<bool> {
    let name = name.trim().to_ascii_lowercase();
    match name.as_str() {
        "color"
        | "color-gamut"
        | "display-mode"
        | "dynamic-range"
        | "environment-blending"
        | "horizontal-viewport-segments"
        | "-moz-device-pixel-ratio"
        | "resolution"
        | "scan"
        | "video-color-gamut"
        | "video-dynamic-range"
        | "-webkit-device-pixel-ratio"
        | "vertical-viewport-segments"
        | "overflow-block"
        | "device-posture"
        | "shape"
        | "-webkit-transform-2d" => Some(true),
        "any-hover"
        | "any-pointer"
        | "hover"
        | "pointer"
        | "color-index"
        | "forced-colors"
        | "grid"
        | "inverted-colors"
        | "monochrome"
        | "nav-controls"
        | "overflow-inline"
        | "prefers-contrast"
        | "prefers-reduced-data"
        | "prefers-reduced-motion"
        | "prefers-reduced-transparency"
        | "scripting"
        | "update"
        | "-webkit-transform-3d"
        | "-webkit-animation"
        | "-webkit-transition" => Some(false),
        _ => None,
    }
}

fn compare_media_feature(
    name: &str,
    operator: Comparison,
    value: &str,
    viewport: Size,
) -> Option<bool> {
    let name = name.trim().to_ascii_lowercase();
    match name.as_str() {
        "width" | "device-width" => compare_length(viewport.width, operator, value),
        "height" | "device-height" => compare_length(viewport.height, operator, value),
        "aspect-ratio" | "device-aspect-ratio" => {
            if viewport.height == Pt::ZERO {
                return None;
            }
            compare_number(
                viewport.width.to_f32() / viewport.height.to_f32(),
                operator,
                parse_ratio(value)?,
            )
        }
        "orientation" => compare_ident(
            if viewport.width > viewport.height {
                "landscape"
            } else {
                "portrait"
            },
            operator,
            value,
        ),
        "prefers-color-scheme" => compare_ident("light", operator, value),
        "prefers-reduced-motion"
        | "prefers-reduced-transparency"
        | "prefers-reduced-data"
        | "prefers-contrast" => compare_ident("no-preference", operator, value),
        "forced-colors" | "inverted-colors" => compare_ident("none", operator, value),
        "overflow-block" => compare_ident("paged", operator, value),
        "overflow-inline" | "update" | "scripting" | "any-hover" | "any-pointer" | "hover"
        | "pointer" | "nav-controls" => compare_ident("none", operator, value),
        "color-gamut" | "video-color-gamut" => compare_ident("srgb", operator, value),
        "dynamic-range" | "video-dynamic-range" => compare_ident("standard", operator, value),
        "environment-blending" => compare_ident("opaque", operator, value),
        "display-mode" => compare_ident("browser", operator, value),
        "scan" => compare_ident("progressive", operator, value),
        "device-posture" => compare_ident("continuous", operator, value),
        "shape" => compare_ident("rect", operator, value),
        "resolution" => compare_number(96.0, operator, parse_resolution_dpi(value)?),
        "-webkit-device-pixel-ratio" | "-moz-device-pixel-ratio" => {
            compare_number(1.0, operator, parse_css_number(value)?)
        }
        "color" => compare_integer(8, operator, value),
        "color-index" | "monochrome" | "grid" => compare_integer(0, operator, value),
        "horizontal-viewport-segments" | "vertical-viewport-segments" => {
            compare_integer(1, operator, value)
        }
        _ => None,
    }
}

fn compare_length(actual: Pt, operator: Comparison, value: &str) -> Option<bool> {
    let expected = parse_absolute_length_px(value).map(|px| Pt::from_f32(px * 0.75))?;
    Some(match operator {
        Comparison::Equal => actual == expected,
        Comparison::GreaterThan => actual > expected,
        Comparison::GreaterThanEqual => actual >= expected,
        Comparison::LessThan => actual < expected,
        Comparison::LessThanEqual => actual <= expected,
    })
}

fn compare_integer(actual: i32, operator: Comparison, value: &str) -> Option<bool> {
    let expected = value.trim().parse::<i32>().ok()?;
    Some(match operator {
        Comparison::Equal => actual == expected,
        Comparison::GreaterThan => actual > expected,
        Comparison::GreaterThanEqual => actual >= expected,
        Comparison::LessThan => actual < expected,
        Comparison::LessThanEqual => actual <= expected,
    })
}

fn compare_number(actual: f32, operator: Comparison, expected: f32) -> Option<bool> {
    Some(match operator {
        Comparison::Equal => (actual - expected).abs() <= 0.0001,
        Comparison::GreaterThan => actual > expected,
        Comparison::GreaterThanEqual => actual >= expected,
        Comparison::LessThan => actual < expected,
        Comparison::LessThanEqual => actual <= expected,
    })
}

fn compare_ident(actual: &str, operator: Comparison, expected: &str) -> Option<bool> {
    Some(matches!(operator, Comparison::Equal) && actual.eq_ignore_ascii_case(expected.trim()))
}

fn parse_ratio(raw: &str) -> Option<f32> {
    let parts = split_top_level(raw, '/').ok()?;
    if parts.len() != 2 {
        return None;
    }
    let numerator = parse_css_number(&parts[0])?;
    let denominator = parse_css_number(&parts[1])?;
    if denominator.abs() <= f32::EPSILON {
        None
    } else {
        Some(numerator / denominator)
    }
}

fn parse_resolution_dpi(raw: &str) -> Option<f32> {
    let raw = raw.trim();
    let number_end = css_number_end(raw)?;
    let number = raw[..number_end].parse::<f32>().ok()?;
    if !number.is_finite() {
        return None;
    }
    let dpi = match raw[number_end..].trim().to_ascii_lowercase().as_str() {
        "dpi" => number,
        "dpcm" => number * 2.54,
        "dppx" | "x" => number * 96.0,
        _ => return None,
    };
    dpi.is_finite().then_some(dpi)
}

fn parse_css_number(raw: &str) -> Option<f32> {
    let raw = raw.trim();
    let end = css_number_end(raw)?;
    if end != raw.len() {
        return None;
    }
    raw.parse::<f32>().ok().filter(|value| value.is_finite())
}

fn css_number_end(input: &str) -> Option<usize> {
    let bytes = input.as_bytes();
    let mut cursor = usize::from(matches!(bytes.first(), Some(b'+') | Some(b'-')));
    let integer_start = cursor;
    while bytes.get(cursor).is_some_and(u8::is_ascii_digit) {
        cursor += 1;
    }
    let mut digits = cursor - integer_start;
    if bytes.get(cursor) == Some(&b'.') {
        cursor += 1;
        let fraction_start = cursor;
        while bytes.get(cursor).is_some_and(u8::is_ascii_digit) {
            cursor += 1;
        }
        digits += cursor - fraction_start;
    }
    if digits == 0 {
        return None;
    }
    if matches!(bytes.get(cursor), Some(b'e') | Some(b'E')) {
        let exponent = cursor;
        cursor += 1;
        if matches!(bytes.get(cursor), Some(b'+') | Some(b'-')) {
            cursor += 1;
        }
        let start = cursor;
        while bytes.get(cursor).is_some_and(u8::is_ascii_digit) {
            cursor += 1;
        }
        if cursor == start {
            cursor = exponent;
        }
    }
    Some(cursor)
}

fn is_feature_name(raw: &str) -> bool {
    let raw = raw.trim();
    !raw.is_empty()
        && raw
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_') || byte >= 0x80)
}

fn find_top_level_comparisons(raw: &str) -> Option<Vec<(usize, Comparison, usize)>> {
    let mut output = Vec::new();
    let mut depth = 0usize;
    let mut quote = None;
    let bytes = raw.as_bytes();
    let mut cursor = 0usize;
    while cursor < bytes.len() {
        let byte = bytes[cursor];
        if let Some(active) = quote {
            if byte == b'\\' {
                cursor = (cursor + 2).min(bytes.len());
                continue;
            }
            if byte == active {
                quote = None;
            }
            cursor += 1;
            continue;
        }
        match byte {
            b'\'' | b'"' => quote = Some(byte),
            b'(' | b'[' | b'{' => depth = depth.saturating_add(1),
            b')' | b']' | b'}' => depth = depth.checked_sub(1)?,
            b'<' | b'>' | b'=' if depth == 0 => {
                let has_equal = bytes.get(cursor + 1) == Some(&b'=');
                let (operator, width) = match (byte, has_equal) {
                    (b'<', false) => (Comparison::LessThan, 1),
                    (b'<', true) => (Comparison::LessThanEqual, 2),
                    (b'>', false) => (Comparison::GreaterThan, 1),
                    (b'>', true) => (Comparison::GreaterThanEqual, 2),
                    (b'=', false) => (Comparison::Equal, 1),
                    _ => return None,
                };
                output.push((cursor, operator, width));
                cursor += width;
                continue;
            }
            _ => {}
        }
        cursor += 1;
    }
    (quote.is_none() && depth == 0).then_some(output)
}

fn find_top_level_byte(raw: &str, target: u8) -> Option<Option<usize>> {
    let mut depth = 0usize;
    let mut quote = None;
    let bytes = raw.as_bytes();
    let mut cursor = 0usize;
    while cursor < bytes.len() {
        let byte = bytes[cursor];
        if let Some(active) = quote {
            if byte == b'\\' {
                cursor = (cursor + 2).min(bytes.len());
                continue;
            }
            if byte == active {
                quote = None;
            }
            cursor += 1;
            continue;
        }
        match byte {
            b'\'' | b'"' => quote = Some(byte),
            b'(' | b'[' | b'{' => depth = depth.saturating_add(1),
            b')' | b']' | b'}' => depth = depth.checked_sub(1)?,
            _ if byte == target && depth == 0 => return Some(Some(cursor)),
            _ => {}
        }
        cursor += 1;
    }
    (quote.is_none() && depth == 0).then_some(None)
}

fn split_top_level_keyword<'a>(raw: &'a str, keyword: &str) -> Option<Vec<&'a str>> {
    let mut output = Vec::new();
    let mut start = 0usize;
    let mut depth = 0usize;
    let mut quote = None;
    let bytes = raw.as_bytes();
    let mut cursor = 0usize;
    while cursor < bytes.len() {
        let byte = bytes[cursor];
        if let Some(active) = quote {
            if byte == b'\\' {
                cursor = (cursor + 2).min(bytes.len());
                continue;
            }
            if byte == active {
                quote = None;
            }
            cursor += 1;
            continue;
        }
        match byte {
            b'\'' | b'"' => quote = Some(byte),
            b'(' | b'[' | b'{' => depth = depth.saturating_add(1),
            b')' | b']' | b'}' => depth = depth.checked_sub(1)?,
            _ if depth == 0 && word_at(raw, cursor, keyword) => {
                output.push(raw[start..cursor].trim());
                cursor += keyword.len();
                start = cursor;
                continue;
            }
            _ => {}
        }
        cursor += 1;
    }
    if quote.is_some() || depth != 0 {
        return None;
    }
    output.push(raw[start..].trim());
    (!output.iter().any(|item| item.is_empty())).then_some(output)
}

fn has_top_level_keyword(raw: &str, keyword: &str) -> Option<bool> {
    split_top_level_keyword(raw, keyword).map(|items| items.len() > 1)
}

fn strip_entire_parentheses(raw: &str) -> Option<&str> {
    let raw = raw.trim();
    if !raw.starts_with('(') {
        return None;
    }
    let mut depth = 0usize;
    let mut quote = None;
    let bytes = raw.as_bytes();
    let mut cursor = 0usize;
    while cursor < bytes.len() {
        let byte = bytes[cursor];
        if let Some(active) = quote {
            if byte == b'\\' {
                cursor = (cursor + 2).min(bytes.len());
                continue;
            }
            if byte == active {
                quote = None;
            }
            cursor += 1;
            continue;
        }
        match byte {
            b'\'' | b'"' => quote = Some(byte),
            b'(' => depth = depth.saturating_add(1),
            b')' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return (cursor == bytes.len() - 1).then_some(&raw[1..cursor]);
                }
            }
            _ => {}
        }
        cursor += 1;
    }
    None
}

fn strip_entire_function<'a>(raw: &'a str, name: &str) -> Option<&'a str> {
    let raw = raw.trim();
    let prefix = leading_word(raw)?;
    if !prefix.eq_ignore_ascii_case(name) {
        return None;
    }
    let arguments = raw[prefix.len()..].trim_start();
    strip_entire_parentheses(arguments)
}

fn strip_leading_word<'a>(raw: &'a str, word: &str) -> Option<&'a str> {
    let raw = raw.trim_start();
    if word_at(raw, 0, word) {
        Some(&raw[word.len()..])
    } else {
        None
    }
}

fn starts_with_word(raw: &str, word: &str) -> bool {
    strip_leading_word(raw, word).is_some()
}

fn leading_word(raw: &str) -> Option<&str> {
    let raw = raw.trim_start();
    let end = raw
        .char_indices()
        .find_map(|(index, ch)| (!is_ident_char(ch)).then_some(index))
        .unwrap_or(raw.len());
    (end > 0).then_some(&raw[..end])
}

fn word_at(raw: &str, position: usize, word: &str) -> bool {
    let Some(candidate) = raw.get(position..position.saturating_add(word.len())) else {
        return false;
    };
    if !candidate.eq_ignore_ascii_case(word) {
        return false;
    }
    let before = raw[..position].chars().next_back();
    let after = raw[position + word.len()..].chars().next();
    before.is_none_or(|ch| !is_ident_char(ch)) && after.is_none_or(|ch| !is_ident_char(ch))
}

fn is_ident_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') || !ch.is_ascii()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn viewport(width: f32, height: f32) -> Size {
        Size {
            width: Pt::from_f32(width),
            height: Pt::from_f32(height),
        }
    }

    #[test]
    fn media_types_and_lists_follow_print_preference() {
        assert!(evaluate_media_list("screen, print", viewport(600.0, 800.0), true).matched);
        assert!(!evaluate_media_list("screen", viewport(600.0, 800.0), true).matched);
        assert!(evaluate_media_list("screen", viewport(600.0, 800.0), false).matched);
        assert!(!evaluate_media_list("not print", viewport(600.0, 800.0), true).matched);
        assert!(media_list_has_print_type("not print and (color)"));
        assert!(!media_list_has_print_type("screen and (shape: print)"));
    }

    #[test]
    fn media_features_support_colon_and_range_syntax() {
        let size = viewport(450.0, 300.0);
        assert!(evaluate_media_list("(width: 600px)", size, false).matched);
        assert!(evaluate_media_list("(300px < width <= 700px)", size, false).matched);
        assert!(!evaluate_media_list("(width > 700px)", size, false).matched);
        assert!(evaluate_media_list("(aspect-ratio: 3/2)", size, false).matched);
        assert!(evaluate_media_list("(resolution: 1dppx)", size, false).matched);
    }

    #[test]
    fn media_conditions_preserve_boolean_logic_and_unsupported_state() {
        let size = viewport(450.0, 300.0);
        assert!(
            evaluate_media_list("(orientation: landscape) and not (grid)", size, false).matched
        );
        assert!(evaluate_media_list("(grid) or (color)", size, false).matched);
        let unsupported = evaluate_media_list("(made-up-feature)", size, false);
        assert!(!unsupported.matched);
        assert_eq!(unsupported.unsupported_queries, 1);
    }

    #[test]
    fn supports_conditions_delegate_declarations_and_selectors() {
        let supported = evaluate_supports_condition(
            "(display: flex) and (selector(.item > .child) or (made-up: value))",
            |name, value| name == "display" && value == "flex",
            |selector| selector == ".item > .child",
        );
        assert!(supported);
        assert!(evaluate_supports_condition(
            "not (display: nonsense)",
            |name, value| name == "display" && value == "flex",
            |_| false,
        ));
        assert!(!evaluate_supports_condition(
            "font-tech(variations)",
            |_, _| true,
            |_| true,
        ));
    }

    #[test]
    fn scope_preludes_preserve_selector_lists_and_limits() {
        let scope = parse_scope_prelude("(.card, article.feature) to (.stop, #limit)")
            .expect("scope prelude");
        assert_eq!(scope.starts, [".card", "article.feature"]);
        assert_eq!(scope.ends, [".stop", "#limit"]);
        assert!(parse_scope_prelude(":scope").is_none());
        assert!(parse_scope_prelude("(.card) to").is_none());
    }

    #[test]
    fn container_preludes_parse_names_ranges_and_logic() {
        let parsed = parse_container_prelude(
            "card (300px < width <= 600px) and not (orientation: portrait)",
        )
        .expect("container query");
        assert_eq!(parsed.name.as_deref(), Some("card"));
        let Some(ContainerCondition::And(items)) = parsed.condition else {
            panic!("expected conjunction");
        };
        assert!(matches!(
            items[0],
            ContainerCondition::Feature(ContainerFeature::Interval { .. })
        ));
        assert!(matches!(items[1], ContainerCondition::Not(_)));

        let named = parse_container_prelude("sidebar").expect("named container");
        assert_eq!(named.name.as_deref(), Some("sidebar"));
        assert!(named.condition.is_none());
        assert!(parse_container_prelude("(width > nonsense)").is_none());
    }
}
