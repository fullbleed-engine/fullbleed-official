use super::*;
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn inspect_hints(pdf: &LoDocument, record_level: Option<usize>) -> (usize, usize, bool) {
    let (mut count, mut encapsulated, mut valid) = (0, 0, true);
    for stream in pdf.objects.values().filter_map(|o| o.as_stream().ok()) {
        let dict = &stream.dict;
        if !dict_has_type(dict, b"XObject") {
            continue;
        }
        if let Ok(scope) = dict.get(b"GTS_Scope") {
            count += 1;
            valid &= scope.as_name().is_ok_and(|name| match name {
                b"SingleUse" | b"File" | b"Unknown" => true,
                b"Record" => record_level.is_some(),
                b"Stream" | b"Global" => dict
                    .get(b"GTS_Env")
                    .is_ok_and(|o| o.as_str().is_ok_and(|s| !s.is_empty())),
                _ => false,
            });
        }
        if let Ok(hint) = dict.get(b"GTS_Encapsulated") {
            match hint {
                LoObject::Boolean(true) => {
                    encapsulated += 1;
                    let subtype = dict.get(b"Subtype").ok().and_then(|o| o.as_name().ok());
                    if subtype == Some(b"Image") {
                        valid &= dict.get(b"Intent").is_ok_and(|o| {
                            o.as_name().is_ok_and(|v| {
                                matches!(
                                    v,
                                    b"AbsoluteColorimetric"
                                        | b"RelativeColorimetric"
                                        | b"Perceptual"
                                        | b"Saturation"
                                )
                            })
                        }) || dict.get(b"ImageMask") == Ok(&LoObject::Boolean(true));
                    }
                }
                LoObject::Boolean(false) => {}
                _ => valid = false,
            }
        }
    }
    (count, encapsulated, valid)
}

fn decode(value: &LoObject, depth: usize) -> Option<crate::DpmValue> {
    use crate::DpmValue;
    // Writer metadata allows 16 levels below the private Fullbleed/Metadata
    // envelope. Keep those envelope dictionaries outside the user depth budget.
    if depth > 18 {
        return None;
    }
    Some(match value {
        LoObject::String(..) => {
            DpmValue::String(crate::pdf_native::decode_text_string(value).ok()?)
        }
        LoObject::Name(name) => DpmValue::String(String::from_utf8_lossy(name).into_owned()),
        LoObject::Integer(v) => DpmValue::Integer(*v),
        LoObject::Real(v) if v.is_finite() => DpmValue::Real(f64::from(*v)),
        LoObject::Boolean(v) => DpmValue::Boolean(*v),
        LoObject::Array(v) => DpmValue::Array(
            v.iter()
                .map(|v| decode(v, depth + 1))
                .collect::<Option<_>>()?,
        ),
        LoObject::Dictionary(v) => DpmValue::Dictionary(
            v.iter()
                .map(|(k, v)| Some((String::from_utf8(k.clone()).ok()?, decode(v, depth + 1)?)))
                .collect::<Option<_>>()?,
        ),
        _ => return None,
    })
}

pub(super) fn inspect(pdf: &LoDocument) -> PdfVtDPartInspection {
    let mut out = PdfVtDPartInspection::default();
    let pages: Vec<_> = pdf.get_pages().values().copied().collect();
    let page_indices: BTreeMap<_, _> = pages.iter().enumerate().map(|(i, id)| (*id, i)).collect();
    let Some(catalog) = crate::pdf_print_contract::catalog(pdf) else {
        return out;
    };
    let Some(root_id) = dict_reference(catalog, b"DPartRoot") else {
        return out;
    };
    let Some(root) = object_dict(pdf, root_id) else {
        return out;
    };
    out.dpart_root_present = dict_has_type(root, b"DPartRoot");
    let names = root
        .get(b"NodeNameList")
        .ok()
        .and_then(|o| o.as_array().ok())
        .and_then(|v| {
            v.iter()
                .map(|o| o.as_name().ok().filter(|s| !s.is_empty()))
                .collect::<Option<Vec<_>>>()
        });
    let names = names.unwrap_or_default();
    out.node_name_list_valid = !names.is_empty() && names.len() <= 64;
    out.record_level = root
        .get(b"RecordLevel")
        .ok()
        .and_then(|o| o.as_i64().ok())
        .and_then(|n| usize::try_from(n).ok());
    if root.get(b"RecordLevel").is_ok() && out.record_level.is_none_or(|n| n >= names.len()) {
        out.node_name_list_valid = false;
    }
    let Some(node_id) = dict_reference(root, b"DPartRootNode") else {
        return out;
    };
    out.root_node_valid = object_dict(pdf, node_id).is_some_and(|d| dict_has_type(d, b"DPart"));
    out.dpart_present = out.root_node_valid;
    out.parent_valid = true;
    out.leaf_valid = true;
    out.page_range_valid = !pages.is_empty();
    out.dpm_valid = true;
    out.page_dpart_present = !pages.is_empty()
        && pages.iter().all(|page| {
            page_dpart_reference(pdf, *page)
                .and_then(|id| object_dict(pdf, id))
                .is_some_and(|d| dict_has_type(d, b"DPart"))
        });
    let mut visited = BTreeSet::new();
    let mut stack = vec![(node_id, root_id, 0_usize)];
    let mut next_page = 0;
    while let Some((id, parent, depth)) = stack.pop() {
        if !visited.insert(id) || depth >= 64 {
            out.leaf_valid = false;
            out.parent_valid = false;
            continue;
        }
        let Some(node) = object_dict(pdf, id) else {
            out.leaf_valid = false;
            continue;
        };
        out.parent_valid &=
            dict_reference(node, b"Parent") == Some(parent) && dict_has_type(node, b"DPart");
        out.node_name_list_valid &= depth < names.len();
        if Some(depth) == out.record_level {
            out.record_count += 1;
        }
        let mut part = PdfVtPartInspection {
            object_id: id.0,
            depth,
            name: names
                .get(depth)
                .map(|s| String::from_utf8_lossy(s).into_owned())
                .unwrap_or_default(),
            ..Default::default()
        };
        if let Ok(dpm) = node.get(b"DPM") {
            out.dpm_node_count += 1;
            let dpm = match dpm {
                LoObject::Reference(id) => pdf.get_object(*id).ok(),
                _ => Some(dpm),
            };
            match dpm.and_then(|o| decode(o, 0)) {
                Some(crate::DpmValue::Dictionary(metadata)) => {
                    if let Some(crate::DpmValue::Dictionary(fb)) = metadata.get("Fullbleed") {
                        if let Some(crate::DpmValue::String(id)) = fb.get("ID") {
                            part.id = Some(id.clone());
                        }
                    }
                    part.metadata = metadata;
                }
                _ => out.dpm_valid = false,
            }
        }
        if let Ok(children) = node.get(b"DParts") {
            out.leaf_valid &= node.get(b"Start").is_err() && node.get(b"End").is_err();
            if let Ok(children) = children.as_array() {
                out.leaf_valid &= !children.is_empty();
                for child in children.iter().rev() {
                    match child.as_reference() {
                        Ok(child) => stack.push((child, id, depth + 1)),
                        Err(_) => out.leaf_valid = false,
                    }
                }
            } else {
                out.leaf_valid = false;
            }
        } else {
            out.document_count += 1;
            let start =
                dict_reference(node, b"Start").and_then(|id| page_indices.get(&id).copied());
            let end = if node.get(b"End").is_ok() {
                dict_reference(node, b"End").and_then(|id| page_indices.get(&id).copied())
            } else {
                start
            };
            out.leaf_valid &= start.is_some() && end.is_some();
            if let Some((start, end)) = start.zip(end).filter(|(start, end)| start <= end) {
                part.first_page = Some(start + 1);
                part.last_page = Some(end + 1);
                out.page_range_valid &= start == next_page
                    && pages[start..=end]
                        .iter()
                        .all(|page| page_dpart_reference(pdf, *page) == Some(id));
                next_page = end + 1;
            } else {
                out.page_range_valid = false;
            }
        }
        out.parts.push(part);
    }
    out.page_range_valid &= next_page == pages.len();
    out.graph_valid = out.dpart_root_present
        && out.root_node_valid
        && out.parent_valid
        && out.leaf_valid
        && out.page_range_valid
        && out.page_dpart_present
        && out.node_name_list_valid
        && out.dpm_valid;
    out
}
