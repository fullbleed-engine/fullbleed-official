//! Parsed checks for the print writer contract. These complement, and do not
//! replace, independent ISO PDF/X and PDF/VT preflight.

use crate::pdf_identity::PdfTimestamp;
use crate::pdf_native::{Dictionary, Document, Object, decode_text_string};
use crate::xml;

const XMP: &str = "http://ns.adobe.com/xap/1.0/";
const MM: &str = "http://ns.adobe.com/xap/1.0/mm/";
const PDF: &str = "http://ns.adobe.com/pdf/1.3/";
const DC: &str = "http://purl.org/dc/elements/1.1/";
const RDF: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";
const VT: &str = "http://www.npes.org/pdfvt/ns/id/";
const X: &str = "http://www.npes.org/pdfx/ns/id/";

fn named(mut node: xml::Node<'_>, qualified: &str, namespace: &str, local: &str) -> bool {
    let Some((prefix, name)) = qualified.split_once(':') else {
        return false;
    };
    if name != local {
        return false;
    }
    let xmlns = format!("xmlns:{prefix}");
    loop {
        if let Some(value) = node.attribute(&xmlns) {
            return value == namespace;
        }
        match node.parent() {
            Some(parent) => node = parent,
            None => return false,
        }
    }
}

fn property(doc: &xml::Document, namespace: &str, name: &str) -> Option<String> {
    let mut values = Vec::new();
    for node in doc.descendants() {
        for (key, value) in node.attributes() {
            if named(node, key, namespace, name) {
                values.push(value.to_owned());
            }
        }
        if named(node, node.qualified_name(), namespace, name) {
            if namespace == DC && name == "title" {
                for alt in node
                    .children()
                    .filter(|n| named(*n, n.qualified_name(), RDF, "Alt"))
                {
                    for item in alt
                        .children()
                        .filter(|n| named(*n, n.qualified_name(), RDF, "li"))
                    {
                        if item.attribute("xml:lang") == Some("x-default") {
                            values.push(item.text().unwrap_or("").to_owned());
                        }
                    }
                }
            } else if let Some(value) = node.text() {
                values.push(value.to_owned());
            }
        }
    }
    // Ambiguous duplicate properties must not hide an inconsistent identity.
    (values.len() == 1).then(|| values.remove(0))
}

fn resolved<'a>(pdf: &'a Document, value: &'a Object) -> Option<&'a Object> {
    match value {
        Object::Reference(id) => pdf.get_object(*id).ok(),
        _ => Some(value),
    }
}

fn dictionary_at<'a>(
    pdf: &'a Document,
    dict: &'a Dictionary,
    key: &[u8],
) -> Option<&'a Dictionary> {
    resolved(pdf, dict.get(key).ok()?)?.as_dict().ok()
}

pub(crate) fn catalog(pdf: &Document) -> Option<&Dictionary> {
    dictionary_at(pdf, &pdf.trailer, b"Root")
}

pub(crate) fn metadata(pdf: &Document) -> Option<Vec<u8>> {
    let cat = catalog(pdf)?;
    resolved(pdf, cat.get(b"Metadata").ok()?)?
        .as_stream()
        .ok()?
        .get_plain_content()
        .ok()
}

fn pdf_text(dict: &Dictionary, key: &[u8]) -> Option<String> {
    dict.get(key).ok().and_then(|o| decode_text_string(o).ok())
}

fn rect(dict: &Dictionary, key: &[u8]) -> Option<[f32; 4]> {
    let values = dict.get(key).ok()?.as_array().ok()?;
    if values.len() != 4 {
        return None;
    }
    let mut r = [0.0; 4];
    for (i, value) in values.iter().enumerate() {
        r[i] = value.as_float().ok()?;
    }
    (r.iter().all(|n| n.is_finite()) && r[0] < r[2] && r[1] < r[3]).then_some(r)
}

fn contains(outer: [f32; 4], inner: [f32; 4]) -> bool {
    outer[0] <= inner[0] && outer[1] <= inner[1] && outer[2] >= inner[2] && outer[3] >= inner[3]
}

fn forbidden(object: &Object, depth: usize) -> bool {
    if depth > 64 {
        return true;
    }
    let dict = match object {
        Object::Dictionary(dict) => dict,
        Object::Stream(stream) => {
            if stream.dict.get(b"F").is_ok() {
                return true;
            }
            &stream.dict
        }
        Object::Array(items) => return items.iter().any(|o| forbidden(o, depth + 1)),
        _ => return false,
    };
    if [
        b"JavaScript".as_slice(),
        b"JS",
        b"AA",
        b"OpenAction",
        b"AcroForm",
        b"EmbeddedFiles",
        b"OPI",
        b"Ref",
    ]
    .iter()
    .any(|key| dict.get(key).is_ok())
    {
        return true;
    }
    if dict
        .get(b"S")
        .ok()
        .and_then(|o| o.as_name().ok())
        .is_some_and(|name| {
            matches!(
                name,
                b"JavaScript" | b"Launch" | b"Movie" | b"Sound" | b"ImportData" | b"SubmitForm"
            )
        })
    {
        return true;
    }
    dict.iter()
        .any(|(key, value)| key.as_slice() != b"DPM" && forbidden(value, depth + 1))
}

pub(crate) fn inspect(pdf: &Document, vt: bool) -> (Vec<String>, Option<String>) {
    let mut errors = Vec::new();
    let mut check = |valid: bool, code: &str| {
        if !valid {
            errors.push(format!("pdfx_{code}"));
        }
    };
    check(pdf.version == "1.6", "requires_pdf16");
    check(!pdf.is_encrypted(), "encryption_forbidden");
    let Some(cat) = catalog(pdf) else {
        return (vec!["pdfx_missing_catalog".to_owned()], None);
    };
    check(
        cat.get(b"Version").is_err()
            || cat.get(b"Version").ok().and_then(|v| v.as_name().ok()) == Some(b"1.6"),
        "catalog_version",
    );
    let metadata_stream = cat
        .get(b"Metadata")
        .ok()
        .and_then(|o| resolved(pdf, o))
        .and_then(|o| o.as_stream().ok());
    check(
        metadata_stream.is_some_and(|s| s.dict.get(b"Filter").is_err()),
        "metadata_missing_or_filtered",
    );
    let xml_bytes = metadata(pdf).unwrap_or_default();
    let xml = std::str::from_utf8(&xml_bytes)
        .ok()
        .and_then(|text| xml::Document::parse(text).ok());
    check(xml.is_some(), "metadata_invalid_xml");
    let prop = |namespace, name| xml.as_ref().and_then(|doc| property(doc, namespace, name));
    check(
        prop(X, "GTS_PDFXVersion").as_deref() == Some("PDF/X-4"),
        "xmp_profile_claim",
    );
    if vt {
        check(
            prop(VT, "GTS_PDFVTVersion").as_deref() == Some("PDF/VT-1"),
            "xmp_vt_profile_claim",
        );
    }
    let title = prop(DC, "title");
    check(
        title.as_ref().is_some_and(|s| !s.trim().is_empty()),
        "title_required",
    );
    for name in ["DocumentID", "InstanceID", "VersionID", "RenditionClass"] {
        check(
            prop(MM, name).is_some_and(|s| !s.trim().is_empty()),
            &format!("missing_{name}"),
        );
    }
    for name in ["CreateDate", "ModifyDate", "MetadataDate"] {
        check(
            prop(XMP, name).is_some_and(|s| s.len() == 20 && s.parse::<PdfTimestamp>().is_ok()),
            &format!("invalid_{name}"),
        );
    }
    let modify = prop(XMP, "ModifyDate");
    let trapped = prop(PDF, "Trapped");
    check(
        trapped
            .as_deref()
            .is_some_and(|s| matches!(s, "True" | "False")),
        "trapping_status",
    );
    if let Some(info) = dictionary_at(pdf, &pdf.trailer, b"Info") {
        check(pdf_text(info, b"Title") == title, "info_title_mismatch");
        check(
            info.get(b"Trapped").ok().and_then(|o| o.as_name().ok())
                == trapped.as_ref().map(|s| s.as_bytes()),
            "info_trapped_mismatch",
        );
        for (key, name) in [
            (b"CreationDate".as_slice(), "CreateDate"),
            (b"ModDate", "ModifyDate"),
        ] {
            if info.get(key).is_ok() {
                check(
                    pdf_text(info, key)
                        == prop(XMP, name)
                            .and_then(|s| s.parse::<PdfTimestamp>().ok())
                            .map(|date| date.pdf_date()),
                    "info_date_mismatch",
                );
            }
        }
    }
    if vt {
        check(
            modify.is_some() && prop(VT, "GTS_PDFVTModDate") == modify,
            "vt_mod_date_mismatch",
        );
    }
    check(
        pdf.trailer
            .get(b"ID")
            .ok()
            .and_then(|o| o.as_array().ok())
            .is_some_and(|ids| {
                ids.len() == 2
                    && ids
                        .iter()
                        .all(|id| id.as_str().is_ok_and(|s| s.len() == 16))
            }),
        "trailer_id",
    );
    let intents = cat
        .get(b"OutputIntents")
        .ok()
        .and_then(|o| resolved(pdf, o))
        .and_then(|o| o.as_array().ok());
    let print_intents: Vec<_> = intents
        .into_iter()
        .flatten()
        .filter_map(|o| resolved(pdf, o)?.as_dict().ok())
        .filter(|d| d.get(b"S").ok().and_then(|o| o.as_name().ok()) == Some(b"GTS_PDFX"))
        .collect();
    check(print_intents.len() == 1, "output_intent_count");
    if let Some(intent) = print_intents.first() {
        check(
            pdf_text(intent, b"OutputConditionIdentifier").is_some_and(|s| !s.trim().is_empty()),
            "output_intent_identifier",
        );
        let icc = intent
            .get(b"DestOutputProfile")
            .ok()
            .and_then(|o| resolved(pdf, o))
            .and_then(|o| o.as_stream().ok());
        let valid = icc.is_some_and(|stream| {
            let Ok(bytes) = stream.get_plain_content() else {
                return false;
            };
            crate::inspect_output_intent_icc(&bytes).is_ok_and(|profile| {
                stream.dict.get(b"N").ok().and_then(|o| o.as_i64().ok())
                    == Some(i64::from(profile.components))
            })
        });
        check(valid, "invalid_output_intent_icc");
    }
    check(
        !pdf.objects.values().any(|o| forbidden(o, 0)),
        "forbidden_construct",
    );
    for page in pdf
        .get_pages()
        .values()
        .filter_map(|id| pdf.get_object(*id).ok()?.as_dict().ok())
    {
        let media = rect(page, b"MediaBox");
        let crop = rect(page, b"CropBox").or(media);
        let bleed = rect(page, b"BleedBox").or(crop);
        let trim = rect(page, b"TrimBox").or_else(|| rect(page, b"ArtBox"));
        let valid = media
            .zip(crop)
            .zip(bleed)
            .zip(trim)
            .is_some_and(|(((m, c), b), t)| contains(m, c) && contains(c, b) && contains(b, t));
        check(
            valid && !(page.get(b"TrimBox").is_ok() && page.get(b"ArtBox").is_ok()),
            "invalid_page_boxes",
        );
        if let Some(annots) = page
            .get(b"Annots")
            .ok()
            .and_then(|o| resolved(pdf, o))
            .and_then(|o| o.as_array().ok())
        {
            for annotation in annots {
                let bounds = resolved(pdf, annotation)
                    .and_then(|o| o.as_dict().ok())
                    .and_then(|d| rect(d, b"Rect"));
                check(
                    bounds.zip(bleed).is_some_and(|(a, b)| {
                        a[2] < b[0] || a[0] > b[2] || a[3] < b[1] || a[1] > b[3]
                    }),
                    "annotation_in_print_area",
                );
            }
        }
    }
    errors.sort();
    errors.dedup();
    (errors, modify)
}
