use crate::{Command, Document};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PageDataOp {
    Every,
    Count,
    Sum { scale: u32 },
}

#[derive(Debug, Clone, Default)]
pub struct PaginatedContextSpec {
    pub ops: HashMap<String, PageDataOp>,
}

impl PaginatedContextSpec {
    pub fn new(ops: HashMap<String, PageDataOp>) -> Self {
        Self { ops }
    }

    pub fn parse_op(raw: &str) -> Option<PageDataOp> {
        let raw = raw.trim().to_ascii_lowercase();
        if raw == "every" {
            return Some(PageDataOp::Every);
        }
        if raw == "count" {
            return Some(PageDataOp::Count);
        }
        if raw == "sum" {
            // Default: cents (scale=2) to support money-like values.
            return Some(PageDataOp::Sum { scale: 2 });
        }
        if let Some(rest) = raw.strip_prefix("sum:") {
            let scale = rest.trim().parse::<u32>().ok()?;
            return Some(PageDataOp::Sum { scale });
        }
        None
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PageDataValue {
    Every(Vec<String>),
    Count(usize),
    Sum { scale: u32, value: i64 },
}

#[derive(Debug, Clone, Default)]
pub struct PageDataContext {
    pub page_count: usize,
    // Per page (1-based externally, 0-based index here): key -> computed value.
    pub pages: Vec<HashMap<String, PageDataValue>>,
    // Document totals across pages: key -> computed value.
    pub totals: HashMap<String, PageDataValue>,
}

pub fn compute_page_data_context(doc: &Document, spec: &PaginatedContextSpec) -> PageDataContext {
    let page_count = doc.pages.len();
    let mut pages_out: Vec<HashMap<String, PageDataValue>> = Vec::with_capacity(page_count);
    let mut totals: HashMap<String, PageDataValue> = HashMap::new();

    for (key, op) in &spec.ops {
        let init = match op {
            PageDataOp::Every => PageDataValue::Every(Vec::new()),
            PageDataOp::Count => PageDataValue::Count(0),
            PageDataOp::Sum { scale } => PageDataValue::Sum {
                scale: *scale,
                value: 0,
            },
        };
        totals.insert(key.clone(), init);
    }

    for page in &doc.pages {
        let mut raw: HashMap<String, Vec<String>> = HashMap::new();
        for cmd in &page.commands {
            if let Command::Meta { key, value } = cmd {
                if spec.ops.contains_key(key) {
                    raw.entry(key.clone()).or_default().push(value.clone());
                }
            }
        }

        let mut computed: HashMap<String, PageDataValue> = HashMap::new();

        for (key, op) in &spec.ops {
            let values = raw.get(key).cloned().unwrap_or_default();
            let val = match op {
                PageDataOp::Every => PageDataValue::Every(values.clone()),
                PageDataOp::Count => PageDataValue::Count(values.len()),
                PageDataOp::Sum { scale } => {
                    let mut sum = 0i64;
                    for v in &values {
                        if let Some(n) = parse_scaled_int(v, *scale) {
                            sum = sum.saturating_add(n);
                        }
                    }
                    PageDataValue::Sum {
                        scale: *scale,
                        value: sum,
                    }
                }
            };

            // Update totals
            if let Some(total) = totals.get_mut(key) {
                match (total, &val) {
                    (PageDataValue::Every(total_v), PageDataValue::Every(v)) => {
                        total_v.extend(v.iter().cloned());
                    }
                    (PageDataValue::Count(total_c), PageDataValue::Count(c)) => {
                        *total_c = total_c.saturating_add(*c);
                    }
                    (
                        PageDataValue::Sum {
                            scale: _ts,
                            value: tv,
                        },
                        PageDataValue::Sum {
                            scale: _s,
                            value: v,
                        },
                    ) => {
                        *tv = tv.saturating_add(*v);
                    }
                    _ => {}
                }
            }

            computed.insert(key.clone(), val);
        }

        pages_out.push(computed);
    }

    PageDataContext {
        page_count,
        pages: pages_out,
        totals,
    }
}

pub fn substitute_placeholders(
    template: &str,
    page_number: usize,
    page_count: usize,
    ctx: Option<&PageDataContext>,
) -> String {
    // First handle simple page placeholders.
    let mut rendered = template
        .replace("{page}", &page_number.to_string())
        .replace("{pages}", &page_count.to_string());

    // Then handle dynamic tokens like {sum:items.cost} / {total:items.cost}.
    let mut out = String::with_capacity(rendered.len());
    let mut rest: &str = &rendered;

    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        rest = &rest[start + 1..];

        let Some(end) = rest.find('}') else {
            // Unclosed token; keep as-is.
            out.push('{');
            out.push_str(rest);
            return out;
        };

        let token = &rest[..end];
        let replacement = resolve_token(token, page_number, ctx);
        if let Some(rep) = replacement {
            out.push_str(&rep);
        } else {
            out.push('{');
            out.push_str(token);
            out.push('}');
        }

        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    rendered = out;
    rendered
}

fn resolve_token(token: &str, page_number: usize, ctx: Option<&PageDataContext>) -> Option<String> {
    let ctx = ctx?;
    let page_index = page_number.checked_sub(1)?;

    let (kind, key) = token.split_once(':')?;
    let kind = kind.trim();
    let key = key.trim();
    if key.is_empty() {
        return None;
    }

    match kind {
        "sum" => match ctx.pages.get(page_index)?.get(key)? {
            PageDataValue::Sum { scale, value } => Some(format_scaled_int(*value, *scale)),
            _ => None,
        },
        "total" => match ctx.totals.get(key)? {
            PageDataValue::Sum { scale, value } => Some(format_scaled_int(*value, *scale)),
            _ => None,
        },
        "count" => match ctx.pages.get(page_index)?.get(key)? {
            PageDataValue::Count(v) => Some(v.to_string()),
            _ => None,
        },
        "total_count" => match ctx.totals.get(key)? {
            PageDataValue::Count(v) => Some(v.to_string()),
            _ => None,
        },
        "every" => match ctx.pages.get(page_index)?.get(key)? {
            PageDataValue::Every(v) => Some(v.join(",")),
            _ => None,
        },
        "total_every" => match ctx.totals.get(key)? {
            PageDataValue::Every(v) => Some(v.join(",")),
            _ => None,
        },
        _ => None,
    }
}

// Parse a decimal-like string into a scaled integer (e.g., scale=2 => cents) without using floats.
// Accepts values like "$1,234.56" or "1234.56". Extra fractional digits are truncated.
pub fn parse_scaled_int(raw: &str, scale: u32) -> Option<i64> {
    let mut sign = 1i64;
    let mut seen_sign = false;
    let mut int_digits: Vec<u8> = Vec::new();
    let mut frac_digits: Vec<u8> = Vec::new();
    let mut in_frac = false;

    for ch in raw.chars() {
        if !seen_sign && (ch == '-' || ch == '+') {
            seen_sign = true;
            if ch == '-' {
                sign = -1;
            }
            continue;
        }
        if ch == '.' && !in_frac {
            in_frac = true;
            continue;
        }
        if ch.is_ascii_digit() {
            if in_frac {
                frac_digits.push(ch as u8 - b'0');
            } else {
                int_digits.push(ch as u8 - b'0');
            }
            continue;
        }
        // Ignore common formatting characters.
        if ch == ',' || ch == '$' || ch.is_whitespace() {
            continue;
        }
        // Any other character is ignored (e.g., currency codes), for robustness.
    }

    if int_digits.is_empty() && frac_digits.is_empty() {
        return None;
    }

    let mut int_part: i64 = 0;
    for d in int_digits {
        int_part = int_part.saturating_mul(10).saturating_add(d as i64);
    }

    // Build fractional part at requested scale.
    let mut frac_part: i64 = 0;
    let mut i = 0u32;
    for d in frac_digits {
        if i >= scale {
            break;
        }
        frac_part = frac_part.saturating_mul(10).saturating_add(d as i64);
        i += 1;
    }
    while i < scale {
        frac_part = frac_part.saturating_mul(10);
        i += 1;
    }

    let pow10 = 10i64.saturating_pow(scale);
    Some(sign.saturating_mul(int_part.saturating_mul(pow10).saturating_add(frac_part)))
}

pub fn format_scaled_int(value: i64, scale: u32) -> String {
    if scale == 0 {
        return value.to_string();
    }

    let sign = if value < 0 { "-" } else { "" };
    let abs = value.abs();
    let pow10 = 10i64.saturating_pow(scale);
    let int_part = abs / pow10;
    let frac_part = abs % pow10;
    let frac_width = scale as usize;

    format!(
        "{}{}.{:0width$}",
        sign,
        int_part,
        frac_part,
        width = frac_width
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_scaled_int_money_like() {
        assert_eq!(parse_scaled_int("$35.07", 2), Some(3507));
        assert_eq!(parse_scaled_int("35.07", 2), Some(3507));
        assert_eq!(parse_scaled_int("1,234.56", 2), Some(123456));
        assert_eq!(parse_scaled_int("-0.10", 2), Some(-10));
        assert_eq!(parse_scaled_int("10", 2), Some(1000));
    }

    #[test]
    fn format_scaled_int_money_like() {
        assert_eq!(format_scaled_int(3507, 2), "35.07");
        assert_eq!(format_scaled_int(-10, 2), "-0.10");
        assert_eq!(format_scaled_int(1000, 2), "10.00");
        assert_eq!(format_scaled_int(12, 0), "12");
    }

    #[test]
    fn placeholder_substitution() {
        let mut ops = HashMap::new();
        ops.insert("items.cost".to_string(), PageDataOp::Sum { scale: 2 });
        let spec = PaginatedContextSpec::new(ops);

        let doc = Document {
            page_size: crate::Size::a4(),
            pages: vec![
                crate::Page {
                    commands: vec![
                        Command::Meta {
                            key: "items.cost".to_string(),
                            value: "$1.00".to_string(),
                        },
                        Command::Meta {
                            key: "items.cost".to_string(),
                            value: "$2.50".to_string(),
                        },
                    ],
                },
                crate::Page {
                    commands: vec![Command::Meta {
                        key: "items.cost".to_string(),
                        value: "$3.25".to_string(),
                    }],
                },
            ],
        };

        let ctx = compute_page_data_context(&doc, &spec);
        assert_eq!(
            substitute_placeholders(
                "P{page}/{pages} sum={sum:items.cost} total={total:items.cost}",
                1,
                2,
                Some(&ctx)
            ),
            "P1/2 sum=3.50 total=6.75"
        );
        assert_eq!(
            substitute_placeholders(
                "sum={sum:items.cost} total={total:items.cost}",
                2,
                2,
                Some(&ctx)
            ),
            "sum=3.25 total=6.75"
        );
    }
}
