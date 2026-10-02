//! Recognize the bounded radial-band calculator emitted by Fullbleed's writer.
//! This is intentionally not a general PostScript calculator interpreter.
use super::*;

struct Tokens<'a> {
    values: Vec<&'a str>,
    index: usize,
}

impl<'a> Tokens<'a> {
    fn take(&mut self) -> Result<&'a str> {
        let token = self
            .values
            .get(self.index)
            .copied()
            .ok_or_else(|| invalid("incomplete radial-band function"))?;
        self.index += 1;
        Ok(token)
    }
    fn expect(&mut self, expected: &str) -> Result<()> {
        if self.take()? != expected {
            return Err(invalid("unsupported radial-band calculator program"));
        }
        Ok(())
    }
    fn number(&mut self) -> Result<f32> {
        let number = self
            .take()?
            .parse::<f32>()
            .map_err(|_| invalid("invalid radial-band number"))?;
        if !number.is_finite() {
            return Err(invalid("nonfinite radial-band number"));
        }
        Ok(number)
    }
    fn color(&mut self, space: RasterDirectColor) -> Result<Color> {
        self.expect("pop")?;
        let components = (0..space.channels())
            .map(|_| self.number())
            .collect::<Result<Vec<_>>>()?;
        color_from_components_in_space(&components, Some(&RasterColorSpace::Direct(space)))
            .ok_or_else(|| invalid("unsupported radial-band color"))
    }
}

pub(super) fn paint(
    doc: &LoDocument,
    dict: &LoDictionary,
    space: RasterDirectColor,
    state: &ParseState,
    height: f32,
    commands: &mut Vec<Command>,
) -> Result<()> {
    let function = resolve_object(doc, dict.get(b"Function").map_err(pdf_err)?)?
        .as_stream()
        .map_err(|_| invalid("unsupported function-based shading"))?;
    if resolve_object(doc, function.dict.get(b"FunctionType").map_err(pdf_err)?)?
        .as_i64()
        .ok()
        != Some(4)
    {
        return Err(invalid("unsupported function-based shading"));
    }
    let domain =
        optional_numbers(doc, dict, b"Domain")?.unwrap_or_else(|| vec![0.0, 1.0, 0.0, 1.0]);
    if domain.len() != 4
        || domain[0] >= domain[1]
        || domain[2] >= domain[3]
        || numbers(doc, function.dict.get(b"Domain").map_err(pdf_err)?)? != domain
    {
        return Err(invalid("unsupported radial-band domain"));
    }
    let range = numbers(doc, function.dict.get(b"Range").map_err(pdf_err)?)?;
    if range
        != (0..space.channels())
            .flat_map(|_| [0.0, 1.0])
            .collect::<Vec<_>>()
    {
        return Err(invalid("unsupported radial-band range"));
    }
    let bytes = function.get_plain_content().map_err(pdf_err)?;
    if bytes.len() > 1_048_576 {
        return Err(invalid("radial-band program exceeds the gradient limit"));
    }
    let program =
        std::str::from_utf8(&bytes).map_err(|_| invalid("invalid radial-band program encoding"))?;
    let spaced = program.replace('{', " { ").replace('}', " } ");
    let mut tokens = Tokens {
        values: spaced.split_whitespace().collect(),
        index: 0,
    };
    for word in ["{", "dup", "mul", "exch", "dup", "mul", "add", "sqrt"] {
        tokens.expect(word)?;
    }
    let mut stops = Vec::new();
    let mut start = 0.0;
    let mut levels = 0;
    while tokens.values.get(tokens.index) == Some(&"dup") {
        tokens.expect("dup")?;
        let end = tokens.number()?;
        if end < start || end > 1.0 || levels >= MAX_FUNCTIONS {
            return Err(invalid("invalid radial-band boundary"));
        }
        for word in ["le", "{"] {
            tokens.expect(word)?;
        }
        let color = tokens.color(space)?;
        stops.extend([
            ShadingStop {
                offset: start,
                color,
                alpha: 1.0,
            },
            ShadingStop {
                offset: end,
                color,
                alpha: 1.0,
            },
        ]);
        for word in ["}", "{"] {
            tokens.expect(word)?;
        }
        start = end;
        levels += 1;
    }
    let color = tokens.color(space)?;
    stops.extend([
        ShadingStop {
            offset: start,
            color,
            alpha: 1.0,
        },
        ShadingStop {
            offset: 1.0,
            color,
            alpha: 1.0,
        },
    ]);
    for _ in 0..levels {
        tokens.expect("}")?;
        tokens.expect("ifelse")?;
    }
    tokens.expect("}")?;
    if tokens.index != tokens.values.len() {
        return Err(invalid("unexpected radial-band program suffix"));
    }
    let matrix = optional_numbers(doc, dict, b"Matrix")?
        .unwrap_or_else(|| vec![1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
    let [a, b, c, d, e, f]: [f64; 6] = matrix
        .try_into()
        .map_err(|_| invalid("invalid radial-band matrix"))?;
    let ctm = state.ctm.concat(Matrix::from_operands(
        a as f32, b as f32, c as f32, d as f32, e as f32, f as f32,
    ));
    if inverse(ctm).is_none() {
        return Ok(());
    }
    commands.push(Command::SaveState);
    forms::clip_bbox(doc, dict, state.ctm, height, commands)?;
    commands.push(Command::ConcatMatrix {
        a: ctm.a,
        b: -ctm.b,
        c: -ctm.c,
        d: ctm.d,
        e: Pt::from_f32(ctm.e),
        f: Pt::from_f32(-ctm.f),
    });
    commands.push(Command::ClipRect {
        x: Pt::from_f32(domain[0] as f32),
        y: Pt::from_f32(height - domain[3] as f32),
        width: Pt::from_f32((domain[1] - domain[0]) as f32),
        height: Pt::from_f32((domain[3] - domain[2]) as f32),
    });
    commands.push(Command::ShadingFill(Shading::Radial {
        x0: 0.0,
        y0: height,
        r0: 0.0,
        x1: 0.0,
        y1: height,
        r1: 1.0,
        stops,
        hard_stops: true,
    }));
    commands.push(Command::RestoreState);
    Ok(())
}
