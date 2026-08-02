//! Dependency-free SFNT outline extraction.
//!
//! The TrueType path is deliberately materialized as contours before emission. Besides making
//! every byte range easy to validate, this supports composite point attachment correctly instead
//! of assuming every component uses an x/y translation.

use crate::sfnt::{Face, GlyphId, Rect};

const MAX_COMPOSITE_DEPTH: usize = 32;

const ON_CURVE_POINT: u8 = 0x01;
const X_SHORT_VECTOR: u8 = 0x02;
const Y_SHORT_VECTOR: u8 = 0x04;
const REPEAT_FLAG: u8 = 0x08;
const X_IS_SAME_OR_POSITIVE: u8 = 0x10;
const Y_IS_SAME_OR_POSITIVE: u8 = 0x20;

const ARG_1_AND_2_ARE_WORDS: u16 = 0x0001;
const ARGS_ARE_XY_VALUES: u16 = 0x0002;
const ROUND_XY_TO_GRID: u16 = 0x0004;
const WE_HAVE_A_SCALE: u16 = 0x0008;
const MORE_COMPONENTS: u16 = 0x0020;
const WE_HAVE_AN_X_AND_Y_SCALE: u16 = 0x0040;
const WE_HAVE_A_TWO_BY_TWO: u16 = 0x0080;
const WE_HAVE_INSTRUCTIONS: u16 = 0x0100;
const SCALED_COMPONENT_OFFSET: u16 = 0x0800;
const UNSCALED_COMPONENT_OFFSET: u16 = 0x1000;

pub(crate) trait OutlineBuilder {
    fn move_to(&mut self, x: f32, y: f32);
    fn line_to(&mut self, x: f32, y: f32);
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32);
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32);
    fn close(&mut self);
}

#[derive(Clone, Copy, Debug)]
struct Point {
    x: f32,
    y: f32,
    on_curve: bool,
}

impl Point {
    fn midpoint(self, other: Self) -> Self {
        Self {
            x: (self.x + other.x) * 0.5,
            y: (self.y + other.y) * 0.5,
            on_curve: true,
        }
    }
}

type Contour = Vec<Point>;

#[derive(Clone, Copy, Debug)]
struct Transform {
    a: f32,
    b: f32,
    c: f32,
    d: f32,
}

impl Transform {
    const IDENTITY: Self = Self {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
    };

    fn apply(self, point: Point) -> Point {
        Point {
            x: self.a * point.x + self.c * point.y,
            y: self.b * point.x + self.d * point.y,
            on_curve: point.on_curve,
        }
    }

    fn apply_vector(self, x: f32, y: f32) -> (f32, f32) {
        (self.a * x + self.c * y, self.b * x + self.d * y)
    }
}

impl Face<'_> {
    pub(crate) fn outline_glyph(
        &self,
        glyph: GlyphId,
        builder: &mut impl OutlineBuilder,
    ) -> Option<Rect> {
        if self.has_true_type_outlines() {
            let contours = parse_true_type_glyph(self, glyph, 0, &mut Vec::new())?;
            emit_contours(&contours, builder)
        } else {
            crate::sfnt_cff::outline(self, glyph, builder)
        }
    }
}

fn parse_true_type_glyph(
    face: &Face<'_>,
    glyph: GlyphId,
    depth: usize,
    stack: &mut Vec<GlyphId>,
) -> Option<Vec<Contour>> {
    if depth >= MAX_COMPOSITE_DEPTH || stack.contains(&glyph) {
        return None;
    }
    let data = glyph_data(face, glyph)?;
    let contour_count = read_i16(data, 0)?;
    checked_slice(data, 0, 10)?;
    stack.push(glyph);
    let result = if contour_count >= 0 {
        parse_simple_glyph(data, contour_count as usize)
    } else {
        parse_composite_glyph(face, data, depth, stack)
    };
    stack.pop();
    result
}

fn glyph_data<'a>(face: &'a Face<'a>, glyph: GlyphId) -> Option<&'a [u8]> {
    if glyph.0 >= face.number_of_glyphs() {
        return None;
    }
    let head = face.table(*b"head")?;
    let loca = face.table(*b"loca")?;
    let glyf = face.table(*b"glyf")?;
    let index = usize::from(glyph.0);
    let long_offsets = match read_i16(head, 50)? {
        0 => false,
        1 => true,
        _ => return None,
    };
    let offset_at = |entry: usize| -> Option<usize> {
        if long_offsets {
            usize::try_from(read_u32(loca, entry.checked_mul(4)?)?).ok()
        } else {
            Some(usize::from(read_u16(loca, entry.checked_mul(2)?)?) * 2)
        }
    };
    let start = offset_at(index)?;
    let end = offset_at(index.checked_add(1)?)?;
    if start >= end {
        return None;
    }
    checked_slice(glyf, start, end.checked_sub(start)?)
}

fn parse_simple_glyph(data: &[u8], contour_count: usize) -> Option<Vec<Contour>> {
    if contour_count == 0 {
        return Some(Vec::new());
    }
    let endpoints_start = 10usize;
    let endpoints_len = contour_count.checked_mul(2)?;
    checked_slice(data, endpoints_start, endpoints_len)?;
    let mut endpoints = Vec::with_capacity(contour_count);
    let mut previous = None;
    for index in 0..contour_count {
        let endpoint = read_u16(data, endpoints_start + index * 2)?;
        if previous.is_some_and(|value| endpoint <= value) {
            return None;
        }
        endpoints.push(endpoint);
        previous = Some(endpoint);
    }
    let point_count = usize::from(endpoints.last()?.checked_add(1)?);
    // Match the prior parser's handling of a degenerate one-point glyph.
    if point_count == 1 {
        return Some(Vec::new());
    }

    let instruction_length_offset = endpoints_start.checked_add(endpoints_len)?;
    let instruction_length = usize::from(read_u16(data, instruction_length_offset)?);
    let mut cursor = instruction_length_offset
        .checked_add(2)?
        .checked_add(instruction_length)?;
    if cursor > data.len() {
        return None;
    }

    let mut flags = Vec::with_capacity(point_count);
    while flags.len() < point_count {
        let flag = *data.get(cursor)?;
        cursor += 1;
        let repeats = if flag & REPEAT_FLAG != 0 {
            let count = usize::from(*data.get(cursor)?).checked_add(1)?;
            cursor += 1;
            count
        } else {
            1
        };
        if repeats > point_count - flags.len() {
            return None;
        }
        flags.extend(std::iter::repeat_n(flag, repeats));
    }

    let mut x_values = Vec::with_capacity(point_count);
    let mut x = 0i32;
    for &flag in &flags {
        let delta = coordinate_delta(
            data,
            &mut cursor,
            flag,
            X_SHORT_VECTOR,
            X_IS_SAME_OR_POSITIVE,
        )?;
        x = x.checked_add(delta)?;
        x_values.push(i16::try_from(x).ok()?);
    }
    let mut y_values = Vec::with_capacity(point_count);
    let mut y = 0i32;
    for &flag in &flags {
        let delta = coordinate_delta(
            data,
            &mut cursor,
            flag,
            Y_SHORT_VECTOR,
            Y_IS_SAME_OR_POSITIVE,
        )?;
        y = y.checked_add(delta)?;
        y_values.push(i16::try_from(y).ok()?);
    }

    let mut contours = Vec::with_capacity(contour_count);
    let mut start = 0usize;
    for endpoint in endpoints {
        let end = usize::from(endpoint).checked_add(1)?;
        if end > point_count || start >= end {
            return None;
        }
        let mut contour = Vec::with_capacity(end - start);
        for index in start..end {
            contour.push(Point {
                x: f32::from(x_values[index]),
                y: f32::from(y_values[index]),
                on_curve: flags[index] & ON_CURVE_POINT != 0,
            });
        }
        contours.push(contour);
        start = end;
    }
    if start != point_count {
        return None;
    }
    Some(contours)
}

fn coordinate_delta(
    data: &[u8],
    cursor: &mut usize,
    flag: u8,
    short_bit: u8,
    same_or_positive_bit: u8,
) -> Option<i32> {
    if flag & short_bit != 0 {
        let magnitude = i32::from(*data.get(*cursor)?);
        *cursor = cursor.checked_add(1)?;
        Some(if flag & same_or_positive_bit != 0 {
            magnitude
        } else {
            -magnitude
        })
    } else if flag & same_or_positive_bit != 0 {
        Some(0)
    } else {
        let value = i32::from(read_i16(data, *cursor)?);
        *cursor = cursor.checked_add(2)?;
        Some(value)
    }
}

fn parse_composite_glyph(
    face: &Face<'_>,
    data: &[u8],
    depth: usize,
    stack: &mut Vec<GlyphId>,
) -> Option<Vec<Contour>> {
    let mut cursor = 10usize;
    let mut result: Vec<Contour> = Vec::new();
    let mut final_flags = MORE_COMPONENTS;
    while final_flags & MORE_COMPONENTS != 0 {
        let flags = read_u16(data, cursor)?;
        let component_glyph = GlyphId(read_u16(data, cursor + 2)?);
        cursor = cursor.checked_add(4)?;

        let words = flags & ARG_1_AND_2_ARE_WORDS != 0;
        let xy_values = flags & ARGS_ARE_XY_VALUES != 0;
        let (arg1, arg2) = if words {
            let first = if xy_values {
                i32::from(read_i16(data, cursor)?)
            } else {
                i32::from(read_u16(data, cursor)?)
            };
            let second = if xy_values {
                i32::from(read_i16(data, cursor + 2)?)
            } else {
                i32::from(read_u16(data, cursor + 2)?)
            };
            cursor = cursor.checked_add(4)?;
            (first, second)
        } else {
            let first = if xy_values {
                i32::from(*data.get(cursor)? as i8)
            } else {
                i32::from(*data.get(cursor)?)
            };
            let second = if xy_values {
                i32::from(*data.get(cursor + 1)? as i8)
            } else {
                i32::from(*data.get(cursor + 1)?)
            };
            cursor = cursor.checked_add(2)?;
            (first, second)
        };

        let transform = if flags & WE_HAVE_A_TWO_BY_TWO != 0 {
            let value = Transform {
                a: read_f2dot14(data, cursor)?,
                b: read_f2dot14(data, cursor + 2)?,
                c: read_f2dot14(data, cursor + 4)?,
                d: read_f2dot14(data, cursor + 6)?,
            };
            cursor = cursor.checked_add(8)?;
            value
        } else if flags & WE_HAVE_AN_X_AND_Y_SCALE != 0 {
            let value = Transform {
                a: read_f2dot14(data, cursor)?,
                d: read_f2dot14(data, cursor + 2)?,
                ..Transform::IDENTITY
            };
            cursor = cursor.checked_add(4)?;
            value
        } else if flags & WE_HAVE_A_SCALE != 0 {
            let scale = read_f2dot14(data, cursor)?;
            cursor = cursor.checked_add(2)?;
            Transform {
                a: scale,
                d: scale,
                ..Transform::IDENTITY
            }
        } else {
            Transform::IDENTITY
        };

        let mut component = parse_true_type_glyph(face, component_glyph, depth + 1, stack)?;
        for point in component.iter_mut().flatten() {
            *point = transform.apply(*point);
        }

        let (mut dx, mut dy) = if xy_values {
            let mut offset = (arg1 as f32, arg2 as f32);
            if flags & SCALED_COMPONENT_OFFSET != 0 {
                if flags & UNSCALED_COMPONENT_OFFSET != 0 {
                    return None;
                }
                offset = transform.apply_vector(offset.0, offset.1);
            }
            offset
        } else {
            let parent_index = usize::try_from(arg1).ok()?;
            let component_index = usize::try_from(arg2).ok()?;
            let parent = result.iter().flatten().nth(parent_index).copied()?;
            let child = component.iter().flatten().nth(component_index).copied()?;
            (parent.x - child.x, parent.y - child.y)
        };
        if flags & ROUND_XY_TO_GRID != 0 {
            dx = dx.round();
            dy = dy.round();
        }
        for point in component.iter_mut().flatten() {
            point.x += dx;
            point.y += dy;
        }
        result.extend(component);
        final_flags = flags;
    }

    if final_flags & WE_HAVE_INSTRUCTIONS != 0 {
        let length = usize::from(read_u16(data, cursor)?);
        checked_slice(data, cursor.checked_add(2)?, length)?;
    }
    Some(result)
}

fn emit_contours(contours: &[Contour], builder: &mut impl OutlineBuilder) -> Option<Rect> {
    let mut bounds = Bounds::default();
    for contour in contours {
        emit_contour(contour, builder, &mut bounds);
    }
    bounds.to_rect()
}

fn emit_contour(contour: &[Point], builder: &mut impl OutlineBuilder, bounds: &mut Bounds) {
    let mut first_on_curve: Option<Point> = None;
    let mut first_off_curve: Option<Point> = None;
    let mut last_off_curve: Option<Point> = None;

    for &point in contour {
        if first_on_curve.is_none() {
            if point.on_curve {
                first_on_curve = Some(point);
                emit_move(builder, bounds, point);
            } else if let Some(first_off) = first_off_curve {
                let midpoint = first_off.midpoint(point);
                first_on_curve = Some(midpoint);
                last_off_curve = Some(point);
                emit_move(builder, bounds, midpoint);
            } else {
                first_off_curve = Some(point);
            }
        } else {
            match (last_off_curve, point.on_curve) {
                (Some(control), true) => {
                    last_off_curve = None;
                    emit_quad(builder, bounds, control, point);
                }
                (Some(control), false) => {
                    let midpoint = control.midpoint(point);
                    last_off_curve = Some(point);
                    emit_quad(builder, bounds, control, midpoint);
                }
                (None, true) => emit_line(builder, bounds, point),
                (None, false) => last_off_curve = Some(point),
            }
        }
    }

    if let (Some(first_off), Some(last_off)) = (first_off_curve, last_off_curve) {
        last_off_curve = None;
        emit_quad(builder, bounds, last_off, last_off.midpoint(first_off));
    }
    if let (Some(first_on), Some(first_off)) = (first_on_curve, first_off_curve) {
        emit_quad(builder, bounds, first_off, first_on);
    } else if let (Some(first_on), Some(last_off)) = (first_on_curve, last_off_curve) {
        emit_quad(builder, bounds, last_off, first_on);
    } else if let Some(first_on) = first_on_curve {
        emit_line(builder, bounds, first_on);
    }
    builder.close();
}

fn emit_move(builder: &mut impl OutlineBuilder, bounds: &mut Bounds, point: Point) {
    bounds.extend(point);
    builder.move_to(point.x, point.y);
}

fn emit_line(builder: &mut impl OutlineBuilder, bounds: &mut Bounds, point: Point) {
    bounds.extend(point);
    builder.line_to(point.x, point.y);
}

fn emit_quad(builder: &mut impl OutlineBuilder, bounds: &mut Bounds, control: Point, point: Point) {
    bounds.extend(control);
    bounds.extend(point);
    builder.quad_to(control.x, control.y, point.x, point.y);
}

#[derive(Default)]
struct Bounds {
    value: Option<(f32, f32, f32, f32)>,
}

impl Bounds {
    fn extend(&mut self, point: Point) {
        self.value = Some(match self.value {
            Some((x_min, y_min, x_max, y_max)) => (
                x_min.min(point.x),
                y_min.min(point.y),
                x_max.max(point.x),
                y_max.max(point.y),
            ),
            None => (point.x, point.y, point.x, point.y),
        });
    }

    fn to_rect(&self) -> Option<Rect> {
        let (x_min, y_min, x_max, y_max) = self.value?;
        Some(Rect {
            x_min: float_to_i16(x_min)?,
            y_min: float_to_i16(y_min)?,
            x_max: float_to_i16(x_max)?,
            y_max: float_to_i16(y_max)?,
        })
    }
}

fn float_to_i16(value: f32) -> Option<i16> {
    if value.is_finite() && value >= f32::from(i16::MIN) && value <= f32::from(i16::MAX) {
        Some(value as i16)
    } else {
        None
    }
}

fn read_f2dot14(data: &[u8], offset: usize) -> Option<f32> {
    read_i16(data, offset).map(|value| f32::from(value) / 16_384.0)
}

fn checked_slice(data: &[u8], offset: usize, length: usize) -> Option<&[u8]> {
    data.get(offset..offset.checked_add(length)?)
}

fn read_u16(data: &[u8], offset: usize) -> Option<u16> {
    let bytes = checked_slice(data, offset, 2)?;
    Some(u16::from_be_bytes([bytes[0], bytes[1]]))
}

fn read_i16(data: &[u8], offset: usize) -> Option<i16> {
    read_u16(data, offset).map(|value| value as i16)
}

fn read_u32(data: &[u8], offset: usize) -> Option<u32> {
    let bytes = checked_slice(data, offset, 4)?;
    Some(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

#[cfg(test)]
mod tests {
    use super::OutlineBuilder;
    use crate::sfnt::{Face, GlyphId};
    use fullbleed_audit_contract::sha256::Sha256;

    const INTER: &[u8] = include_bytes!("../python/fullbleed_assets/fonts/Inter-Variable.ttf");
    const NOTO: &[u8] = include_bytes!("../python/fullbleed_assets/fonts/NotoSans-Regular.ttf");
    const MATH: &[u8] = include_bytes!("../python/fullbleed_assets/fonts/NotoSansMath-Regular.ttf");
    const SYMBOLS: &[u8] =
        include_bytes!("../python/fullbleed_assets/fonts/NotoSansSymbols-Regular.ttf");
    const SYMBOLS2: &[u8] =
        include_bytes!("../python/fullbleed_assets/fonts/NotoSansSymbols2-Regular.ttf");

    #[derive(Clone, Copy, Debug)]
    enum Command {
        Move(f32, f32),
        Line(f32, f32),
        Quad(f32, f32, f32, f32),
        Curve(f32, f32, f32, f32, f32, f32),
        Close,
    }

    #[derive(Default)]
    struct Recorder(Vec<Command>);

    impl OutlineBuilder for Recorder {
        fn move_to(&mut self, x: f32, y: f32) {
            self.0.push(Command::Move(x, y));
        }

        fn line_to(&mut self, x: f32, y: f32) {
            self.0.push(Command::Line(x, y));
        }

        fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
            self.0.push(Command::Quad(x1, y1, x, y));
        }

        fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
            self.0.push(Command::Curve(x1, y1, x2, y2, x, y));
        }

        fn close(&mut self) {
            self.0.push(Command::Close);
        }
    }

    fn update_f32(contract: &mut Sha256, value: f32) {
        contract.update(&value.to_bits().to_be_bytes());
    }

    fn update_command(contract: &mut Sha256, command: Command) {
        match command {
            Command::Move(x, y) => {
                contract.update(&[0]);
                update_f32(contract, x);
                update_f32(contract, y);
            }
            Command::Line(x, y) => {
                contract.update(&[1]);
                update_f32(contract, x);
                update_f32(contract, y);
            }
            Command::Quad(x1, y1, x, y) => {
                contract.update(&[2]);
                for value in [x1, y1, x, y] {
                    update_f32(contract, value);
                }
            }
            Command::Curve(x1, y1, x2, y2, x, y) => {
                contract.update(&[3]);
                for value in [x1, y1, x2, y2, x, y] {
                    update_f32(contract, value);
                }
            }
            Command::Close => contract.update(&[4]),
        }
    }

    fn hex_digest(hasher: Sha256) -> String {
        hasher
            .finalize()
            .into_iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    #[test]
    fn every_bundled_true_type_outline_matches_frozen_contracts() {
        for (label, data) in [
            ("inter", INTER),
            ("noto sans", NOTO),
            ("noto math", MATH),
            ("noto symbols", SYMBOLS),
            ("noto symbols 2", SYMBOLS2),
        ] {
            let face = Face::parse(data, 0).expect("face");
            let mut contract = Sha256::new();
            contract.update(label.as_bytes());
            contract.update(&face.number_of_glyphs().to_be_bytes());
            for glyph in 0..face.number_of_glyphs() {
                contract.update(&glyph.to_be_bytes());
                let mut recorder = Recorder::default();
                if let Some(bbox) = face.outline_glyph(GlyphId(glyph), &mut recorder) {
                    contract.update(&[1]);
                    contract.update(&bbox.x_min.to_be_bytes());
                    contract.update(&bbox.y_min.to_be_bytes());
                    contract.update(&bbox.x_max.to_be_bytes());
                    contract.update(&bbox.y_max.to_be_bytes());
                } else {
                    contract.update(&[0]);
                }
                contract.update(&(recorder.0.len() as u32).to_be_bytes());
                for command in recorder.0 {
                    update_command(&mut contract, command);
                }
            }
            let expected = match label {
                "inter" => "27f21591c32f0f40ea10b94778b51cc3e2000e35f41bda7cd69a66a7e1a34ecb",
                "noto sans" => "f8827651f895774319414cdb3fcd4ad96f86c4fd880d7aab1ab25cf364f4c97e",
                "noto math" => "9df45f2097aa574aee696a95d8c75099c9702c2875ec3fdc884d32af557a82f3",
                "noto symbols" => {
                    "ad291061c08aff2bb778d8f005816120cbe406cb678b8f568061d0efaf943699"
                }
                "noto symbols 2" => {
                    "35dca8b7c11106f4fe3e49512c4f991cb3d13a0285d0f8696856fb3250e03f51"
                }
                _ => unreachable!("known bundled font"),
            };
            assert_eq!(
                hex_digest(contract),
                expected,
                "{label} TrueType outline contract"
            );
        }
    }
}
