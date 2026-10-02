//! Lower bounded PDF gradient functions to the native shading display list.
//! PDF 32000-1, 7.10 and 8.7.4: `sh` uses the current CTM/clip and ignores Background.
use super::*;
use crate::types::{Shading, ShadingStop};

type Result<T> = std::result::Result<T, FullBleedError>;
const MAX_FUNCTIONS: usize = 4096;
const MAX_STOPS: usize = 32768;

mod radial_bands;
#[cfg(test)]
mod tests;

fn invalid(message: impl Into<String>) -> FullBleedError {
    FullBleedError::InvalidConfiguration(format!("pdf raster shading error: {}", message.into()))
}

pub(super) fn numbers(doc: &LoDocument, object: &LoObject) -> Result<Vec<f64>> {
    let array = resolve_object(doc, object)?.as_array().map_err(pdf_err)?;
    if array.len() > MAX_STOPS {
        return Err(invalid("numeric array exceeds the gradient limit"));
    }
    array
        .iter()
        .map(|object| {
            let value = obj_to_f32(resolve_object(doc, object)?)
                .ok_or_else(|| invalid("expected a number"))?;
            if !value.is_finite() {
                return Err(invalid("nonfinite number"));
            }
            Ok(f64::from(value))
        })
        .collect()
}

fn pair(values: Vec<f64>) -> Result<[f64; 2]> {
    values
        .try_into()
        .map_err(|_| invalid("expected a pair of numbers"))
}

fn optional_numbers(doc: &LoDocument, dict: &LoDictionary, key: &[u8]) -> Result<Option<Vec<f64>>> {
    dict.get(key)
        .ok()
        .map(|value| numbers(doc, value))
        .transpose()
}

#[derive(Debug)]
struct Function {
    domain: [f64; 2],
    range: Option<Vec<f64>>,
    kind: FunctionKind,
    outputs: usize,
}

#[derive(Debug)]
enum FunctionKind {
    Exponential {
        c0: Vec<f64>,
        c1: Vec<f64>,
        exponent: f64,
    },
    Stitching {
        children: Vec<Function>,
        bounds: Vec<f64>,
        encode: Vec<f64>,
    },
}

impl Function {
    fn parse(doc: &LoDocument, object: &LoObject, depth: usize, count: &mut usize) -> Result<Self> {
        *count += 1;
        if depth > 16 || *count > MAX_FUNCTIONS {
            return Err(invalid(
                "function nesting or count exceeds the gradient limit",
            ));
        }
        let dict = resolve_object(doc, object)?.as_dict().map_err(pdf_err)?;
        let domain = pair(numbers(doc, dict.get(b"Domain").map_err(pdf_err)?)?)?;
        if domain[0] >= domain[1] {
            return Err(invalid("function domain must increase"));
        }
        let range = optional_numbers(doc, dict, b"Range")?;
        let function_type = resolve_object(doc, dict.get(b"FunctionType").map_err(pdf_err)?)?
            .as_i64()
            .map_err(pdf_err)?;
        let (kind, outputs) = match function_type {
            2 => {
                let c0 = optional_numbers(doc, dict, b"C0")?.unwrap_or_else(|| vec![0.0]);
                let c1 = optional_numbers(doc, dict, b"C1")?.unwrap_or_else(|| vec![1.0]);
                if c0.is_empty() || c0.len() > 4 || c0.len() != c1.len() {
                    return Err(invalid("inconsistent exponential function components"));
                }
                let exponent = resolved_obj_to_f32(doc, dict.get(b"N").map_err(pdf_err)?)
                    .ok_or_else(|| invalid("missing exponent"))?
                    as f64;
                if !exponent.is_finite() || exponent < 0.0 {
                    return Err(invalid("invalid exponential function exponent"));
                }
                let outputs = c0.len();
                (FunctionKind::Exponential { c0, c1, exponent }, outputs)
            }
            3 => {
                let children = resolve_object(doc, dict.get(b"Functions").map_err(pdf_err)?)?
                    .as_array()
                    .map_err(pdf_err)?;
                if children.is_empty() || children.len() > MAX_FUNCTIONS {
                    return Err(invalid("invalid stitching function count"));
                }
                let children = children
                    .iter()
                    .map(|child| Self::parse(doc, child, depth + 1, count))
                    .collect::<Result<Vec<_>>>()?;
                let outputs = children[0].outputs;
                let bounds = numbers(doc, dict.get(b"Bounds").map_err(pdf_err)?)?;
                let encode = numbers(doc, dict.get(b"Encode").map_err(pdf_err)?)?;
                if children.iter().any(|child| child.outputs != outputs)
                    || bounds.len() + 1 != children.len()
                    || encode.len() != children.len() * 2
                    || bounds.windows(2).any(|pair| pair[0] > pair[1])
                    || bounds
                        .iter()
                        .any(|value| *value < domain[0] || *value > domain[1])
                {
                    return Err(invalid(
                        "inconsistent stitching function bounds or components",
                    ));
                }
                (
                    FunctionKind::Stitching {
                        children,
                        bounds,
                        encode,
                    },
                    outputs,
                )
            }
            other => {
                return Err(invalid(format!(
                    "unsupported gradient FunctionType {other}"
                )));
            }
        };
        if let Some(range) = &range {
            if range.len() != outputs * 2 || range.chunks_exact(2).any(|pair| pair[0] > pair[1]) {
                return Err(invalid("invalid function range"));
            }
        }
        Ok(Self {
            domain,
            range,
            kind,
            outputs,
        })
    }

    fn evaluate(&self, input: f64, left_limit: bool) -> Result<Vec<f64>> {
        let x = input.clamp(self.domain[0], self.domain[1]);
        let mut values = match &self.kind {
            FunctionKind::Exponential { c0, c1, exponent } => {
                let factor = x.powf(*exponent);
                c0.iter()
                    .zip(c1)
                    .map(|(start, end)| start + factor * (end - start))
                    .collect()
            }
            FunctionKind::Stitching {
                children,
                bounds,
                encode,
            } => {
                let index = bounds
                    .partition_point(|bound| if left_limit { x > *bound } else { x >= *bound });
                let low = if index == 0 {
                    self.domain[0]
                } else {
                    bounds[index - 1]
                };
                let high = bounds.get(index).copied().unwrap_or(self.domain[1]);
                let e0 = encode[2 * index];
                let e1 = encode[2 * index + 1];
                let mapped = if high == low {
                    e1
                } else {
                    e0 + (x - low) * (e1 - e0) / (high - low)
                };
                children[index].evaluate(mapped, if e1 >= e0 { left_limit } else { !left_limit })?
            }
        };
        for (index, value) in values.iter_mut().enumerate() {
            if !value.is_finite() {
                return Err(invalid("function produced a nonfinite component"));
            }
            if let Some(range) = &self.range {
                *value = value.clamp(range[index * 2], range[index * 2 + 1]);
            }
        }
        Ok(values)
    }

    // Propagate every piece boundary through Domain/Encode mappings. Uniform
    // sampling alone would miss narrow color bands and hard discontinuities.
    fn knots(
        &self,
        scale: f64,
        offset: f64,
        low: f64,
        high: f64,
        out: &mut Vec<f64>,
    ) -> Result<()> {
        if scale == 0.0 {
            return Ok(());
        }
        let mut add = |value: f64| {
            let t = (value - offset) / scale;
            if t > low && t < high {
                out.push(t);
            }
        };
        for value in self.domain {
            add(value);
        }
        if let FunctionKind::Stitching {
            children,
            bounds,
            encode,
        } = &self.kind
        {
            for &value in bounds {
                add(value);
            }
            for (index, child) in children.iter().enumerate() {
                let x0 = if index == 0 {
                    self.domain[0]
                } else {
                    bounds[index - 1]
                };
                let x1 = bounds.get(index).copied().unwrap_or(self.domain[1]);
                if x0 == x1 {
                    continue;
                }
                let a = (x0 - offset) / scale;
                let b = (x1 - offset) / scale;
                let branch_low = low.max(a.min(b));
                let branch_high = high.min(a.max(b));
                if branch_low >= branch_high {
                    continue;
                }
                let factor = (encode[2 * index + 1] - encode[2 * index]) / (x1 - x0);
                child.knots(
                    scale * factor,
                    (offset - x0) * factor + encode[2 * index],
                    branch_low,
                    branch_high,
                    out,
                )?;
            }
        }
        if out.len() > MAX_STOPS {
            return Err(invalid("too many function boundaries"));
        }
        Ok(())
    }
}

struct ColorFunction {
    functions: Vec<Function>,
    domain: [f64; 2],
    space: RasterDirectColor,
}

impl ColorFunction {
    fn color(&self, t: f64, left: bool) -> Result<Color> {
        let x = self.domain[0] + t * (self.domain[1] - self.domain[0]);
        let mut values = Vec::with_capacity(4);
        for function in &self.functions {
            values.extend(function.evaluate(x, left)?);
        }
        let values: Vec<_> = values.into_iter().map(|value| value as f32).collect();
        color_from_direct_components(self.space, &values)
            .ok_or_else(|| invalid("invalid gradient color"))
    }

    fn refine(
        &self,
        a: f64,
        b: f64,
        ca: Color,
        cb: Color,
        depth: usize,
        stops: &mut Vec<ShadingStop>,
    ) -> Result<()> {
        let mut error = 0.0f32;
        for fraction in [0.25, 0.5, 0.75] {
            let actual = self.color(a + (b - a) * fraction, false)?;
            for (actual, start, end) in [
                (actual.r, ca.r, cb.r),
                (actual.g, ca.g, cb.g),
                (actual.b, ca.b, cb.b),
            ] {
                error = error.max((actual - (start + (end - start) * fraction as f32)).abs());
            }
        }
        if error > 1.0 / 2048.0 {
            if depth >= 20 || stops.len() >= MAX_STOPS {
                return Err(invalid("gradient interpolation limit exceeded"));
            }
            let mid = (a + b) * 0.5;
            let color = self.color(mid, false)?;
            self.refine(a, mid, ca, color, depth + 1, stops)?;
            self.refine(mid, b, color, cb, depth + 1, stops)?;
        } else {
            if stops.len() >= MAX_STOPS {
                return Err(invalid("too many gradient stops"));
            }
            stops.push(ShadingStop {
                offset: b as f32,
                color: cb,
                alpha: 1.0,
            });
        }
        Ok(())
    }

    fn stops(&self) -> Result<Vec<ShadingStop>> {
        let mut knots = vec![0.0, 1.0];
        for function in &self.functions {
            function.knots(
                self.domain[1] - self.domain[0],
                self.domain[0],
                0.0,
                1.0,
                &mut knots,
            )?;
        }
        knots.sort_by(f64::total_cmp);
        knots.dedup();
        let mut stops = Vec::new();
        for interval in knots.windows(2) {
            let ca = self.color(interval[0], false)?;
            let cb = self.color(interval[1], true)?;
            stops.push(ShadingStop {
                offset: interval[0] as f32,
                color: ca,
                alpha: 1.0,
            });
            self.refine(interval[0], interval[1], ca, cb, 0, &mut stops)?;
        }
        Ok(stops)
    }
}

fn inverse(matrix: Matrix) -> Option<Matrix> {
    let det = f64::from(matrix.a) * f64::from(matrix.d) - f64::from(matrix.b) * f64::from(matrix.c);
    if !det.is_finite() || det.abs() <= f64::EPSILON {
        return None;
    }
    let a = (f64::from(matrix.d) / det) as f32;
    let b = (-f64::from(matrix.b) / det) as f32;
    let c = (-f64::from(matrix.c) / det) as f32;
    let d = (f64::from(matrix.a) / det) as f32;
    let e = -a * matrix.e - c * matrix.f;
    let f = -b * matrix.e - d * matrix.f;
    [a, b, c, d, e, f]
        .iter()
        .all(|x| x.is_finite())
        .then_some(Matrix::from_operands(a, b, c, d, e, f))
}

fn clip_half_plane(
    polygon: &[(f32, f32)],
    x0: f32,
    y0: f32,
    dx: f32,
    dy: f32,
    bound: f32,
    keep_greater: bool,
) -> Vec<(f32, f32)> {
    let mut out = Vec::new();
    if polygon.is_empty() {
        return out;
    }
    let side = |point: (f32, f32)| {
        ((point.0 - x0) * dx + (point.1 - y0) * dy - bound) * if keep_greater { 1.0 } else { -1.0 }
    };
    let mut previous = *polygon.last().unwrap();
    for &current in polygon {
        let a = side(previous);
        let b = side(current);
        if (a >= 0.0) != (b >= 0.0) {
            let fraction = a / (a - b);
            out.push((
                previous.0 + (current.0 - previous.0) * fraction,
                previous.1 + (current.1 - previous.1) * fraction,
            ));
        }
        if b >= 0.0 {
            out.push(current);
        }
        previous = current;
    }
    out
}

pub(super) fn paint(
    doc: &LoDocument,
    object: &LoObject,
    resources: &PdfResources,
    state: &ParseState,
    width: f32,
    height: f32,
    commands: &mut Vec<Command>,
) -> Result<()> {
    let dict = resolve_object(doc, object)?.as_dict().map_err(pdf_err)?;
    let kind = resolve_object(doc, dict.get(b"ShadingType").map_err(pdf_err)?)?
        .as_i64()
        .map_err(pdf_err)?;
    if kind != 1 && kind != 2 && kind != 3 {
        return Err(invalid(format!("unsupported ShadingType {kind}")));
    }
    let raw_space = resolve_object(doc, dict.get(b"ColorSpace").map_err(pdf_err)?)?;
    let space = if let Ok(name) = raw_space.as_name() {
        resolve_named_color_space(resources, &name_bytes_to_string(name))
    } else {
        parse_raster_color_space(doc, raw_space)
    };
    let Some(RasterColorSpace::Direct(space)) = space else {
        return Err(invalid("unsupported gradient color space"));
    };
    if kind == 1 {
        return radial_bands::paint(doc, dict, space, state, height, commands);
    }
    let domain = pair(optional_numbers(doc, dict, b"Domain")?.unwrap_or_else(|| vec![0.0, 1.0]))?;
    if domain[0] >= domain[1] {
        return Err(invalid("shading domain must increase"));
    }
    let object = resolve_object(doc, dict.get(b"Function").map_err(pdf_err)?)?;
    let mut count = 0;
    let functions = if let LoObject::Array(array) = object {
        if array.len() != space.channels() {
            return Err(invalid(
                "gradient function array must match the color space",
            ));
        }
        let functions = array
            .iter()
            .map(|object| Function::parse(doc, object, 0, &mut count))
            .collect::<Result<Vec<_>>>()?;
        if functions.iter().any(|function| function.outputs != 1) {
            return Err(invalid("component functions must have one output"));
        }
        functions
    } else {
        let function = Function::parse(doc, object, 0, &mut count)?;
        if function.outputs != space.channels() {
            return Err(invalid(
                "gradient function outputs must match the color space",
            ));
        }
        vec![function]
    };
    let stops = ColorFunction {
        functions,
        domain,
        space,
    }
    .stops()?;
    let coords = numbers(doc, dict.get(b"Coords").map_err(pdf_err)?)?;
    if coords.len() != if kind == 2 { 4 } else { 6 } {
        return Err(invalid("invalid shading coordinate count"));
    }
    let coords: Vec<_> = coords.into_iter().map(|x| x as f32).collect();
    let extend = match dict.get(b"Extend") {
        Ok(object) => {
            let array = resolve_object(doc, object)?.as_array().map_err(pdf_err)?;
            if array.len() != 2 {
                return Err(invalid("invalid Extend array"));
            }
            let mut extend = [false; 2];
            for (index, object) in array.iter().enumerate() {
                let LoObject::Boolean(value) = resolve_object(doc, object)? else {
                    return Err(invalid("Extend entries must be booleans"));
                };
                extend[index] = *value;
            }
            extend
        }
        Err(_) => [false, false],
    };
    if kind == 3 && extend != [true, true] {
        return Err(invalid("nonextended radial shading is not supported"));
    }
    let Some(inverse) = inverse(state.ctm) else {
        return Ok(());
    };
    let mut clip = [(0.0, 0.0), (width, 0.0), (width, height), (0.0, height)]
        .into_iter()
        .map(|(x, y)| inverse.transform_point(x, y))
        .collect::<Vec<_>>();
    let shading = if kind == 2 {
        let [x0, y0, x1, y1] = [coords[0], coords[1], coords[2], coords[3]];
        let dx = x1 - x0;
        let dy = y1 - y0;
        let length = dx * dx + dy * dy;
        if length <= f32::EPSILON {
            return Ok(());
        }
        if !extend[0] {
            clip = clip_half_plane(&clip, x0, y0, dx, dy, 0.0, true);
        }
        if !extend[1] {
            clip = clip_half_plane(&clip, x0, y0, dx, dy, length, false);
        }
        Shading::Axial {
            x0,
            y0: height - y0,
            x1,
            y1: height - y1,
            stops,
        }
    } else {
        if coords[2] < 0.0 || coords[5] < 0.0 {
            return Err(invalid("negative radial radius"));
        }
        Shading::Radial {
            x0: coords[0],
            y0: height - coords[1],
            r0: coords[2],
            x1: coords[3],
            y1: height - coords[4],
            r1: coords[5],
            stops,
            hard_stops: false,
        }
    };
    if clip.len() < 3 {
        return Ok(());
    }
    commands.push(Command::SaveState);
    commands.push(Command::ConcatMatrix {
        a: state.ctm.a,
        b: -state.ctm.b,
        c: -state.ctm.c,
        d: state.ctm.d,
        e: Pt::from_f32(state.ctm.e),
        f: Pt::from_f32(-state.ctm.f),
    });
    for (index, (x, y)) in clip.into_iter().enumerate() {
        commands.push(if index == 0 {
            Command::MoveTo {
                x: Pt::from_f32(x),
                y: Pt::from_f32(height - y),
            }
        } else {
            Command::LineTo {
                x: Pt::from_f32(x),
                y: Pt::from_f32(height - y),
            }
        });
    }
    commands.push(Command::ClosePath);
    commands.push(Command::ClipPath { evenodd: false });
    if let Some(bbox) = optional_numbers(doc, dict, b"BBox")? {
        if bbox.len() != 4 || bbox[0] >= bbox[2] || bbox[1] >= bbox[3] {
            return Err(invalid("invalid shading BBox"));
        }
        commands.push(Command::ClipRect {
            x: Pt::from_f32(bbox[0] as f32),
            y: Pt::from_f32(height - bbox[3] as f32),
            width: Pt::from_f32((bbox[2] - bbox[0]) as f32),
            height: Pt::from_f32((bbox[3] - bbox[1]) as f32),
        });
    }
    commands.push(Command::ShadingFill(shading));
    commands.push(Command::RestoreState);
    Ok(())
}
