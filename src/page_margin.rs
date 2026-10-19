//! Joint variable-dimension layout for the three margin boxes on a page edge.
//!
//! CSS Paged Media 3, section 5.3.2. The caller supplies intrinsic dimensions
//! after resolving page counters, named strings and captured running elements.

use crate::types::Pt;

#[derive(Debug, Clone, Copy)]
pub(crate) struct AxisSize {
    pub specified: Option<Pt>,
    pub minimum: Pt,
    pub maximum: Pt,
}

impl AxisSize {
    pub fn new(specified: Option<Pt>, minimum: Pt, maximum: Pt) -> Self {
        if let Some(value) = specified {
            return Self::fixed(value);
        }
        let minimum = minimum.max(Pt::ZERO);
        Self {
            specified: None,
            minimum,
            maximum: maximum.max(minimum),
        }
    }

    fn fixed(value: Pt) -> Self {
        let value = value.max(Pt::ZERO);
        Self {
            specified: Some(value),
            minimum: value,
            maximum: value,
        }
    }
}

fn weighted_share(value: Pt, first: Pt, second: Pt) -> Pt {
    let first = first.max(Pt::ZERO).to_milli_i64() as i128;
    let second = second.max(Pt::ZERO).to_milli_i64() as i128;
    let sum = first + second;
    if sum == 0 {
        return value / 2;
    }
    let numerator = value.to_milli_i64() as i128 * first;
    let rounded = (numerator + if numerator >= 0 { sum / 2 } else { -sum / 2 }) / sum;
    Pt::from_milli_i64(rounded.clamp(i64::MIN as i128, i64::MAX as i128) as i64)
}

fn resolve_pair(available: Pt, first: AxisSize, second: AxisSize) -> [Pt; 2] {
    match (first.specified, second.specified) {
        (Some(a), Some(b)) => [a, b],
        (Some(a), None) => [a, (available - a).max(Pt::ZERO)],
        (None, Some(b)) => [(available - b).max(Pt::ZERO), b],
        (None, None) => {
            let (base_a, base_b, weight_a, weight_b) = if first.maximum + second.maximum < available
            {
                (first.maximum, second.maximum, first.maximum, second.maximum)
            } else if first.minimum + second.minimum < available {
                (
                    first.minimum,
                    second.minimum,
                    first.maximum - first.minimum,
                    second.maximum - second.minimum,
                )
            } else {
                (first.minimum, second.minimum, first.minimum, second.minimum)
            };
            let a = (base_a + weighted_share(available - base_a - base_b, weight_a, weight_b))
                .max(Pt::ZERO)
                .min(available);
            // Assign the quantization remainder to the second box, so automatic
            // allocations never create a seam or overlap through rounding.
            [a, available - a]
        }
    }
}

pub(crate) fn resolve_axis(available: Pt, boxes: [Option<AxisSize>; 3]) -> [Pt; 3] {
    let available = available.max(Pt::ZERO);
    let middle_generated = boxes[1].is_some();
    let [first, middle, last] = boxes.map(|item| item.unwrap_or(AxisSize::fixed(Pt::ZERO)));
    if !middle_generated {
        let [a, c] = resolve_pair(available, first, last);
        return [a, Pt::ZERO, c];
    }
    let b = middle.specified.unwrap_or_else(|| {
        let sides = AxisSize::new(
            first
                .specified
                .zip(last.specified)
                .map(|(a, c)| a.max(c) * 2),
            first.minimum.max(last.minimum) * 2,
            first.maximum.max(last.maximum) * 2,
        );
        resolve_pair(available, middle, sides)[0]
    });
    let remaining = (available - b).max(Pt::ZERO);
    let a = remaining / 2;
    let c = remaining - a;
    [first.specified.unwrap_or(a), b, last.specified.unwrap_or(c)]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn auto(minimum: i32, maximum: i32) -> Option<AxisSize> {
        Some(AxisSize::new(
            None,
            Pt::from_i32(minimum),
            Pt::from_i32(maximum),
        ))
    }

    fn sizes(available: i32, boxes: [Option<AxisSize>; 3]) -> [i64; 3] {
        resolve_axis(Pt::from_i32(available), boxes).map(Pt::to_milli_i64)
    }

    #[test]
    fn a_single_generated_box_uses_the_available_edge() {
        for index in 0..3 {
            let mut boxes = [None; 3];
            boxes[index] = auto(20, 20);
            let mut expected = [0; 3];
            expected[index] = 240_000;
            assert_eq!(sizes(240, boxes), expected);
        }
    }

    #[test]
    fn two_boxes_share_extra_space_by_max_content() {
        assert_eq!(
            sizes(240, [auto(10, 20), None, auto(20, 40)]),
            [80_000, 0, 160_000]
        );
    }

    #[test]
    fn two_boxes_shrink_between_min_and_max_content() {
        assert_eq!(
            sizes(120, [auto(20, 60), None, auto(40, 120)]),
            [40_000, 0, 80_000]
        );
        assert_eq!(
            sizes(30, [auto(20, 60), None, auto(40, 120)]),
            [10_000, 0, 20_000]
        );
    }

    #[test]
    fn centered_content_reserves_equal_space_on_each_side() {
        assert_eq!(
            sizes(240, [auto(20, 20), auto(40, 40), auto(60, 60)]),
            [90_000, 60_000, 90_000]
        );
        assert_eq!(
            sizes(240, [auto(20, 20), auto(20, 20), auto(20, 20)]),
            [80_000; 3]
        );
    }

    #[test]
    fn definite_dimensions_are_preserved_even_when_overconstrained() {
        let fixed = |value| Some(AxisSize::fixed(Pt::from_i32(value)));
        assert_eq!(
            sizes(240, [fixed(40), auto(20, 20), fixed(60)]),
            [40_000, 120_000, 60_000]
        );
        assert_eq!(
            sizes(240, [fixed(200), fixed(180), auto(20, 40)]),
            [200_000, 180_000, 30_000]
        );
    }

    #[test]
    fn zero_intrinsics_and_subpoint_rounding_are_bounded() {
        assert_eq!(
            sizes(240, [auto(0, 0), None, auto(0, 0)]),
            [120_000, 0, 120_000]
        );
        let [a, _, c] = resolve_axis(Pt::from_milli_i64(7), [auto(1, 1), None, auto(2, 2)]);
        assert_eq!(a + c, Pt::from_milli_i64(7));
        let [a, b, c] = resolve_axis(Pt::from_milli_i64(7), [auto(1, 1); 3]);
        assert_eq!(a + b + c, Pt::from_milli_i64(7));
        assert_eq!(sizes(0, [auto(20, 20); 3]), [0; 3]);
    }
}
