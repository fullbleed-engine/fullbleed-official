//! PDF forms and gradient soft masks lowered to existing native form commands.
use super::*;
use crate::canvas::CompiledMaskLayer;
use crate::flowable::{MaskComposite, MaskMode};

type Result<T> = std::result::Result<T, FullBleedError>;

fn invalid(message: &str) -> FullBleedError {
    FullBleedError::InvalidConfiguration(format!("pdf raster mask error: {message}"))
}

fn define_form(
    form_commands: Vec<Command>,
    height: f32,
    cache: &mut PdfRasterCache,
    commands: &mut Vec<Command>,
) -> String {
    let resource_id = format!("pdf-preview-form-{}", cache.next_form);
    cache.next_form += 1;
    commands.push(Command::DefineForm {
        resource_id: resource_id.clone(),
        width: Pt::from_f32(cache.page_width),
        height: Pt::from_f32(height),
        commands: form_commands,
    });
    resource_id
}

pub(super) fn emit_form(
    form_commands: Vec<Command>,
    height: f32,
    cache: &mut PdfRasterCache,
    commands: &mut Vec<Command>,
) {
    let resource_id = define_form(form_commands, height, cache, commands);
    commands.push(Command::DrawForm {
        x: Pt::ZERO,
        y: Pt::ZERO,
        width: Pt::from_f32(cache.page_width),
        height: Pt::from_f32(height),
        resource_id,
    });
}

pub(super) fn clip_bbox(
    doc: &LoDocument,
    dict: &LoDictionary,
    ctm: Matrix,
    height: f32,
    commands: &mut Vec<Command>,
) -> Result<()> {
    if let Ok(object) = dict.get(b"BBox") {
        let bbox = shading::numbers(doc, object)?;
        if bbox.len() != 4 || bbox[0] > bbox[2] || bbox[1] > bbox[3] {
            return Err(invalid("invalid form BBox"));
        }
        for (index, (x, y)) in [
            (bbox[0], bbox[1]),
            (bbox[2], bbox[1]),
            (bbox[2], bbox[3]),
            (bbox[0], bbox[3]),
        ]
        .into_iter()
        .enumerate()
        {
            let (x, y) = ctm.transform_point(x as f32, y as f32);
            let (x, y) = (Pt::from_f32(x), Pt::from_f32(height - y));
            commands.push(if index == 0 {
                Command::MoveTo { x, y }
            } else {
                Command::LineTo { x, y }
            });
        }
        commands.push(Command::ClosePath);
        commands.push(Command::ClipPath { evenodd: false });
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(super) fn paint_shading(
    doc: &LoDocument,
    object: &LoObject,
    resources: &PdfResources,
    state: &ParseState,
    height: f32,
    commands: &mut Vec<Command>,
    visited_forms: &mut HashSet<ObjectId>,
    cache: &mut PdfRasterCache,
    embedded_fonts: &mut HashMap<String, Arc<Vec<u8>>>,
) -> Result<()> {
    let mut source = Vec::new();
    shading::paint(
        doc,
        object,
        resources,
        state,
        cache.page_width,
        height,
        &mut source,
    )?;
    let Some((mask, ctm)) = &state.soft_mask else {
        // The `sh` operator leaves the current path and painting color intact.
        emit_form(source, height, cache, commands);
        return Ok(());
    };
    let dict = resolve_dict(doc, mask)?;
    let mode = match resolve_object(doc, dict.get(b"S").map_err(pdf_err)?)?
        .as_name()
        .map_err(pdf_err)?
    {
        b"Alpha" => MaskMode::Alpha,
        b"Luminosity" => MaskMode::Luminance,
        _ => return Err(invalid("unsupported soft-mask subtype")),
    };
    if let Ok(transfer) = dict.get(b"TR") {
        if resolve_object(doc, transfer)?.as_name().ok() != Some(b"Identity") {
            return Err(invalid(
                "nonidentity soft-mask transfer functions are not supported",
            ));
        }
    }
    let group_id = dict
        .get(b"G")
        .map_err(pdf_err)?
        .as_reference()
        .map_err(pdf_err)?;
    let stream = doc
        .get_object(group_id)
        .map_err(pdf_err)?
        .as_stream()
        .map_err(pdf_err)?;
    let group = resolve_dict(doc, stream.dict.get(b"Group").map_err(pdf_err)?)?;
    if resolve_object(doc, group.get(b"S").map_err(pdf_err)?)?
        .as_name()
        .ok()
        != Some(b"Transparency")
    {
        return Err(invalid("soft mask must reference a transparency group"));
    }
    let mut mask_commands = vec![
        Command::SetOpacity {
            fill: 1.0,
            stroke: 1.0,
        },
        Command::SetFillColor(Color::BLACK),
        Command::SetStrokeColor(Color::BLACK),
    ];
    if mode == MaskMode::Luminance {
        let space = group
            .get(b"CS")
            .ok()
            .and_then(|value| parse_raster_color_space(doc, value));
        let Some(RasterColorSpace::Direct(direct)) = space.as_ref() else {
            return Err(invalid("unsupported luminosity-mask blending color space"));
        };
        let components = if let Ok(backdrop) = dict.get(b"BC") {
            shading::numbers(doc, backdrop)?
                .into_iter()
                .map(|v| v as f32)
                .collect::<Vec<_>>()
        } else {
            match direct {
                RasterDirectColor::Cmyk => vec![0.0, 0.0, 0.0, 1.0],
                _ => vec![0.0; direct.channels()],
            }
        };
        if components.len() != direct.channels() {
            return Err(invalid(
                "backdrop components do not match the mask color space",
            ));
        }
        let color = color_from_components_in_space(&components, space.as_ref())
            .ok_or_else(|| invalid("unsupported mask backdrop"))?;
        mask_commands.push(Command::SetFillColor(color));
        mask_commands.extend([
            Command::MoveTo {
                x: Pt::ZERO,
                y: Pt::ZERO,
            },
            Command::LineTo {
                x: Pt::from_f32(cache.page_width),
                y: Pt::ZERO,
            },
            Command::LineTo {
                x: Pt::from_f32(cache.page_width),
                y: Pt::from_f32(height),
            },
            Command::LineTo {
                x: Pt::ZERO,
                y: Pt::from_f32(height),
            },
            Command::ClosePath,
            Command::Fill,
        ]);
        mask_commands.push(Command::SetFillColor(Color::BLACK));
    }
    let mask_state = ParseState {
        ctm: *ctm,
        ..ParseState::default()
    };
    parse_xobject(
        doc,
        group_id,
        resources,
        height,
        &mask_state,
        &mut mask_commands,
        visited_forms,
        cache,
        embedded_fonts,
    )?;
    let mask_id = define_form(mask_commands, height, cache, commands);
    let source_id = define_form(source, height, cache, commands);
    commands.push(Command::DrawMaskedForm {
        x: Pt::ZERO,
        y: Pt::ZERO,
        width: Pt::from_f32(cache.page_width),
        height: Pt::from_f32(height),
        resource_id: source_id,
        layers: vec![CompiledMaskLayer {
            resource_id: mask_id,
            mode,
            composite: MaskComposite::Add,
        }],
    });
    Ok(())
}
