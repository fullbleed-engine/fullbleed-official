//! Clip imported link activation and retain the original appearance placement.
use super::*;
use crate::pdf_link_geometry::{Matrix, Point, Quad, Region};

fn values(doc: &LoDocument, object: &LoObject) -> Result<Vec<f64>, FullBleedError> {
    resolve_finalize_object(doc, object)?
        .as_array()
        .map_err(pdf_err)?
        .iter()
        .map(|value| {
            let number = resolve_finalize_object(doc, value)?
                .as_float()
                .map_err(pdf_err)? as f64;
            if number.is_finite() {
                Ok(number)
            } else {
                Err(invalid("non-finite clip or appearance coordinate"))
            }
        })
        .collect()
}

pub(super) fn bounds(doc: &LoDocument, array: &[LoObject]) -> Result<[f64; 4], FullBleedError> {
    let numbers = values(doc, &LoObject::Array(array.to_vec()))?;
    if numbers.len() != 4 {
        return Err(invalid("page or annotation box must have four coordinates"));
    }
    Ok([
        numbers[0].min(numbers[2]),
        numbers[1].min(numbers[3]),
        numbers[0].max(numbers[2]),
        numbers[1].max(numbers[3]),
    ])
}

fn array(numbers: &[f64]) -> LoObject {
    LoObject::Array(
        numbers
            .iter()
            .map(|&number| LoObject::Real(number as f32))
            .collect(),
    )
}

fn intersection(a: [f64; 4], b: [f64; 4]) -> Option<[f64; 4]> {
    let r = [
        a[0].max(b[0]),
        a[1].max(b[1]),
        a[2].min(b[2]),
        a[3].min(b[3]),
    ];
    (r[2] > r[0] && r[3] > r[1]).then_some(r)
}

fn appearance_variant(
    doc: &mut LoDocument,
    object: &LoObject,
    original: [f64; 4],
    clipped: [f64; 4],
    depth: usize,
) -> Result<LoObject, FullBleedError> {
    if depth > 32 {
        return Err(invalid(
            "appearance states contain a cycle or exceed the nesting limit",
        ));
    }
    let resolved = resolve_finalize_object(doc, object)?.clone();
    match resolved {
        LoObject::Dictionary(states) => {
            let mut wrapped = LoDictionary::new();
            for (name, state) in states.iter() {
                wrapped.set(
                    name,
                    appearance_variant(doc, state, original, clipped, depth + 1)?,
                );
            }
            Ok(LoObject::Dictionary(wrapped))
        }
        LoObject::Stream(stream) => {
            let bbox_object =
                resolve_finalize_object(doc, stream.dict.get(b"BBox").map_err(pdf_err)?)?;
            let bbox = bounds(doc, bbox_object.as_array().map_err(pdf_err)?)?;
            let matrix = match stream.dict.get(b"Matrix") {
                Ok(value) => {
                    let v = values(doc, value)?;
                    if v.len() != 6 {
                        return Err(invalid("appearance matrix must have six coordinates"));
                    }
                    Matrix([v[0], v[1], v[2], v[3], v[4], v[5]])
                }
                Err(_) => Matrix::IDENTITY,
            };
            let transformed = Quad::rectangle(
                matrix,
                bbox[0],
                bbox[1],
                bbox[2] - bbox[0],
                bbox[3] - bbox[1],
            )
            .and_then(|quad| Region::quad(quad).bounds());
            let mut resources = LoDictionary::new();
            let mut content = Vec::new();
            if let Some(b) = transformed.filter(|b| b[2] > b[0] && b[3] > b[1]) {
                let sx = (original[2] - original[0]) / (b[2] - b[0]);
                let sy = (original[3] - original[1]) / (b[3] - b[1]);
                let tx = original[0] - sx * b[0];
                let ty = original[1] - sy * b[1];
                let source = match object {
                    LoObject::Reference(id) => *id,
                    _ => doc.add_object(stream),
                };
                resources.set("XObject", dictionary! {"FB_SOURCE_AP" => source});
                content = format!("q {sx} 0 0 {sy} {tx} {ty} cm /FB_SOURCE_AP Do Q\n").into_bytes();
            }
            let wrapper = doc.add_object(LoStream::new(
                dictionary! {
                    "Type" => "XObject", "Subtype" => "Form", "FormType" => 1,
                    "BBox" => array(&clipped), "Resources" => resources,
                },
                content,
            ));
            Ok(LoObject::Reference(wrapper))
        }
        _ => Err(invalid(
            "appearance entry is not a stream or state dictionary",
        )),
    }
}

fn border_appearance(
    doc: &mut LoDocument,
    annotation: &LoDictionary,
    rect: [f64; 4],
    clipped: [f64; 4],
) -> Result<LoObject, FullBleedError> {
    let border = annotation
        .get(b"Border")
        .ok()
        .map(|value| {
            resolve_finalize_object(doc, value)?
                .as_array()
                .map_err(pdf_err)
                .cloned()
        })
        .transpose()?;
    let bs = annotation
        .get(b"BS")
        .ok()
        .map(|value| {
            resolve_finalize_object(doc, value)?
                .as_dict()
                .map_err(pdf_err)
                .cloned()
        })
        .transpose()?;
    let number = |value: &LoObject| -> Result<f64, FullBleedError> {
        Ok(resolve_finalize_object(doc, value)?
            .as_float()
            .map_err(pdf_err)? as f64)
    };
    let width = if let Some(bs) = &bs {
        bs.get(b"W").ok().map(number).transpose()?.unwrap_or(1.0)
    } else {
        border
            .as_ref()
            .and_then(|b| b.get(2))
            .map(number)
            .transpose()?
            .unwrap_or(1.0)
    };
    if !width.is_finite() || width < 0.0 {
        return Err(invalid("invalid link border width"));
    }
    let color = annotation
        .get(b"C")
        .ok()
        .map(|value| values(doc, value))
        .transpose()?
        .unwrap_or(vec![0.0]);
    let style = bs
        .as_ref()
        .and_then(|bs| bs.get(b"S").ok())
        .map(|v| {
            resolve_finalize_object(doc, v)?
                .as_name()
                .map_err(pdf_err)
                .map(|v| v.to_vec())
        })
        .transpose()?
        .unwrap_or_else(|| b"S".to_vec());
    let mut content = String::new();
    if width > 0.0 && !color.is_empty() {
        let color_op = match color.len() {
            1 => "G",
            3 => "RG",
            4 => "K",
            _ => return Err(invalid("invalid link border color")),
        };
        let color_values = color
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(" ");
        content.push_str(&format!(
            "q /FB_BORDER_ALPHA gs {color_values} {color_op} {width} w\n"
        ));
        let dash = if let Some(bs) = &bs {
            if style == b"D" {
                bs.get(b"D")
                    .ok()
                    .map(|v| values(doc, v))
                    .transpose()?
                    .or(Some(vec![3.0]))
            } else {
                None
            }
        } else {
            border
                .as_ref()
                .and_then(|b| b.get(3))
                .map(|v| values(doc, v))
                .transpose()?
        };
        if let Some(dash) = dash {
            if dash.iter().any(|&v| v < 0.0) || (!dash.is_empty() && dash.iter().all(|&v| v == 0.0))
            {
                return Err(invalid("invalid link border dash pattern"));
            }
            content.push_str(&format!(
                "[{}] 0 d\n",
                dash.iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(" ")
            ));
        }
        let [left, bottom, right, top] = rect;
        let half = width / 2.0;
        let (x, y, w, h) = (
            left + half,
            bottom + half,
            (right - left - width).max(0.0),
            (top - bottom - width).max(0.0),
        );
        if style == b"U" {
            content.push_str(&format!("{left} {y} m {right} {y} l S\n"));
        } else if style == b"B" || style == b"I" {
            let rgb = match color.len() {
                1 => [color[0]; 3],
                3 => [color[0], color[1], color[2]],
                _ => [
                    1.0 - (color[0] + color[3]).min(1.0),
                    1.0 - (color[1] + color[3]).min(1.0),
                    1.0 - (color[2] + color[3]).min(1.0),
                ],
            };
            let mut light = rgb.map(|v| (v + 1.0) * 0.5);
            let mut dark = rgb.map(|v| v * 0.5);
            if style == b"I" {
                std::mem::swap(&mut light, &mut dark);
            }
            let inset = width.min((right - left) / 2.0).min((top - bottom) / 2.0);
            let (il, ib, ir, it) = (left + inset, bottom + inset, right - inset, top - inset);
            content.push_str(&format!("{} {} {} rg {left} {bottom} m {left} {top} l {right} {top} l {ir} {it} l {il} {it} l {il} {ib} l h f\n",light[0],light[1],light[2]));
            content.push_str(&format!("{} {} {} rg {left} {bottom} m {right} {bottom} l {right} {top} l {ir} {it} l {ir} {ib} l {il} {ib} l h f\n",dark[0],dark[1],dark[2]));
        } else {
            let rx = if bs.is_none() {
                border
                    .as_ref()
                    .and_then(|b| b.first())
                    .map(number)
                    .transpose()?
                    .unwrap_or(0.0)
                    .max(0.0)
                    .min(w / 2.0)
            } else {
                0.0
            };
            let ry = if bs.is_none() {
                border
                    .as_ref()
                    .and_then(|b| b.get(1))
                    .map(number)
                    .transpose()?
                    .unwrap_or(0.0)
                    .max(0.0)
                    .min(h / 2.0)
            } else {
                0.0
            };
            if rx == 0.0 || ry == 0.0 {
                content.push_str(&format!("{x} {y} {w} {h} re S\n"));
            } else {
                let (r, t, k) = (x + w, y + h, 0.5522847498307936);
                content.push_str(&format!("{} {y} m {} {y} l {} {y} {r} {} {r} {} c {r} {} l {r} {} {} {t} {} {t} c {} {t} l {} {t} {x} {} {x} {} c {x} {} l {x} {} {} {y} {} {y} c h S\n",
                    x+rx,r-rx,r-rx+k*rx,y+ry-k*ry,y+ry,t-ry,t-ry+k*ry,r-rx+k*rx,r-rx,x+rx,x+rx-k*rx,t-ry+k*ry,t-ry,y+ry,y+ry-k*ry,x+rx-k*rx,x+rx));
            }
        }
        content.push_str("Q\n");
    }
    let opacity = annotation
        .get(b"CA")
        .ok()
        .map(number)
        .transpose()?
        .unwrap_or(1.0);
    let fill_opacity = annotation
        .get(b"ca")
        .ok()
        .map(number)
        .transpose()?
        .unwrap_or(opacity);
    if !opacity.is_finite() || !fill_opacity.is_finite() {
        return Err(invalid("non-finite annotation opacity"));
    }
    Ok(LoObject::Reference(doc.add_object(LoStream::new(dictionary! {
        "Type" => "XObject", "Subtype" => "Form", "FormType" => 1, "BBox" => array(&clipped),
        "Resources" => dictionary! {"ExtGState" => dictionary! {"FB_BORDER_ALPHA" => dictionary! {
            "Type" => "ExtGState", "CA" => opacity.clamp(0.0,1.0) as f32, "ca" => fill_opacity.clamp(0.0,1.0) as f32,
        }}},
    },content.into_bytes()))))
}

pub(super) fn annotation(
    doc: &mut LoDocument,
    annotation: &mut LoDictionary,
    source_clip: [f64; 4],
    output_clip: [f64; 4],
) -> Result<bool, FullBleedError> {
    if !annotation
        .get(b"Subtype")
        .is_ok_and(|v| object_is_name(doc, v, b"Link"))
    {
        return Ok(true);
    }
    let rect = resolve_finalize_object(doc, annotation.get(b"Rect").map_err(pdf_err)?)?;
    let original = bounds(doc, rect.as_array().map_err(pdf_err)?)?;
    let Some(clipped) =
        intersection(original, source_clip).and_then(|v| intersection(v, output_clip))
    else {
        return Ok(false);
    };
    if original == clipped {
        return Ok(true);
    }
    let mut active = true;
    if let Ok(points) = annotation.get(b"QuadPoints") {
        let points = values(doc, points)?;
        let mut region = Region::default();
        let mut valid = !points.is_empty() && points.len() % 8 == 0;
        for p in points.chunks_exact(8) {
            let quad = Quad::new([
                Point::new(p[0], p[1]),
                Point::new(p[2], p[3]),
                Point::new(p[4], p[5]),
                Point::new(p[6], p[7]),
            ]);
            if let Some(quad) = quad.filter(|q| {
                q.0.iter().all(|p| {
                    p.x >= original[0]
                        && p.y >= original[1]
                        && p.x <= original[2]
                        && p.y <= original[3]
                })
            }) {
                region.quads.push(quad);
            } else {
                valid = false;
            }
        }
        if valid {
            let clip = Region::quad(
                Quad::rectangle(
                    Matrix::IDENTITY,
                    clipped[0],
                    clipped[1],
                    clipped[2] - clipped[0],
                    clipped[3] - clipped[1],
                )
                .unwrap(),
            );
            let region = region.intersect(&clip);
            active = !region.quads.is_empty();
            annotation.set(
                "QuadPoints",
                array(
                    &region
                        .quads
                        .iter()
                        .flat_map(|q| q.0.iter().flat_map(|p| [p.x, p.y]))
                        .collect::<Vec<_>>(),
                ),
            );
        } else {
            // ISO 32000: malformed/out-of-Rect QuadPoints use Rect activation.
            annotation.remove(b"QuadPoints");
        }
    }
    let mut appearances = match annotation.get(b"AP") {
        Ok(value) => resolve_finalize_object(doc, value)?
            .as_dict()
            .map_err(pdf_err)?
            .clone(),
        Err(_) => LoDictionary::new(),
    };
    let entries: Vec<_> = appearances
        .iter()
        .filter(|(key, _)| matches!(key.as_slice(), b"N" | b"R" | b"D"))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    for (key, value) in entries {
        appearances.set(key, appearance_variant(doc, &value, original, clipped, 0)?);
    }
    if appearances.get(b"N").is_err() {
        appearances.set("N", border_appearance(doc, annotation, original, clipped)?);
    }
    annotation.set("AP", appearances);
    annotation.set("Rect", array(&clipped));
    // The appearance already preserves the original, clipped artwork. Leaving
    // a fallback border makes some readers draw a second edge at the new Rect.
    annotation.remove(b"BS");
    annotation.set("Border", array(&[0.0, 0.0, 0.0]));
    if !active {
        // Keep the original annotation handler: converting Link to Stamp can
        // introduce a popup note or change which readers draw the appearance.
        // ReadOnly permits the remaining artwork without mouse interaction.
        let flags = match annotation.get(b"F") {
            Ok(value) => resolve_finalize_object(doc, value)?
                .as_i64()
                .map_err(pdf_err)?,
            Err(_) => 0,
        };
        annotation.set("F", flags | 64);
        for key in [b"A".as_slice(), b"AA", b"Dest", b"QuadPoints", b"H"] {
            annotation.remove(key);
        }
    }
    Ok(true)
}
