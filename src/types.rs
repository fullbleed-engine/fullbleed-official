/// Signed Q32.32 fixed-point value used for deterministic layout scale factors.
///
/// Arithmetic saturates at the representable range instead of changing behavior between debug
/// and release builds on overflow.
#[derive(Debug, Clone, Copy, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct I32F32(i64);

impl I32F32 {
    const FRACTION_BITS: u32 = 32;
    const SCALE: i128 = 1i128 << Self::FRACTION_BITS;

    pub const fn from_bits(bits: i64) -> Self {
        Self(bits)
    }

    pub const fn to_bits(self) -> i64 {
        self.0
    }

    pub fn from_num(value: f32) -> Self {
        if !value.is_finite() {
            return Self::default();
        }
        let scaled = (f64::from(value) * Self::SCALE as f64).round_ties_even();
        Self(scaled.clamp(i64::MIN as f64, i64::MAX as f64) as i64)
    }

    pub fn to_f32(self) -> f32 {
        (self.0 as f64 / Self::SCALE as f64) as f32
    }

    pub fn round(self) -> Self {
        let fraction_mask = (Self::SCALE - 1) as i64;
        let floor = self.0 & !fraction_mask;
        let fraction = self.0 & fraction_mask;
        let half = (Self::SCALE / 2) as i64;
        if fraction < half || (fraction == half && self.0 < 0) {
            Self(floor)
        } else {
            Self(floor.saturating_add(Self::SCALE as i64))
        }
    }

    pub const fn to_i64_floor(self) -> i64 {
        self.0 >> Self::FRACTION_BITS
    }
}

impl std::ops::Add for I32F32 {
    type Output = Self;

    fn add(self, rhs: Self) -> Self {
        Self(self.0.saturating_add(rhs.0))
    }
}

impl std::ops::Sub for I32F32 {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self {
        Self(self.0.saturating_sub(rhs.0))
    }
}

impl std::ops::Mul for I32F32 {
    type Output = Self;

    fn mul(self, rhs: Self) -> Self {
        let product = (self.0 as i128 * rhs.0 as i128) >> Self::FRACTION_BITS;
        Self(product.clamp(i64::MIN as i128, i64::MAX as i128) as i64)
    }
}

impl std::ops::Neg for I32F32 {
    type Output = Self;

    fn neg(self) -> Self {
        Self(self.0.saturating_neg())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct Pt(I32F32);

impl Pt {
    pub const ZERO: Pt = Pt(I32F32::from_bits(0));

    pub fn from_f32(value: f32) -> Pt {
        if !value.is_finite() {
            return Pt::ZERO;
        }
        let milli = (value as f64 * 1000.0).round();
        let milli = milli.clamp(i64::MIN as f64, i64::MAX as f64) as i64;
        Pt::from_milli_i64(milli)
    }

    pub fn from_i32(value: i32) -> Pt {
        Pt::from_milli_i64((value as i64) * 1000)
    }

    pub fn to_f32(self) -> f32 {
        self.0.to_f32()
    }

    pub fn to_milli_i64(self) -> i64 {
        let bits = self.0.to_bits() as i128;
        let denom = 1i128 << 32;
        let scaled = bits * 1000;
        let adj = if scaled >= 0 { denom / 2 } else { -denom / 2 };
        let milli = (scaled + adj) / denom;
        milli.clamp(i64::MIN as i128, i64::MAX as i128) as i64
    }

    pub fn max(self, other: Pt) -> Pt {
        if self >= other { self } else { other }
    }

    pub fn min(self, other: Pt) -> Pt {
        if self <= other { self } else { other }
    }

    pub fn abs(self) -> Pt {
        if self.to_milli_i64() < 0 { -self } else { self }
    }

    pub fn mul_fixed(self, factor: I32F32) -> Pt {
        Pt(self.0 * factor)
    }

    pub fn mul_ratio(self, num: i32, denom: i32) -> Pt {
        if denom == 0 {
            return Pt::ZERO;
        }
        let milli = self.to_milli_i64() as i128;
        let num = num as i128;
        let denom = denom as i128;
        let value = div_round_i128(milli.saturating_mul(num), denom);
        Pt::from_milli_i128(value)
    }

    pub fn from_milli_i64(milli: i64) -> Pt {
        Pt::from_milli_i128(milli as i128)
    }

    fn from_milli_i128(milli: i128) -> Pt {
        let denom = 1i128 << 32;
        let adj = if milli >= 0 { 500 } else { -500 };
        let bits = (milli * denom + adj) / 1000;
        let bits = bits.clamp(i64::MIN as i128, i64::MAX as i128) as i64;
        Pt(I32F32::from_bits(bits))
    }
}

impl std::ops::Add for Pt {
    type Output = Pt;
    fn add(self, rhs: Pt) -> Pt {
        Pt::from_milli_i128(self.to_milli_i64() as i128 + rhs.to_milli_i64() as i128)
    }
}

impl std::ops::AddAssign for Pt {
    fn add_assign(&mut self, rhs: Pt) {
        *self = *self + rhs;
    }
}

impl std::ops::Sub for Pt {
    type Output = Pt;
    fn sub(self, rhs: Pt) -> Pt {
        Pt::from_milli_i128(self.to_milli_i64() as i128 - rhs.to_milli_i64() as i128)
    }
}

impl std::ops::SubAssign for Pt {
    fn sub_assign(&mut self, rhs: Pt) {
        *self = *self - rhs;
    }
}

impl std::ops::Mul<i32> for Pt {
    type Output = Pt;
    fn mul(self, rhs: i32) -> Pt {
        let milli = self.to_milli_i64() as i128;
        Pt::from_milli_i128(milli.saturating_mul(rhs as i128))
    }
}

impl std::ops::Div<i32> for Pt {
    type Output = Pt;
    fn div(self, rhs: i32) -> Pt {
        if rhs == 0 {
            Pt::ZERO
        } else {
            let milli = self.to_milli_i64() as i128;
            let value = div_round_i128(milli, rhs as i128);
            Pt::from_milli_i128(value)
        }
    }
}

impl std::ops::Mul<f32> for Pt {
    type Output = Pt;
    fn mul(self, rhs: f32) -> Pt {
        if !rhs.is_finite() {
            return Pt::ZERO;
        }
        Pt::from_f32(self.to_f32() * rhs)
    }
}

impl std::ops::Div<f32> for Pt {
    type Output = Pt;
    fn div(self, rhs: f32) -> Pt {
        if rhs == 0.0 || !rhs.is_finite() {
            Pt::ZERO
        } else {
            Pt::from_f32(self.to_f32() / rhs)
        }
    }
}

fn div_round_i128(num: i128, den: i128) -> i128 {
    if den == 0 {
        return 0;
    }
    let den_abs = den.abs();
    if num >= 0 {
        (num + (den_abs / 2)) / den
    } else {
        -(((-num) + (den_abs / 2)) / den)
    }
}

impl std::ops::Neg for Pt {
    type Output = Pt;
    fn neg(self) -> Pt {
        Pt::from_milli_i128(-(self.to_milli_i64() as i128))
    }
}

impl std::iter::Sum for Pt {
    fn sum<I: Iterator<Item = Pt>>(iter: I) -> Pt {
        iter.fold(Pt::ZERO, |acc, v| acc + v)
    }
}

impl<'a> std::iter::Sum<&'a Pt> for Pt {
    fn sum<I: Iterator<Item = &'a Pt>>(iter: I) -> Pt {
        iter.fold(Pt::ZERO, |acc, v| acc + *v)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Size {
    pub width: Pt,
    pub height: Pt,
}

impl Size {
    pub fn a4() -> Self {
        Self {
            width: Pt::from_f32(595.28),
            height: Pt::from_f32(841.89),
        }
    }

    pub fn letter() -> Self {
        // 8.5in x 11in at 72pt/in.
        Self {
            width: Pt::from_f32(612.0),
            height: Pt::from_f32(792.0),
        }
    }

    pub fn from_inches(width_in: f32, height_in: f32) -> Self {
        Self {
            width: Pt::from_f32(width_in * 72.0),
            height: Pt::from_f32(height_in * 72.0),
        }
    }

    pub fn from_mm(width_mm: f32, height_mm: f32) -> Self {
        Self {
            width: Pt::from_f32(width_mm * 72.0 / 25.4),
            height: Pt::from_f32(height_mm * 72.0 / 25.4),
        }
    }

    pub fn quantized(self) -> Self {
        Self {
            width: self.width,
            height: self.height,
        }
    }
}

/// Final sheet orientation applied after layout. Keeping this out of the
/// layout coordinate system lets compiled pages and variable-data overlays
/// reuse the same display list regardless of how the physical sheet is
/// presented.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum PageOrientation {
    #[default]
    Upright,
    RotateLeft,
    RotateRight,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct PageMarks {
    pub crop: bool,
    pub cross: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct PagePresentation {
    pub bleed: Pt,
    pub marks: PageMarks,
    pub orientation: PageOrientation,
}

impl Default for PagePresentation {
    fn default() -> Self {
        Self {
            bleed: Pt::ZERO,
            marks: PageMarks::default(),
            orientation: PageOrientation::Upright,
        }
    }
}

impl PagePresentation {
    /// CSS marks reserve eight CSS pixels (six PDF points) when the authored
    /// bleed is smaller. This matches the print geometry used by browsers and
    /// leaves layout coordinates anchored at the trim box.
    pub(crate) fn media_extent(self) -> Pt {
        let authored = self.bleed.max(Pt::ZERO);
        let marks_default = if self.marks.crop || self.marks.cross {
            Pt::from_f32(6.0)
        } else {
            Pt::ZERO
        };
        if authored > marks_default {
            authored
        } else {
            marks_default
        }
    }

    pub(crate) fn encode(self) -> String {
        format!(
            "{},{},{},{}",
            self.bleed.max(Pt::ZERO).to_milli_i64(),
            u8::from(self.marks.crop),
            u8::from(self.marks.cross),
            match self.orientation {
                PageOrientation::Upright => 0,
                PageOrientation::RotateLeft => 1,
                PageOrientation::RotateRight => 2,
            }
        )
    }

    pub(crate) fn decode(raw: &str) -> Option<Self> {
        let mut values = raw.split(',');
        let bleed = Pt::from_milli_i64(values.next()?.parse().ok()?).max(Pt::ZERO);
        let crop = values.next()? == "1";
        let cross = values.next()? == "1";
        let orientation = match values.next()? {
            "0" => PageOrientation::Upright,
            "1" => PageOrientation::RotateLeft,
            "2" => PageOrientation::RotateRight,
            _ => return None,
        };
        if values.next().is_some() {
            return None;
        }
        Some(Self {
            bleed,
            marks: PageMarks { crop, cross },
            orientation,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: Pt,
    pub y: Pt,
    pub width: Pt,
    pub height: Pt,
}

impl Rect {
    pub fn quantized(self) -> Self {
        Self {
            x: self.x,
            y: self.y,
            width: self.width,
            height: self.height,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Margins {
    pub top: Pt,
    pub right: Pt,
    pub bottom: Pt,
    pub left: Pt,
}

impl Margins {
    pub fn all(value: f32) -> Self {
        let v = Pt::from_f32(value);
        Self {
            top: v,
            right: v,
            bottom: v,
            left: v,
        }
    }

    pub fn quantized(self) -> Self {
        Self {
            top: self.top,
            right: self.right,
            bottom: self.bottom,
            left: self.left,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
}

impl Color {
    pub const BLACK: Color = Color {
        r: 0.0,
        g: 0.0,
        b: 0.0,
    };

    /// Internal paint sentinel for a fully transparent CSS border color.
    /// Border geometry still participates in layout and conflict resolution,
    /// while the paint layer leaves the cell background visible underneath.
    pub const TRANSPARENT: Color = Color {
        r: -1.0,
        g: -1.0,
        b: -1.0,
    };

    pub fn rgb(r: f32, g: f32, b: f32) -> Self {
        Self { r, g, b }
    }

    pub(crate) fn is_transparent(self) -> bool {
        self == Self::TRANSPARENT
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorSpace {
    Rgb,
    Cmyk,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoxSizingMode {
    ContentBox,
    BorderBox,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MixBlendMode {
    Normal,
    Multiply,
    Screen,
    Overlay,
    Darken,
    Lighten,
    ColorDodge,
    ColorBurn,
    HardLight,
    SoftLight,
    Difference,
    Exclusion,
    Hue,
    Saturation,
    Color,
    Luminosity,
    PlusLighter,
    PlusDarker,
}

impl Default for MixBlendMode {
    fn default() -> Self {
        Self::Normal
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShadingStop {
    pub offset: f32, // 0..=1
    pub color: Color,
    pub alpha: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Shading {
    // Axial (linear) shading: (x0,y0) -> (x1,y1), with 0..1 stops.
    Axial {
        x0: f32,
        y0: f32,
        x1: f32,
        y1: f32,
        stops: Vec<ShadingStop>,
    },
    // Radial shading: (x0,y0,r0) -> (x1,y1,r1), with 0..1 stops.
    Radial {
        x0: f32,
        y0: f32,
        r0: f32,
        x1: f32,
        y1: f32,
        r1: f32,
        stops: Vec<ShadingStop>,
        hard_stops: bool,
    },
    // Compact sweep/conic shader IR. Backends execute this analytically or
    // lower it once at emission time instead of expanding hundreds of wedges
    // into every compiled page program.
    Conic {
        center_x: f32,
        center_y: f32,
        radius: f32,
        start_angle_deg: f32,
        stops: Vec<ShadingStop>,
        hard_stops: bool,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn q32_32_rounds_ties_away_from_zero() {
        assert_eq!(I32F32::from_num(2.5).round().to_i64_floor(), 3);
        assert_eq!(I32F32::from_num(-2.5).round().to_i64_floor(), -3);
        assert_eq!(I32F32::from_num(2.49).round().to_i64_floor(), 2);
        assert_eq!(I32F32::from_num(-2.49).round().to_i64_floor(), -2);
    }

    #[test]
    fn q32_32_multiplication_preserves_fractional_scales() {
        let result = I32F32::from_num(12.5) * I32F32::from_num(0.8);
        assert!((result.to_f32() - 10.0).abs() <= f32::EPSILON);
    }

    #[test]
    fn q32_32_arithmetic_saturates_deterministically() {
        assert_eq!(
            (I32F32::from_bits(i64::MAX) + I32F32::from_bits(1)).to_bits(),
            i64::MAX
        );
        assert_eq!(
            (I32F32::from_bits(i64::MIN) - I32F32::from_bits(1)).to_bits(),
            i64::MIN
        );
    }

    #[test]
    fn point_quantization_remains_symmetric_at_millipoint_precision() {
        for milli in [-12_345, -1, 0, 1, 12_345] {
            let point = Pt::from_milli_i64(milli);
            assert_eq!(point.to_milli_i64(), milli);
        }
        assert_eq!(Pt::from_f32(1.2345).to_milli_i64(), 1_235);
        assert_eq!(Pt::from_f32(-1.2345).to_milli_i64(), -1_235);
    }
}
