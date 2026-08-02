use crate::pdf_native::{
    Dictionary as LoDictionary, Document as LoDocument, Object as LoObject, ObjectId as LoObjectId,
};
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PdfInspectErrorCode {
    PdfParseFailed,
    PdfEncryptedUnsupported,
    PdfEmptyOrNoPages,
    PdfIoError,
}

impl PdfInspectErrorCode {
    pub fn as_str(&self) -> &'static str {
        match self {
            PdfInspectErrorCode::PdfParseFailed => "PDF_PARSE_FAILED",
            PdfInspectErrorCode::PdfEncryptedUnsupported => "PDF_ENCRYPTED_UNSUPPORTED",
            PdfInspectErrorCode::PdfEmptyOrNoPages => "PDF_EMPTY_OR_NO_PAGES",
            PdfInspectErrorCode::PdfIoError => "PDF_IO_ERROR",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PdfInspectError {
    pub code: PdfInspectErrorCode,
    pub message: String,
}

impl std::fmt::Display for PdfInspectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code.as_str(), self.message)
    }
}

impl std::error::Error for PdfInspectError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PdfInspectWarning {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PdfProfileInspection {
    pub claims: Vec<String>,
    pub metadata_present: bool,
    pub output_intent_present: bool,
    pub struct_tree_root_present: bool,
    pub mark_info_present: bool,
    pub lang_present: bool,
    pub embedded_font_count: usize,
    pub embedded_files_present: bool,
    pub pdf_declaration_present: bool,
    pub dpart_root_present: bool,
    pub dpart_present: bool,
    pub page_dpart_present: bool,
    pub pdfvt_dpart_root_node_valid: bool,
    pub pdfvt_dpart_parent_valid: bool,
    pub pdfvt_dpart_node_name_list_valid: bool,
    pub pdfvt_dpart_leaf_valid: bool,
    pub pdfvt_dpart_page_range_valid: bool,
    pub pdfvt_dpart_graph_valid: bool,
    pub pdfvt_mod_date_matches_xmp: Option<bool>,
    pub seed_blockers: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PdfInspectReport {
    pub pdf_version: String,
    pub page_count: usize,
    pub encrypted: bool,
    pub file_size_bytes: usize,
    pub warnings: Vec<PdfInspectWarning>,
    pub profile: PdfProfileInspection,
}

pub fn inspect_pdf_bytes(bytes: &[u8]) -> Result<PdfInspectReport, PdfInspectError> {
    let pdf = LoDocument::load_mem(bytes).map_err(|err| PdfInspectError {
        code: PdfInspectErrorCode::PdfParseFailed,
        message: err.to_string(),
    })?;

    Ok(PdfInspectReport {
        pdf_version: pdf.version.clone(),
        page_count: pdf.get_pages().len(),
        encrypted: pdf.is_encrypted(),
        file_size_bytes: bytes.len(),
        warnings: Vec::new(),
        profile: inspect_profile_markers(bytes, &pdf),
    })
}

pub fn inspect_pdf_path(path: &Path) -> Result<PdfInspectReport, PdfInspectError> {
    let data = std::fs::read(path).map_err(|err| PdfInspectError {
        code: PdfInspectErrorCode::PdfIoError,
        message: err.to_string(),
    })?;
    inspect_pdf_bytes(&data)
}

fn contains_token(bytes: &[u8], token: &[u8]) -> bool {
    !token.is_empty() && bytes.windows(token.len()).any(|window| window == token)
}

fn count_token(bytes: &[u8], token: &[u8]) -> usize {
    if token.is_empty() || bytes.len() < token.len() {
        return 0;
    }
    bytes
        .windows(token.len())
        .filter(|window| *window == token)
        .count()
}

fn extract_xml_attr(bytes: &[u8], attr: &[u8]) -> Option<String> {
    let pos = bytes
        .windows(attr.len())
        .position(|window| window == attr)?;
    let start = pos + attr.len();
    let quote = *bytes.get(start)?;
    if quote != b'"' && quote != b'\'' {
        return None;
    }
    let value_start = start + 1;
    let value_end = bytes[value_start..]
        .iter()
        .position(|b| *b == quote)
        .map(|idx| value_start + idx)?;
    std::str::from_utf8(&bytes[value_start..value_end])
        .ok()
        .map(str::to_string)
}

fn contains_pdfa_identification(bytes: &[u8], part: &[u8], conformance: Option<&[u8]>) -> bool {
    contains_token(bytes, part) && conformance.map_or(true, |token| contains_token(bytes, token))
}

#[derive(Debug, Clone, Copy, Default)]
struct PdfVtDPartInspection {
    dpart_root_present: bool,
    dpart_present: bool,
    page_dpart_present: bool,
    root_node_valid: bool,
    parent_valid: bool,
    node_name_list_valid: bool,
    leaf_valid: bool,
    page_range_valid: bool,
    graph_valid: bool,
}

fn dict_reference(dict: &LoDictionary, key: &[u8]) -> Option<LoObjectId> {
    dict.get(key).ok()?.as_reference().ok()
}

fn object_dict<'a>(pdf: &'a LoDocument, id: LoObjectId) -> Option<&'a LoDictionary> {
    pdf.get_object(id).and_then(LoObject::as_dict).ok()
}

fn dict_has_type(dict: &LoDictionary, expected: &[u8]) -> bool {
    dict.get(b"Type")
        .ok()
        .and_then(|obj| obj.as_name().ok())
        .map_or(false, |name| name == expected)
}

fn node_name_list_is_single_document_level(dict: &LoDictionary) -> bool {
    dict.get(b"NodeNameList")
        .ok()
        .and_then(|obj| obj.as_array().ok())
        .map_or(false, |items| {
            items.len() == 1
                && items.first().and_then(|item| item.as_name().ok())
                    == Some(b"Document".as_slice())
        })
}

fn page_dpart_reference(pdf: &LoDocument, page_id: LoObjectId) -> Option<LoObjectId> {
    object_dict(pdf, page_id).and_then(|page| dict_reference(page, b"DPart"))
}

fn inspect_pdfvt_dpart_graph(pdf: &LoDocument) -> PdfVtDPartInspection {
    let mut out = PdfVtDPartInspection::default();
    let pages: Vec<LoObjectId> = pdf.get_pages().values().copied().collect();
    if pages.is_empty() {
        return out;
    }

    let Some(catalog_id) = pdf
        .trailer
        .get(b"Root")
        .ok()
        .and_then(|obj| obj.as_reference().ok())
    else {
        return out;
    };
    let Some(catalog) = object_dict(pdf, catalog_id) else {
        return out;
    };
    let Some(dpart_root_id) = dict_reference(catalog, b"DPartRoot") else {
        return out;
    };
    let Some(dpart_root) = object_dict(pdf, dpart_root_id) else {
        return out;
    };
    out.dpart_root_present = dict_has_type(dpart_root, b"DPartRoot");
    out.node_name_list_valid = node_name_list_is_single_document_level(dpart_root);

    let Some(dpart_node_id) = dict_reference(dpart_root, b"DPartRootNode") else {
        return out;
    };
    let Some(dpart_node) = object_dict(pdf, dpart_node_id) else {
        return out;
    };
    out.dpart_present = dict_has_type(dpart_node, b"DPart");
    out.root_node_valid = out.dpart_root_present && out.dpart_present;

    let page_dparts_match = pages
        .iter()
        .all(|page_id| page_dpart_reference(pdf, *page_id) == Some(dpart_node_id));
    out.page_dpart_present = page_dparts_match;

    let parent_ok = dict_reference(dpart_node, b"Parent") == Some(dpart_root_id);
    let start_ok = pages.first().copied() == dict_reference(dpart_node, b"Start");
    let end_ref = dict_reference(dpart_node, b"End");
    let end_ok = if pages.len() > 1 {
        pages.last().copied() == end_ref
    } else {
        end_ref.is_none()
    };
    out.parent_valid = parent_ok;
    out.page_range_valid = start_ok && end_ok;
    out.leaf_valid =
        dpart_node.get(b"DParts").is_err() && dict_reference(dpart_node, b"Start").is_some();

    out.graph_valid = out.dpart_root_present
        && out.dpart_present
        && out.page_dpart_present
        && out.root_node_valid
        && out.parent_valid
        && out.node_name_list_valid
        && out.leaf_valid
        && out.page_range_valid;
    out
}

fn inspect_profile_markers(bytes: &[u8], pdf: &LoDocument) -> PdfProfileInspection {
    let pdfvt_dpart = inspect_pdfvt_dpart_graph(pdf);
    let mut out = PdfProfileInspection {
        metadata_present: contains_token(bytes, b"/Metadata"),
        output_intent_present: contains_token(bytes, b"/OutputIntents"),
        struct_tree_root_present: contains_token(bytes, b"/StructTreeRoot"),
        mark_info_present: contains_token(bytes, b"/MarkInfo"),
        lang_present: contains_token(bytes, b"/Lang"),
        embedded_font_count: count_token(bytes, b"/FontFile"),
        embedded_files_present: contains_token(bytes, b"/EmbeddedFiles"),
        pdf_declaration_present: contains_token(bytes, b"<pdfd:declarations>"),
        dpart_root_present: pdfvt_dpart.dpart_root_present,
        dpart_present: pdfvt_dpart.dpart_present,
        page_dpart_present: pdfvt_dpart.page_dpart_present,
        pdfvt_dpart_root_node_valid: pdfvt_dpart.root_node_valid,
        pdfvt_dpart_parent_valid: pdfvt_dpart.parent_valid,
        pdfvt_dpart_node_name_list_valid: pdfvt_dpart.node_name_list_valid,
        pdfvt_dpart_leaf_valid: pdfvt_dpart.leaf_valid,
        pdfvt_dpart_page_range_valid: pdfvt_dpart.page_range_valid,
        pdfvt_dpart_graph_valid: pdfvt_dpart.graph_valid,
        ..PdfProfileInspection::default()
    };

    for (claim, part, conformance) in [
        (
            "pdfa1a",
            b"pdfaid:part=\"1\"".as_slice(),
            Some(b"pdfaid:conformance=\"A\"".as_slice()),
        ),
        (
            "pdfa1b",
            b"pdfaid:part=\"1\"".as_slice(),
            Some(b"pdfaid:conformance=\"B\"".as_slice()),
        ),
        (
            "pdfa2a",
            b"pdfaid:part=\"2\"".as_slice(),
            Some(b"pdfaid:conformance=\"A\"".as_slice()),
        ),
        (
            "pdfa2b",
            b"pdfaid:part=\"2\"".as_slice(),
            Some(b"pdfaid:conformance=\"B\"".as_slice()),
        ),
        (
            "pdfa2u",
            b"pdfaid:part=\"2\"".as_slice(),
            Some(b"pdfaid:conformance=\"U\"".as_slice()),
        ),
        (
            "pdfa3a",
            b"pdfaid:part=\"3\"".as_slice(),
            Some(b"pdfaid:conformance=\"A\"".as_slice()),
        ),
        (
            "pdfa3b",
            b"pdfaid:part=\"3\"".as_slice(),
            Some(b"pdfaid:conformance=\"B\"".as_slice()),
        ),
        (
            "pdfa3u",
            b"pdfaid:part=\"3\"".as_slice(),
            Some(b"pdfaid:conformance=\"U\"".as_slice()),
        ),
        (
            "pdfa4",
            b"pdfaid:part=\"4\"".as_slice(),
            Some(b"pdfaid:rev=\"2020\"".as_slice()),
        ),
        (
            "pdfa4e",
            b"pdfaid:part=\"4\"".as_slice(),
            Some(b"pdfaid:conformance=\"E\"".as_slice()),
        ),
        (
            "pdfa4f",
            b"pdfaid:part=\"4\"".as_slice(),
            Some(b"pdfaid:conformance=\"F\"".as_slice()),
        ),
    ] {
        if claim == "pdfa4" {
            if contains_pdfa_identification(bytes, part, conformance)
                && !contains_token(bytes, b"pdfaid:conformance=")
            {
                out.claims.push(claim.to_string());
            }
        } else if contains_pdfa_identification(bytes, part, conformance) {
            out.claims.push(claim.to_string());
        }
    }

    for (claim, token) in [
        ("pdfua1", b"pdfuaid:part=\"1\"".as_slice()),
        ("pdfua2", b"pdfuaid:part=\"2\"".as_slice()),
        (
            "pdfx4",
            b"<pdfxid:GTS_PDFXVersion>PDF/X-4</pdfxid:GTS_PDFXVersion>".as_slice(),
        ),
        (
            "pdfvt1",
            b"pdfvtid:GTS_PDFVTVersion=\"PDF/VT-1\"".as_slice(),
        ),
        (
            "wtpdf1r",
            b"http://pdfa.org/declarations/wtpdf#reuse1.0".as_slice(),
        ),
        (
            "wtpdf1a",
            b"http://pdfa.org/declarations/wtpdf#accessibility1.0".as_slice(),
        ),
    ] {
        if contains_token(bytes, token) {
            out.claims.push(claim.to_string());
        }
    }

    if out.struct_tree_root_present
        && out.mark_info_present
        && !out.claims.iter().any(|c| c == "pdfua1")
        && !out.claims.iter().any(|c| c == "pdfua2")
    {
        out.claims.push("tagged".to_string());
    }
    out.claims.sort();
    out.claims.dedup();

    if out.claims.iter().any(|c| {
        matches!(
            c.as_str(),
            "pdfa1a"
                | "pdfa1b"
                | "pdfa2a"
                | "pdfa2b"
                | "pdfa2u"
                | "pdfa3a"
                | "pdfa3b"
                | "pdfa3u"
                | "pdfa4"
                | "pdfa4e"
                | "pdfa4f"
                | "pdfx4"
                | "pdfvt1"
        )
    }) && !out.output_intent_present
    {
        out.seed_blockers
            .push("profile_requires_output_intent".to_string());
    }
    if out.claims.iter().any(|c| c == "pdfa4f") && !out.embedded_files_present {
        out.seed_blockers
            .push("pdfa4f_missing_embedded_files".to_string());
    }
    if out
        .claims
        .iter()
        .any(|c| matches!(c.as_str(), "pdfa1a" | "pdfa2a" | "pdfa3a"))
    {
        if !out.struct_tree_root_present {
            out.seed_blockers
                .push("pdfa_missing_struct_tree_root".to_string());
        }
        if !out.mark_info_present {
            out.seed_blockers.push("pdfa_missing_mark_info".to_string());
        }
        if !out.lang_present {
            out.seed_blockers.push("pdfa_missing_lang".to_string());
        }
    }
    if out
        .claims
        .iter()
        .any(|c| matches!(c.as_str(), "pdfua1" | "pdfua2" | "wtpdf1r" | "wtpdf1a"))
    {
        if !out.struct_tree_root_present {
            out.seed_blockers
                .push("pdfua_missing_struct_tree_root".to_string());
        }
        if !out.mark_info_present {
            out.seed_blockers
                .push("pdfua_missing_mark_info".to_string());
        }
        if !out.lang_present {
            out.seed_blockers.push("pdfua_missing_lang".to_string());
        }
    }
    if out
        .claims
        .iter()
        .any(|c| matches!(c.as_str(), "wtpdf1r" | "wtpdf1a"))
        && !out.pdf_declaration_present
    {
        out.seed_blockers
            .push("wtpdf_missing_pdf_declaration".to_string());
    }
    if out.claims.iter().any(|c| c == "pdfvt1") {
        let pdfvt_mod = extract_xml_attr(bytes, b"pdfvtid:GTS_PDFVTModDate=");
        let xmp_mod = extract_xml_attr(bytes, b"xmp:ModifyDate=");
        out.pdfvt_mod_date_matches_xmp = match (pdfvt_mod, xmp_mod) {
            (Some(a), Some(b)) => Some(a == b),
            _ => Some(false),
        };
        if out.pdfvt_mod_date_matches_xmp != Some(true) {
            out.seed_blockers
                .push("pdfvt_mod_date_mismatch".to_string());
        }
        if !out.dpart_root_present {
            out.seed_blockers
                .push("pdfvt_missing_dpart_root".to_string());
        }
        if !out.dpart_present {
            out.seed_blockers.push("pdfvt_missing_dpart".to_string());
        }
        if !out.page_dpart_present {
            out.seed_blockers
                .push("pdfvt_missing_page_dpart".to_string());
        }
        if !out.pdfvt_dpart_root_node_valid {
            out.seed_blockers
                .push("pdfvt_invalid_dpart_root_node".to_string());
        }
        if !out.pdfvt_dpart_parent_valid {
            out.seed_blockers
                .push("pdfvt_invalid_dpart_parent".to_string());
        }
        if !out.pdfvt_dpart_node_name_list_valid {
            out.seed_blockers
                .push("pdfvt_invalid_dpart_node_name_list".to_string());
        }
        if !out.pdfvt_dpart_leaf_valid {
            out.seed_blockers
                .push("pdfvt_invalid_dpart_leaf".to_string());
        }
        if !out.pdfvt_dpart_page_range_valid {
            out.seed_blockers
                .push("pdfvt_invalid_dpart_page_range".to_string());
        }
        if !out.pdfvt_dpart_graph_valid {
            out.seed_blockers
                .push("pdfvt_invalid_dpart_graph".to_string());
        }
    }

    out
}

pub fn composition_compatibility_issues(report: &PdfInspectReport) -> Vec<PdfInspectErrorCode> {
    let mut issues = Vec::new();
    if report.encrypted {
        issues.push(PdfInspectErrorCode::PdfEncryptedUnsupported);
    }
    if report.page_count == 0 {
        issues.push(PdfInspectErrorCode::PdfEmptyOrNoPages);
    }
    issues
}

pub fn require_pdf_composition_compatibility(
    report: &PdfInspectReport,
) -> Result<(), PdfInspectError> {
    for issue in composition_compatibility_issues(report) {
        match issue {
            PdfInspectErrorCode::PdfEncryptedUnsupported => {
                return Err(PdfInspectError {
                    code: PdfInspectErrorCode::PdfEncryptedUnsupported,
                    message: "encrypted pdf assets are not supported".to_string(),
                });
            }
            PdfInspectErrorCode::PdfEmptyOrNoPages => {
                return Err(PdfInspectError {
                    code: PdfInspectErrorCode::PdfEmptyOrNoPages,
                    message: "pdf has no pages".to_string(),
                });
            }
            PdfInspectErrorCode::PdfParseFailed | PdfInspectErrorCode::PdfIoError => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::{Document, Page};
    use crate::pdf::{
        OutputIntent, PdfOptions, PdfProfile, document_to_pdf_with_metrics_and_registry,
    };
    use crate::pdf_native::{Object as LoObject, Stream as LoStream, dictionary};
    use crate::types::Size;
    use std::io::Write;

    fn make_single_page_pdf_bytes(text: &str) -> Vec<u8> {
        let mut doc = LoDocument::with_version("1.5");
        let pages_id = doc.new_object_id();
        let font_id = doc.add_object(dictionary! {
            "Type" => "Font",
            "Subtype" => "Type1",
            "BaseFont" => "Helvetica",
        });
        let resources_id = doc.add_object(dictionary! {
            "Font" => dictionary! { "F1" => font_id },
        });
        let content = format!("BT /F1 18 Tf 72 720 Td ({}) Tj ET", text).into_bytes();
        let content_id = doc.add_object(LoStream::new(dictionary! {}, content));
        let page_id = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "Contents" => content_id,
            "Resources" => resources_id,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
        });
        let pages = dictionary! {
            "Type" => "Pages",
            "Kids" => vec![page_id.into()],
            "Count" => 1,
        };
        doc.objects.insert(pages_id, LoObject::Dictionary(pages));
        let catalog_id = doc.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
        });
        doc.trailer.set("Root", catalog_id);
        doc.compress();

        let mut out = Vec::new();
        doc.save_to(&mut out).expect("save");
        out
    }

    fn rendered_empty_profile_pdf_with_page_count(
        profile: PdfProfile,
        page_count: usize,
    ) -> Vec<u8> {
        let doc = Document {
            page_size: Size::a4(),
            pages: (0..page_count)
                .map(|_| Page {
                    commands: Vec::new(),
                })
                .collect(),
        };
        let mut options = PdfOptions::default();
        options.pdf_profile = profile;
        if profile.requires_output_intent() {
            options.output_intent = Some(OutputIntent::new(
                vec![0x00, 0x01, 0x02],
                3,
                "sRGB IEC61966-2.1",
                Some("sRGB".to_string()),
            ));
        }
        document_to_pdf_with_metrics_and_registry(&doc, None, None, &options)
            .expect("render profile pdf")
    }

    fn rendered_empty_profile_pdf(profile: PdfProfile) -> Vec<u8> {
        rendered_empty_profile_pdf_with_page_count(profile, 1)
    }

    #[test]
    fn inspect_pdf_bytes_reads_version_and_page_count() {
        let bytes = make_single_page_pdf_bytes("HELLO");
        let report = inspect_pdf_bytes(&bytes).expect("inspect");
        assert_eq!(report.page_count, 1);
        assert!(!report.encrypted);
        assert_eq!(report.file_size_bytes, bytes.len());
        assert!(!report.pdf_version.is_empty());
        assert!(report.profile.claims.is_empty());
    }

    #[test]
    fn inspect_pdf_bytes_reports_pdfa_profile_markers() {
        let bytes = rendered_empty_profile_pdf(PdfProfile::PdfA2b);
        let report = inspect_pdf_bytes(&bytes).expect("inspect");
        assert!(report.profile.claims.contains(&"pdfa2b".to_string()));
        assert!(!report.profile.claims.contains(&"pdfa2u".to_string()));
        assert!(report.profile.metadata_present);
        assert!(report.profile.output_intent_present);
        assert!(report.profile.seed_blockers.is_empty());
    }

    #[test]
    fn inspect_pdf_bytes_reports_pdfau_profile_markers() {
        let bytes = rendered_empty_profile_pdf(PdfProfile::PdfA2u);
        let report = inspect_pdf_bytes(&bytes).expect("inspect");
        assert!(report.profile.claims.contains(&"pdfa2u".to_string()));
        assert!(!report.profile.claims.contains(&"pdfa2b".to_string()));
        assert!(report.profile.metadata_present);
        assert!(report.profile.output_intent_present);
        assert!(report.profile.seed_blockers.is_empty());
    }

    #[test]
    fn inspect_pdf_bytes_reports_pdfa4_profile_markers() {
        let bytes = rendered_empty_profile_pdf(PdfProfile::PdfA4);
        let report = inspect_pdf_bytes(&bytes).expect("inspect");
        assert_eq!(report.pdf_version, "2.0");
        assert!(report.profile.claims.contains(&"pdfa4".to_string()));
        assert!(!report.profile.claims.contains(&"pdfa4e".to_string()));
        assert!(!report.profile.claims.contains(&"pdfa4f".to_string()));
        assert!(report.profile.metadata_present);
        assert!(report.profile.output_intent_present);
        assert!(report.profile.seed_blockers.is_empty());
    }

    #[test]
    fn inspect_pdf_bytes_reports_pdfa4_conformance_profile_markers() {
        for (profile, claim, embedded_files) in [
            (PdfProfile::PdfA4e, "pdfa4e", false),
            (PdfProfile::PdfA4f, "pdfa4f", true),
        ] {
            let bytes = rendered_empty_profile_pdf(profile);
            let report = inspect_pdf_bytes(&bytes).expect("inspect");
            assert_eq!(report.pdf_version, "2.0");
            assert!(report.profile.claims.contains(&claim.to_string()));
            assert!(!report.profile.claims.contains(&"pdfa4".to_string()));
            assert_eq!(report.profile.embedded_files_present, embedded_files);
            assert!(report.profile.metadata_present);
            assert!(report.profile.output_intent_present);
            assert!(report.profile.seed_blockers.is_empty());
        }
    }

    #[test]
    fn inspect_pdf_bytes_reports_wtpdf_profile_markers() {
        for (profile, claim) in [
            (PdfProfile::Wtpdf1r, "wtpdf1r"),
            (PdfProfile::Wtpdf1a, "wtpdf1a"),
        ] {
            let doc = Document {
                page_size: Size::a4(),
                pages: vec![Page {
                    commands: Vec::new(),
                }],
            };
            let mut options = PdfOptions::default();
            options.pdf_profile = profile;
            options.document_lang = Some("en-US".to_string());
            let bytes = document_to_pdf_with_metrics_and_registry(&doc, None, None, &options)
                .expect("render wtpdf profile pdf");
            let report = inspect_pdf_bytes(&bytes).expect("inspect");
            assert_eq!(report.pdf_version, "2.0");
            assert!(report.profile.claims.contains(&claim.to_string()));
            assert!(report.profile.pdf_declaration_present);
            assert!(report.profile.struct_tree_root_present);
            assert!(report.profile.mark_info_present);
            assert!(report.profile.lang_present);
            assert!(report.profile.seed_blockers.is_empty());
        }
    }

    #[test]
    fn inspect_pdf_bytes_reports_pdfvt_profile_markers() {
        let bytes = rendered_empty_profile_pdf(PdfProfile::PdfVt1);
        let report = inspect_pdf_bytes(&bytes).expect("inspect");
        assert!(report.profile.claims.contains(&"pdfvt1".to_string()));
        assert!(report.profile.claims.contains(&"pdfx4".to_string()));
        assert_eq!(report.profile.pdfvt_mod_date_matches_xmp, Some(true));
        assert!(report.profile.output_intent_present);
        assert!(report.profile.dpart_root_present);
        assert!(report.profile.dpart_present);
        assert!(report.profile.page_dpart_present);
        assert!(report.profile.pdfvt_dpart_root_node_valid);
        assert!(report.profile.pdfvt_dpart_parent_valid);
        assert!(report.profile.pdfvt_dpart_node_name_list_valid);
        assert!(report.profile.pdfvt_dpart_leaf_valid);
        assert!(report.profile.pdfvt_dpart_page_range_valid);
        assert!(report.profile.pdfvt_dpart_graph_valid);
        assert!(report.profile.seed_blockers.is_empty());
    }

    #[test]
    fn inspect_pdf_bytes_reports_multipage_pdfvt_dpart_range() {
        let bytes = rendered_empty_profile_pdf_with_page_count(PdfProfile::PdfVt1, 2);
        let report = inspect_pdf_bytes(&bytes).expect("inspect");
        assert_eq!(report.page_count, 2);
        assert!(report.profile.claims.contains(&"pdfvt1".to_string()));
        assert!(report.profile.dpart_root_present);
        assert!(report.profile.dpart_present);
        assert!(report.profile.page_dpart_present);
        assert!(report.profile.pdfvt_dpart_root_node_valid);
        assert!(report.profile.pdfvt_dpart_parent_valid);
        assert!(report.profile.pdfvt_dpart_node_name_list_valid);
        assert!(report.profile.pdfvt_dpart_leaf_valid);
        assert!(report.profile.pdfvt_dpart_page_range_valid);
        assert!(report.profile.pdfvt_dpart_graph_valid);
        assert!(report.profile.seed_blockers.is_empty());
    }

    #[test]
    fn inspect_pdf_bytes_flags_invalid_pdfvt_dpart_graph() {
        let bytes = rendered_empty_profile_pdf(PdfProfile::PdfVt1);
        let mut doc = LoDocument::load_mem(&bytes).expect("load pdfvt");
        let catalog_id = doc
            .trailer
            .get(b"Root")
            .and_then(LoObject::as_reference)
            .expect("catalog ref");
        let dpart_root_id = {
            let catalog = object_dict(&doc, catalog_id).expect("catalog dict");
            dict_reference(catalog, b"DPartRoot").expect("dpart root ref")
        };
        let dpart_node_id = {
            let dpart_root = object_dict(&doc, dpart_root_id).expect("dpart root dict");
            dict_reference(dpart_root, b"DPartRootNode").expect("dpart node ref")
        };
        let dpart_node = doc
            .objects
            .get_mut(&dpart_node_id)
            .and_then(|obj| obj.as_dict_mut().ok())
            .expect("dpart node dict");
        dpart_node.set("DParts", LoObject::Array(Vec::new()));

        let mut malformed = Vec::new();
        doc.save_to(&mut malformed).expect("save malformed pdfvt");
        let report = inspect_pdf_bytes(&malformed).expect("inspect malformed pdfvt");

        assert!(report.profile.claims.contains(&"pdfvt1".to_string()));
        assert!(report.profile.dpart_root_present);
        assert!(report.profile.dpart_present);
        assert!(report.profile.page_dpart_present);
        assert!(report.profile.pdfvt_dpart_root_node_valid);
        assert!(report.profile.pdfvt_dpart_parent_valid);
        assert!(report.profile.pdfvt_dpart_node_name_list_valid);
        assert!(!report.profile.pdfvt_dpart_leaf_valid);
        assert!(report.profile.pdfvt_dpart_page_range_valid);
        assert!(!report.profile.pdfvt_dpart_graph_valid);
        assert!(
            report
                .profile
                .seed_blockers
                .contains(&"pdfvt_invalid_dpart_leaf".to_string())
        );
        assert!(
            report
                .profile
                .seed_blockers
                .contains(&"pdfvt_invalid_dpart_graph".to_string())
        );
    }

    #[test]
    fn inspect_pdf_bytes_rejects_malformed_data() {
        let err = inspect_pdf_bytes(b"not a pdf").expect_err("invalid");
        assert_eq!(err.code, PdfInspectErrorCode::PdfParseFailed);
    }

    #[test]
    fn inspect_pdf_path_reports_io_error_for_missing_file() {
        let missing = std::env::temp_dir().join(format!(
            "fullbleed_pdfinspect_missing_{}_{}.pdf",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        let err = inspect_pdf_path(&missing).expect_err("missing");
        assert_eq!(err.code, PdfInspectErrorCode::PdfIoError);
    }

    #[test]
    fn composition_compatibility_rejects_encrypted() {
        let report = PdfInspectReport {
            pdf_version: "1.7".to_string(),
            page_count: 1,
            encrypted: true,
            file_size_bytes: 0,
            warnings: Vec::new(),
            profile: PdfProfileInspection::default(),
        };
        let issues = composition_compatibility_issues(&report);
        assert!(issues.contains(&PdfInspectErrorCode::PdfEncryptedUnsupported));

        let err = require_pdf_composition_compatibility(&report).expect_err("must fail");
        assert_eq!(err.code, PdfInspectErrorCode::PdfEncryptedUnsupported);
    }

    #[test]
    fn composition_compatibility_rejects_empty_page_count() {
        let report = PdfInspectReport {
            pdf_version: "1.7".to_string(),
            page_count: 0,
            encrypted: false,
            file_size_bytes: 0,
            warnings: Vec::new(),
            profile: PdfProfileInspection::default(),
        };
        let issues = composition_compatibility_issues(&report);
        assert_eq!(issues, vec![PdfInspectErrorCode::PdfEmptyOrNoPages]);
        let err = require_pdf_composition_compatibility(&report).expect_err("must fail");
        assert_eq!(err.code, PdfInspectErrorCode::PdfEmptyOrNoPages);
    }

    #[test]
    fn inspect_pdf_path_matches_bytes_report() {
        let bytes = make_single_page_pdf_bytes("PATH");
        let temp_dir = std::env::temp_dir().join(format!(
            "fullbleed_pdfinspect_path_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        std::fs::create_dir_all(&temp_dir).expect("mkdir");
        let path = temp_dir.join("one.pdf");
        let mut f = std::fs::File::create(&path).expect("create");
        f.write_all(&bytes).expect("write");

        let from_path = inspect_pdf_path(&path).expect("inspect path");
        let from_bytes = inspect_pdf_bytes(&bytes).expect("inspect bytes");
        assert_eq!(from_path.page_count, from_bytes.page_count);
        assert_eq!(from_path.encrypted, from_bytes.encrypted);
        assert_eq!(from_path.pdf_version, from_bytes.pdf_version);
    }
}
