//! Floating-point operations used by geometry and PDF transforms.
//!
//! Keeping these calls behind one module makes the numeric contract explicit and avoids a Cargo
//! dependency for operations already supplied by Rust's target-aware standard library.

#[inline]
pub(crate) fn sin_cos(value: f32) -> (f32, f32) {
    value.sin_cos()
}

#[inline]
pub(crate) fn sqrt(value: f32) -> f32 {
    value.sqrt()
}

#[inline]
pub(crate) fn atan2(y: f32, x: f32) -> f32 {
    y.atan2(x)
}

#[inline]
pub(crate) fn ceil(value: f32) -> f32 {
    value.ceil()
}

#[inline]
pub(crate) fn floor(value: f32) -> f32 {
    value.floor()
}

#[inline]
pub(crate) fn tan(value: f32) -> f32 {
    value.tan()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::{FRAC_PI_2, FRAC_PI_4, PI};

    fn close(actual: f32, expected: f32, tolerance: f32) {
        assert!(
            (actual - expected).abs() <= tolerance,
            "actual={actual:?}, expected={expected:?}, tolerance={tolerance:?}"
        );
    }

    #[test]
    fn trigonometry_covers_geometry_quadrants() {
        for (angle, expected_sin, expected_cos) in [
            (0.0, 0.0, 1.0),
            (FRAC_PI_2, 1.0, 0.0),
            (PI, 0.0, -1.0),
            (-FRAC_PI_2, -1.0, 0.0),
        ] {
            let (sin, cos) = sin_cos(angle);
            close(sin, expected_sin, 2.0e-6);
            close(cos, expected_cos, 2.0e-6);
        }
        close(tan(FRAC_PI_4), 1.0, 2.0e-6);
    }

    #[test]
    fn roots_angles_and_rounding_cover_svg_arc_inputs() {
        assert_eq!(sqrt(0.0).to_bits(), 0.0f32.to_bits());
        assert_eq!(sqrt(4.0), 2.0);
        close(atan2(1.0, 0.0), FRAC_PI_2, f32::EPSILON);
        close(atan2(0.0, -1.0).abs(), PI, f32::EPSILON);
        assert_eq!(ceil(1.01), 2.0);
        assert_eq!(ceil(-1.01), -1.0);
        assert_eq!(floor(1.99), 1.0);
        assert_eq!(floor(-1.01), -2.0);
    }
}
