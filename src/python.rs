#![allow(unsafe_op_in_unsafe_fn)]

use crate::assets::is_supported_font_path;
use crate::{
    A11yVerifierCoreReport, A11yVerifierEvidence, A11yVerifierFinding, Asset, AssetBundle,
    AssetKind, Color, ColorSpace, FullBleed, FullBleedBuilder, FullBleedError, GlyphCoverageReport,
    JitMode, LayoutStrategy, Margins, OutputIntent, PageDataContext, PageDataValue, PdfProfile,
    PdfVersion, PmrCoreAudit, PmrCoreContext, PmrCoreEvidence, PmrCoreReport, Pt, Size,
    WatermarkLayer, WatermarkSemantics, WatermarkSpec, Command, Document,
    composition_compatibility_issues, inspect_pdf_bytes, inspect_pdf_path,
    require_pdf_composition_compatibility,
};
use base64::Engine;
use fullbleed_audit_contract as audit_contract;
use lopdf::{Document as LoDocument, Object as LoObject};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict, PyList, PyModule};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::Read;
use std::path::{Path, PathBuf};

#[pyclass(name = "AssetKind")]
struct PyAssetKind;

#[pymethods]
impl PyAssetKind {
    #[allow(non_upper_case_globals)]
    #[classattr]
    const Css: &'static str = "css";
    #[allow(non_upper_case_globals)]
    #[classattr]
    const Font: &'static str = "font";
    #[allow(non_upper_case_globals)]
    #[classattr]
    const Image: &'static str = "image";
    #[allow(non_upper_case_globals)]
    #[classattr]
    const Pdf: &'static str = "pdf";
    #[allow(non_upper_case_globals)]
    #[classattr]
    const Svg: &'static str = "svg";
    #[allow(non_upper_case_globals)]
    #[classattr]
    const Other: &'static str = "other";
}

#[pyclass(name = "Asset")]
#[derive(Clone)]
struct PyAsset {
    asset: Asset,
}

#[pymethods]
impl PyAsset {
    fn info(&self, py: Python<'_>) -> PyResult<PyObject> {
        let info = PyDict::new_bound(py);
        info.set_item("name", self.asset.name.clone())?;
        info.set_item("kind", self.asset.kind.as_str())?;
        info.set_item("bytes", self.asset.bytes_len())?;
        info.set_item("trusted", self.asset.trusted)?;
        if let Some(source) = &self.asset.source {
            info.set_item("source", source.clone())?;
        }
        if self.asset.kind == AssetKind::Font {
            if let Some(font_name) =
                crate::font::font_primary_name_from_bytes(&self.asset.data, Some(&self.asset.name))
            {
                info.set_item("font", font_name)?;
            }
        } else if self.asset.kind == AssetKind::Pdf {
            let report =
                inspect_pdf_bytes(&self.asset.data).map_err(pdf_asset_inspect_err_to_py)?;
            info.set_item("pdf_version", report.pdf_version.as_str())?;
            info.set_item("page_count", report.page_count)?;
            info.set_item("encrypted", report.encrypted)?;
            let issues = composition_compatibility_issues(&report);
            let issue_list = PyList::empty_bound(py);
            for issue in &issues {
                issue_list.append(issue.as_str())?;
            }
            info.set_item("composition_supported", issues.is_empty())?;
            info.set_item("composition_issues", issue_list)?;
        }
        Ok(info.to_object(py))
    }
}

#[pyclass(name = "AssetBundle")]
#[derive(Clone, Default)]
struct PyAssetBundle {
    bundle: AssetBundle,
}

#[pymethods]
impl PyAssetBundle {
    #[new]
    fn new() -> Self {
        Self {
            bundle: AssetBundle::default(),
        }
    }

    fn add(&mut self, asset: PyRef<'_, PyAsset>) {
        self.bundle.add(asset.asset.clone());
    }

    #[pyo3(signature = (path, kind, name=None, trusted=false, remote=false))]
    fn add_file(
        &mut self,
        py: Python<'_>,
        path: &str,
        kind: &str,
        name: Option<String>,
        trusted: bool,
        remote: bool,
    ) -> PyResult<PyObject> {
        let kind = parse_asset_kind(kind)?;
        let asset = build_asset(py, path, kind, name, trusted, remote)?;
        let wrapper = PyAsset {
            asset: asset.clone(),
        };
        self.bundle.add(asset);
        let py_obj = Py::new(py, wrapper)?;
        Ok(py_obj.to_object(py))
    }

    fn css(&self) -> String {
        self.bundle.css_text()
    }

    fn assets_info(&self, py: Python<'_>) -> PyResult<PyObject> {
        let list = PyList::empty_bound(py);
        for asset in &self.bundle.assets {
            let wrapper = PyAsset {
                asset: asset.clone(),
            };
            list.append(wrapper.info(py)?)?;
        }
        Ok(list.to_object(py))
    }
}

fn parse_asset_kind(raw: &str) -> PyResult<AssetKind> {
    AssetKind::from_str(raw).ok_or_else(|| {
        PyValueError::new_err(format!(
            "unknown asset kind: {raw:?} (expected css|font|image|pdf|svg|other)"
        ))
    })
}

fn infer_asset_name(source: &str, name: Option<String>) -> String {
    if let Some(name) = name {
        return name;
    }
    if source.starts_with("http://") || source.starts_with("https://") {
        if let Some(last) = source.rsplit('/').next() {
            if !last.is_empty() {
                return last.to_string();
            }
        }
        return "remote_asset".to_string();
    }
    Path::new(source)
        .file_name()
        .and_then(|v| v.to_str())
        .unwrap_or(source)
        .to_string()
}

fn load_asset_bytes(py: Python<'_>, source: &str, remote: bool) -> PyResult<Vec<u8>> {
    if source.starts_with("http://") || source.starts_with("https://") {
        if !remote {
            return Err(PyValueError::new_err(
                "remote asset disabled (set remote=True to fetch)",
            ));
        }
        let urllib = py.import_bound("urllib.request")?;
        let response = urllib.call_method1("urlopen", (source,))?;
        let data = response.call_method0("read")?;
        let bytes = data.downcast::<PyBytes>()?;
        return Ok(bytes.as_bytes().to_vec());
    }
    std::fs::read(source).map_err(|err| PyValueError::new_err(err.to_string()))
}

fn build_asset(
    py: Python<'_>,
    source: &str,
    kind: AssetKind,
    name: Option<String>,
    trusted: bool,
    remote: bool,
) -> PyResult<Asset> {
    let asset_name = infer_asset_name(source, name);
    if kind == AssetKind::Font && !remote {
        let path = Path::new(source);
        if !is_supported_font_path(path) {
            return Err(PyValueError::new_err(
                "unsupported font format (expected .ttf or .otf)",
            ));
        }
    }
    let data = load_asset_bytes(py, source, remote)?;
    if kind == AssetKind::Font {
        if crate::font::font_primary_name_from_bytes(&data, Some(&asset_name)).is_none() {
            return Err(PyValueError::new_err("invalid font data (unable to parse)"));
        }
    } else if kind == AssetKind::Pdf {
        let report = inspect_pdf_bytes(&data).map_err(pdf_asset_inspect_err_to_py)?;
        require_pdf_composition_compatibility(&report).map_err(pdf_asset_inspect_err_to_py)?;
    }
    Ok(Asset::new(
        asset_name,
        kind,
        data,
        Some(source.to_string()),
        trusted,
    ))
}

#[derive(Debug, Clone)]
struct TemplateCatalogReportItem {
    template_id: String,
    pdf_path: String,
    report: crate::PdfInspectReport,
    issues: Vec<crate::PdfInspectErrorCode>,
}

fn inspect_report_to_py(
    py: Python<'_>,
    path: &str,
    report: &crate::PdfInspectReport,
) -> PyResult<PyObject> {
    let out = PyDict::new_bound(py);
    out.set_item("path", path)?;
    out.set_item("pdf_version", report.pdf_version.as_str())?;
    out.set_item("page_count", report.page_count)?;
    out.set_item("encrypted", report.encrypted)?;
    out.set_item("file_size_bytes", report.file_size_bytes)?;

    let warnings = PyList::empty_bound(py);
    for warning in &report.warnings {
        let d = PyDict::new_bound(py);
        d.set_item("code", warning.code.clone())?;
        d.set_item("message", warning.message.clone())?;
        warnings.append(d)?;
    }
    out.set_item("warnings", warnings)?;

    let issues = composition_compatibility_issues(report);
    let issue_codes = PyList::empty_bound(py);
    for issue in &issues {
        issue_codes.append(issue.as_str())?;
    }
    let compat = PyDict::new_bound(py);
    compat.set_item("supported", issues.is_empty())?;
    compat.set_item("issues", issue_codes)?;
    out.set_item("composition", compat)?;

    Ok(out.to_object(py))
}

fn inspect_template_catalog_entries(
    templates: &[(String, String)],
) -> PyResult<Vec<TemplateCatalogReportItem>> {
    if templates.is_empty() {
        return Err(PyValueError::new_err("template catalog cannot be empty"));
    }

    let mut seen_ids: BTreeSet<&str> = BTreeSet::new();
    let mut out = Vec::with_capacity(templates.len());
    for (idx, (template_id_raw, pdf_path_raw)) in templates.iter().enumerate() {
        let template_id = template_id_raw.trim();
        if template_id.is_empty() {
            return Err(PyValueError::new_err(format!(
                "template catalog item {} has empty template_id",
                idx
            )));
        }
        if !seen_ids.insert(template_id) {
            return Err(PyValueError::new_err(format!(
                "duplicate template_id in template catalog: {}",
                template_id
            )));
        }

        let pdf_path = pdf_path_raw.trim();
        if pdf_path.is_empty() {
            return Err(PyValueError::new_err(format!(
                "template catalog item {} has empty pdf_path",
                idx
            )));
        }

        let report = inspect_pdf_path(Path::new(pdf_path)).map_err(pdf_inspect_err_to_py)?;
        let issues = composition_compatibility_issues(&report);
        out.push(TemplateCatalogReportItem {
            template_id: template_id.to_string(),
            pdf_path: pdf_path.to_string(),
            report,
            issues,
        });
    }

    Ok(out)
}

fn template_catalog_entries_to_py(
    py: Python<'_>,
    entries: &[TemplateCatalogReportItem],
) -> PyResult<PyObject> {
    let templates = PyList::empty_bound(py);
    let mut compatible = 0usize;
    let mut total_pages = 0usize;
    for entry in entries {
        if entry.issues.is_empty() {
            compatible += 1;
        }
        total_pages += entry.report.page_count;

        let d = PyDict::new_bound(py);
        d.set_item("template_id", entry.template_id.clone())?;
        d.set_item("path", entry.pdf_path.clone())?;
        d.set_item("pdf_version", entry.report.pdf_version.as_str())?;
        d.set_item("page_count", entry.report.page_count)?;
        d.set_item("encrypted", entry.report.encrypted)?;
        d.set_item("file_size_bytes", entry.report.file_size_bytes)?;

        let warnings = PyList::empty_bound(py);
        for warning in &entry.report.warnings {
            let w = PyDict::new_bound(py);
            w.set_item("code", warning.code.clone())?;
            w.set_item("message", warning.message.clone())?;
            warnings.append(w)?;
        }
        d.set_item("warnings", warnings)?;

        let issues = PyList::empty_bound(py);
        for issue in &entry.issues {
            issues.append(issue.as_str())?;
        }
        let composition = PyDict::new_bound(py);
        composition.set_item("supported", entry.issues.is_empty())?;
        composition.set_item("issues", issues)?;
        d.set_item("composition", composition)?;

        templates.append(d)?;
    }

    let metrics = PyDict::new_bound(py);
    metrics.set_item("templates", entries.len())?;
    metrics.set_item("compatible_templates", compatible)?;
    metrics.set_item(
        "incompatible_templates",
        entries.len().saturating_sub(compatible),
    )?;
    metrics.set_item("total_template_pages", total_pages)?;

    let out = PyDict::new_bound(py);
    out.set_item("ok", true)?;
    out.set_item("templates", templates)?;
    out.set_item("metrics", metrics)?;
    Ok(out.to_object(py))
}

#[pyfunction]
fn inspect_pdf(py: Python<'_>, path: &str) -> PyResult<PyObject> {
    let report = inspect_pdf_path(Path::new(path)).map_err(pdf_inspect_err_to_py)?;
    let out = inspect_report_to_py(py, path, &report)?;
    let dict = out.bind(py).downcast::<PyDict>()?;
    dict.set_item("ok", true)?;
    Ok(out)
}

fn resolve_lopdf_obj<'a>(doc: &'a LoDocument, mut obj: &'a LoObject) -> Result<&'a LoObject, lopdf::Error> {
    loop {
        match obj {
            LoObject::Reference(id) => {
                obj = doc.get_object(*id)?;
            }
            _ => return Ok(obj),
        }
    }
}

fn read_pdf_catalog_flags(path: &Path) -> Result<(LoDocument, bool, bool, bool, bool, bool, usize), lopdf::Error> {
    let doc = LoDocument::load(path)?;
    let page_count = doc.get_pages().len();
    let mut struct_tree_root_present = false;
    let mut mark_info_present = false;
    let mut marked_true_present = false;
    let mut lang_token_present = false;
    let mut title_token_present = false;

    if let Ok(root_obj) = doc.trailer.get(b"Root") {
        let root_obj = resolve_lopdf_obj(&doc, root_obj)?;
        if let Ok(root_dict) = root_obj.as_dict() {
            if root_dict.get(b"StructTreeRoot").is_ok() {
                struct_tree_root_present = true;
            }
            if root_dict.get(b"Lang").is_ok() {
                lang_token_present = true;
            }
            if let Ok(mark_info_obj) = root_dict.get(b"MarkInfo") {
                mark_info_present = true;
                let mark_info_obj = resolve_lopdf_obj(&doc, mark_info_obj)?;
                if let Ok(mark_info_dict) = mark_info_obj.as_dict() {
                    if let Ok(LoObject::Boolean(v)) = mark_info_dict.get(b"Marked") {
                        marked_true_present = *v;
                    }
                }
            }
        }
    }
    if let Ok(info_obj) = doc.trailer.get(b"Info") {
        let info_obj = resolve_lopdf_obj(&doc, info_obj)?;
        if let Ok(info_dict) = info_obj.as_dict() {
            if info_dict.get(b"Title").is_ok() {
                title_token_present = true;
            }
        }
    }
    Ok((
        doc,
        struct_tree_root_present,
        mark_info_present,
        marked_true_present,
        lang_token_present,
        title_token_present,
        page_count,
    ))
}

#[pyfunction]
fn export_pdf_reading_order_trace(py: Python<'_>, pdf_path: &str) -> PyResult<PyObject> {
    let path = Path::new(pdf_path);
    let out = PyDict::new_bound(py);
    out.set_item("schema", "fullbleed.pdf.reading_order_trace.v1")?;
    out.set_item("schema_version", 1)?;
    out.set_item("seed_only", true)?;
    out.set_item("pdf_path", pdf_path)?;
    out.set_item("generated_at_unix_ms", (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()) as u64)?;

    let warnings = PyList::empty_bound(py);
    let pages_out = PyList::empty_bound(py);
    let mut total_blocks: usize = 0;
    let mut non_empty_pages: usize = 0;
    let mut page_count: usize = 0;
    let mut ok = false;

    match LoDocument::load(path) {
        Ok(doc) => {
            let pages = doc.get_pages();
            page_count = pages.len();
            for (page_num, _id) in &pages {
                let chunks = doc.extract_text_chunks(&[*page_num]);
                let blocks = PyList::empty_bound(py);
                let mut block_count = 0usize;
                for (idx, chunk) in chunks.into_iter().enumerate() {
                    match chunk {
                        Ok(text) => {
                            let text = text.trim();
                            if text.is_empty() {
                                continue;
                            }
                            let d = PyDict::new_bound(py);
                            d.set_item("index", idx)?;
                            d.set_item("text", text)?;
                            blocks.append(d)?;
                            block_count += 1;
                        }
                        Err(err) => {
                            let w = PyDict::new_bound(py);
                            w.set_item("code", "READING_TRACE_CHUNK_ERROR")?;
                            w.set_item("message", format!("page {}: {}", page_num, err))?;
                            warnings.append(w)?;
                        }
                    }
                }
                let page_row = PyDict::new_bound(py);
                page_row.set_item("page_index", (*page_num as usize).saturating_sub(1))?;
                page_row.set_item("page", *page_num)?;
                page_row.set_item("width", py.None())?;
                page_row.set_item("height", py.None())?;
                page_row.set_item("block_count", block_count)?;
                page_row.set_item("blocks", blocks)?;
                pages_out.append(page_row)?;
                total_blocks += block_count;
                if block_count > 0 {
                    non_empty_pages += 1;
                }
            }
            ok = true;
            out.set_item("extractor", "lopdf")?;
        }
        Err(err) => {
            let w = PyDict::new_bound(py);
            w.set_item("code", "PDF_PARSE_ERROR")?;
            w.set_item("message", err.to_string())?;
            warnings.append(w)?;
            out.set_item("extractor", "lopdf")?;
        }
    }

    let summary = PyDict::new_bound(py);
    summary.set_item("page_count", page_count)?;
    summary.set_item("total_blocks", total_blocks)?;
    summary.set_item("non_empty_pages", non_empty_pages)?;
    out.set_item("ok", ok)?;
    out.set_item("pages", pages_out)?;
    out.set_item("summary", summary)?;
    out.set_item("warnings", warnings)?;
    Ok(out.to_object(py))
}

#[pyfunction]
fn export_pdf_structure_trace(py: Python<'_>, pdf_path: &str) -> PyResult<PyObject> {
    let path = Path::new(pdf_path);
    let out = PyDict::new_bound(py);
    out.set_item("schema", "fullbleed.pdf.structure_trace.v1")?;
    out.set_item("schema_version", 1)?;
    out.set_item("seed_only", true)?;
    out.set_item("pdf_path", pdf_path)?;
    out.set_item("extractor", "lopdf")?;
    out.set_item("generated_at_unix_ms", (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()) as u64)?;
    let warnings = PyList::empty_bound(py);

    let bytes = std::fs::read(path).map_err(|e| PyValueError::new_err(format!("failed to read pdf: {e}")))?;
    let token_counts = PyDict::new_bound(py);
    for token in [
        "/StructTreeRoot",
        "/StructElem",
        "/MarkInfo",
        "/Marked",
        "/MCID",
        "/Alt",
        "/ActualText",
        "/Figure",
        "/Table",
        "/TH",
        "/TD",
        "/TR",
    ] {
        token_counts.set_item(token.trim_start_matches('/'), bytes.windows(token.len()).filter(|w| *w == token.as_bytes()).count())?;
    }
    match read_pdf_catalog_flags(path) {
        Ok((_doc, struct_tree_root_present, mark_info_present, marked_true_present, lang_token_present, title_token_present, _page_count)) => {
            let summary = PyDict::new_bound(py);
            summary.set_item("bytes_len", bytes.len())?;
            summary.set_item("struct_tree_root_present", struct_tree_root_present)?;
            summary.set_item("mark_info_present", mark_info_present)?;
            summary.set_item("marked_true_present", marked_true_present)?;
            summary.set_item("lang_token_present", lang_token_present)?;
            summary.set_item("title_token_present", title_token_present)?;
            out.set_item("ok", true)?;
            out.set_item("summary", summary)?;
        }
        Err(err) => {
            let w = PyDict::new_bound(py);
            w.set_item("code", "PDF_PARSE_ERROR")?;
            w.set_item("message", err.to_string())?;
            warnings.append(w)?;
            let summary = PyDict::new_bound(py);
            summary.set_item("bytes_len", bytes.len())?;
            summary.set_item("struct_tree_root_present", false)?;
            summary.set_item("mark_info_present", false)?;
            summary.set_item("marked_true_present", false)?;
            summary.set_item("lang_token_present", false)?;
            summary.set_item("title_token_present", false)?;
            out.set_item("ok", false)?;
            out.set_item("summary", summary)?;
        }
    }
    out.set_item("token_counts", token_counts)?;
    out.set_item("warnings", warnings)?;
    Ok(out.to_object(py))
}

#[pyfunction]
#[pyo3(signature = (pdf_path, mode="error"))]
fn verify_pdf_ua_seed(py: Python<'_>, pdf_path: &str, mode: &str) -> PyResult<PyObject> {
    let structure = export_pdf_structure_trace(py, pdf_path)?;
    let reading = export_pdf_reading_order_trace(py, pdf_path)?;
    let structure_dict = structure.bind(py).downcast::<PyDict>()?;
    let reading_dict = reading.bind(py).downcast::<PyDict>()?;
    let extract_summary_bool = |root: &Bound<'_, PyDict>, key: &str| -> bool {
        if let Ok(Some(summary_obj)) = root.get_item("summary") {
            if let Ok(summary_dict) = summary_obj.downcast::<PyDict>() {
                if let Ok(Some(v)) = summary_dict.get_item(key) {
                    if let Ok(b) = v.extract::<bool>() {
                        return b;
                    }
                }
            }
        }
        false
    };
    let extract_summary_usize = |root: &Bound<'_, PyDict>, key: &str| -> usize {
        if let Ok(Some(summary_obj)) = root.get_item("summary") {
            if let Ok(summary_dict) = summary_obj.downcast::<PyDict>() {
                if let Ok(Some(v)) = summary_dict.get_item(key) {
                    if let Ok(n) = v.extract::<usize>() {
                        return n;
                    }
                }
            }
        }
        0
    };

    let struct_tree_root_present = extract_summary_bool(structure_dict, "struct_tree_root_present");
    let mark_info_present = extract_summary_bool(structure_dict, "mark_info_present");
    let marked_true_present = extract_summary_bool(structure_dict, "marked_true_present");
    let lang_token_present = extract_summary_bool(structure_dict, "lang_token_present");
    let title_token_present = extract_summary_bool(structure_dict, "title_token_present");
    let total_blocks = extract_summary_usize(reading_dict, "total_blocks");

    let checks = PyList::empty_bound(py);
    let mut critical_fail_count = 0usize;
    let mut nonpass_count = 0usize;
    let mut push_check = |id: &str, verdict: &str, severity: &str, critical: bool, message: String, evidence: Option<Bound<'_, PyDict>>| -> PyResult<()> {
        let d = PyDict::new_bound(py);
        d.set_item("id", id)?;
        d.set_item("verdict", verdict)?;
        d.set_item("severity", severity)?;
        d.set_item("critical", critical)?;
        d.set_item("message", message)?;
        if let Some(ev) = evidence {
            d.set_item("evidence", ev)?;
        }
        if critical && verdict == "fail" {
            critical_fail_count += 1;
        }
        if matches!(verdict, "fail" | "warn" | "manual_needed") {
            nonpass_count += 1;
        }
        checks.append(d)?;
        Ok(())
    };
    push_check("pdf.mark_info.present", if mark_info_present { "pass" } else { "fail" }, "error", true, if mark_info_present { "PDF MarkInfo token present".into() } else { "PDF MarkInfo token not found".into() }, None)?;
    push_check("pdf.mark_info.marked_true", if marked_true_present { "pass" } else { "fail" }, "error", true, if marked_true_present { "PDF /Marked true present".into() } else { "PDF /Marked true not found".into() }, None)?;
    push_check("pdf.structure_root.present", if struct_tree_root_present { "pass" } else { "fail" }, "error", true, if struct_tree_root_present { "PDF StructTreeRoot present".into() } else { "PDF StructTreeRoot not found".into() }, None)?;
    push_check("pdf.catalog.lang.present_seed", if lang_token_present { "pass" } else { "warn" }, "warn", false, if lang_token_present { "PDF /Lang present".into() } else { "PDF /Lang not found".into() }, None)?;
    push_check("pdf.metadata.title.present_seed", if title_token_present { "pass" } else { "warn" }, "warn", false, if title_token_present { "PDF title metadata token present".into() } else { "PDF title metadata token not found".into() }, None)?;
    let ro_ev = PyDict::new_bound(py);
    ro_ev.set_item("extractor", reading_dict.get_item("extractor")?)?;
    ro_ev.set_item("total_blocks", total_blocks)?;
    push_check(
        "pdf.trace.reading_order.emitted",
        if total_blocks > 0 { "pass" } else { "manual_needed" },
        "warn",
        false,
        if total_blocks > 0 { "Reading-order trace contains extractable text chunks".into() } else { "Reading-order trace emitted but no text chunks extracted; manual verification required".into() },
        Some(ro_ev),
    )?;
    let st_ev = PyDict::new_bound(py);
    st_ev.set_item("extractor", "lopdf")?;
    st_ev.set_item("struct_tree_root_present", struct_tree_root_present)?;
    st_ev.set_item("marked_true_present", marked_true_present)?;
    push_check(
        "pdf.trace.structure.emitted",
        if struct_tree_root_present { "pass" } else { "manual_needed" },
        "warn",
        false,
        if struct_tree_root_present { "Structure trace indicates tagged structure".into() } else { "Structure trace emitted but tagged structure not detected; manual verification required".into() },
        Some(st_ev),
    )?;

    let gate_ok = critical_fail_count == 0 || !mode.eq_ignore_ascii_case("error");
    let out = PyDict::new_bound(py);
    out.set_item("schema", "fullbleed.pdf.ua_seed_verify.v1")?;
    out.set_item("schema_version", 1)?;
    out.set_item("seed_only", true)?;
    out.set_item("pdf_path", pdf_path)?;
    out.set_item("mode", mode)?;
    out.set_item("ok", gate_ok)?;
    let gate = PyDict::new_bound(py);
    gate.set_item("ok", gate_ok)?;
    gate.set_item("critical_fail_count", critical_fail_count)?;
    gate.set_item("nonpass_count", nonpass_count)?;
    gate.set_item("mode", mode)?;
    out.set_item("gate", gate)?;
    out.set_item("checks", checks)?;
    let warnings = PyList::empty_bound(py);
    for src in [structure_dict, reading_dict] {
        if let Ok(Some(src_warnings)) = src.get_item("warnings") {
            for item in src_warnings.downcast::<PyList>()?.iter() {
                warnings.append(item)?;
            }
        }
    }
    out.set_item("warnings", warnings)?;
    out.set_item("generated_at_unix_ms", (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()) as u64)?;
    let meta = audit_contract::metadata();
    let tooling = PyDict::new_bound(py);
    tooling.set_item("audit_contract_id", meta.contract_id)?;
    tooling.set_item("audit_contract_version", meta.contract_version)?;
    tooling.set_item("audit_contract_fingerprint", meta.contract_fingerprint_sha256)?;
    out.set_item("tooling", tooling)?;
    Ok(out.to_object(py))
}

#[pyfunction]
fn inspect_template_catalog(
    py: Python<'_>,
    templates: Vec<(String, String)>,
) -> PyResult<PyObject> {
    let entries = inspect_template_catalog_entries(&templates)?;
    template_catalog_entries_to_py(py, &entries)
}

#[pyfunction]
#[pyo3(signature = (source, kind, name=None, trusted=false, remote=false))]
fn vendored_asset(
    py: Python<'_>,
    source: &str,
    kind: &str,
    name: Option<String>,
    trusted: bool,
    remote: bool,
) -> PyResult<PyAsset> {
    let kind = parse_asset_kind(kind)?;
    let asset = build_asset(py, source, kind, name, trusted, remote)?;
    Ok(PyAsset { asset })
}

#[pyfunction]
fn fetch_asset(py: Python<'_>, url: &str) -> PyResult<Py<PyBytes>> {
    let urllib = py.import_bound("urllib.request")?;
    let response = urllib.call_method1("urlopen", (url,))?;
    let data = response.call_method0("read")?;
    let bytes = data.downcast::<PyBytes>()?;
    Ok(bytes.clone().unbind())
}

#[pyfunction]
fn concat_css(parts: Vec<String>) -> PyResult<String> {
    Ok(parts.join("\n"))
}

#[pyfunction]
#[pyo3(signature = (template, overlay, out, page_map=None, dx=0.0, dy=0.0))]
fn finalize_stamp_pdf(
    template: &str,
    overlay: &str,
    out: &str,
    page_map: Option<Vec<(usize, usize)>>,
    dx: f32,
    dy: f32,
) -> PyResult<PyObject> {
    let summary = crate::stamp_overlay_on_template_pdf(
        std::path::Path::new(template),
        std::path::Path::new(overlay),
        std::path::Path::new(out),
        page_map.as_deref(),
        dx,
        dy,
    )
    .map_err(to_py_err)?;
    Python::with_gil(|py| {
        let d = PyDict::new_bound(py);
        d.set_item("ok", true)?;
        d.set_item("pages_written", summary.pages_written)?;
        Ok(d.to_object(py))
    })
}

#[pyfunction]
#[pyo3(signature = (templates, plan, overlay, out, annotation_mode=None))]
fn finalize_compose_pdf(
    templates: Vec<(String, String)>,
    plan: Vec<(String, usize, usize, f32, f32)>,
    overlay: &str,
    out: &str,
    annotation_mode: Option<&str>,
) -> PyResult<PyObject> {
    let mode = parse_compose_annotation_mode(annotation_mode)?;
    let mut catalog = crate::TemplateCatalog::default();
    for (template_id, pdf_path) in templates {
        catalog
            .insert(crate::TemplateAsset {
                template_id,
                pdf_path: PathBuf::from(pdf_path),
                sha256: None,
                page_count: None,
            })
            .map_err(to_py_err)?;
    }
    let mut page_plan = Vec::with_capacity(plan.len());
    for (template_id, template_page_index, overlay_page_index, dx, dy) in plan {
        page_plan.push(crate::ComposePagePlan {
            template_id,
            template_page_index,
            overlay_page_index,
            dx,
            dy,
        });
    }
    let summary = crate::compose_overlay_with_template_catalog_with_annotation_mode(
        &catalog,
        std::path::Path::new(overlay),
        std::path::Path::new(out),
        &page_plan,
        mode,
    )
    .map_err(to_py_err)?;

    Python::with_gil(|py| {
        let d = PyDict::new_bound(py);
        d.set_item("ok", true)?;
        d.set_item("pages_written", summary.pages_written)?;
        d.set_item("annotation_mode", compose_annotation_mode_name(mode))?;
        Ok(d.to_object(py))
    })
}

fn parse_compose_annotation_mode(raw: Option<&str>) -> PyResult<crate::ComposeAnnotationMode> {
    let Some(raw) = raw else {
        return Ok(crate::ComposeAnnotationMode::default());
    };
    let mode = raw.trim().to_ascii_lowercase();
    if mode.is_empty() || mode == "default" || mode == "link_only" || mode == "link-only" {
        return Ok(crate::ComposeAnnotationMode::LinkOnly);
    }
    if mode == "none" || mode == "off" {
        return Ok(crate::ComposeAnnotationMode::None);
    }
    if mode == "carry_widgets"
        || mode == "carry-widgets"
        || mode == "widgets"
        || mode == "link_and_widgets"
    {
        return Ok(crate::ComposeAnnotationMode::CarryWidgets);
    }
    Err(PyValueError::new_err(
        "annotation_mode must be one of: link_only, none, carry_widgets",
    ))
}

fn compose_annotation_mode_name(mode: crate::ComposeAnnotationMode) -> &'static str {
    match mode {
        crate::ComposeAnnotationMode::None => "none",
        crate::ComposeAnnotationMode::LinkOnly => "link_only",
        crate::ComposeAnnotationMode::CarryWidgets => "carry_widgets",
    }
}

fn div_round_i128(num: i128, den: i128) -> Option<i64> {
    if den == 0 {
        return None;
    }
    let den = den.abs();
    let out = if num >= 0 {
        (num + (den / 2)) / den
    } else {
        -(((-num) + (den / 2)) / den)
    };
    i64::try_from(out).ok()
}

fn parse_decimal_to_rational(s: &str) -> Option<(i128, i128)> {
    // Parse a decimal string into an exact rational num/den.
    // Examples: "8.5" => (85,10), "-0.125" => (-125,1000)
    let s = s.trim();
    if s.is_empty() {
        return None;
    }

    let mut sign: i128 = 1;
    let mut rest = s;
    if let Some(stripped) = rest.strip_prefix('-') {
        sign = -1;
        rest = stripped;
    } else if let Some(stripped) = rest.strip_prefix('+') {
        rest = stripped;
    }

    let (whole, frac) = rest.split_once('.').unwrap_or((rest, ""));
    if whole.is_empty() && frac.is_empty() {
        return None;
    }
    if !whole.chars().all(|c| c.is_ascii_digit()) || !frac.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }

    let whole_val: i128 = if whole.is_empty() {
        0
    } else {
        whole.parse::<i128>().ok()?
    };

    if frac.is_empty() {
        return Some((sign * whole_val, 1));
    }

    // Limit fractional digits to avoid overflow; round the last digit if longer.
    let max_frac = 9usize;
    let mut frac_str = frac.to_string();
    let mut round_up = false;
    if frac_str.len() > max_frac {
        let (keep, drop) = frac_str.split_at(max_frac);
        let next_digit = drop.chars().next().unwrap_or('0');
        round_up = next_digit >= '5';
        frac_str = keep.to_string();
    }

    let frac_val: i128 = frac_str.parse::<i128>().ok()?;
    let mut den: i128 = 1;
    for _ in 0..frac_str.len() {
        den = den.saturating_mul(10);
    }
    let mut num = whole_val.saturating_mul(den).saturating_add(frac_val);
    if round_up {
        num = num.saturating_add(1);
    }

    Some((sign * num, den))
}

fn parse_length_to_points(s: &str) -> Option<f32> {
    // Deterministic parser: returns points rounded to 0.001pt (milli-point) via integer math.
    // Accepts plain numbers (points) or "<number><unit>" where unit is one of: pt, in, mm, cm, px.
    let raw = s.trim();
    if raw.is_empty() {
        return None;
    }

    // Split numeric prefix from unit suffix.
    let mut split = raw.len();
    for (i, ch) in raw.char_indices() {
        if !(ch.is_ascii_digit() || ch == '.' || ch == '-' || ch == '+') {
            split = i;
            break;
        }
    }
    let (num_str, unit_str) = raw.split_at(split);
    let (num, den) = parse_decimal_to_rational(num_str)?;
    let unit = unit_str.trim().to_ascii_lowercase();

    // Convert to milli-points (1/1000 pt), then back to f32.
    let mpt: i64 = match unit.as_str() {
        "" | "pt" => div_round_i128(num.saturating_mul(1000), den)?,
        "in" => div_round_i128(num.saturating_mul(72_000), den)?,
        "px" => div_round_i128(num.saturating_mul(750), den)?, // 1px = 0.75pt
        "mm" => div_round_i128(num.saturating_mul(720_000), den.saturating_mul(254))?,
        "cm" => div_round_i128(num.saturating_mul(7_200_000), den.saturating_mul(254))?,
        _ => return None,
    };

    Some((mpt as f32) / 1000.0)
}

fn parse_py_length(arg: Option<&Bound<'_, PyAny>>) -> PyResult<Option<f32>> {
    let Some(arg) = arg else {
        return Ok(None);
    };
    if let Ok(v) = arg.extract::<f32>() {
        return Ok(Some(v));
    }
    if let Ok(v) = arg.extract::<i64>() {
        return Ok(Some(v as f32));
    }
    if let Ok(s) = arg.extract::<String>() {
        return parse_length_to_points(&s)
            .ok_or_else(|| PyValueError::new_err(format!("Invalid length: {s:?}")))
            .map(Some);
    }
    Err(PyValueError::new_err(
        "page_width/page_height must be a number (points) or a string like '8.5in'/'210mm'",
    ))
}

fn parse_py_margins(arg: Option<&Bound<'_, PyAny>>) -> PyResult<Option<Margins>> {
    let Some(arg) = arg else { return Ok(None) };
    if let Ok(v) = arg.extract::<f32>() {
        return Ok(Some(Margins::all(v)));
    }
    if let Ok(v) = arg.extract::<i64>() {
        return Ok(Some(Margins::all(v as f32)));
    }
    if let Ok(s) = arg.extract::<String>() {
        return parse_length_to_points(&s)
            .ok_or_else(|| PyValueError::new_err(format!("Invalid length: {s:?}")))
            .map(|v| Some(Margins::all(v)));
    }
    if let Ok(d) = arg.downcast::<PyDict>() {
        // Accept {top/right/bottom/left: <length>} where <length> can be number or "12mm" etc.
        let get_len = |k: &str| -> PyResult<Option<f32>> {
            let v = d.get_item(k)?;
            match v {
                Some(v) => parse_py_length(Some(&v)),
                None => Ok(None),
            }
        };
        let top = Pt::from_f32(get_len("top")?.unwrap_or(0.0));
        let right = Pt::from_f32(get_len("right")?.unwrap_or(0.0));
        let bottom = Pt::from_f32(get_len("bottom")?.unwrap_or(0.0));
        let left = Pt::from_f32(get_len("left")?.unwrap_or(0.0));
        return Ok(Some(Margins {
            top,
            right,
            bottom,
            left,
        }));
    }
    Err(PyValueError::new_err(
        "page_margins values must be a number (points) or a dict like {'top':'20mm','right':'10mm','bottom':'10mm','left':'10mm'}",
    ))
}

fn parse_template_binding_spec(value: &Bound<'_, PyAny>) -> PyResult<crate::TemplateBindingSpec> {
    let dict = value.downcast::<PyDict>().map_err(|_| {
        PyValueError::new_err(
            "template_binding must be a dict like {'default_template_id':'tpl-default','by_page_template':{'Page1':'tpl-a'},'by_feature':{'i9':'tpl-b'},'feature_prefix':'fb.feature.'}",
        )
    })?;
    let mut spec = crate::TemplateBindingSpec::default();

    if let Some(v) = dict.get_item("default_template_id")? {
        let id = v.extract::<String>().map_err(|_| {
            PyValueError::new_err("template_binding.default_template_id must be a string")
        })?;
        if id.trim().is_empty() {
            return Err(PyValueError::new_err(
                "template_binding.default_template_id cannot be empty",
            ));
        }
        spec.default_template_id = Some(id);
    }

    if let Some(v) = dict.get_item("feature_prefix")? {
        let prefix = v.extract::<String>().map_err(|_| {
            PyValueError::new_err("template_binding.feature_prefix must be a string")
        })?;
        if prefix.trim().is_empty() {
            return Err(PyValueError::new_err(
                "template_binding.feature_prefix cannot be empty",
            ));
        }
        spec.feature_prefix = prefix;
    }

    if let Some(v) = dict.get_item("by_page_template")? {
        let by = v.downcast::<PyDict>().map_err(|_| {
            PyValueError::new_err("template_binding.by_page_template must be a dict[str, str]")
        })?;
        let mut out: BTreeMap<String, String> = BTreeMap::new();
        for (k, val) in by.iter() {
            let key = k.extract::<String>().map_err(|_| {
                PyValueError::new_err("template_binding.by_page_template keys must be strings")
            })?;
            let mapped = val.extract::<String>().map_err(|_| {
                PyValueError::new_err("template_binding.by_page_template values must be strings")
            })?;
            if key.trim().is_empty() || mapped.trim().is_empty() {
                return Err(PyValueError::new_err(
                    "template_binding.by_page_template entries cannot be empty",
                ));
            }
            out.insert(key, mapped);
        }
        spec.by_page_template = out;
    }

    if let Some(v) = dict.get_item("by_feature")? {
        let by = v.downcast::<PyDict>().map_err(|_| {
            PyValueError::new_err("template_binding.by_feature must be a dict[str, str]")
        })?;
        let mut out: BTreeMap<String, String> = BTreeMap::new();
        for (k, val) in by.iter() {
            let key = k.extract::<String>().map_err(|_| {
                PyValueError::new_err("template_binding.by_feature keys must be strings")
            })?;
            let mapped = val.extract::<String>().map_err(|_| {
                PyValueError::new_err("template_binding.by_feature values must be strings")
            })?;
            if key.trim().is_empty() || mapped.trim().is_empty() {
                return Err(PyValueError::new_err(
                    "template_binding.by_feature entries cannot be empty",
                ));
            }
            out.insert(key, mapped);
        }
        spec.by_feature = out;
    }

    Ok(spec)
}

fn parse_color_hex(s: &str) -> Option<Color> {
    let s = s.trim();
    let hex = s.strip_prefix('#')?;
    if hex.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()? as f32 / 255.0;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()? as f32 / 255.0;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()? as f32 / 255.0;
    Some(Color { r, g, b })
}

fn parse_color_space(raw: &str) -> PyResult<ColorSpace> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "rgb" | "srgb" => Ok(ColorSpace::Rgb),
        "cmyk" => Ok(ColorSpace::Cmyk),
        _ => Err(PyValueError::new_err(format!(
            "unsupported color_space: {} (expected 'rgb' or 'cmyk')",
            raw
        ))),
    }
}

fn parse_layout_strategy(raw: &str) -> PyResult<LayoutStrategy> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "eager" => Ok(LayoutStrategy::Eager),
        "lazy" => Ok(LayoutStrategy::Lazy),
        _ => Err(PyValueError::new_err(format!(
            "invalid layout_strategy '{raw}' (expected 'eager' or 'lazy')"
        ))),
    }
}

#[pyclass(name = "WatermarkSpec")]
#[derive(Clone)]
struct PyWatermarkSpec {
    kind: String,
    value: String,
    layer: String,
    semantics: Option<String>,
    opacity: f32,
    rotation_deg: f32,
    font_name: Option<String>,
    font_size: Option<f32>,
    color: Option<String>,
}

#[pymethods]
impl PyWatermarkSpec {
    #[new]
    #[pyo3(signature = (
        kind,
        value,
        layer="overlay",
        semantics=None,
        opacity=0.15,
        rotation_deg=0.0,
        font_name=None,
        font_size=None,
        color=None
    ))]
    fn new(
        kind: String,
        value: String,
        layer: &str,
        semantics: Option<String>,
        opacity: f32,
        rotation_deg: f32,
        font_name: Option<String>,
        font_size: Option<f32>,
        color: Option<String>,
    ) -> Self {
        Self {
            kind,
            value,
            layer: layer.to_string(),
            semantics,
            opacity,
            rotation_deg,
            font_name,
            font_size,
            color,
        }
    }
}

fn parse_watermark_layer(raw: &str) -> PyResult<WatermarkLayer> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "background" | "bg" | "back" => Ok(WatermarkLayer::Background),
        "overlay" | "fg" | "front" => Ok(WatermarkLayer::Overlay),
        _ => Err(PyValueError::new_err(format!(
            "Invalid watermark layer: {raw:?}. Expected 'background' or 'overlay'."
        ))),
    }
}

fn parse_watermark_semantics(raw: &str) -> PyResult<WatermarkSemantics> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "visual" => Ok(WatermarkSemantics::Visual),
        "artifact" => Ok(WatermarkSemantics::Artifact),
        "ocg" => Ok(WatermarkSemantics::Ocg),
        _ => Err(PyValueError::new_err(format!(
            "Invalid watermark semantics: {raw:?}. Expected 'visual', 'artifact', or 'ocg'."
        ))),
    }
}

fn load_output_intent_profile(source: &str) -> PyResult<Vec<u8>> {
    if source.starts_with("data:") {
        let mut parts = source.splitn(2, ',');
        let header = parts.next().unwrap_or_default();
        let payload = parts.next().ok_or_else(|| {
            PyValueError::new_err("output_intent_icc data URI is missing payload")
        })?;
        if header.contains("base64") {
            return base64::engine::general_purpose::STANDARD
                .decode(payload)
                .map_err(|err| {
                    PyValueError::new_err(format!(
                        "failed to decode base64 output_intent_icc data URI: {err}"
                    ))
                });
        }
        return Ok(payload.as_bytes().to_vec());
    }

    std::fs::read(source).map_err(|err| {
        PyValueError::new_err(format!(
            "failed to read output_intent_icc path {}: {}",
            source, err
        ))
    })
}

fn parse_output_intent(
    icc_source: Option<String>,
    identifier: Option<String>,
    info: Option<String>,
    n_components: Option<u8>,
) -> PyResult<Option<OutputIntent>> {
    if icc_source.is_none() && identifier.is_none() && info.is_none() && n_components.is_none() {
        return Ok(None);
    }

    let source = icc_source.ok_or_else(|| {
        PyValueError::new_err(
            "output intent parameters require output_intent_icc (path or data URI)",
        )
    })?;
    let icc_profile = load_output_intent_profile(&source)?;
    if icc_profile.is_empty() {
        return Err(PyValueError::new_err(
            "output_intent_icc resolved to empty bytes",
        ));
    }
    let n_components = n_components.unwrap_or(3);
    if !matches!(n_components, 1 | 3 | 4) {
        return Err(PyValueError::new_err(format!(
            "output_intent_components must be one of 1, 3, or 4 (got {n_components})"
        )));
    }
    let identifier = identifier
        .unwrap_or_else(|| "Custom".to_string())
        .trim()
        .to_string();
    if identifier.is_empty() {
        return Err(PyValueError::new_err(
            "output_intent_identifier cannot be empty",
        ));
    }
    Ok(Some(OutputIntent::new(
        icc_profile,
        n_components,
        identifier,
        info,
    )))
}

fn watermark_spec_from_py(spec: &PyWatermarkSpec) -> PyResult<WatermarkSpec> {
    let layer = parse_watermark_layer(&spec.layer)?;
    let opacity = spec.opacity.clamp(0.0, 1.0);
    let rotation_deg = spec.rotation_deg;

    let mut out = match spec.kind.trim().to_ascii_lowercase().as_str() {
        "text" => WatermarkSpec::text(spec.value.clone()),
        "html" => WatermarkSpec::html(spec.value.clone()),
        "image" | "img" => WatermarkSpec::image(spec.value.clone()),
        _ => {
            return Err(PyValueError::new_err(format!(
                "Invalid watermark kind: {:?}. Expected 'text', 'html', or 'image'.",
                spec.kind
            )));
        }
    };

    out.layer = layer;
    out.semantics = match spec.semantics.as_deref() {
        Some(raw) => parse_watermark_semantics(raw)?,
        None => WatermarkSemantics::Artifact,
    };
    out.opacity = opacity;
    out.rotation_deg = rotation_deg;

    if let Some(font_name) = &spec.font_name {
        out.font_name = font_name.clone();
    }
    if let Some(font_size) = spec.font_size {
        out.font_size = Pt::from_f32(font_size);
    }
    if let Some(color_str) = &spec.color {
        if let Some(color) = parse_color_hex(color_str) {
            out.color = color;
        } else {
            return Err(PyValueError::new_err(format!(
                "Invalid watermark color: {color_str:?}. Expected '#RRGGBB'."
            )));
        }
    }

    Ok(out)
}

fn parse_pdf_profile(arg: Option<&Bound<'_, PyAny>>) -> PyResult<Option<PdfProfile>> {
    let Some(arg) = arg else {
        return Ok(None);
    };
    if arg.is_none() {
        return Ok(None);
    }
    if let Ok(s) = arg.extract::<String>() {
        let raw = s.trim().to_ascii_lowercase();
        let profile = match raw.as_str() {
            "" | "none" => PdfProfile::None,
            "pdfa2b" | "pdfa-2b" | "pdfa_2b" => PdfProfile::PdfA2b,
            "pdfx4" | "pdfx-4" | "pdfx_4" => PdfProfile::PdfX4,
            "tagged" | "pdfua" | "pdf/ua" => PdfProfile::Tagged,
            _ => {
                return Err(PyValueError::new_err(format!(
                    "Invalid pdf_profile: {s:?}. Expected one of: none, pdfa2b, pdfx4, tagged"
                )));
            }
        };
        return Ok(Some(profile));
    }
    Err(PyValueError::new_err(
        "pdf_profile must be a string like 'tagged', 'pdfa2b', or 'pdfx4'",
    ))
}

fn parse_pdf_version(arg: Option<&Bound<'_, PyAny>>) -> PyResult<Option<PdfVersion>> {
    let Some(arg) = arg else {
        return Ok(None);
    };
    if arg.is_none() {
        return Ok(None);
    }
    if let Ok(s) = arg.extract::<String>() {
        let raw = s.trim().to_ascii_lowercase();
        let version = match raw.as_str() {
            "" => return Ok(None),
            "1.7" | "1" | "17" | "pdf1.7" | "pdf17" => PdfVersion::Pdf17,
            "2.0" | "2" | "20" | "pdf2.0" | "pdf20" => PdfVersion::Pdf20,
            _ => {
                return Err(PyValueError::new_err(format!(
                    "Invalid pdf_version: {s:?}. Expected one of: 1.7, 2.0"
                )));
            }
        };
        return Ok(Some(version));
    }
    Err(PyValueError::new_err(
        "pdf_version must be a string like '1.7' or '2.0'",
    ))
}

fn page_data_value_to_py(py: Python<'_>, v: &PageDataValue) -> PyResult<PyObject> {
    let d = PyDict::new_bound(py);
    match v {
        PageDataValue::Every(items) => {
            d.set_item("op", "every")?;
            d.set_item("value", PyList::new_bound(py, items))?;
        }
        PageDataValue::Count(n) => {
            d.set_item("op", "count")?;
            d.set_item("value", *n)?;
        }
        PageDataValue::Sum { scale, value } => {
            d.set_item("op", "sum")?;
            d.set_item("scale", *scale)?;
            d.set_item("value", *value)?;
            d.set_item(
                "formatted",
                crate::page_data::format_scaled_int(*value, *scale),
            )?;
        }
    }
    Ok(d.to_object(py))
}

fn page_data_context_to_py(py: Python<'_>, ctx: &PageDataContext) -> PyResult<PyObject> {
    let root = PyDict::new_bound(py);
    root.set_item("page_count", ctx.page_count)?;

    let pages = PyList::empty_bound(py);
    for (idx0, page) in ctx.pages.iter().enumerate() {
        let pd = PyDict::new_bound(py);
        pd.set_item("page", idx0 + 1)?;
        for (k, v) in page {
            pd.set_item(k, page_data_value_to_py(py, v)?)?;
        }
        pages.append(pd)?;
    }
    root.set_item("pages", pages)?;

    let totals = PyDict::new_bound(py);
    for (k, v) in &ctx.totals {
        totals.set_item(k, page_data_value_to_py(py, v)?)?;
    }
    root.set_item("totals", totals)?;

    Ok(root.to_object(py))
}

fn template_binding_decisions_to_py(
    py: Python<'_>,
    decisions: &[crate::PageBindingDecision],
) -> PyResult<PyObject> {
    let list = PyList::empty_bound(py);
    for decision in decisions {
        let d = PyDict::new_bound(py);
        d.set_item("page_index", decision.page_index)?;
        d.set_item("page", decision.page_index + 1)?;
        d.set_item("page_template_name", decision.page_template_name.clone())?;
        d.set_item(
            "feature_hits",
            PyList::new_bound(py, &decision.feature_hits),
        )?;
        d.set_item("template_id", decision.template_id.clone())?;
        let source = match decision.source {
            crate::BindingSource::Feature => "feature",
            crate::BindingSource::PageTemplate => "page_template",
            crate::BindingSource::Default => "default",
        };
        d.set_item("source", source)?;
        list.append(d)?;
    }
    Ok(list.to_object(py))
}

fn compose_plan_to_py(
    py: Python<'_>,
    decisions: &[crate::PageBindingDecision],
    template_page_counts: &BTreeMap<String, usize>,
    dx: f32,
    dy: f32,
) -> PyResult<PyObject> {
    if decisions.is_empty() {
        return Err(PyValueError::new_err(
            "compose planning requires non-empty template bindings",
        ));
    }

    let mut sorted = decisions.to_vec();
    sorted.sort_by_key(|d| d.page_index);

    let mut seen_pages: BTreeSet<usize> = BTreeSet::new();
    let plan = PyList::empty_bound(py);
    for decision in &sorted {
        if !seen_pages.insert(decision.page_index) {
            return Err(PyValueError::new_err(format!(
                "duplicate template binding for overlay page {}",
                decision.page_index
            )));
        }
        let template_page_count =
            *template_page_counts
                .get(&decision.template_id)
                .ok_or_else(|| {
                    PyValueError::new_err(format!(
                        "binding references unknown template_id at page {}: {}",
                        decision.page_index + 1,
                        decision.template_id
                    ))
                })?;
        if template_page_count == 0 {
            return Err(PyValueError::new_err(format!(
                "template has zero pages for template_id: {}",
                decision.template_id
            )));
        }
        let template_page = decision.page_index % template_page_count;
        let row = PyDict::new_bound(py);
        row.set_item("page_index", decision.page_index)?;
        row.set_item("page", decision.page_index + 1)?;
        row.set_item("template_id", decision.template_id.clone())?;
        row.set_item("template_page", template_page)?;
        row.set_item("overlay_page", decision.page_index)?;
        row.set_item("dx", dx)?;
        row.set_item("dy", dy)?;
        plan.append(row)?;
    }
    Ok(plan.to_object(py))
}

fn build_render_time_reading_order_trace_py(py: Python<'_>, doc: &Document) -> PyResult<PyObject> {
    let out = PyDict::new_bound(py);
    out.set_item("schema", "fullbleed.pdf.reading_order_trace.v1")?;
    out.set_item("schema_version", 1)?;
    out.set_item("seed_only", true)?;
    out.set_item("extractor", "render_time_commands")?;
    out.set_item("source", "engine_render_time")?;

    let pages_out = PyList::empty_bound(py);
    let warnings = PyList::empty_bound(py);
    let mut total_blocks = 0usize;
    let mut non_empty_pages = 0usize;
    let mut artifact_text_blocks_excluded = 0usize;
    let mut draw_form_count_total = 0usize;
    let mut define_form_count_total = 0usize;
    let mut untagged_text_blocks = 0usize;

    for (page_index, page) in doc.pages.iter().enumerate() {
        let blocks = PyList::empty_bound(py);
        let mut tag_stack: Vec<String> = Vec::new();
        let mut artifact_depth = 0usize;
        let mut block_count = 0usize;
        let mut page_draw_form_count = 0usize;
        let mut page_define_form_count = 0usize;
        let mut page_artifact_excluded = 0usize;

        for (cmd_index, cmd) in page.commands.iter().enumerate() {
            match cmd {
                Command::BeginTag { role, .. } => tag_stack.push(role.clone()),
                Command::EndTag => {
                    let _ = tag_stack.pop();
                }
                Command::BeginArtifact { .. } => artifact_depth = artifact_depth.saturating_add(1),
                Command::EndMarkedContent => {
                    if artifact_depth > 0 {
                        artifact_depth -= 1;
                    }
                }
                Command::DefineForm { .. } => {
                    page_define_form_count = page_define_form_count.saturating_add(1);
                }
                Command::DrawForm { .. } => {
                    page_draw_form_count = page_draw_form_count.saturating_add(1);
                }
                Command::DrawString { x, y, text } => {
                    if artifact_depth > 0 {
                        page_artifact_excluded = page_artifact_excluded.saturating_add(1);
                        continue;
                    }
                    let row = PyDict::new_bound(py);
                    row.set_item("index", block_count)?;
                    row.set_item("command_index", cmd_index)?;
                    row.set_item("kind", "draw_string")?;
                    row.set_item("text", text.clone())?;
                    row.set_item("x", x.to_f32())?;
                    row.set_item("y", y.to_f32())?;
                    if let Some(top_role) = tag_stack.last() {
                        row.set_item("top_role", top_role.clone())?;
                    } else {
                        row.set_item("top_role", py.None())?;
                        untagged_text_blocks = untagged_text_blocks.saturating_add(1);
                    }
                    row.set_item("tag_path", PyList::new_bound(py, &tag_stack))?;
                    blocks.append(row)?;
                    block_count = block_count.saturating_add(1);
                }
                Command::DrawStringTransformed { x, y, text, .. } => {
                    if artifact_depth > 0 {
                        page_artifact_excluded = page_artifact_excluded.saturating_add(1);
                        continue;
                    }
                    let row = PyDict::new_bound(py);
                    row.set_item("index", block_count)?;
                    row.set_item("command_index", cmd_index)?;
                    row.set_item("kind", "draw_string_transformed")?;
                    row.set_item("text", text.clone())?;
                    row.set_item("x", x.to_f32())?;
                    row.set_item("y", y.to_f32())?;
                    if let Some(top_role) = tag_stack.last() {
                        row.set_item("top_role", top_role.clone())?;
                    } else {
                        row.set_item("top_role", py.None())?;
                        untagged_text_blocks = untagged_text_blocks.saturating_add(1);
                    }
                    row.set_item("tag_path", PyList::new_bound(py, &tag_stack))?;
                    blocks.append(row)?;
                    block_count = block_count.saturating_add(1);
                }
                _ => {}
            }
        }

        total_blocks = total_blocks.saturating_add(block_count);
        if block_count > 0 {
            non_empty_pages = non_empty_pages.saturating_add(1);
        }
        artifact_text_blocks_excluded = artifact_text_blocks_excluded.saturating_add(page_artifact_excluded);
        draw_form_count_total = draw_form_count_total.saturating_add(page_draw_form_count);
        define_form_count_total = define_form_count_total.saturating_add(page_define_form_count);

        let page_row = PyDict::new_bound(py);
        page_row.set_item("page_index", page_index)?;
        page_row.set_item("page", page_index + 1)?;
        page_row.set_item("width", doc.page_size.width.to_f32())?;
        page_row.set_item("height", doc.page_size.height.to_f32())?;
        page_row.set_item("block_count", block_count)?;
        page_row.set_item("blocks", blocks)?;
        page_row.set_item("draw_form_count", page_draw_form_count)?;
        page_row.set_item("define_form_count", page_define_form_count)?;
        page_row.set_item("artifact_text_blocks_excluded", page_artifact_excluded)?;
        pages_out.append(page_row)?;
    }

    if draw_form_count_total > 0 {
        let w = PyDict::new_bound(py);
        w.set_item("code", "RENDER_TIME_TRACE_DRAW_FORM_PRESENT")?;
        w.set_item("message", "Render-time reading-order trace excludes text inside drawn form XObjects (seed limitation).")?;
        warnings.append(w)?;
    }

    let summary = PyDict::new_bound(py);
    summary.set_item("page_count", doc.pages.len())?;
    summary.set_item("total_blocks", total_blocks)?;
    summary.set_item("non_empty_pages", non_empty_pages)?;
    summary.set_item("artifact_text_blocks_excluded", artifact_text_blocks_excluded)?;
    summary.set_item("untagged_text_blocks", untagged_text_blocks)?;
    summary.set_item("draw_form_count_total", draw_form_count_total)?;
    summary.set_item("define_form_count_total", define_form_count_total)?;

    out.set_item("ok", true)?;
    out.set_item("pages", pages_out)?;
    out.set_item("summary", summary)?;
    out.set_item("warnings", warnings)?;
    out.set_item(
        "generated_at_unix_ms",
        (std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()) as u64,
    )?;
    Ok(out.to_object(py))
}

fn build_render_time_structure_trace_py(py: Python<'_>, doc: &Document) -> PyResult<PyObject> {
    let out = PyDict::new_bound(py);
    out.set_item("schema", "fullbleed.pdf.structure_trace.v1")?;
    out.set_item("schema_version", 1)?;
    out.set_item("seed_only", true)?;
    out.set_item("extractor", "render_time_commands")?;
    out.set_item("source", "engine_render_time")?;

    let warnings = PyList::empty_bound(py);
    let pages_out = PyList::empty_bound(py);
    let role_counts = PyDict::new_bound(py);

    let mut begin_tag_count = 0usize;
    let mut end_tag_count = 0usize;
    let mut begin_artifact_count = 0usize;
    let mut end_marked_content_count = 0usize;
    let mut tagged_text_draw_count = 0usize;
    let mut untagged_text_draw_count = 0usize;
    let mut artifact_text_draw_count = 0usize;
    let mut tagged_pages = 0usize;
    let mut tag_balance_underflow = 0usize;

    for (page_index, page) in doc.pages.iter().enumerate() {
        let events = PyList::empty_bound(py);
        let mut tag_stack: Vec<String> = Vec::new();
        let mut artifact_depth = 0usize;
        let mut page_begin_tags = 0usize;
        let mut page_text_draws = 0usize;
        let mut page_tagged_text_draws = 0usize;

        for (cmd_index, cmd) in page.commands.iter().enumerate() {
            match cmd {
                Command::BeginTag { role, mcid, alt, scope, .. } => {
                    begin_tag_count = begin_tag_count.saturating_add(1);
                    page_begin_tags = page_begin_tags.saturating_add(1);
                    let role_key = role.clone();
                    let cur = role_counts
                        .get_item(&role_key)?
                        .and_then(|v| v.extract::<usize>().ok())
                        .unwrap_or(0);
                    role_counts.set_item(&role_key, cur.saturating_add(1))?;
                    tag_stack.push(role.clone());
                    if events.len() < 32 {
                        let ev = PyDict::new_bound(py);
                        ev.set_item("command_index", cmd_index)?;
                        ev.set_item("kind", "begin_tag")?;
                        ev.set_item("role", role.clone())?;
                        ev.set_item("mcid", mcid.map(|v| v as u64))?;
                        ev.set_item("alt_present", alt.is_some())?;
                        ev.set_item("scope", scope.clone())?;
                        events.append(ev)?;
                    }
                }
                Command::EndTag => {
                    end_tag_count = end_tag_count.saturating_add(1);
                    if tag_stack.pop().is_none() {
                        tag_balance_underflow = tag_balance_underflow.saturating_add(1);
                    }
                }
                Command::BeginArtifact { subtype } => {
                    begin_artifact_count = begin_artifact_count.saturating_add(1);
                    artifact_depth = artifact_depth.saturating_add(1);
                    if events.len() < 32 {
                        let ev = PyDict::new_bound(py);
                        ev.set_item("command_index", cmd_index)?;
                        ev.set_item("kind", "begin_artifact")?;
                        ev.set_item("subtype", subtype.clone())?;
                        events.append(ev)?;
                    }
                }
                Command::EndMarkedContent => {
                    end_marked_content_count = end_marked_content_count.saturating_add(1);
                    if artifact_depth > 0 {
                        artifact_depth -= 1;
                    }
                }
                Command::DrawString { .. } | Command::DrawStringTransformed { .. } => {
                    page_text_draws = page_text_draws.saturating_add(1);
                    if artifact_depth > 0 {
                        artifact_text_draw_count = artifact_text_draw_count.saturating_add(1);
                    } else if tag_stack.is_empty() {
                        untagged_text_draw_count = untagged_text_draw_count.saturating_add(1);
                    } else {
                        tagged_text_draw_count = tagged_text_draw_count.saturating_add(1);
                        page_tagged_text_draws = page_tagged_text_draws.saturating_add(1);
                    }
                }
                _ => {}
            }
        }

        if page_begin_tags > 0 {
            tagged_pages = tagged_pages.saturating_add(1);
        }
        let page_row = PyDict::new_bound(py);
        page_row.set_item("page_index", page_index)?;
        page_row.set_item("page", page_index + 1)?;
        page_row.set_item("begin_tag_count", page_begin_tags)?;
        page_row.set_item("text_draw_count", page_text_draws)?;
        page_row.set_item("tagged_text_draw_count", page_tagged_text_draws)?;
        page_row.set_item("sample_events", events)?;
        pages_out.append(page_row)?;
    }

    let summary = PyDict::new_bound(py);
    summary.set_item("page_count", doc.pages.len())?;
    summary.set_item("struct_tree_root_present", begin_tag_count > 0)?;
    summary.set_item("mark_info_present", begin_tag_count > 0)?;
    summary.set_item("marked_true_present", begin_tag_count > 0)?;
    summary.set_item("lang_token_present", py.None())?;
    summary.set_item("title_token_present", py.None())?;
    summary.set_item("begin_tag_count", begin_tag_count)?;
    summary.set_item("end_tag_count", end_tag_count)?;
    summary.set_item("begin_artifact_count", begin_artifact_count)?;
    summary.set_item("end_marked_content_count", end_marked_content_count)?;
    summary.set_item("tagged_text_draw_count", tagged_text_draw_count)?;
    summary.set_item("untagged_text_draw_count", untagged_text_draw_count)?;
    summary.set_item("artifact_text_draw_count", artifact_text_draw_count)?;
    summary.set_item("tagged_pages", tagged_pages)?;
    summary.set_item("tag_balance_underflow_count", tag_balance_underflow)?;
    summary.set_item("tag_balance_ok", tag_balance_underflow == 0 && end_tag_count <= begin_tag_count)?;

    out.set_item("ok", true)?;
    out.set_item("summary", summary)?;
    out.set_item("token_counts", role_counts)?;
    out.set_item("pages", pages_out)?;
    out.set_item("warnings", warnings)?;
    out.set_item(
        "generated_at_unix_ms",
        (std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()) as u64,
    )?;
    Ok(out.to_object(py))
}

fn glyph_report_to_py(py: Python<'_>, report: &GlyphCoverageReport) -> PyResult<PyObject> {
    let list = PyList::empty_bound(py);
    for missing in report.missing() {
        let d = PyDict::new_bound(py);
        d.set_item("codepoint", missing.codepoint)?;
        d.set_item("char", missing.ch.to_string())?;
        d.set_item("fonts_tried", missing.fonts_tried)?;
        d.set_item("count", missing.count)?;
        list.append(d)?;
    }
    Ok(list.to_object(py))
}

#[pyclass]
struct PdfEngine {
    engine: FullBleed,
    builder: FullBleedBuilder,
}

impl PdfEngine {
    fn rebuild_from_builder(&mut self) -> PyResult<()> {
        self.engine = self.builder.clone().build().map_err(to_py_err)?;
        Ok(())
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut out = String::with_capacity(digest.len() * 2);
    for b in digest {
        use std::fmt::Write;
        let _ = write!(&mut out, "{:02x}", b);
    }
    out
}

fn sha256_file_hex(path: &str) -> PyResult<String> {
    let mut file = std::fs::File::open(path).map_err(|e| {
        PyValueError::new_err(format!("failed to open output file for hashing: {e}"))
    })?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 8192];
    loop {
        let read = file.read(&mut buf).map_err(|e| {
            PyValueError::new_err(format!("failed to read output file for hashing: {e}"))
        })?;
        if read == 0 {
            break;
        }
        hasher.update(&buf[..read]);
    }
    let digest = hasher.finalize();
    let mut out = String::with_capacity(digest.len() * 2);
    for b in digest {
        use std::fmt::Write;
        let _ = write!(&mut out, "{:02x}", b);
    }
    Ok(out)
}

fn write_hash_file(path: &str, hash: &str) -> PyResult<()> {
    std::fs::write(path, hash)
        .map_err(|e| PyValueError::new_err(format!("failed to write deterministic hash file: {e}")))
}

fn wcag20aa_coverage_summary_to_py(
    py: Python<'_>,
    summary: &audit_contract::Wcag20AaCoverageSummary,
) -> PyResult<PyObject> {
    let out = PyDict::new_bound(py);
    out.set_item("registry_id", summary.registry_id.clone())?;
    out.set_item("registry_version", summary.registry_version)?;
    out.set_item("wcag_version", summary.wcag_version.clone())?;
    out.set_item("target_level", summary.target_level.clone())?;
    out.set_item("total_entries", summary.total_entries)?;
    out.set_item("success_criteria_total", summary.success_criteria_total)?;
    out.set_item(
        "conformance_requirements_total",
        summary.conformance_requirements_total,
    )?;
    out.set_item("mapped_entry_count", summary.mapped_entry_count)?;
    out.set_item(
        "mapped_success_criteria_count",
        summary.mapped_success_criteria_count,
    )?;
    out.set_item(
        "mapped_conformance_requirement_count",
        summary.mapped_conformance_requirement_count,
    )?;
    out.set_item(
        "implemented_mapped_entry_count",
        summary.implemented_mapped_entry_count,
    )?;
    out.set_item(
        "implemented_mapped_entry_evaluated_count",
        summary.implemented_mapped_entry_evaluated_count,
    )?;
    out.set_item(
        "implemented_mapped_entry_pending_count",
        summary.implemented_mapped_entry_pending_count,
    )?;
    out.set_item(
        "supporting_only_mapped_entry_count",
        summary.supporting_only_mapped_entry_count,
    )?;
    out.set_item(
        "planned_only_mapped_entry_count",
        summary.planned_only_mapped_entry_count,
    )?;
    out.set_item("unmapped_entry_count", summary.unmapped_entry_count)?;
    let counts = PyDict::new_bound(py);
    counts.set_item("pass", summary.implemented_mapped_result_counts.pass)?;
    counts.set_item("fail", summary.implemented_mapped_result_counts.fail)?;
    counts.set_item("warn", summary.implemented_mapped_result_counts.warn)?;
    counts.set_item(
        "manual_needed",
        summary.implemented_mapped_result_counts.manual_needed,
    )?;
    counts.set_item(
        "not_applicable",
        summary.implemented_mapped_result_counts.not_applicable,
    )?;
    counts.set_item("unknown", summary.implemented_mapped_result_counts.unknown)?;
    out.set_item("implemented_mapped_result_counts", counts)?;
    Ok(out.to_object(py))
}

fn section508_html_coverage_summary_to_py(
    py: Python<'_>,
    summary: &audit_contract::Section508HtmlCoverageSummary,
) -> PyResult<PyObject> {
    let out = PyDict::new_bound(py);
    out.set_item("registry_id", summary.registry_id.clone())?;
    out.set_item("registry_version", summary.registry_version)?;
    out.set_item("profile_id", summary.profile_id.clone())?;
    out.set_item("total_entries", summary.total_entries)?;
    out.set_item("specific_entries_total", summary.specific_entries_total)?;
    out.set_item("inherited_wcag_entries_total", summary.inherited_wcag_entries_total)?;
    out.set_item("mapped_entry_count", summary.mapped_entry_count)?;
    out.set_item(
        "implemented_mapped_entry_count",
        summary.implemented_mapped_entry_count,
    )?;
    out.set_item(
        "implemented_mapped_entry_evaluated_count",
        summary.implemented_mapped_entry_evaluated_count,
    )?;
    out.set_item(
        "implemented_mapped_entry_pending_count",
        summary.implemented_mapped_entry_pending_count,
    )?;
    out.set_item(
        "supporting_only_mapped_entry_count",
        summary.supporting_only_mapped_entry_count,
    )?;
    out.set_item(
        "planned_only_mapped_entry_count",
        summary.planned_only_mapped_entry_count,
    )?;
    out.set_item("unmapped_entry_count", summary.unmapped_entry_count)?;
    out.set_item("specific_mapped_entry_count", summary.specific_mapped_entry_count)?;
    out.set_item(
        "specific_implemented_mapped_entry_count",
        summary.specific_implemented_mapped_entry_count,
    )?;
    out.set_item(
        "specific_implemented_mapped_entry_evaluated_count",
        summary.specific_implemented_mapped_entry_evaluated_count,
    )?;
    out.set_item(
        "specific_implemented_mapped_entry_pending_count",
        summary.specific_implemented_mapped_entry_pending_count,
    )?;
    out.set_item("specific_unmapped_entry_count", summary.specific_unmapped_entry_count)?;
    out.set_item(
        "inherited_wcag_registry_id",
        summary.inherited_wcag_registry_id.clone(),
    )?;
    out.set_item(
        "inherited_wcag_implemented_mapped_entry_count",
        summary.inherited_wcag_implemented_mapped_entry_count,
    )?;
    out.set_item(
        "inherited_wcag_implemented_mapped_entry_evaluated_count",
        summary.inherited_wcag_implemented_mapped_entry_evaluated_count,
    )?;
    out.set_item(
        "inherited_wcag_unmapped_entry_count",
        summary.inherited_wcag_unmapped_entry_count,
    )?;
    let counts = PyDict::new_bound(py);
    counts.set_item("pass", summary.implemented_mapped_result_counts.pass)?;
    counts.set_item("fail", summary.implemented_mapped_result_counts.fail)?;
    counts.set_item("warn", summary.implemented_mapped_result_counts.warn)?;
    counts.set_item(
        "manual_needed",
        summary.implemented_mapped_result_counts.manual_needed,
    )?;
    counts.set_item(
        "not_applicable",
        summary.implemented_mapped_result_counts.not_applicable,
    )?;
    counts.set_item("unknown", summary.implemented_mapped_result_counts.unknown)?;
    out.set_item("implemented_mapped_result_counts", counts)?;
    Ok(out.to_object(py))
}

#[pyfunction]
fn audit_contract_metadata(py: Python<'_>) -> PyResult<PyObject> {
    let meta = audit_contract::metadata();
    let out = PyDict::new_bound(py);
    out.set_item("contract_id", meta.contract_id)?;
    out.set_item("contract_version", meta.contract_version)?;
    out.set_item("contract_fingerprint", format!("sha256:{}", meta.contract_fingerprint_sha256))?;
    let registries = PyList::empty_bound(py);

    let reg_audit = PyDict::new_bound(py);
    reg_audit.set_item("id", meta.audit_registry_id)?;
    reg_audit.set_item("hash", format!("sha256:{}", meta.audit_registry_hash_sha256))?;
    registries.append(reg_audit)?;

    let reg_wcag = PyDict::new_bound(py);
    reg_wcag.set_item("id", meta.wcag20aa_registry_id)?;
    reg_wcag.set_item("hash", format!("sha256:{}", meta.wcag20aa_registry_hash_sha256))?;
    registries.append(reg_wcag)?;

    let reg_s508 = PyDict::new_bound(py);
    reg_s508.set_item("id", meta.section508_html_registry_id)?;
    reg_s508.set_item(
        "hash",
        format!("sha256:{}", meta.section508_html_registry_hash_sha256),
    )?;
    registries.append(reg_s508)?;

    out.set_item("registries", registries)?;
    Ok(out.to_object(py))
}

#[pyfunction]
fn audit_contract_registry(name: &str) -> PyResult<String> {
    audit_contract::registry_json(name)
        .map(|s| s.to_string())
        .ok_or_else(|| PyValueError::new_err(format!("unknown audit contract registry: {name}")))
}

#[pyfunction]
fn audit_contract_wcag20aa_coverage(py: Python<'_>, findings: &Bound<'_, PyAny>) -> PyResult<PyObject> {
    let mut owned_pairs: Vec<(String, String)> = Vec::new();
    let iter = findings.iter().map_err(|_| {
        PyValueError::new_err("findings must be an iterable of mappings with rule_id/verdict")
    })?;
    for item in iter {
        let item = item?;
        let dict = item.downcast::<PyDict>().map_err(|_| {
            PyValueError::new_err("findings entries must be dict-like mappings")
        })?;
        let rule_id = dict
            .get_item("rule_id")?
            .and_then(|v| v.extract::<String>().ok())
            .unwrap_or_default();
        if rule_id.trim().is_empty() {
            continue;
        }
        let verdict = dict
            .get_item("verdict")?
            .and_then(|v| v.extract::<String>().ok())
            .unwrap_or_default();
        owned_pairs.push((rule_id, verdict));
    }
    let pair_refs: Vec<(&str, &str)> = owned_pairs
        .iter()
        .map(|(rid, verdict)| (rid.as_str(), verdict.as_str()))
        .collect();
    let summary = audit_contract::wcag20aa_coverage_from_rule_verdicts(pair_refs);
    wcag20aa_coverage_summary_to_py(py, &summary)
}

#[pyfunction]
fn audit_contract_section508_html_coverage(
    py: Python<'_>,
    findings: &Bound<'_, PyAny>,
) -> PyResult<PyObject> {
    let mut owned_pairs: Vec<(String, String)> = Vec::new();
    let iter = findings.iter().map_err(|_| {
        PyValueError::new_err("findings must be an iterable of mappings with rule_id/verdict")
    })?;
    for item in iter {
        let item = item?;
        let dict = item.downcast::<PyDict>().map_err(|_| {
            PyValueError::new_err("findings entries must be dict-like mappings")
        })?;
        let rule_id = dict
            .get_item("rule_id")?
            .and_then(|v| v.extract::<String>().ok())
            .unwrap_or_default();
        if rule_id.trim().is_empty() {
            continue;
        }
        let verdict = dict
            .get_item("verdict")?
            .and_then(|v| v.extract::<String>().ok())
            .unwrap_or_default();
        owned_pairs.push((rule_id, verdict));
    }
    let pair_refs: Vec<(&str, &str)> = owned_pairs
        .iter()
        .map(|(rid, verdict)| (rid.as_str(), verdict.as_str()))
        .collect();
    let summary = audit_contract::section508_html_coverage_from_rule_verdicts(pair_refs);
    section508_html_coverage_summary_to_py(py, &summary)
}

#[derive(Debug, Clone)]
struct RenderContrastSeedAnalysis {
    width: u32,
    height: u32,
    opaque_pixel_count: u64,
    ink_pixel_count: u64,
    background_luminance: f64,
    foreground_luminance: Option<f64>,
    estimated_contrast_ratio: Option<f64>,
    verdict: &'static str,
    confidence: &'static str,
    message: String,
}

fn _srgb_u8_to_linear(v: u8) -> f64 {
    let x = (v as f64) / 255.0;
    if x <= 0.04045 {
        x / 12.92
    } else {
        ((x + 0.055) / 1.055).powf(2.4)
    }
}

fn _relative_luminance(r: u8, g: u8, b: u8) -> f64 {
    (0.2126 * _srgb_u8_to_linear(r)) + (0.7152 * _srgb_u8_to_linear(g)) + (0.0722 * _srgb_u8_to_linear(b))
}

fn _percentile(sorted: &[f64], q: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let q = q.clamp(0.0, 1.0);
    let idx = ((sorted.len() - 1) as f64 * q).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

fn analyze_render_contrast_seed_png(path: &str) -> Result<RenderContrastSeedAnalysis, String> {
    let img = image::ImageReader::open(path)
        .map_err(|e| format!("failed to open render preview PNG: {e}"))?
        .with_guessed_format()
        .map_err(|e| format!("failed to detect render preview image format: {e}"))?
        .decode()
        .map_err(|e| format!("failed to decode render preview image: {e}"))?;
    let rgba = img.to_rgba8();
    let (width, height) = rgba.dimensions();
    let mut lumas: Vec<f64> = Vec::with_capacity((width as usize) * (height as usize));
    for px in rgba.pixels() {
        if px[3] == 0 {
            continue;
        }
        lumas.push(_relative_luminance(px[0], px[1], px[2]));
    }
    if lumas.is_empty() {
        return Ok(RenderContrastSeedAnalysis {
            width,
            height,
            opaque_pixel_count: 0,
            ink_pixel_count: 0,
            background_luminance: 0.0,
            foreground_luminance: None,
            estimated_contrast_ratio: None,
            verdict: "manual_needed",
            confidence: "low",
            message: "Render preview PNG has no opaque pixels; contrast could not be estimated."
                .to_string(),
        });
    }
    lumas.sort_by(|a, b| a.total_cmp(b));
    let opaque_pixel_count = lumas.len() as u64;
    let background_luminance = _percentile(&lumas, 0.98);
    let ink_threshold = if background_luminance > 0.9 {
        background_luminance - 0.02
    } else {
        background_luminance - 0.05
    };
    let ink: Vec<f64> = lumas
        .iter()
        .copied()
        .filter(|v| *v < ink_threshold)
        .collect();
    let ink_pixel_count = ink.len() as u64;
    if ink_pixel_count < 32 || (ink_pixel_count as f64 / opaque_pixel_count as f64) < 0.0001 {
        return Ok(RenderContrastSeedAnalysis {
            width,
            height,
            opaque_pixel_count,
            ink_pixel_count,
            background_luminance,
            foreground_luminance: None,
            estimated_contrast_ratio: None,
            verdict: "manual_needed",
            confidence: "low",
            message:
                "Insufficient foreground signal in render preview PNG for contrast estimation."
                    .to_string(),
        });
    }
    let mut ink_sorted = ink;
    ink_sorted.sort_by(|a, b| a.total_cmp(b));
    let foreground_luminance = _percentile(&ink_sorted, 0.05);
    let contrast_ratio =
        ((background_luminance.max(foreground_luminance)) + 0.05)
            / ((background_luminance.min(foreground_luminance)) + 0.05);
    let (verdict, confidence, message) = if contrast_ratio >= 4.5 {
        (
            "pass",
            "medium",
            format!(
                "Render-based contrast seed estimate is {:.2}:1 (>= 4.5:1).",
                contrast_ratio
            ),
        )
    } else {
        (
            "warn",
            "medium",
            format!(
                "Render-based contrast seed estimate is {:.2}:1 (< 4.5:1); manual confirmation required.",
                contrast_ratio
            ),
        )
    };
    Ok(RenderContrastSeedAnalysis {
        width,
        height,
        opaque_pixel_count,
        ink_pixel_count,
        background_luminance,
        foreground_luminance: Some(foreground_luminance),
        estimated_contrast_ratio: Some(contrast_ratio),
        verdict,
        confidence,
        message,
    })
}

fn render_contrast_seed_analysis_to_py(py: Python<'_>, a: &RenderContrastSeedAnalysis) -> PyResult<PyObject> {
    let out = PyDict::new_bound(py);
    out.set_item("schema", "fullbleed.contrast.render_seed.v1")?;
    out.set_item("width", a.width)?;
    out.set_item("height", a.height)?;
    out.set_item("opaque_pixel_count", a.opaque_pixel_count)?;
    out.set_item("ink_pixel_count", a.ink_pixel_count)?;
    out.set_item("background_luminance", a.background_luminance)?;
    if let Some(v) = a.foreground_luminance {
        out.set_item("foreground_luminance", v)?;
    } else {
        out.set_item("foreground_luminance", py.None())?;
    }
    if let Some(v) = a.estimated_contrast_ratio {
        out.set_item("estimated_contrast_ratio", v)?;
    } else {
        out.set_item("estimated_contrast_ratio", py.None())?;
    }
    out.set_item("verdict", a.verdict)?;
    out.set_item("confidence", a.confidence)?;
    out.set_item("message", a.message.clone())?;
    Ok(out.to_object(py))
}

#[pyfunction]
fn audit_contrast_render_png(py: Python<'_>, png_path: &str) -> PyResult<PyObject> {
    let analysis = analyze_render_contrast_seed_png(png_path).map_err(PyValueError::new_err)?;
    render_contrast_seed_analysis_to_py(py, &analysis)
}

fn a11y_core_evidence_to_py(py: Python<'_>, evidence: &A11yVerifierEvidence) -> PyResult<PyObject> {
    let out = PyDict::new_bound(py);
    if let Some(selector) = &evidence.selector {
        out.set_item("selector", selector.clone())?;
    }
    if !evidence.values.is_empty() {
        let values = PyDict::new_bound(py);
        for (k, v) in &evidence.values {
            values.set_item(k.clone(), v.clone())?;
        }
        out.set_item("values", values)?;
    }
    Ok(out.to_object(py))
}

fn a11y_core_finding_to_py(py: Python<'_>, finding: &A11yVerifierFinding) -> PyResult<PyObject> {
    let out = PyDict::new_bound(py);
    out.set_item("rule_id", finding.rule_id.clone())?;
    out.set_item("applicability", finding.applicability.clone())?;
    out.set_item("verification_mode", finding.verification_mode.clone())?;
    out.set_item("verdict", finding.verdict.clone())?;
    out.set_item("severity", finding.severity.clone())?;
    out.set_item("confidence", finding.confidence.clone())?;
    out.set_item("stage", finding.stage.clone())?;
    out.set_item("source", finding.source.clone())?;
    out.set_item("message", finding.message.clone())?;
    if !finding.evidence.is_empty() {
        let evid = PyList::empty_bound(py);
        for e in &finding.evidence {
            evid.append(a11y_core_evidence_to_py(py, e)?)?;
        }
        out.set_item("evidence", evid)?;
    }
    Ok(out.to_object(py))
}

#[derive(Debug, Clone, Default)]
struct A11yObservabilitySummaryRs {
    original_finding_count: usize,
    reported_finding_count: usize,
    dedup_event_count: usize,
    dedup_merged_finding_count: usize,
    correlated_finding_count: usize,
    stage_counts: std::collections::BTreeMap<String, usize>,
    source_counts: std::collections::BTreeMap<String, usize>,
    original_stage_counts: std::collections::BTreeMap<String, usize>,
    original_source_counts: std::collections::BTreeMap<String, usize>,
    correlation_index: Vec<A11yCorrelationIndexEntryRs>,
}

#[derive(Debug, Clone, Default)]
struct A11yCorrelationIndexEntryRs {
    rule_id: String,
    canonical_stage: String,
    canonical_source: String,
    canonical_verdict: String,
    merged_finding_count: usize,
    merged_pre_render_count: usize,
    merged_stage_counts: std::collections::BTreeMap<String, usize>,
    merged_source_counts: std::collections::BTreeMap<String, usize>,
}

#[derive(Debug, Clone, Default)]
struct A11yClaimEvidenceFlagsRs {
    technology_support_assessed: bool,
    technology_support_basis_recorded: bool,
    wcag20_consistent_identification_assessed: bool,
    wcag20_consistent_identification_basis_recorded: bool,
    wcag20_multiple_ways_scope_declared: bool,
    wcag20_multiple_ways_assessed: bool,
    wcag20_multiple_ways_basis_recorded: bool,
    wcag20_keyboard_assessed: bool,
    wcag20_keyboard_basis_recorded: bool,
    wcag20_keyboard_trap_assessed: bool,
    wcag20_keyboard_trap_basis_recorded: bool,
    wcag20_error_suggestion_scope_declared: bool,
    wcag20_error_suggestion_assessed: bool,
    wcag20_error_suggestion_basis_recorded: bool,
    wcag20_error_prevention_scope_declared: bool,
    wcag20_error_prevention_assessed: bool,
    wcag20_error_prevention_basis_recorded: bool,
    wcag20_on_input_assessed: bool,
    wcag20_on_input_basis_recorded: bool,
    wcag20_on_focus_assessed: bool,
    wcag20_on_focus_basis_recorded: bool,
    wcag20_timing_adjustable_scope_declared: bool,
    wcag20_timing_adjustable_assessed: bool,
    wcag20_timing_adjustable_basis_recorded: bool,
    wcag20_pause_stop_hide_scope_declared: bool,
    wcag20_pause_stop_hide_assessed: bool,
    wcag20_pause_stop_hide_basis_recorded: bool,
    wcag20_three_flashes_scope_declared: bool,
    wcag20_three_flashes_assessed: bool,
    wcag20_three_flashes_basis_recorded: bool,
    wcag20_audio_control_scope_declared: bool,
    wcag20_audio_control_assessed: bool,
    wcag20_audio_control_basis_recorded: bool,
    wcag20_use_of_color_scope_declared: bool,
    wcag20_use_of_color_assessed: bool,
    wcag20_use_of_color_basis_recorded: bool,
    wcag20_resize_text_scope_declared: bool,
    wcag20_resize_text_assessed: bool,
    wcag20_resize_text_basis_recorded: bool,
    wcag20_images_of_text_scope_declared: bool,
    wcag20_images_of_text_assessed: bool,
    wcag20_images_of_text_basis_recorded: bool,
    wcag20_prerecorded_av_alternative_scope_declared: bool,
    wcag20_prerecorded_av_alternative_assessed: bool,
    wcag20_prerecorded_av_alternative_basis_recorded: bool,
    wcag20_prerecorded_captions_scope_declared: bool,
    wcag20_prerecorded_captions_assessed: bool,
    wcag20_prerecorded_captions_basis_recorded: bool,
    wcag20_prerecorded_audio_description_or_media_alternative_scope_declared: bool,
    wcag20_prerecorded_audio_description_or_media_alternative_assessed: bool,
    wcag20_prerecorded_audio_description_or_media_alternative_basis_recorded: bool,
    wcag20_live_captions_scope_declared: bool,
    wcag20_live_captions_assessed: bool,
    wcag20_live_captions_basis_recorded: bool,
    wcag20_prerecorded_audio_description_scope_declared: bool,
    wcag20_prerecorded_audio_description_assessed: bool,
    wcag20_prerecorded_audio_description_basis_recorded: bool,
    wcag20_meaningful_sequence_scope_declared: bool,
    wcag20_meaningful_sequence_assessed: bool,
    wcag20_meaningful_sequence_basis_recorded: bool,
    wcag20_consistent_navigation_scope_declared: bool,
    wcag20_consistent_navigation_assessed: bool,
    wcag20_consistent_navigation_basis_recorded: bool,
    section508_scope_declared: bool,
    section508_public_facing_determination_recorded: bool,
    section508_official_communications_determination_recorded: bool,
    section508_nara_exception_determination_recorded: bool,
}

#[derive(Debug, Clone, Copy, Default)]
struct A11yCssFocusSeedFactsRs {
    focus_selector_signal_count: usize,
    outline_suppression_signal_count: usize,
}

fn a11y_css_focus_seed_facts(css_text: &str) -> A11yCssFocusSeedFactsRs {
    let css_l = css_text.to_ascii_lowercase();
    let focus_selector_signal_count = css_l.matches(":focus").count();
    let outline_suppression_signal_count = css_l.matches("outline:none").count()
        + css_l.matches("outline: none").count()
        + css_l.matches("outline:0").count()
        + css_l.matches("outline: 0").count()
        + css_l.matches("outline-width:0").count()
        + css_l.matches("outline-width: 0").count();
    A11yCssFocusSeedFactsRs {
        focus_selector_signal_count,
        outline_suppression_signal_count,
    }
}

fn py_dict_bool_path(obj: Option<&Bound<'_, PyAny>>, path: &[&str]) -> bool {
    let Some(root) = obj else {
        return false;
    };
    let mut cur: Bound<'_, PyAny> = root.clone();
    for (idx, key) in path.iter().enumerate() {
        let dict = match cur.downcast::<PyDict>() {
            Ok(d) => d,
            Err(_) => return false,
        };
        let Some(next) = (match dict.get_item(*key) {
            Ok(v) => v,
            Err(_) => None,
        }) else {
            return false;
        };
        if idx == path.len() - 1 {
            return next.extract::<bool>().unwrap_or(false);
        }
        cur = next;
    }
    false
}

fn a11y_claim_evidence_flags_from_py(
    claim_evidence: Option<&Bound<'_, PyAny>>,
) -> A11yClaimEvidenceFlagsRs {
    A11yClaimEvidenceFlagsRs {
        technology_support_assessed: py_dict_bool_path(
            claim_evidence,
            &["technology_support", "assessed"],
        ),
        technology_support_basis_recorded: py_dict_bool_path(
            claim_evidence,
            &["technology_support", "basis_recorded"],
        ),
        wcag20_consistent_identification_assessed: py_dict_bool_path(
            claim_evidence,
            &["wcag20", "consistent_identification_assessed"],
        ),
        wcag20_consistent_identification_basis_recorded: py_dict_bool_path(
            claim_evidence,
            &["wcag20", "consistent_identification_basis_recorded"],
        ),
        wcag20_multiple_ways_scope_declared: py_dict_bool_path(
            claim_evidence,
            &["wcag20", "multiple_ways_scope_declared"],
        ),
        wcag20_multiple_ways_assessed: py_dict_bool_path(
            claim_evidence,
            &["wcag20", "multiple_ways_assessed"],
        ),
        wcag20_multiple_ways_basis_recorded: py_dict_bool_path(
            claim_evidence,
            &["wcag20", "multiple_ways_basis_recorded"],
        ),
        wcag20_keyboard_assessed: py_dict_bool_path(
            claim_evidence,
            &["wcag20", "keyboard_assessed"],
        ),
        wcag20_keyboard_basis_recorded: py_dict_bool_path(
            claim_evidence,
            &["wcag20", "keyboard_basis_recorded"],
        ),
        wcag20_keyboard_trap_assessed: py_dict_bool_path(
            claim_evidence,
            &["wcag20", "keyboard_trap_assessed"],
        ),
        wcag20_keyboard_trap_basis_recorded: py_dict_bool_path(
            claim_evidence,
            &["wcag20", "keyboard_trap_basis_recorded"],
        ),
        wcag20_error_suggestion_scope_declared: py_dict_bool_path(
            claim_evidence,
            &["wcag20", "error_suggestion_scope_declared"],
        ),
        wcag20_error_suggestion_assessed: py_dict_bool_path(
            claim_evidence,
            &["wcag20", "error_suggestion_assessed"],
        ),
        wcag20_error_suggestion_basis_recorded: py_dict_bool_path(
            claim_evidence,
            &["wcag20", "error_suggestion_basis_recorded"],
        ),
        wcag20_error_prevention_scope_declared: py_dict_bool_path(
            claim_evidence,
            &["wcag20", "error_prevention_scope_declared"],
        ),
        wcag20_error_prevention_assessed: py_dict_bool_path(
            claim_evidence,
            &["wcag20", "error_prevention_assessed"],
        ),
        wcag20_error_prevention_basis_recorded: py_dict_bool_path(
            claim_evidence,
            &["wcag20", "error_prevention_basis_recorded"],
        ),
        wcag20_on_input_assessed: py_dict_bool_path(
            claim_evidence,
            &["wcag20", "on_input_assessed"],
        ),
        wcag20_on_input_basis_recorded: py_dict_bool_path(
            claim_evidence,
            &["wcag20", "on_input_basis_recorded"],
        ),
        wcag20_on_focus_assessed: py_dict_bool_path(
            claim_evidence,
            &["wcag20", "on_focus_assessed"],
        ),
        wcag20_on_focus_basis_recorded: py_dict_bool_path(
            claim_evidence,
            &["wcag20", "on_focus_basis_recorded"],
        ),
        wcag20_timing_adjustable_scope_declared: py_dict_bool_path(
            claim_evidence,
            &["wcag20", "timing_adjustable_scope_declared"],
        ),
        wcag20_timing_adjustable_assessed: py_dict_bool_path(
            claim_evidence,
            &["wcag20", "timing_adjustable_assessed"],
        ),
        wcag20_timing_adjustable_basis_recorded: py_dict_bool_path(
            claim_evidence,
            &["wcag20", "timing_adjustable_basis_recorded"],
        ),
        wcag20_pause_stop_hide_scope_declared: py_dict_bool_path(
            claim_evidence,
            &["wcag20", "pause_stop_hide_scope_declared"],
        ),
        wcag20_pause_stop_hide_assessed: py_dict_bool_path(
            claim_evidence,
            &["wcag20", "pause_stop_hide_assessed"],
        ),
        wcag20_pause_stop_hide_basis_recorded: py_dict_bool_path(
            claim_evidence,
            &["wcag20", "pause_stop_hide_basis_recorded"],
        ),
        wcag20_three_flashes_scope_declared: py_dict_bool_path(
            claim_evidence,
            &["wcag20", "three_flashes_scope_declared"],
        ),
        wcag20_three_flashes_assessed: py_dict_bool_path(
            claim_evidence,
            &["wcag20", "three_flashes_assessed"],
        ),
        wcag20_three_flashes_basis_recorded: py_dict_bool_path(
            claim_evidence,
            &["wcag20", "three_flashes_basis_recorded"],
        ),
        wcag20_audio_control_scope_declared: py_dict_bool_path(
            claim_evidence,
            &["wcag20", "audio_control_scope_declared"],
        ),
        wcag20_audio_control_assessed: py_dict_bool_path(
            claim_evidence,
            &["wcag20", "audio_control_assessed"],
        ),
        wcag20_audio_control_basis_recorded: py_dict_bool_path(
            claim_evidence,
            &["wcag20", "audio_control_basis_recorded"],
        ),
        wcag20_use_of_color_scope_declared: py_dict_bool_path(
            claim_evidence,
            &["wcag20", "use_of_color_scope_declared"],
        ),
        wcag20_use_of_color_assessed: py_dict_bool_path(
            claim_evidence,
            &["wcag20", "use_of_color_assessed"],
        ),
        wcag20_use_of_color_basis_recorded: py_dict_bool_path(
            claim_evidence,
            &["wcag20", "use_of_color_basis_recorded"],
        ),
        wcag20_resize_text_scope_declared: py_dict_bool_path(
            claim_evidence,
            &["wcag20", "resize_text_scope_declared"],
        ),
        wcag20_resize_text_assessed: py_dict_bool_path(
            claim_evidence,
            &["wcag20", "resize_text_assessed"],
        ),
        wcag20_resize_text_basis_recorded: py_dict_bool_path(
            claim_evidence,
            &["wcag20", "resize_text_basis_recorded"],
        ),
        wcag20_images_of_text_scope_declared: py_dict_bool_path(
            claim_evidence,
            &["wcag20", "images_of_text_scope_declared"],
        ),
        wcag20_images_of_text_assessed: py_dict_bool_path(
            claim_evidence,
            &["wcag20", "images_of_text_assessed"],
        ),
        wcag20_images_of_text_basis_recorded: py_dict_bool_path(
            claim_evidence,
            &["wcag20", "images_of_text_basis_recorded"],
        ),
        wcag20_prerecorded_av_alternative_scope_declared: py_dict_bool_path(
            claim_evidence,
            &["wcag20", "prerecorded_av_alternative_scope_declared"],
        ),
        wcag20_prerecorded_av_alternative_assessed: py_dict_bool_path(
            claim_evidence,
            &["wcag20", "prerecorded_av_alternative_assessed"],
        ),
        wcag20_prerecorded_av_alternative_basis_recorded: py_dict_bool_path(
            claim_evidence,
            &["wcag20", "prerecorded_av_alternative_basis_recorded"],
        ),
        wcag20_prerecorded_captions_scope_declared: py_dict_bool_path(
            claim_evidence,
            &["wcag20", "prerecorded_captions_scope_declared"],
        ),
        wcag20_prerecorded_captions_assessed: py_dict_bool_path(
            claim_evidence,
            &["wcag20", "prerecorded_captions_assessed"],
        ),
        wcag20_prerecorded_captions_basis_recorded: py_dict_bool_path(
            claim_evidence,
            &["wcag20", "prerecorded_captions_basis_recorded"],
        ),
        wcag20_prerecorded_audio_description_or_media_alternative_scope_declared:
            py_dict_bool_path(
                claim_evidence,
                &[
                    "wcag20",
                    "prerecorded_audio_description_or_media_alternative_scope_declared",
                ],
            ),
        wcag20_prerecorded_audio_description_or_media_alternative_assessed:
            py_dict_bool_path(
                claim_evidence,
                &[
                    "wcag20",
                    "prerecorded_audio_description_or_media_alternative_assessed",
                ],
            ),
        wcag20_prerecorded_audio_description_or_media_alternative_basis_recorded:
            py_dict_bool_path(
                claim_evidence,
                &[
                    "wcag20",
                    "prerecorded_audio_description_or_media_alternative_basis_recorded",
                ],
            ),
        wcag20_live_captions_scope_declared: py_dict_bool_path(
            claim_evidence,
            &["wcag20", "live_captions_scope_declared"],
        ),
        wcag20_live_captions_assessed: py_dict_bool_path(
            claim_evidence,
            &["wcag20", "live_captions_assessed"],
        ),
        wcag20_live_captions_basis_recorded: py_dict_bool_path(
            claim_evidence,
            &["wcag20", "live_captions_basis_recorded"],
        ),
        wcag20_prerecorded_audio_description_scope_declared: py_dict_bool_path(
            claim_evidence,
            &["wcag20", "prerecorded_audio_description_scope_declared"],
        ),
        wcag20_prerecorded_audio_description_assessed: py_dict_bool_path(
            claim_evidence,
            &["wcag20", "prerecorded_audio_description_assessed"],
        ),
        wcag20_prerecorded_audio_description_basis_recorded: py_dict_bool_path(
            claim_evidence,
            &["wcag20", "prerecorded_audio_description_basis_recorded"],
        ),
        wcag20_meaningful_sequence_scope_declared: py_dict_bool_path(
            claim_evidence,
            &["wcag20", "meaningful_sequence_scope_declared"],
        ),
        wcag20_meaningful_sequence_assessed: py_dict_bool_path(
            claim_evidence,
            &["wcag20", "meaningful_sequence_assessed"],
        ),
        wcag20_meaningful_sequence_basis_recorded: py_dict_bool_path(
            claim_evidence,
            &["wcag20", "meaningful_sequence_basis_recorded"],
        ),
        wcag20_consistent_navigation_scope_declared: py_dict_bool_path(
            claim_evidence,
            &["wcag20", "consistent_navigation_scope_declared"],
        ),
        wcag20_consistent_navigation_assessed: py_dict_bool_path(
            claim_evidence,
            &["wcag20", "consistent_navigation_assessed"],
        ),
        wcag20_consistent_navigation_basis_recorded: py_dict_bool_path(
            claim_evidence,
            &["wcag20", "consistent_navigation_basis_recorded"],
        ),
        section508_scope_declared: py_dict_bool_path(claim_evidence, &["section508", "scope_declared"]),
        section508_public_facing_determination_recorded: py_dict_bool_path(
            claim_evidence,
            &["section508", "public_facing_determination_recorded"],
        ),
        section508_official_communications_determination_recorded: py_dict_bool_path(
            claim_evidence,
            &["section508", "official_communications_determination_recorded"],
        ),
        section508_nara_exception_determination_recorded: py_dict_bool_path(
            claim_evidence,
            &["section508", "nara_exception_determination_recorded"],
        ),
    }
}

fn a11y_diag_map(code: &str) -> Option<&'static str> {
    match code.trim() {
        "DOCUMENT_TITLE_MISSING" => Some("fb.a11y.html.title_present_nonempty"),
        "ID_DUPLICATE" => Some("fb.a11y.ids.duplicate_id"),
        "IDREF_MISSING" => Some("fb.a11y.aria.reference_target_exists"),
        "MAIN_MULTIPLE" => Some("fb.a11y.structure.single_main"),
        "IMAGE_ALT_MISSING" => Some("fb.a11y.images.alt_or_decorative"),
        "IMAGE_ALT_MISSING_TITLE_PRESENT" => Some("fb.a11y.images.alt_or_decorative"),
        "IMAGE_SEMANTIC_CONFLICT" => Some("fb.a11y.images.alt_or_decorative"),
        "SIGNATURE_STATUS_INVALID" => Some("fb.a11y.signatures.text_semantics_present"),
        "SIGNATURE_METHOD_INVALID" => Some("fb.a11y.signatures.text_semantics_present"),
        "HEADING_EMPTY" => Some("fb.a11y.headings_labels.present_nonempty"),
        "LABEL_EMPTY" => Some("fb.a11y.headings_labels.present_nonempty"),
        "ARIA_LABEL_EMPTY" => Some("fb.a11y.headings_labels.present_nonempty"),
        "REGION_UNLABELED" => Some("fb.a11y.headings_labels.present_nonempty"),
        _ => None,
    }
}

fn a11y_bridge_findings_from_contract_report(
    a11y_report: Option<&Bound<'_, PyAny>>,
) -> PyResult<Vec<A11yVerifierFinding>> {
    let Some(obj) = a11y_report else {
        return Ok(Vec::new());
    };
    let dict = match obj.downcast::<PyDict>() {
        Ok(d) => d,
        Err(_) => return Ok(Vec::new()),
    };
    let Some(diags_any) = dict.get_item("diagnostics")? else {
        return Ok(Vec::new());
    };
    let diags = match diags_any.downcast::<PyList>() {
        Ok(v) => v,
        Err(_) => return Ok(Vec::new()),
    };
    let mut out = Vec::new();
    for item in diags.iter() {
        let d = match item.downcast::<PyDict>() {
            Ok(v) => v,
            Err(_) => continue,
        };
        let code = d
            .get_item("code")?
            .and_then(|v| v.extract::<String>().ok())
            .unwrap_or_default();
        let Some(rule_id) = a11y_diag_map(&code) else {
            continue;
        };
        let severity_text = d
            .get_item("severity")?
            .and_then(|v| v.extract::<String>().ok())
            .unwrap_or_default();
        let is_error = severity_text.trim().eq_ignore_ascii_case("error");
        let severity = if is_error
            && matches!(
                rule_id,
                "fb.a11y.ids.duplicate_id" | "fb.a11y.aria.reference_target_exists"
            ) {
            "critical"
        } else if is_error {
            "high"
        } else {
            "medium"
        };
        let message = d
            .get_item("message")?
            .and_then(|v| v.extract::<String>().ok())
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| if code.is_empty() { "A11y diagnostic".to_string() } else { code.clone() });
        let path = d
            .get_item("path")?
            .and_then(|v| v.extract::<String>().ok())
            .unwrap_or_default();
        out.push(A11yVerifierFinding {
            rule_id: rule_id.to_string(),
            applicability: "applicable".to_string(),
            verification_mode: "machine".to_string(),
            verdict: if is_error { "fail" } else { "warn" }.to_string(),
            severity: severity.to_string(),
            confidence: "high".to_string(),
            stage: "pre-render".to_string(),
            source: "a11y_contract".to_string(),
            message,
            evidence: vec![A11yVerifierEvidence {
                selector: if path.trim().is_empty() {
                    None
                } else {
                    Some(path.clone())
                },
                values: vec![
                    ("code".to_string(), code),
                    ("dom_path".to_string(), path),
                ],
            }],
        });
    }
    Ok(out)
}

fn a11y_findings_count_by_stage_source(
    rows: &[A11yVerifierFinding],
) -> (
    std::collections::BTreeMap<String, usize>,
    std::collections::BTreeMap<String, usize>,
) {
    let mut stage_counts = std::collections::BTreeMap::new();
    let mut source_counts = std::collections::BTreeMap::new();
    for f in rows {
        *stage_counts.entry(f.stage.clone()).or_insert(0) += 1;
        *source_counts.entry(f.source.clone()).or_insert(0) += 1;
    }
    (stage_counts, source_counts)
}

fn a11y_verdict_rank(v: &str) -> i32 {
    match v {
        "fail" => 5,
        "warn" => 4,
        "manual_needed" => 3,
        "pass" => 2,
        "not_applicable" => 1,
        _ => 0,
    }
}

fn a11y_severity_rank(v: &str) -> i32 {
    match v {
        "critical" => 5,
        "high" => 4,
        "medium" => 3,
        "low" => 2,
        "info" => 1,
        _ => 0,
    }
}

fn a11y_confidence_rank(v: &str) -> i32 {
    match v {
        "certain" => 4,
        "high" => 3,
        "medium" => 2,
        "low" => 1,
        _ => 0,
    }
}

fn a11y_finding_ref(f: &A11yVerifierFinding, idx: usize) -> String {
    format!("{}:{}:{}:{idx}", f.rule_id, f.stage, f.source)
}

fn a11y_dedup_and_correlate_findings(
    rows: Vec<A11yVerifierFinding>,
) -> (Vec<A11yVerifierFinding>, A11yObservabilitySummaryRs) {
    let allowlist: std::collections::BTreeSet<&str> = [
        "fb.a11y.html.title_present_nonempty",
        "fb.a11y.structure.single_main",
        "fb.a11y.images.alt_or_decorative",
        "fb.a11y.headings_labels.present_nonempty",
    ]
    .into_iter()
    .collect();

    let original = rows;
    let (original_stage_counts, original_source_counts) =
        a11y_findings_count_by_stage_source(&original);
    let mut by_rule: std::collections::BTreeMap<String, Vec<usize>> =
        std::collections::BTreeMap::new();
    for (idx, f) in original.iter().enumerate() {
        by_rule.entry(f.rule_id.clone()).or_default().push(idx);
    }

    let mut merge_plan: std::collections::BTreeMap<usize, Vec<usize>> =
        std::collections::BTreeMap::new();
    let mut skip_indexes: std::collections::BTreeSet<usize> = std::collections::BTreeSet::new();
    for (rule_id, idxs) in by_rule {
        if !allowlist.contains(rule_id.as_str()) || idxs.len() < 2 {
            continue;
        }
        let pre: Vec<usize> = idxs
            .iter()
            .copied()
            .filter(|i| {
                let f = &original[*i];
                f.stage == "pre-render" && f.source == "a11y_contract"
            })
            .collect();
        if pre.is_empty() {
            continue;
        }
        let non_pre: Vec<usize> = idxs
            .iter()
            .copied()
            .filter(|i| !pre.contains(i))
            .collect();
        if non_pre.len() != 1 {
            continue;
        }
        merge_plan.insert(non_pre[0], pre.clone());
        for i in pre {
            skip_indexes.insert(i);
        }
    }

    let mut dedup_event_count = 0usize;
    let mut dedup_merged_finding_count = 0usize;
    let mut correlated_finding_count = 0usize;
    let mut reported: Vec<A11yVerifierFinding> = Vec::new();
    let mut correlation_index: Vec<A11yCorrelationIndexEntryRs> = Vec::new();

    for (idx, row) in original.iter().cloned().enumerate() {
        if skip_indexes.contains(&idx) {
            continue;
        }
        let Some(peer_idxs) = merge_plan.get(&idx).cloned() else {
            reported.push(row);
            continue;
        };
        dedup_event_count += 1;
        dedup_merged_finding_count += peer_idxs.len();
        correlated_finding_count += 1;

        let mut primary = row.clone();
        let mut group: Vec<(usize, &A11yVerifierFinding)> = Vec::new();
        group.push((idx, &original[idx]));
        for peer_idx in &peer_idxs {
            group.push((*peer_idx, &original[*peer_idx]));
        }

        if let Some(worst) = group
            .iter()
            .map(|(_, f)| f.verdict.as_str())
            .max_by_key(|v| a11y_verdict_rank(v))
        {
            primary.verdict = worst.to_string();
        }
        if let Some(worst) = group
            .iter()
            .map(|(_, f)| f.severity.as_str())
            .max_by_key(|v| a11y_severity_rank(v))
        {
            primary.severity = worst.to_string();
        }
        if let Some(lowest) = group
            .iter()
            .map(|(_, f)| f.confidence.as_str())
            .min_by_key(|v| a11y_confidence_rank(v))
        {
            primary.confidence = lowest.to_string();
        }
        if group.iter().any(|(_, f)| f.applicability == "applicable") {
            primary.applicability = "applicable".to_string();
        } else if group.iter().any(|(_, f)| f.applicability == "unknown") {
            primary.applicability = "unknown".to_string();
        } else {
            primary.applicability = "not_applicable".to_string();
        }

        let mut merged_evidence: Vec<A11yVerifierEvidence> = Vec::new();
        for (gidx, f) in &group {
            let mut evs = if f.evidence.is_empty() {
                vec![A11yVerifierEvidence {
                    selector: None,
                    values: Vec::new(),
                }]
            } else {
                f.evidence.clone()
            };
            for ev in evs.iter_mut() {
                ev.values.push((
                    "correlated_origin_stage".to_string(),
                    f.stage.clone(),
                ));
                ev.values.push((
                    "correlated_origin_source".to_string(),
                    f.source.clone(),
                ));
                ev.values.push((
                    "correlated_origin_verdict".to_string(),
                    f.verdict.clone(),
                ));
                ev.values.push((
                    "correlated_primary".to_string(),
                    if *gidx == idx { "true" } else { "false" }.to_string(),
                ));
                ev.values.push((
                    "correlated_origin_ref".to_string(),
                    a11y_finding_ref(f, *gidx),
                ));
            }
            merged_evidence.extend(evs);
        }
        let (group_stage_counts, group_source_counts) =
            a11y_findings_count_by_stage_source(&group.iter().map(|(_, f)| (*f).clone()).collect::<Vec<_>>());
        let mut summary_values = vec![
            ("correlation_role".to_string(), "summary".to_string()),
            (
                "merged_pre_render_count".to_string(),
                peer_idxs.len().to_string(),
            ),
        ];
        for (k, v) in &group_stage_counts {
            summary_values.push((format!("stage_count:{k}"), v.to_string()));
        }
        for (k, v) in &group_source_counts {
            summary_values.push((format!("source_count:{k}"), v.to_string()));
        }
        merged_evidence.push(A11yVerifierEvidence {
            selector: None,
            values: summary_values,
        });
        primary.evidence = merged_evidence;
        primary.message = format!(
            "{} (Correlated {} pre-render diagnostic(s) into canonical {} finding.)",
            primary.message.trim_end(),
            peer_idxs.len(),
            primary.stage
        );
        let merged_pre_render_count = group
            .iter()
            .filter(|(_, f)| f.stage == "pre-render" && f.source == "a11y_contract")
            .count();
        correlation_index.push(A11yCorrelationIndexEntryRs {
            rule_id: primary.rule_id.clone(),
            canonical_stage: primary.stage.clone(),
            canonical_source: primary.source.clone(),
            canonical_verdict: primary.verdict.clone(),
            merged_finding_count: peer_idxs.len(),
            merged_pre_render_count,
            merged_stage_counts: group_stage_counts.clone(),
            merged_source_counts: group_source_counts.clone(),
        });
        reported.push(primary);
    }

    let (stage_counts, source_counts) = a11y_findings_count_by_stage_source(&reported);
    let obs = A11yObservabilitySummaryRs {
        original_finding_count: original.len(),
        reported_finding_count: reported.len(),
        dedup_event_count,
        dedup_merged_finding_count,
        correlated_finding_count,
        stage_counts,
        source_counts,
        original_stage_counts,
        original_source_counts,
        correlation_index,
    };
    (reported, obs)
}

fn build_a11y_verify_report_py(
    py: Python<'_>,
    core: &A11yVerifierCoreReport,
    html_path: &str,
    css_path: &str,
    mode: &str,
    render_preview_png_path: Option<&str>,
    a11y_report: Option<&Bound<'_, PyAny>>,
    claim_evidence: Option<&Bound<'_, PyAny>>,
) -> PyResult<PyObject> {
    let mode_norm = {
        let m = mode.trim().to_ascii_lowercase();
        if m.is_empty() {
            "error".to_string()
        } else {
            m
        }
    };
    if !matches!(mode_norm.as_str(), "off" | "warn" | "error") {
        return Err(PyValueError::new_err(format!(
            "invalid mode {mode:?} (expected off|warn|error)"
        )));
    }

    let claim_flags = a11y_claim_evidence_flags_from_py(claim_evidence);
    let mut adapter_findings: Vec<A11yVerifierFinding> = Vec::new();
    let complete_processes_applicable = core.profile.eq_ignore_ascii_case("transactional");
    adapter_findings.push(A11yVerifierFinding {
        rule_id: "fb.a11y.claim.complete_processes_scope_seed".to_string(),
        applicability: if complete_processes_applicable {
            "applicable"
        } else {
            "not_applicable"
        }
        .to_string(),
        verification_mode: "hybrid".to_string(),
        verdict: if complete_processes_applicable {
            "manual_needed"
        } else {
            "not_applicable"
        }
        .to_string(),
        severity: "medium".to_string(),
        confidence: if complete_processes_applicable { "medium" } else { "high" }.to_string(),
        stage: "adapter".to_string(),
        source: "adapter".to_string(),
        message: if complete_processes_applicable {
            "Transactional/profile process conformance requires complete-process scope evidence; manual review required."
                .to_string()
        } else {
            "Complete-processes conformance requirement not applicable without a declared multi-step process scope."
                .to_string()
        },
        evidence: vec![A11yVerifierEvidence {
            selector: None,
            values: vec![
                ("profile".to_string(), core.profile.clone()),
                (
                    "process_scope_declared".to_string(),
                    "false".to_string(),
                ),
            ],
        }],
    });

    let keyboard_target_count = core.facts.link_count + core.facts.form_control_count;
    let keyboard_custom_click_target_count = core.facts.custom_click_handler_count;
    let keyboard_pointer_only_signal_count = core.facts.pointer_only_click_handler_count;
    let keyboard_applicable =
        keyboard_target_count > 0 || keyboard_custom_click_target_count > 0;
    let keyboard_claim_evidence_satisfied =
        claim_flags.wcag20_keyboard_assessed && claim_flags.wcag20_keyboard_basis_recorded;
    adapter_findings.push(A11yVerifierFinding {
        rule_id: "fb.a11y.keyboard.operable_seed".to_string(),
        applicability: if !keyboard_applicable {
            "not_applicable"
        } else {
            "applicable"
        }
        .to_string(),
        verification_mode: "hybrid".to_string(),
        verdict: if !keyboard_applicable {
            "not_applicable"
        } else if keyboard_pointer_only_signal_count > 0 {
            if keyboard_claim_evidence_satisfied {
                "warn"
            } else {
                "manual_needed"
            }
        } else if keyboard_claim_evidence_satisfied {
            "pass"
        } else {
            "manual_needed"
        }
        .to_string(),
        severity: "medium".to_string(),
        confidence: if !keyboard_applicable {
            "high"
        } else if keyboard_pointer_only_signal_count > 0 {
            if keyboard_claim_evidence_satisfied {
                "medium"
            } else {
                "low"
            }
        } else if keyboard_claim_evidence_satisfied {
            "medium"
        } else {
            "low"
        }
        .to_string(),
        stage: "adapter".to_string(),
        source: "adapter".to_string(),
        message: if !keyboard_applicable {
            "No interactive links, form controls, or custom click-handlers detected; keyboard-operable seed not applicable."
                .to_string()
        } else if keyboard_pointer_only_signal_count > 0 {
            if keyboard_claim_evidence_satisfied {
                "Custom click-handlers without keyboard handlers/tabindex were detected on non-native elements; keyboard review evidence is recorded but manual follow-up remains required."
                    .to_string()
            } else {
                "Custom click-handlers without keyboard handlers/tabindex were detected on non-native elements; keyboard-operability review requires manual evidence."
                    .to_string()
            }
        } else if keyboard_claim_evidence_satisfied {
            "Keyboard-operability review evidence is recorded for interactive components."
                .to_string()
        } else {
            "Interactive components detected; keyboard-operability review requires manual evidence."
                .to_string()
        },
        evidence: vec![A11yVerifierEvidence {
            selector: None,
            values: vec![
                (
                    "interactive_keyboard_target_count".to_string(),
                    keyboard_target_count.to_string(),
                ),
                (
                    "custom_click_handler_count".to_string(),
                    keyboard_custom_click_target_count.to_string(),
                ),
                (
                    "pointer_only_click_handler_count".to_string(),
                    keyboard_pointer_only_signal_count.to_string(),
                ),
                ("link_count".to_string(), core.facts.link_count.to_string()),
                (
                    "form_control_count".to_string(),
                    core.facts.form_control_count.to_string(),
                ),
                (
                    "keyboard_assessed".to_string(),
                    claim_flags.wcag20_keyboard_assessed.to_string(),
                ),
                (
                    "keyboard_basis_recorded".to_string(),
                    claim_flags.wcag20_keyboard_basis_recorded.to_string(),
                ),
            ],
        }],
    });

    let keyboard_trap_claim_evidence_satisfied =
        claim_flags.wcag20_keyboard_trap_assessed && claim_flags.wcag20_keyboard_trap_basis_recorded;
    adapter_findings.push(A11yVerifierFinding {
        rule_id: "fb.a11y.keyboard.no_trap_seed".to_string(),
        applicability: if keyboard_target_count == 0 {
            "not_applicable"
        } else {
            "applicable"
        }
        .to_string(),
        verification_mode: "hybrid".to_string(),
        verdict: if keyboard_target_count == 0 {
            "not_applicable"
        } else if keyboard_trap_claim_evidence_satisfied {
            "pass"
        } else {
            "manual_needed"
        }
        .to_string(),
        severity: "medium".to_string(),
        confidence: if keyboard_target_count == 0 {
            "high"
        } else if keyboard_trap_claim_evidence_satisfied {
            "medium"
        } else {
            "low"
        }
        .to_string(),
        stage: "adapter".to_string(),
        source: "adapter".to_string(),
        message: if keyboard_target_count == 0 {
            "No interactive links or form controls detected; no-keyboard-trap seed not applicable."
                .to_string()
        } else if keyboard_trap_claim_evidence_satisfied {
            "No-keyboard-trap review evidence is recorded for interactive components."
                .to_string()
        } else {
            "Interactive components detected; no-keyboard-trap review requires manual evidence."
                .to_string()
        },
        evidence: vec![A11yVerifierEvidence {
            selector: None,
            values: vec![
                (
                    "interactive_keyboard_target_count".to_string(),
                    keyboard_target_count.to_string(),
                ),
                ("link_count".to_string(), core.facts.link_count.to_string()),
                (
                    "form_control_count".to_string(),
                    core.facts.form_control_count.to_string(),
                ),
                (
                    "keyboard_trap_assessed".to_string(),
                    claim_flags.wcag20_keyboard_trap_assessed.to_string(),
                ),
                (
                    "keyboard_trap_basis_recorded".to_string(),
                    claim_flags.wcag20_keyboard_trap_basis_recorded.to_string(),
                ),
            ],
        }],
    });

    let error_suggestion_scope_declared = claim_flags.wcag20_error_suggestion_scope_declared;
    let error_suggestion_claim_evidence_satisfied = claim_flags.wcag20_error_suggestion_assessed
        && claim_flags.wcag20_error_suggestion_basis_recorded;
    adapter_findings.push(A11yVerifierFinding {
        rule_id: "fb.a11y.forms.error_suggestion_seed".to_string(),
        applicability: if error_suggestion_scope_declared {
            "applicable"
        } else {
            "not_applicable"
        }
        .to_string(),
        verification_mode: "hybrid".to_string(),
        verdict: if !error_suggestion_scope_declared {
            "not_applicable"
        } else if error_suggestion_claim_evidence_satisfied {
            "pass"
        } else {
            "manual_needed"
        }
        .to_string(),
        severity: "medium".to_string(),
        confidence: if !error_suggestion_scope_declared {
            "high"
        } else if error_suggestion_claim_evidence_satisfied {
            "medium"
        } else {
            "low"
        }
        .to_string(),
        stage: "adapter".to_string(),
        source: "adapter".to_string(),
        message: if !error_suggestion_scope_declared {
            "Error-suggestion criterion not applicable without a declared form-flow error-handling scope."
                .to_string()
        } else if error_suggestion_claim_evidence_satisfied {
            "Error-suggestion review evidence is recorded for the declared form-flow scope."
                .to_string()
        } else {
            "Error-suggestion criterion is in scope for the declared form-flow; manual evidence is required."
                .to_string()
        },
        evidence: vec![A11yVerifierEvidence {
            selector: None,
            values: vec![
                ("profile".to_string(), core.profile.clone()),
                (
                    "error_suggestion_scope_declared".to_string(),
                    error_suggestion_scope_declared.to_string(),
                ),
                (
                    "error_suggestion_assessed".to_string(),
                    claim_flags.wcag20_error_suggestion_assessed.to_string(),
                ),
                (
                    "error_suggestion_basis_recorded".to_string(),
                    claim_flags
                        .wcag20_error_suggestion_basis_recorded
                        .to_string(),
                ),
            ],
        }],
    });

    let error_prevention_scope_declared = claim_flags.wcag20_error_prevention_scope_declared;
    let error_prevention_claim_evidence_satisfied = claim_flags.wcag20_error_prevention_assessed
        && claim_flags.wcag20_error_prevention_basis_recorded;
    adapter_findings.push(A11yVerifierFinding {
        rule_id: "fb.a11y.forms.error_prevention_legal_financial_data_seed".to_string(),
        applicability: if error_prevention_scope_declared {
            "applicable"
        } else {
            "not_applicable"
        }
        .to_string(),
        verification_mode: "hybrid".to_string(),
        verdict: if !error_prevention_scope_declared {
            "not_applicable"
        } else if error_prevention_claim_evidence_satisfied {
            "pass"
        } else {
            "manual_needed"
        }
        .to_string(),
        severity: "medium".to_string(),
        confidence: if !error_prevention_scope_declared {
            "high"
        } else if error_prevention_claim_evidence_satisfied {
            "medium"
        } else {
            "low"
        }
        .to_string(),
        stage: "adapter".to_string(),
        source: "adapter".to_string(),
        message: if !error_prevention_scope_declared {
            "Error-prevention (legal/financial/data) criterion not applicable without a declared transactional/legal data form-flow scope."
                .to_string()
        } else if error_prevention_claim_evidence_satisfied {
            "Error-prevention (legal/financial/data) review evidence is recorded for the declared transactional/legal data form-flow scope."
                .to_string()
        } else {
            "Error-prevention (legal/financial/data) criterion is in scope for the declared form-flow; manual evidence is required."
                .to_string()
        },
        evidence: vec![A11yVerifierEvidence {
            selector: None,
            values: vec![
                ("profile".to_string(), core.profile.clone()),
                (
                    "error_prevention_scope_declared".to_string(),
                    error_prevention_scope_declared.to_string(),
                ),
                (
                    "error_prevention_assessed".to_string(),
                    claim_flags.wcag20_error_prevention_assessed.to_string(),
                ),
                (
                    "error_prevention_basis_recorded".to_string(),
                    claim_flags
                        .wcag20_error_prevention_basis_recorded
                        .to_string(),
                ),
            ],
        }],
    });

    let on_input_target_count = core.facts.form_control_count;
    let on_input_claim_evidence_satisfied =
        claim_flags.wcag20_on_input_assessed && claim_flags.wcag20_on_input_basis_recorded;
    adapter_findings.push(A11yVerifierFinding {
        rule_id: "fb.a11y.forms.on_input_behavior_seed".to_string(),
        applicability: if on_input_target_count == 0 {
            "not_applicable"
        } else {
            "applicable"
        }
        .to_string(),
        verification_mode: "hybrid".to_string(),
        verdict: if on_input_target_count == 0 {
            "not_applicable"
        } else if on_input_claim_evidence_satisfied {
            "pass"
        } else {
            "manual_needed"
        }
        .to_string(),
        severity: "medium".to_string(),
        confidence: if on_input_target_count == 0 {
            "high"
        } else if on_input_claim_evidence_satisfied {
            "medium"
        } else {
            "low"
        }
        .to_string(),
        stage: "adapter".to_string(),
        source: "adapter".to_string(),
        message: if on_input_target_count == 0 {
            "No form controls detected; on-input behavior seed not applicable.".to_string()
        } else if on_input_claim_evidence_satisfied {
            "On-input behavior review evidence is recorded for detected form controls."
                .to_string()
        } else {
            "Form controls detected; on-input behavior review requires manual evidence."
                .to_string()
        },
        evidence: vec![A11yVerifierEvidence {
            selector: None,
            values: vec![
                ("profile".to_string(), core.profile.clone()),
                (
                    "form_control_count".to_string(),
                    core.facts.form_control_count.to_string(),
                ),
                (
                    "on_input_assessed".to_string(),
                    claim_flags.wcag20_on_input_assessed.to_string(),
                ),
                (
                    "on_input_basis_recorded".to_string(),
                    claim_flags.wcag20_on_input_basis_recorded.to_string(),
                ),
            ],
        }],
    });

    let on_focus_target_count = keyboard_target_count;
    let on_focus_claim_evidence_satisfied =
        claim_flags.wcag20_on_focus_assessed && claim_flags.wcag20_on_focus_basis_recorded;
    adapter_findings.push(A11yVerifierFinding {
        rule_id: "fb.a11y.focus.on_focus_behavior_seed".to_string(),
        applicability: if on_focus_target_count == 0 {
            "not_applicable"
        } else {
            "applicable"
        }
        .to_string(),
        verification_mode: "hybrid".to_string(),
        verdict: if on_focus_target_count == 0 {
            "not_applicable"
        } else if on_focus_claim_evidence_satisfied {
            "pass"
        } else {
            "manual_needed"
        }
        .to_string(),
        severity: "medium".to_string(),
        confidence: if on_focus_target_count == 0 {
            "high"
        } else if on_focus_claim_evidence_satisfied {
            "medium"
        } else {
            "low"
        }
        .to_string(),
        stage: "adapter".to_string(),
        source: "adapter".to_string(),
        message: if on_focus_target_count == 0 {
            "No interactive links or form controls detected; on-focus behavior seed not applicable."
                .to_string()
        } else if on_focus_claim_evidence_satisfied {
            "On-focus behavior review evidence is recorded for interactive components."
                .to_string()
        } else {
            "Interactive components detected; on-focus behavior review requires manual evidence."
                .to_string()
        },
        evidence: vec![A11yVerifierEvidence {
            selector: None,
            values: vec![
                ("profile".to_string(), core.profile.clone()),
                (
                    "interactive_focus_target_count".to_string(),
                    on_focus_target_count.to_string(),
                ),
                ("link_count".to_string(), core.facts.link_count.to_string()),
                (
                    "form_control_count".to_string(),
                    core.facts.form_control_count.to_string(),
                ),
                (
                    "on_focus_assessed".to_string(),
                    claim_flags.wcag20_on_focus_assessed.to_string(),
                ),
                (
                    "on_focus_basis_recorded".to_string(),
                    claim_flags.wcag20_on_focus_basis_recorded.to_string(),
                ),
            ],
        }],
    });

    let timing_adjustable_scope_declared = claim_flags.wcag20_timing_adjustable_scope_declared;
    let timing_adjustable_claim_evidence_satisfied =
        claim_flags.wcag20_timing_adjustable_assessed
            && claim_flags.wcag20_timing_adjustable_basis_recorded;
    adapter_findings.push(A11yVerifierFinding {
        rule_id: "fb.a11y.timing.adjustable_seed".to_string(),
        applicability: if timing_adjustable_scope_declared {
            "applicable"
        } else {
            "not_applicable"
        }
        .to_string(),
        verification_mode: "hybrid".to_string(),
        verdict: if !timing_adjustable_scope_declared {
            "not_applicable"
        } else if timing_adjustable_claim_evidence_satisfied {
            "pass"
        } else {
            "manual_needed"
        }
        .to_string(),
        severity: "medium".to_string(),
        confidence: if !timing_adjustable_scope_declared {
            "high"
        } else if timing_adjustable_claim_evidence_satisfied {
            "medium"
        } else {
            "low"
        }
        .to_string(),
        stage: "adapter".to_string(),
        source: "adapter".to_string(),
        message: if !timing_adjustable_scope_declared {
            "Timing-adjustable criterion not applicable without a declared timed-interaction scope."
                .to_string()
        } else if timing_adjustable_claim_evidence_satisfied {
            "Timing-adjustable review evidence is recorded for the declared timed-interaction scope."
                .to_string()
        } else {
            "Timing-adjustable criterion is in scope for the declared timed-interaction flow; manual evidence is required."
                .to_string()
        },
        evidence: vec![A11yVerifierEvidence {
            selector: None,
            values: vec![
                ("profile".to_string(), core.profile.clone()),
                (
                    "timing_adjustable_scope_declared".to_string(),
                    timing_adjustable_scope_declared.to_string(),
                ),
                (
                    "timing_adjustable_assessed".to_string(),
                    claim_flags.wcag20_timing_adjustable_assessed.to_string(),
                ),
                (
                    "timing_adjustable_basis_recorded".to_string(),
                    claim_flags
                        .wcag20_timing_adjustable_basis_recorded
                        .to_string(),
                ),
                (
                    "meta_refresh_count".to_string(),
                    core.facts.meta_refresh_count.to_string(),
                ),
            ],
        }],
    });

    let pause_stop_hide_scope_declared = claim_flags.wcag20_pause_stop_hide_scope_declared;
    let pause_stop_hide_claim_evidence_satisfied = claim_flags.wcag20_pause_stop_hide_assessed
        && claim_flags.wcag20_pause_stop_hide_basis_recorded;
    let pause_stop_hide_signal_count = core.facts.autoplay_media_count + core.facts.blink_marquee_count;
    adapter_findings.push(A11yVerifierFinding {
        rule_id: "fb.a11y.timing.pause_stop_hide_seed".to_string(),
        applicability: if pause_stop_hide_scope_declared {
            "applicable"
        } else {
            "not_applicable"
        }
        .to_string(),
        verification_mode: "hybrid".to_string(),
        verdict: if !pause_stop_hide_scope_declared {
            "not_applicable"
        } else if pause_stop_hide_claim_evidence_satisfied {
            "pass"
        } else {
            "manual_needed"
        }
        .to_string(),
        severity: "medium".to_string(),
        confidence: if !pause_stop_hide_scope_declared {
            "high"
        } else if pause_stop_hide_claim_evidence_satisfied {
            "medium"
        } else {
            "low"
        }
        .to_string(),
        stage: "adapter".to_string(),
        source: "adapter".to_string(),
        message: if !pause_stop_hide_scope_declared {
            "Pause/stop/hide criterion not applicable without a declared moving/blinking/updating content scope."
                .to_string()
        } else if pause_stop_hide_claim_evidence_satisfied {
            "Pause/stop/hide review evidence is recorded for the declared moving/blinking/updating content scope."
                .to_string()
        } else {
            "Pause/stop/hide criterion is in scope for declared moving/blinking/updating content; manual evidence is required."
                .to_string()
        },
        evidence: vec![A11yVerifierEvidence {
            selector: None,
            values: vec![
                ("profile".to_string(), core.profile.clone()),
                (
                    "pause_stop_hide_scope_declared".to_string(),
                    pause_stop_hide_scope_declared.to_string(),
                ),
                (
                    "pause_stop_hide_assessed".to_string(),
                    claim_flags.wcag20_pause_stop_hide_assessed.to_string(),
                ),
                (
                    "pause_stop_hide_basis_recorded".to_string(),
                    claim_flags.wcag20_pause_stop_hide_basis_recorded.to_string(),
                ),
                (
                    "autoplay_media_count".to_string(),
                    core.facts.autoplay_media_count.to_string(),
                ),
                (
                    "blink_marquee_count".to_string(),
                    core.facts.blink_marquee_count.to_string(),
                ),
                (
                    "pause_stop_hide_signal_count".to_string(),
                    pause_stop_hide_signal_count.to_string(),
                ),
            ],
        }],
    });

    let three_flashes_scope_declared = claim_flags.wcag20_three_flashes_scope_declared;
    let three_flashes_claim_evidence_satisfied = claim_flags.wcag20_three_flashes_assessed
        && claim_flags.wcag20_three_flashes_basis_recorded;
    let flash_signal_count = core.facts.autoplay_media_count + core.facts.blink_marquee_count;
    adapter_findings.push(A11yVerifierFinding {
        rule_id: "fb.a11y.seizures.three_flashes_seed".to_string(),
        applicability: if three_flashes_scope_declared {
            "applicable"
        } else {
            "not_applicable"
        }
        .to_string(),
        verification_mode: "hybrid".to_string(),
        verdict: if !three_flashes_scope_declared {
            "not_applicable"
        } else if three_flashes_claim_evidence_satisfied {
            "pass"
        } else {
            "manual_needed"
        }
        .to_string(),
        severity: "medium".to_string(),
        confidence: if !three_flashes_scope_declared {
            "high"
        } else if three_flashes_claim_evidence_satisfied {
            "medium"
        } else {
            "low"
        }
        .to_string(),
        stage: "adapter".to_string(),
        source: "adapter".to_string(),
        message: if !three_flashes_scope_declared {
            "Three-flashes criterion not applicable without a declared flashing-content scope."
                .to_string()
        } else if three_flashes_claim_evidence_satisfied {
            "Three-flashes review evidence is recorded for the declared flashing-content scope."
                .to_string()
        } else {
            "Three-flashes criterion is in scope for declared flashing content; manual evidence is required."
                .to_string()
        },
        evidence: vec![A11yVerifierEvidence {
            selector: None,
            values: vec![
                ("profile".to_string(), core.profile.clone()),
                (
                    "three_flashes_scope_declared".to_string(),
                    three_flashes_scope_declared.to_string(),
                ),
                (
                    "three_flashes_assessed".to_string(),
                    claim_flags.wcag20_three_flashes_assessed.to_string(),
                ),
                (
                    "three_flashes_basis_recorded".to_string(),
                    claim_flags.wcag20_three_flashes_basis_recorded.to_string(),
                ),
                (
                    "autoplay_media_count".to_string(),
                    core.facts.autoplay_media_count.to_string(),
                ),
                (
                    "blink_marquee_count".to_string(),
                    core.facts.blink_marquee_count.to_string(),
                ),
                ("flash_signal_count".to_string(), flash_signal_count.to_string()),
            ],
        }],
    });

    let audio_control_scope_declared = claim_flags.wcag20_audio_control_scope_declared;
    let audio_control_claim_evidence_satisfied = claim_flags.wcag20_audio_control_assessed
        && claim_flags.wcag20_audio_control_basis_recorded;
    adapter_findings.push(A11yVerifierFinding {
        rule_id: "fb.a11y.audio.control_seed".to_string(),
        applicability: if audio_control_scope_declared {
            "applicable"
        } else {
            "not_applicable"
        }
        .to_string(),
        verification_mode: "hybrid".to_string(),
        verdict: if !audio_control_scope_declared {
            "not_applicable"
        } else if audio_control_claim_evidence_satisfied {
            "pass"
        } else {
            "manual_needed"
        }
        .to_string(),
        severity: "medium".to_string(),
        confidence: if !audio_control_scope_declared {
            "high"
        } else if audio_control_claim_evidence_satisfied {
            "medium"
        } else {
            "low"
        }
        .to_string(),
        stage: "adapter".to_string(),
        source: "adapter".to_string(),
        message: if !audio_control_scope_declared {
            "Audio-control criterion not applicable without a declared autoplay/audio playback scope."
                .to_string()
        } else if audio_control_claim_evidence_satisfied {
            "Audio-control review evidence is recorded for the declared autoplay/audio playback scope."
                .to_string()
        } else {
            "Audio-control criterion is in scope for declared autoplay/audio playback content; manual evidence is required."
                .to_string()
        },
        evidence: vec![A11yVerifierEvidence {
            selector: None,
            values: vec![
                ("profile".to_string(), core.profile.clone()),
                (
                    "audio_control_scope_declared".to_string(),
                    audio_control_scope_declared.to_string(),
                ),
                (
                    "audio_control_assessed".to_string(),
                    claim_flags.wcag20_audio_control_assessed.to_string(),
                ),
                (
                    "audio_control_basis_recorded".to_string(),
                    claim_flags.wcag20_audio_control_basis_recorded.to_string(),
                ),
                (
                    "autoplay_media_count".to_string(),
                    core.facts.autoplay_media_count.to_string(),
                ),
            ],
        }],
    });

    let use_of_color_scope_declared = claim_flags.wcag20_use_of_color_scope_declared;
    let use_of_color_claim_evidence_satisfied = claim_flags.wcag20_use_of_color_assessed
        && claim_flags.wcag20_use_of_color_basis_recorded;
    adapter_findings.push(A11yVerifierFinding {
        rule_id: "fb.a11y.color.use_of_color_seed".to_string(),
        applicability: if use_of_color_scope_declared {
            "applicable"
        } else {
            "not_applicable"
        }
        .to_string(),
        verification_mode: "hybrid".to_string(),
        verdict: if !use_of_color_scope_declared {
            "not_applicable"
        } else if use_of_color_claim_evidence_satisfied {
            "pass"
        } else {
            "manual_needed"
        }
        .to_string(),
        severity: "medium".to_string(),
        confidence: if !use_of_color_scope_declared {
            "high"
        } else if use_of_color_claim_evidence_satisfied {
            "medium"
        } else {
            "low"
        }
        .to_string(),
        stage: "adapter".to_string(),
        source: "adapter".to_string(),
        message: if !use_of_color_scope_declared {
            "Use-of-color criterion not applicable without a declared color-only meaning scope."
                .to_string()
        } else if use_of_color_claim_evidence_satisfied {
            "Use-of-color review evidence is recorded for the declared color-only meaning scope."
                .to_string()
        } else {
            "Use-of-color criterion is in scope for declared color-only meaning content; manual evidence is required."
                .to_string()
        },
        evidence: vec![A11yVerifierEvidence {
            selector: None,
            values: vec![
                ("profile".to_string(), core.profile.clone()),
                (
                    "use_of_color_scope_declared".to_string(),
                    use_of_color_scope_declared.to_string(),
                ),
                (
                    "use_of_color_assessed".to_string(),
                    claim_flags.wcag20_use_of_color_assessed.to_string(),
                ),
                (
                    "use_of_color_basis_recorded".to_string(),
                    claim_flags.wcag20_use_of_color_basis_recorded.to_string(),
                ),
            ],
        }],
    });

    let resize_text_scope_declared = claim_flags.wcag20_resize_text_scope_declared;
    let resize_text_claim_evidence_satisfied = claim_flags.wcag20_resize_text_assessed
        && claim_flags.wcag20_resize_text_basis_recorded;
    adapter_findings.push(A11yVerifierFinding {
        rule_id: "fb.a11y.text.resize_seed".to_string(),
        applicability: if resize_text_scope_declared {
            "applicable"
        } else {
            "not_applicable"
        }
        .to_string(),
        verification_mode: "hybrid".to_string(),
        verdict: if !resize_text_scope_declared {
            "not_applicable"
        } else if resize_text_claim_evidence_satisfied {
            "pass"
        } else {
            "manual_needed"
        }
        .to_string(),
        severity: "medium".to_string(),
        confidence: if !resize_text_scope_declared {
            "high"
        } else if resize_text_claim_evidence_satisfied {
            "medium"
        } else {
            "low"
        }
        .to_string(),
        stage: "adapter".to_string(),
        source: "adapter".to_string(),
        message: if !resize_text_scope_declared {
            "Resize-text criterion not applicable without a declared text-resize review scope."
                .to_string()
        } else if resize_text_claim_evidence_satisfied {
            "Resize-text review evidence is recorded for the declared text-resize scope."
                .to_string()
        } else {
            "Resize-text criterion is in scope for declared text content; manual evidence is required."
                .to_string()
        },
        evidence: vec![A11yVerifierEvidence {
            selector: None,
            values: vec![
                ("profile".to_string(), core.profile.clone()),
                (
                    "resize_text_scope_declared".to_string(),
                    resize_text_scope_declared.to_string(),
                ),
                (
                    "resize_text_assessed".to_string(),
                    claim_flags.wcag20_resize_text_assessed.to_string(),
                ),
                (
                    "resize_text_basis_recorded".to_string(),
                    claim_flags.wcag20_resize_text_basis_recorded.to_string(),
                ),
                ("link_count".to_string(), core.facts.link_count.to_string()),
                (
                    "form_control_count".to_string(),
                    core.facts.form_control_count.to_string(),
                ),
            ],
        }],
    });

    let images_of_text_scope_declared = claim_flags.wcag20_images_of_text_scope_declared;
    let images_of_text_claim_evidence_satisfied = claim_flags.wcag20_images_of_text_assessed
        && claim_flags.wcag20_images_of_text_basis_recorded;
    adapter_findings.push(A11yVerifierFinding {
        rule_id: "fb.a11y.images.of_text_seed".to_string(),
        applicability: if images_of_text_scope_declared {
            "applicable"
        } else {
            "not_applicable"
        }
        .to_string(),
        verification_mode: "hybrid".to_string(),
        verdict: if !images_of_text_scope_declared {
            "not_applicable"
        } else if images_of_text_claim_evidence_satisfied {
            "pass"
        } else {
            "manual_needed"
        }
        .to_string(),
        severity: "medium".to_string(),
        confidence: if !images_of_text_scope_declared {
            "high"
        } else if images_of_text_claim_evidence_satisfied {
            "medium"
        } else {
            "low"
        }
        .to_string(),
        stage: "adapter".to_string(),
        source: "adapter".to_string(),
        message: if !images_of_text_scope_declared {
            "Images-of-text criterion not applicable without a declared images-of-text review scope."
                .to_string()
        } else if images_of_text_claim_evidence_satisfied {
            "Images-of-text review evidence is recorded for the declared images-of-text scope."
                .to_string()
        } else {
            "Images-of-text criterion is in scope for declared content; manual evidence is required."
                .to_string()
        },
        evidence: vec![A11yVerifierEvidence {
            selector: None,
            values: vec![
                ("profile".to_string(), core.profile.clone()),
                (
                    "images_of_text_scope_declared".to_string(),
                    images_of_text_scope_declared.to_string(),
                ),
                (
                    "images_of_text_assessed".to_string(),
                    claim_flags.wcag20_images_of_text_assessed.to_string(),
                ),
                (
                    "images_of_text_basis_recorded".to_string(),
                    claim_flags.wcag20_images_of_text_basis_recorded.to_string(),
                ),
                ("image_count".to_string(), core.facts.image_count.to_string()),
                (
                    "image_title_only_count".to_string(),
                    core.facts.image_title_only_count.to_string(),
                ),
            ],
        }],
    });

    let prerecorded_av_alternative_scope_declared =
        claim_flags.wcag20_prerecorded_av_alternative_scope_declared;
    let prerecorded_av_alternative_claim_evidence_satisfied =
        claim_flags.wcag20_prerecorded_av_alternative_assessed
            && claim_flags.wcag20_prerecorded_av_alternative_basis_recorded;
    adapter_findings.push(A11yVerifierFinding {
        rule_id: "fb.a11y.media.prerecorded_audio_video_alternative_seed".to_string(),
        applicability: if prerecorded_av_alternative_scope_declared {
            "applicable"
        } else {
            "not_applicable"
        }
        .to_string(),
        verification_mode: "hybrid".to_string(),
        verdict: if !prerecorded_av_alternative_scope_declared {
            "not_applicable"
        } else if prerecorded_av_alternative_claim_evidence_satisfied {
            "pass"
        } else {
            "manual_needed"
        }
        .to_string(),
        severity: "medium".to_string(),
        confidence: if !prerecorded_av_alternative_scope_declared {
            "high"
        } else if prerecorded_av_alternative_claim_evidence_satisfied {
            "medium"
        } else {
            "low"
        }
        .to_string(),
        stage: "adapter".to_string(),
        source: "adapter".to_string(),
        message: if !prerecorded_av_alternative_scope_declared {
            "Prerecorded audio-only/video-only alternative criterion not applicable without a declared media-alternative review scope."
                .to_string()
        } else if prerecorded_av_alternative_claim_evidence_satisfied {
            "Prerecorded audio-only/video-only alternative review evidence is recorded for the declared scope."
                .to_string()
        } else {
            "Prerecorded audio-only/video-only alternative criterion is in scope for declared media content; manual evidence is required."
                .to_string()
        },
        evidence: vec![A11yVerifierEvidence {
            selector: None,
            values: vec![
                ("profile".to_string(), core.profile.clone()),
                (
                    "prerecorded_av_alternative_scope_declared".to_string(),
                    prerecorded_av_alternative_scope_declared.to_string(),
                ),
                (
                    "prerecorded_av_alternative_assessed".to_string(),
                    claim_flags
                        .wcag20_prerecorded_av_alternative_assessed
                        .to_string(),
                ),
                (
                    "prerecorded_av_alternative_basis_recorded".to_string(),
                    claim_flags
                        .wcag20_prerecorded_av_alternative_basis_recorded
                        .to_string(),
                ),
                (
                    "autoplay_media_count".to_string(),
                    core.facts.autoplay_media_count.to_string(),
                ),
            ],
        }],
    });

    let prerecorded_captions_scope_declared =
        claim_flags.wcag20_prerecorded_captions_scope_declared;
    let prerecorded_captions_claim_evidence_satisfied =
        claim_flags.wcag20_prerecorded_captions_assessed
            && claim_flags.wcag20_prerecorded_captions_basis_recorded;
    adapter_findings.push(A11yVerifierFinding {
        rule_id: "fb.a11y.media.prerecorded_captions_seed".to_string(),
        applicability: if prerecorded_captions_scope_declared {
            "applicable"
        } else {
            "not_applicable"
        }
        .to_string(),
        verification_mode: "hybrid".to_string(),
        verdict: if !prerecorded_captions_scope_declared {
            "not_applicable"
        } else if prerecorded_captions_claim_evidence_satisfied {
            "pass"
        } else {
            "manual_needed"
        }
        .to_string(),
        severity: "medium".to_string(),
        confidence: if !prerecorded_captions_scope_declared {
            "high"
        } else if prerecorded_captions_claim_evidence_satisfied {
            "medium"
        } else {
            "low"
        }
        .to_string(),
        stage: "adapter".to_string(),
        source: "adapter".to_string(),
        message: if !prerecorded_captions_scope_declared {
            "Prerecorded captions criterion not applicable without a declared prerecorded-media captions review scope."
                .to_string()
        } else if prerecorded_captions_claim_evidence_satisfied {
            "Prerecorded captions review evidence is recorded for the declared media scope."
                .to_string()
        } else {
            "Prerecorded captions criterion is in scope for declared prerecorded media; manual evidence is required."
                .to_string()
        },
        evidence: vec![A11yVerifierEvidence {
            selector: None,
            values: vec![
                ("profile".to_string(), core.profile.clone()),
                (
                    "prerecorded_captions_scope_declared".to_string(),
                    prerecorded_captions_scope_declared.to_string(),
                ),
                (
                    "prerecorded_captions_assessed".to_string(),
                    claim_flags
                        .wcag20_prerecorded_captions_assessed
                        .to_string(),
                ),
                (
                    "prerecorded_captions_basis_recorded".to_string(),
                    claim_flags
                        .wcag20_prerecorded_captions_basis_recorded
                        .to_string(),
                ),
                (
                    "autoplay_media_count".to_string(),
                    core.facts.autoplay_media_count.to_string(),
                ),
            ],
        }],
    });

    let prerecorded_ad_or_media_alt_scope_declared = claim_flags
        .wcag20_prerecorded_audio_description_or_media_alternative_scope_declared;
    let prerecorded_ad_or_media_alt_claim_evidence_satisfied = claim_flags
        .wcag20_prerecorded_audio_description_or_media_alternative_assessed
        && claim_flags
            .wcag20_prerecorded_audio_description_or_media_alternative_basis_recorded;
    adapter_findings.push(A11yVerifierFinding {
        rule_id: "fb.a11y.media.prerecorded_audio_description_or_media_alternative_seed"
            .to_string(),
        applicability: if prerecorded_ad_or_media_alt_scope_declared {
            "applicable"
        } else {
            "not_applicable"
        }
        .to_string(),
        verification_mode: "hybrid".to_string(),
        verdict: if !prerecorded_ad_or_media_alt_scope_declared {
            "not_applicable"
        } else if prerecorded_ad_or_media_alt_claim_evidence_satisfied {
            "pass"
        } else {
            "manual_needed"
        }
        .to_string(),
        severity: "medium".to_string(),
        confidence: if !prerecorded_ad_or_media_alt_scope_declared {
            "high"
        } else if prerecorded_ad_or_media_alt_claim_evidence_satisfied {
            "medium"
        } else {
            "low"
        }
        .to_string(),
        stage: "adapter".to_string(),
        source: "adapter".to_string(),
        message: if !prerecorded_ad_or_media_alt_scope_declared {
            "Prerecorded audio-description/media-alternative criterion not applicable without a declared review scope."
                .to_string()
        } else if prerecorded_ad_or_media_alt_claim_evidence_satisfied {
            "Prerecorded audio-description/media-alternative review evidence is recorded for the declared media scope."
                .to_string()
        } else {
            "Prerecorded audio-description/media-alternative criterion is in scope for declared prerecorded media; manual evidence is required."
                .to_string()
        },
        evidence: vec![A11yVerifierEvidence {
            selector: None,
            values: vec![
                ("profile".to_string(), core.profile.clone()),
                (
                    "prerecorded_audio_description_or_media_alternative_scope_declared"
                        .to_string(),
                    prerecorded_ad_or_media_alt_scope_declared.to_string(),
                ),
                (
                    "prerecorded_audio_description_or_media_alternative_assessed"
                        .to_string(),
                    claim_flags
                        .wcag20_prerecorded_audio_description_or_media_alternative_assessed
                        .to_string(),
                ),
                (
                    "prerecorded_audio_description_or_media_alternative_basis_recorded"
                        .to_string(),
                    claim_flags
                        .wcag20_prerecorded_audio_description_or_media_alternative_basis_recorded
                        .to_string(),
                ),
                (
                    "autoplay_media_count".to_string(),
                    core.facts.autoplay_media_count.to_string(),
                ),
            ],
        }],
    });

    let live_captions_scope_declared = claim_flags.wcag20_live_captions_scope_declared;
    let live_captions_claim_evidence_satisfied =
        claim_flags.wcag20_live_captions_assessed && claim_flags.wcag20_live_captions_basis_recorded;
    adapter_findings.push(A11yVerifierFinding {
        rule_id: "fb.a11y.media.live_captions_seed".to_string(),
        applicability: if live_captions_scope_declared {
            "applicable"
        } else {
            "not_applicable"
        }
        .to_string(),
        verification_mode: "hybrid".to_string(),
        verdict: if !live_captions_scope_declared {
            "not_applicable"
        } else if live_captions_claim_evidence_satisfied {
            "pass"
        } else {
            "manual_needed"
        }
        .to_string(),
        severity: "medium".to_string(),
        confidence: if !live_captions_scope_declared {
            "high"
        } else if live_captions_claim_evidence_satisfied {
            "medium"
        } else {
            "low"
        }
        .to_string(),
        stage: "adapter".to_string(),
        source: "adapter".to_string(),
        message: if !live_captions_scope_declared {
            "Live-captions criterion not applicable without a declared live-media captions review scope."
                .to_string()
        } else if live_captions_claim_evidence_satisfied {
            "Live captions review evidence is recorded for the declared live-media scope."
                .to_string()
        } else {
            "Live-captions criterion is in scope for declared live media; manual evidence is required."
                .to_string()
        },
        evidence: vec![A11yVerifierEvidence {
            selector: None,
            values: vec![
                ("profile".to_string(), core.profile.clone()),
                (
                    "live_captions_scope_declared".to_string(),
                    live_captions_scope_declared.to_string(),
                ),
                (
                    "live_captions_assessed".to_string(),
                    claim_flags.wcag20_live_captions_assessed.to_string(),
                ),
                (
                    "live_captions_basis_recorded".to_string(),
                    claim_flags.wcag20_live_captions_basis_recorded.to_string(),
                ),
                (
                    "autoplay_media_count".to_string(),
                    core.facts.autoplay_media_count.to_string(),
                ),
            ],
        }],
    });

    let prerecorded_audio_description_scope_declared =
        claim_flags.wcag20_prerecorded_audio_description_scope_declared;
    let prerecorded_audio_description_claim_evidence_satisfied =
        claim_flags.wcag20_prerecorded_audio_description_assessed
            && claim_flags.wcag20_prerecorded_audio_description_basis_recorded;
    adapter_findings.push(A11yVerifierFinding {
        rule_id: "fb.a11y.media.prerecorded_audio_description_seed".to_string(),
        applicability: if prerecorded_audio_description_scope_declared {
            "applicable"
        } else {
            "not_applicable"
        }
        .to_string(),
        verification_mode: "hybrid".to_string(),
        verdict: if !prerecorded_audio_description_scope_declared {
            "not_applicable"
        } else if prerecorded_audio_description_claim_evidence_satisfied {
            "pass"
        } else {
            "manual_needed"
        }
        .to_string(),
        severity: "medium".to_string(),
        confidence: if !prerecorded_audio_description_scope_declared {
            "high"
        } else if prerecorded_audio_description_claim_evidence_satisfied {
            "medium"
        } else {
            "low"
        }
        .to_string(),
        stage: "adapter".to_string(),
        source: "adapter".to_string(),
        message: if !prerecorded_audio_description_scope_declared {
            "Prerecorded audio-description criterion not applicable without a declared prerecorded-audio-description review scope."
                .to_string()
        } else if prerecorded_audio_description_claim_evidence_satisfied {
            "Prerecorded audio-description review evidence is recorded for the declared media scope."
                .to_string()
        } else {
            "Prerecorded audio-description criterion is in scope for declared prerecorded media; manual evidence is required."
                .to_string()
        },
        evidence: vec![A11yVerifierEvidence {
            selector: None,
            values: vec![
                ("profile".to_string(), core.profile.clone()),
                (
                    "prerecorded_audio_description_scope_declared".to_string(),
                    prerecorded_audio_description_scope_declared.to_string(),
                ),
                (
                    "prerecorded_audio_description_assessed".to_string(),
                    claim_flags
                        .wcag20_prerecorded_audio_description_assessed
                        .to_string(),
                ),
                (
                    "prerecorded_audio_description_basis_recorded".to_string(),
                    claim_flags
                        .wcag20_prerecorded_audio_description_basis_recorded
                        .to_string(),
                ),
                (
                    "autoplay_media_count".to_string(),
                    core.facts.autoplay_media_count.to_string(),
                ),
            ],
        }],
    });

    let meaningful_sequence_scope_declared = claim_flags.wcag20_meaningful_sequence_scope_declared;
    let meaningful_sequence_claim_evidence_satisfied = claim_flags.wcag20_meaningful_sequence_assessed
        && claim_flags.wcag20_meaningful_sequence_basis_recorded;
    adapter_findings.push(A11yVerifierFinding {
        rule_id: "fb.a11y.sequence.meaningful_sequence_seed".to_string(),
        applicability: if meaningful_sequence_scope_declared {
            "applicable"
        } else {
            "not_applicable"
        }
        .to_string(),
        verification_mode: "hybrid".to_string(),
        verdict: if !meaningful_sequence_scope_declared {
            "not_applicable"
        } else if meaningful_sequence_claim_evidence_satisfied {
            "pass"
        } else {
            "manual_needed"
        }
        .to_string(),
        severity: "medium".to_string(),
        confidence: if !meaningful_sequence_scope_declared {
            "high"
        } else if meaningful_sequence_claim_evidence_satisfied {
            "medium"
        } else {
            "low"
        }
        .to_string(),
        stage: "adapter".to_string(),
        source: "adapter".to_string(),
        message: if !meaningful_sequence_scope_declared {
            "Meaningful-sequence criterion not applicable without a declared sequence-dependent content scope."
                .to_string()
        } else if meaningful_sequence_claim_evidence_satisfied {
            "Meaningful-sequence review evidence is recorded for the declared content sequence scope."
                .to_string()
        } else {
            "Meaningful-sequence criterion is in scope for declared content; manual evidence is required."
                .to_string()
        },
        evidence: vec![A11yVerifierEvidence {
            selector: None,
            values: vec![
                ("profile".to_string(), core.profile.clone()),
                (
                    "meaningful_sequence_scope_declared".to_string(),
                    meaningful_sequence_scope_declared.to_string(),
                ),
                (
                    "meaningful_sequence_assessed".to_string(),
                    claim_flags.wcag20_meaningful_sequence_assessed.to_string(),
                ),
                (
                    "meaningful_sequence_basis_recorded".to_string(),
                    claim_flags.wcag20_meaningful_sequence_basis_recorded.to_string(),
                ),
                (
                    "table_count".to_string(),
                    core.facts.tables.len().to_string(),
                ),
                (
                    "body_text_char_count".to_string(),
                    core.facts.body_text.chars().count().to_string(),
                ),
            ],
        }],
    });

    let multiple_ways_scope_declared = claim_flags.wcag20_multiple_ways_scope_declared;
    let multiple_ways_claim_evidence_satisfied = claim_flags.wcag20_multiple_ways_assessed
        && claim_flags.wcag20_multiple_ways_basis_recorded;
    adapter_findings.push(A11yVerifierFinding {
        rule_id: "fb.a11y.navigation.multiple_ways_seed".to_string(),
        applicability: if multiple_ways_scope_declared {
            "applicable"
        } else {
            "not_applicable"
        }
        .to_string(),
        verification_mode: "hybrid".to_string(),
        verdict: if !multiple_ways_scope_declared {
            "not_applicable"
        } else if multiple_ways_claim_evidence_satisfied {
            "pass"
        } else {
            "manual_needed"
        }
        .to_string(),
        severity: "medium".to_string(),
        confidence: if !multiple_ways_scope_declared {
            "high"
        } else if multiple_ways_claim_evidence_satisfied {
            "medium"
        } else {
            "low"
        }
        .to_string(),
        stage: "adapter".to_string(),
        source: "adapter".to_string(),
        message: if !multiple_ways_scope_declared {
            "Multiple-ways criterion not applicable without a declared page-set navigation scope."
                .to_string()
        } else if multiple_ways_claim_evidence_satisfied {
            "Multiple-ways navigation/access-path review evidence is recorded for the declared page-set scope."
                .to_string()
        } else {
            "Multiple-ways criterion is in scope for the declared page-set; manual evidence is required."
                .to_string()
        },
        evidence: vec![A11yVerifierEvidence {
            selector: None,
            values: vec![
                ("profile".to_string(), core.profile.clone()),
                (
                    "multiple_ways_scope_declared".to_string(),
                    multiple_ways_scope_declared.to_string(),
                ),
                (
                    "multiple_ways_assessed".to_string(),
                    claim_flags.wcag20_multiple_ways_assessed.to_string(),
                ),
                (
                    "multiple_ways_basis_recorded".to_string(),
                    claim_flags
                        .wcag20_multiple_ways_basis_recorded
                        .to_string(),
                ),
            ],
        }],
    });

    let consistent_navigation_scope_declared =
        claim_flags.wcag20_consistent_navigation_scope_declared;
    let consistent_navigation_claim_evidence_satisfied =
        claim_flags.wcag20_consistent_navigation_assessed
            && claim_flags.wcag20_consistent_navigation_basis_recorded;
    adapter_findings.push(A11yVerifierFinding {
        rule_id: "fb.a11y.navigation.consistent_navigation_seed".to_string(),
        applicability: if consistent_navigation_scope_declared {
            "applicable"
        } else {
            "not_applicable"
        }
        .to_string(),
        verification_mode: "hybrid".to_string(),
        verdict: if !consistent_navigation_scope_declared {
            "not_applicable"
        } else if consistent_navigation_claim_evidence_satisfied {
            "pass"
        } else {
            "manual_needed"
        }
        .to_string(),
        severity: "medium".to_string(),
        confidence: if !consistent_navigation_scope_declared {
            "high"
        } else if consistent_navigation_claim_evidence_satisfied {
            "medium"
        } else {
            "low"
        }
        .to_string(),
        stage: "adapter".to_string(),
        source: "adapter".to_string(),
        message: if !consistent_navigation_scope_declared {
            "Consistent-navigation criterion not applicable without a declared page-set navigation scope."
                .to_string()
        } else if consistent_navigation_claim_evidence_satisfied {
            "Consistent-navigation review evidence is recorded for the declared page-set scope."
                .to_string()
        } else {
            "Consistent-navigation criterion is in scope for the declared page-set; manual evidence is required."
                .to_string()
        },
        evidence: vec![A11yVerifierEvidence {
            selector: None,
            values: vec![
                ("profile".to_string(), core.profile.clone()),
                (
                    "consistent_navigation_scope_declared".to_string(),
                    consistent_navigation_scope_declared.to_string(),
                ),
                (
                    "consistent_navigation_assessed".to_string(),
                    claim_flags
                        .wcag20_consistent_navigation_assessed
                        .to_string(),
                ),
                (
                    "consistent_navigation_basis_recorded".to_string(),
                    claim_flags
                        .wcag20_consistent_navigation_basis_recorded
                        .to_string(),
                ),
            ],
        }],
    });

    let ast_signal_count = core.facts.script_element_count
        + core.facts.embedded_active_content_count
        + core.facts.autoplay_media_count
        + core.facts.blink_marquee_count
        + core.facts.inline_event_handler_attr_count
        + core.facts.meta_refresh_count;
    let tech_claim_evidence_satisfied =
        claim_flags.technology_support_assessed && claim_flags.technology_support_basis_recorded;
    adapter_findings.push(A11yVerifierFinding {
        rule_id: "fb.a11y.claim.accessibility_supported_technologies_seed".to_string(),
        applicability: "applicable".to_string(),
        verification_mode: "hybrid".to_string(),
        verdict: if ast_signal_count > 0 {
            "warn"
        } else if tech_claim_evidence_satisfied {
            "pass"
        } else {
            "manual_needed"
        }
        .to_string(),
        severity: "medium".to_string(),
        confidence: if ast_signal_count > 0 {
            "medium"
        } else if tech_claim_evidence_satisfied {
            "medium"
        } else {
            "low"
        }
        .to_string(),
        stage: "adapter".to_string(),
        source: "adapter".to_string(),
        message: if ast_signal_count > 0 {
            "Potential technology support risk signals detected; accessibility-supported technology claim requires manual evidence."
                .to_string()
        } else if tech_claim_evidence_satisfied {
            "Accessibility-supported technology claim evidence is recorded and no obvious technology-risk signals were detected."
                .to_string()
        } else {
            "No obvious technology-support risk signals detected, but accessibility-supported technology claim still requires manual evidence."
                .to_string()
        },
        evidence: vec![A11yVerifierEvidence {
            selector: None,
            values: vec![
                (
                    "embedded_active_content_count".to_string(),
                    core.facts.embedded_active_content_count.to_string(),
                ),
                (
                    "script_element_count".to_string(),
                    core.facts.script_element_count.to_string(),
                ),
                (
                    "autoplay_media_count".to_string(),
                    core.facts.autoplay_media_count.to_string(),
                ),
                (
                    "blink_marquee_count".to_string(),
                    core.facts.blink_marquee_count.to_string(),
                ),
                (
                    "inline_event_handler_attr_count".to_string(),
                    core.facts.inline_event_handler_attr_count.to_string(),
                ),
                (
                    "meta_refresh_count".to_string(),
                    core.facts.meta_refresh_count.to_string(),
                ),
                ("css_linked".to_string(), core.facts.has_css_link.to_string()),
                (
                    "technology_support_assessed".to_string(),
                    claim_flags.technology_support_assessed.to_string(),
                ),
                (
                    "technology_support_basis_recorded".to_string(),
                    claim_flags.technology_support_basis_recorded.to_string(),
                ),
            ],
        }],
    });

    let s508_public_scope_evidence =
        claim_flags.section508_scope_declared && claim_flags.section508_public_facing_determination_recorded;
    adapter_findings.push(A11yVerifierFinding {
        rule_id: "fb.a11y.claim.section508.public_facing_content_applicability_seed".to_string(),
        applicability: "applicable".to_string(),
        verification_mode: "hybrid".to_string(),
        verdict: if s508_public_scope_evidence { "pass" } else { "manual_needed" }.to_string(),
        severity: "medium".to_string(),
        confidence: if s508_public_scope_evidence { "medium" } else { "low" }.to_string(),
        stage: "adapter".to_string(),
        source: "adapter".to_string(),
        message: if s508_public_scope_evidence {
            "Section 508 E205.2 public-facing applicability decision evidence is recorded."
                .to_string()
        } else {
            "Section 508 E205.2 public-facing applicability requires agency/content scope evidence; manual review required.".to_string()
        },
        evidence: vec![A11yVerifierEvidence {
            selector: None,
            values: vec![
                ("delivery_target".to_string(), "html".to_string()),
                (
                    "section508_scope_declared".to_string(),
                    claim_flags.section508_scope_declared.to_string(),
                ),
                (
                    "public_facing_determination_recorded".to_string(),
                    claim_flags
                        .section508_public_facing_determination_recorded
                        .to_string(),
                ),
            ],
        }],
    });

    let s508_official_scope_evidence = claim_flags.section508_scope_declared
        && claim_flags.section508_official_communications_determination_recorded;
    adapter_findings.push(A11yVerifierFinding {
        rule_id: "fb.a11y.claim.section508.official_communications_applicability_seed".to_string(),
        applicability: "applicable".to_string(),
        verification_mode: "hybrid".to_string(),
        verdict: if s508_official_scope_evidence { "pass" } else { "manual_needed" }.to_string(),
        severity: "medium".to_string(),
        confidence: if s508_official_scope_evidence { "medium" } else { "low" }.to_string(),
        stage: "adapter".to_string(),
        source: "adapter".to_string(),
        message: if s508_official_scope_evidence {
            "Section 508 E205.3 official communications applicability decision evidence is recorded."
                .to_string()
        } else {
            "Section 508 E205.3 agency official communications applicability requires agency communication-scope evidence; manual review required.".to_string()
        },
        evidence: vec![A11yVerifierEvidence {
            selector: None,
            values: vec![
                ("delivery_target".to_string(), "html".to_string()),
                (
                    "section508_scope_declared".to_string(),
                    claim_flags.section508_scope_declared.to_string(),
                ),
                (
                    "official_communications_determination_recorded".to_string(),
                    claim_flags
                        .section508_official_communications_determination_recorded
                        .to_string(),
                ),
            ],
        }],
    });

    let s508_nara_scope_evidence =
        claim_flags.section508_nara_exception_determination_recorded;
    adapter_findings.push(A11yVerifierFinding {
        rule_id: "fb.a11y.claim.section508.nara_exception_applicability_seed".to_string(),
        applicability: "applicable".to_string(),
        verification_mode: "hybrid".to_string(),
        verdict: if s508_nara_scope_evidence { "pass" } else { "manual_needed" }.to_string(),
        severity: "low".to_string(),
        confidence: if s508_nara_scope_evidence { "medium" } else { "low" }.to_string(),
        stage: "adapter".to_string(),
        source: "adapter".to_string(),
        message: if s508_nara_scope_evidence {
            "Section 508 E205.3 NARA exception applicability decision evidence is recorded."
                .to_string()
        } else {
            "Section 508 E205.3 NARA exception applicability requires organization/content stewardship evidence; manual review required.".to_string()
        },
        evidence: vec![A11yVerifierEvidence {
            selector: None,
            values: vec![
                ("delivery_target".to_string(), "html".to_string()),
                (
                    "nara_exception_determination_recorded".to_string(),
                    claim_flags
                        .section508_nara_exception_determination_recorded
                        .to_string(),
                ),
            ],
        }],
    });

    adapter_findings.push(A11yVerifierFinding {
        rule_id: "fb.a11y.claim.section508.non_web_document_exceptions_html_seed".to_string(),
        applicability: "not_applicable".to_string(),
        verification_mode: "hybrid".to_string(),
        verdict: "not_applicable".to_string(),
        severity: "low".to_string(),
        confidence: "high".to_string(),
        stage: "adapter".to_string(),
        source: "adapter".to_string(),
        message: "Section 508 E205.4 Exception and E205.4.1 word-substitution rules are not applicable to HTML deliverables (non-web document path not in scope).".to_string(),
        evidence: vec![A11yVerifierEvidence {
            selector: None,
            values: vec![
                ("delivery_target".to_string(), "html".to_string()),
                ("non_web_document_path".to_string(), "false".to_string()),
            ],
        }],
    });

    let consistent_identification_target_count = core.facts.link_count + core.facts.form_control_count;
    let consistent_identification_claim_evidence_satisfied =
        claim_flags.wcag20_consistent_identification_assessed
            && claim_flags.wcag20_consistent_identification_basis_recorded;
    adapter_findings.push(A11yVerifierFinding {
        rule_id: "fb.a11y.identification.consistent_identification_seed".to_string(),
        applicability: if consistent_identification_target_count == 0 {
            "not_applicable"
        } else {
            "applicable"
        }
        .to_string(),
        verification_mode: "hybrid".to_string(),
        verdict: if consistent_identification_target_count == 0 {
            "not_applicable"
        } else if consistent_identification_claim_evidence_satisfied {
            "pass"
        } else {
            "manual_needed"
        }
        .to_string(),
        severity: "medium".to_string(),
        confidence: if consistent_identification_target_count == 0 {
            "high"
        } else if consistent_identification_claim_evidence_satisfied {
            "medium"
        } else {
            "low"
        }
        .to_string(),
        stage: "adapter".to_string(),
        source: "adapter".to_string(),
        message: if consistent_identification_target_count == 0 {
            "No interactive links or form controls detected; consistent-identification seed not applicable."
                .to_string()
        } else if consistent_identification_claim_evidence_satisfied {
            "Consistent-identification review evidence is recorded for interactive components."
                .to_string()
        } else {
            "Interactive components detected; consistent-identification review requires manual evidence."
                .to_string()
        },
        evidence: vec![A11yVerifierEvidence {
            selector: None,
            values: vec![
                (
                    "interactive_identification_target_count".to_string(),
                    consistent_identification_target_count.to_string(),
                ),
                ("link_count".to_string(), core.facts.link_count.to_string()),
                (
                    "form_control_count".to_string(),
                    core.facts.form_control_count.to_string(),
                ),
                (
                    "consistent_identification_assessed".to_string(),
                    claim_flags
                        .wcag20_consistent_identification_assessed
                        .to_string(),
                ),
                (
                    "consistent_identification_basis_recorded".to_string(),
                    claim_flags
                        .wcag20_consistent_identification_basis_recorded
                        .to_string(),
                ),
            ],
        }],
    });

    let focus_seed_css_text = std::fs::read_to_string(css_path).unwrap_or_default();
    let focus_seed_css_facts = a11y_css_focus_seed_facts(&focus_seed_css_text);
    let focus_interactive_target_count = core.facts.link_count + core.facts.form_control_count;
    let focus_seed_has_focus_selectors = focus_seed_css_facts.focus_selector_signal_count > 0;
    let focus_seed_has_outline_suppression =
        focus_seed_css_facts.outline_suppression_signal_count > 0;
    adapter_findings.push(A11yVerifierFinding {
        rule_id: "fb.a11y.focus.visible_seed".to_string(),
        applicability: if focus_interactive_target_count == 0 {
            "not_applicable"
        } else {
            "applicable"
        }
        .to_string(),
        verification_mode: "hybrid".to_string(),
        verdict: if focus_interactive_target_count == 0 {
            "not_applicable"
        } else if focus_seed_has_focus_selectors {
            "pass"
        } else if focus_seed_has_outline_suppression {
            "warn"
        } else {
            "manual_needed"
        }
        .to_string(),
        severity: "medium".to_string(),
        confidence: if focus_interactive_target_count == 0 {
            "high"
        } else if focus_seed_has_focus_selectors {
            "medium"
        } else if focus_seed_has_outline_suppression {
            "medium"
        } else {
            "low"
        }
        .to_string(),
        stage: "adapter".to_string(),
        source: "adapter".to_string(),
        message: if focus_interactive_target_count == 0 {
            "No interactive links or form controls detected; focus-visible seed not applicable."
                .to_string()
        } else if focus_seed_has_focus_selectors {
            "Focus-style selector signals detected in CSS for interactive content."
                .to_string()
        } else if focus_seed_has_outline_suppression {
            "Outline suppression signals detected without focus-style selector signals; focus visibility may be reduced."
                .to_string()
        } else {
            "Interactive content detected but no explicit focus-style CSS signals found; manual review required."
                .to_string()
        },
        evidence: vec![A11yVerifierEvidence {
            selector: None,
            values: vec![
                (
                    "interactive_focus_target_count".to_string(),
                    focus_interactive_target_count.to_string(),
                ),
                ("link_count".to_string(), core.facts.link_count.to_string()),
                (
                    "form_control_count".to_string(),
                    core.facts.form_control_count.to_string(),
                ),
                (
                    "focus_selector_signal_count".to_string(),
                    focus_seed_css_facts.focus_selector_signal_count.to_string(),
                ),
                (
                    "outline_suppression_signal_count".to_string(),
                    focus_seed_css_facts.outline_suppression_signal_count.to_string(),
                ),
            ],
        }],
    });

    if let Some(png_path) = render_preview_png_path {
        let contrast_finding = match analyze_render_contrast_seed_png(png_path) {
            Ok(analysis) => A11yVerifierFinding {
                rule_id: "fb.a11y.contrast.minimum_render_seed".to_string(),
                applicability: "applicable".to_string(),
                verification_mode: "hybrid".to_string(),
                verdict: analysis.verdict.to_string(),
                severity: "medium".to_string(),
                confidence: analysis.confidence.to_string(),
                stage: "post-render".to_string(),
                source: "adapter".to_string(),
                message: analysis.message.clone(),
                evidence: vec![A11yVerifierEvidence {
                    selector: None,
                    values: vec![
                        ("render_preview_png_path".to_string(), png_path.to_string()),
                        ("width".to_string(), analysis.width.to_string()),
                        ("height".to_string(), analysis.height.to_string()),
                        (
                            "opaque_pixel_count".to_string(),
                            analysis.opaque_pixel_count.to_string(),
                        ),
                        ("ink_pixel_count".to_string(), analysis.ink_pixel_count.to_string()),
                        (
                            "background_luminance".to_string(),
                            format!("{:.6}", analysis.background_luminance),
                        ),
                        (
                            "foreground_luminance".to_string(),
                            analysis
                                .foreground_luminance
                                .map(|v| format!("{v:.6}"))
                                .unwrap_or_default(),
                        ),
                        (
                            "estimated_contrast_ratio".to_string(),
                            analysis
                                .estimated_contrast_ratio
                                .map(|v| format!("{v:.4}"))
                                .unwrap_or_default(),
                        ),
                    ],
                }],
            },
            Err(err) => A11yVerifierFinding {
                rule_id: "fb.a11y.contrast.minimum_render_seed".to_string(),
                applicability: "unknown".to_string(),
                verification_mode: "hybrid".to_string(),
                verdict: "manual_needed".to_string(),
                severity: "medium".to_string(),
                confidence: "low".to_string(),
                stage: "post-render".to_string(),
                source: "adapter".to_string(),
                message: format!("Render-based contrast seed could not run: {err}"),
                evidence: vec![A11yVerifierEvidence {
                    selector: None,
                    values: vec![("render_preview_png_path".to_string(), png_path.to_string())],
                }],
            },
        };
        adapter_findings.push(contrast_finding);
    }

    let mut all_findings: Vec<A11yVerifierFinding> = Vec::new();
    all_findings.extend(core.findings.iter().cloned());
    all_findings.extend(a11y_bridge_findings_from_contract_report(a11y_report)?);
    all_findings.extend(adapter_findings.iter().cloned());

    // Compute claim-readiness status from the pre-claim finding set, then append a scaffold finding
    // so wcag20.conf.level can be tracked as an evaluated implemented mapping.
    let claim_machine_blocker_count = all_findings
        .iter()
        .filter(|f| f.verdict == "fail")
        .count();
    let wcag_pairs_pre: Vec<(&str, &str)> = all_findings
        .iter()
        .map(|f| (f.rule_id.as_str(), f.verdict.as_str()))
        .collect();
    let wcag_summary_pre = audit_contract::wcag20aa_coverage_from_rule_verdicts(wcag_pairs_pre);
    let claim_coverage_gap_count = wcag_summary_pre.unmapped_entry_count
        + wcag_summary_pre.implemented_mapped_entry_pending_count;
    let claim_status = if claim_machine_blocker_count > 0 {
        "blocked_machine_failures"
    } else if claim_coverage_gap_count > 0 {
        "blocked_coverage_gaps"
    } else {
        "manual_evidence_required"
    };
    let claim_verdict = if matches!(claim_status, "blocked_machine_failures" | "blocked_coverage_gaps")
    {
        "warn"
    } else {
        "manual_needed"
    };
    let claim_finding = A11yVerifierFinding {
        rule_id: "fb.a11y.claim.wcag20aa_level_readiness".to_string(),
        applicability: "applicable".to_string(),
        verification_mode: "hybrid".to_string(),
        verdict: claim_verdict.to_string(),
        severity: if claim_status == "blocked_machine_failures" {
            "high"
        } else {
            "medium"
        }
        .to_string(),
        confidence: "high".to_string(),
        stage: "adapter".to_string(),
        source: "adapter".to_string(),
        message: format!("WCAG 2.0 AA conformance-level claim scaffold status: {claim_status}."),
        evidence: vec![A11yVerifierEvidence {
            selector: None,
            values: vec![
                ("status".to_string(), claim_status.to_string()),
                (
                    "machine_blocker_count".to_string(),
                    claim_machine_blocker_count.to_string(),
                ),
                (
                    "coverage_gap_count".to_string(),
                    claim_coverage_gap_count.to_string(),
                ),
            ],
        }],
    };
    all_findings.push(claim_finding.clone());

    let (reported_findings, observability_summary) = a11y_dedup_and_correlate_findings(all_findings);

    let findings_py = PyList::empty_bound(py);
    let mut pass_count = 0usize;
    let mut fail_count = 0usize;
    let mut warn_count = 0usize;
    let mut manual_needed_count = 0usize;
    let mut not_applicable_count = 0usize;
    let mut failed_rule_ids: Vec<String> = Vec::new();
    for finding in &reported_findings {
        findings_py.append(a11y_core_finding_to_py(py, finding)?)?;
        match finding.verdict.as_str() {
            "pass" => pass_count += 1,
            "fail" => {
                fail_count += 1;
                failed_rule_ids.push(finding.rule_id.clone());
            }
            "warn" => warn_count += 1,
            "manual_needed" => manual_needed_count += 1,
            "not_applicable" => not_applicable_count += 1,
            _ => {}
        }
    }

    let error_count = if mode_norm == "error" { fail_count } else { 0 };
    let gate_warn_count = if mode_norm == "off" {
        0
    } else if mode_norm == "warn" {
        fail_count + warn_count
    } else {
        warn_count
    };

    let manual_required = manual_needed_count > 0;
    let conformance_status_text = if fail_count > 0 {
        "fail_machine_subset"
    } else if manual_required {
        "manual_review_required"
    } else {
        "pass_machine_subset"
    };

    let report = PyDict::new_bound(py);
    report.set_item("schema", "fullbleed.a11y.verify.v1")?;

    let target = PyDict::new_bound(py);
    target.set_item("html_path", html_path)?;
    target.set_item("css_path", css_path)?;
    target.set_item("target_hash", format!("sha256:{}", sha256_file_hex(html_path)?))?;
    report.set_item("target", target)?;

    report.set_item("profile", core.profile.clone())?;

    let conformance_status = PyDict::new_bound(py);
    conformance_status.set_item("status", conformance_status_text)?;
    conformance_status.set_item(
        "claim_scope",
        if manual_required {
            "manual_required"
        } else {
            "machine_subset"
        },
    )?;
    conformance_status.set_item("manual_review_required", manual_required)?;
    report.set_item("conformance_status", conformance_status)?;

    let gate = PyDict::new_bound(py);
    gate.set_item("ok", error_count == 0)?;
    gate.set_item("mode", mode_norm.clone())?;
    gate.set_item("error_count", error_count)?;
    gate.set_item("warn_count", gate_warn_count)?;
    let failed_list = PyList::empty_bound(py);
    for rid in &failed_rule_ids {
        failed_list.append(rid.clone())?;
    }
    gate.set_item("failed_rule_ids", failed_list)?;
    report.set_item("gate", gate)?;

    let summary = PyDict::new_bound(py);
    summary.set_item("pass_count", pass_count)?;
    summary.set_item("fail_count", fail_count)?;
    summary.set_item("warn_count", warn_count)?;
    summary.set_item("manual_needed_count", manual_needed_count)?;
    summary.set_item("not_applicable_count", not_applicable_count)?;
    report.set_item("summary", summary)?;

    report.set_item("findings", &findings_py)?;

    let observability = PyDict::new_bound(py);
    observability.set_item("original_finding_count", observability_summary.original_finding_count)?;
    observability.set_item("reported_finding_count", observability_summary.reported_finding_count)?;
    observability.set_item("dedup_event_count", observability_summary.dedup_event_count)?;
    observability.set_item(
        "dedup_merged_finding_count",
        observability_summary.dedup_merged_finding_count,
    )?;
    observability.set_item(
        "correlated_finding_count",
        observability_summary.correlated_finding_count,
    )?;
    let stage_counts_py = PyDict::new_bound(py);
    for (k, v) in &observability_summary.stage_counts {
        stage_counts_py.set_item(k, *v)?;
    }
    observability.set_item("stage_counts", stage_counts_py)?;
    let source_counts_py = PyDict::new_bound(py);
    for (k, v) in &observability_summary.source_counts {
        source_counts_py.set_item(k, *v)?;
    }
    observability.set_item("source_counts", source_counts_py)?;
    let original_stage_counts_py = PyDict::new_bound(py);
    for (k, v) in &observability_summary.original_stage_counts {
        original_stage_counts_py.set_item(k, *v)?;
    }
    observability.set_item("original_stage_counts", original_stage_counts_py)?;
    let original_source_counts_py = PyDict::new_bound(py);
    for (k, v) in &observability_summary.original_source_counts {
        original_source_counts_py.set_item(k, *v)?;
    }
    observability.set_item("original_source_counts", original_source_counts_py)?;
    let correlation_index_py = PyList::empty_bound(py);
    for item in &observability_summary.correlation_index {
        let row = PyDict::new_bound(py);
        row.set_item("rule_id", item.rule_id.clone())?;
        row.set_item("canonical_stage", item.canonical_stage.clone())?;
        row.set_item("canonical_source", item.canonical_source.clone())?;
        row.set_item("canonical_verdict", item.canonical_verdict.clone())?;
        row.set_item("merged_finding_count", item.merged_finding_count)?;
        row.set_item("merged_pre_render_count", item.merged_pre_render_count)?;
        let stage_map = PyDict::new_bound(py);
        for (k, v) in &item.merged_stage_counts {
            stage_map.set_item(k, *v)?;
        }
        row.set_item("merged_stage_counts", stage_map)?;
        let source_map = PyDict::new_bound(py);
        for (k, v) in &item.merged_source_counts {
            source_map.set_item(k, *v)?;
        }
        row.set_item("merged_source_counts", source_map)?;
        correlation_index_py.append(row)?;
    }
    observability.set_item("correlation_index", correlation_index_py)?;
    report.set_item("observability", observability)?;

    let coverage = PyDict::new_bound(py);
    coverage.set_item("evaluated_rule_count", reported_findings.len())?;
    coverage.set_item(
        "applicable_rule_count",
        reported_findings
            .iter()
            .filter(|f| f.applicability == "applicable")
            .count(),
    )?;
    coverage.set_item(
        "machine_rule_count",
        reported_findings
            .iter()
            .filter(|f| f.verification_mode == "machine")
            .count(),
    )?;
    coverage.set_item(
        "manual_rule_count",
        reported_findings
            .iter()
            .filter(|f| f.verification_mode == "manual")
            .count(),
    )?;
    coverage.set_item("manual_needed_count", manual_needed_count)?;
    coverage.set_item("not_evaluated_rule_count", 0)?;
    let rule_pack_coverage = PyList::empty_bound(py);
    let engine_pack = PyDict::new_bound(py);
    engine_pack.set_item("pack_id", "fullbleed.a11y.engine_core.v1")?;
    engine_pack.set_item("evaluated", reported_findings.len())?;
    engine_pack.set_item("total", reported_findings.len())?;
    rule_pack_coverage.append(engine_pack)?;

    let wcag_pairs: Vec<(&str, &str)> = reported_findings
        .iter()
        .map(|f| (f.rule_id.as_str(), f.verdict.as_str()))
        .collect();
    let wcag_summary = audit_contract::wcag20aa_coverage_from_rule_verdicts(wcag_pairs);
    let section508_summary = audit_contract::section508_html_coverage_from_rule_verdicts(
        reported_findings
            .iter()
            .map(|f| (f.rule_id.as_str(), f.verdict.as_str())),
    );
    let wcag_cov = wcag20aa_coverage_summary_to_py(py, &wcag_summary)?;
    coverage.set_item("wcag20aa", &wcag_cov)?;
    let section508_cov = section508_html_coverage_summary_to_py(py, &section508_summary)?;
    coverage.set_item("section508", &section508_cov)?;
    let wcag_pack = PyDict::new_bound(py);
    wcag_pack.set_item("pack_id", "wcag20aa.implemented_map.v1")?;
    wcag_pack.set_item(
        "evaluated",
        wcag_summary.implemented_mapped_entry_evaluated_count,
    )?;
    wcag_pack.set_item("total", wcag_summary.implemented_mapped_entry_count)?;
    rule_pack_coverage.append(wcag_pack)?;
    let s508_pack = PyDict::new_bound(py);
    s508_pack.set_item("pack_id", "section508_html.implemented_map.v1")?;
    s508_pack.set_item(
        "evaluated",
        section508_summary.implemented_mapped_entry_evaluated_count,
    )?;
    s508_pack.set_item("total", section508_summary.implemented_mapped_entry_count)?;
    rule_pack_coverage.append(s508_pack)?;
    coverage.set_item("rule_pack_coverage", rule_pack_coverage)?;
    report.set_item("coverage", coverage)?;

    let wcag_claim = PyDict::new_bound(py);
    wcag_claim.set_item("target", "wcag20aa")?;
    wcag_claim.set_item("status", claim_status)?;
    wcag_claim.set_item("claim_ready", false)?;
    wcag_claim.set_item("manual_review_required", true)?;
    wcag_claim.set_item("manual_review_debt_count", 0usize)?;
    wcag_claim.set_item("machine_blocker_count", claim_machine_blocker_count)?;
    wcag_claim.set_item("coverage_gap_count", claim_coverage_gap_count)?;
    wcag_claim.set_item(
        "implemented_mapped_entry_count",
        wcag_summary.implemented_mapped_entry_count,
    )?;
    wcag_claim.set_item(
        "implemented_mapped_entry_evaluated_count",
        wcag_summary.implemented_mapped_entry_evaluated_count,
    )?;
    wcag_claim.set_item(
        "implemented_mapped_entry_pending_count",
        wcag_summary.implemented_mapped_entry_pending_count,
    )?;
    wcag_claim.set_item("unmapped_entry_count", wcag_summary.unmapped_entry_count)?;
    let wcag_claim_notes = PyList::empty_bound(py);
    if wcag_summary.unmapped_entry_count > 0 {
        wcag_claim_notes.append("WCAG target registry still contains unmapped entries.")?;
    }
    if wcag_summary.implemented_mapped_entry_pending_count > 0 {
        wcag_claim_notes.append(
            "Implemented mapped WCAG entries remain unevaluated in this report.",
        )?;
    }
    wcag_claim_notes.append(
        "Manual claim evidence is required for WCAG conformance assertions.",
    )?;
    wcag_claim.set_item("notes", wcag_claim_notes)?;
    report.set_item("wcag20aa_claim_readiness", wcag_claim)?;

    let tooling = PyDict::new_bound(py);
    tooling.set_item("fullbleed_version", env!("CARGO_PKG_VERSION"))?;
    tooling.set_item("engine_version", env!("CARGO_PKG_VERSION"))?;
    tooling.set_item("report_schema_version", "1.0.0-draft")?;
    let contract_meta = audit_contract::metadata();
    tooling.set_item("audit_contract_id", contract_meta.contract_id)?;
    tooling.set_item("audit_contract_version", contract_meta.contract_version)?;
    tooling.set_item(
        "audit_contract_fingerprint",
        format!("sha256:{}", contract_meta.contract_fingerprint_sha256),
    )?;
    tooling.set_item(
        "audit_registry_hash",
        format!("sha256:{}", contract_meta.audit_registry_hash_sha256),
    )?;
    tooling.set_item(
        "wcag20aa_registry_hash",
        format!("sha256:{}", contract_meta.wcag20aa_registry_hash_sha256),
    )?;
    tooling.set_item(
        "section508_html_registry_hash",
        format!("sha256:{}", contract_meta.section508_html_registry_hash_sha256),
    )?;
    let dt = py.import_bound("datetime")?;
    let tz = dt.getattr("timezone")?.getattr("utc")?;
    let now = dt.getattr("datetime")?.call_method1("now", (tz,))?;
    let iso = now.call_method0("isoformat")?.extract::<String>()?;
    tooling.set_item("generated_at", iso.replace("+00:00", "Z"))?;
    report.set_item("tooling", tooling)?;

    let artifacts = PyDict::new_bound(py);
    artifacts.set_item("html_hash", format!("sha256:{}", sha256_file_hex(html_path)?))?;
    artifacts.set_item("css_hash", format!("sha256:{}", sha256_file_hex(css_path)?))?;
    artifacts.set_item("css_linked", core.facts.has_css_link)?;
    artifacts.set_item(
        "packaging_mode",
        if core.facts.has_css_link { "linked-css" } else { "separate-files" },
    )?;
    report.set_item("artifacts", artifacts)?;

    Ok(report.to_object(py))
}

fn pmr_core_evidence_to_py(py: Python<'_>, evidence: &PmrCoreEvidence) -> PyResult<PyObject> {
    let out = PyDict::new_bound(py);
    if let Some(selector) = &evidence.selector {
        out.set_item("selector", selector.clone())?;
    }
    if let Some(diagnostic_ref) = &evidence.diagnostic_ref {
        out.set_item("diagnostic_ref", diagnostic_ref.clone())?;
    }
    if !evidence.values.is_empty() {
        let values = PyDict::new_bound(py);
        for (k, v) in &evidence.values {
            values.set_item(k.clone(), v.clone())?;
        }
        out.set_item("values", values)?;
    }
    Ok(out.to_object(py))
}

fn pmr_core_audit_to_py(py: Python<'_>, audit: &PmrCoreAudit) -> PyResult<PyObject> {
    let out = PyDict::new_bound(py);
    out.set_item("audit_id", audit.audit_id.clone())?;
    out.set_item("category", audit.category.clone())?;
    out.set_item("weight", audit.weight)?;
    out.set_item("class", audit.class_name.clone())?;
    out.set_item("verification_mode", audit.verification_mode.clone())?;
    out.set_item("severity", audit.severity.clone())?;
    out.set_item("stage", audit.stage.clone())?;
    out.set_item("source", audit.source.clone())?;
    out.set_item("verdict", audit.verdict.clone())?;
    out.set_item("scored", audit.scored)?;
    if let Some(score) = audit.score {
        out.set_item("score", score)?;
    }
    out.set_item("message", audit.message.clone())?;
    if let Some(fix_hint) = &audit.fix_hint {
        out.set_item("fix_hint", fix_hint.clone())?;
    }
    if !audit.evidence.is_empty() {
        let evid = PyList::empty_bound(py);
        for e in &audit.evidence {
            evid.append(pmr_core_evidence_to_py(py, e)?)?;
        }
        out.set_item("evidence", evid)?;
    }
    Ok(out.to_object(py))
}

fn build_pmr_report_py(
    py: Python<'_>,
    core: &PmrCoreReport,
    html_path: &str,
    css_path: &str,
) -> PyResult<PyObject> {
    let report = PyDict::new_bound(py);
    report.set_item("schema", "fullbleed.pmr.v1")?;

    let target = PyDict::new_bound(py);
    target.set_item("html_path", html_path)?;
    target.set_item("css_path", css_path)?;
    report.set_item("target", target)?;

    report.set_item("profile", core.profile.clone())?;

    let rank = PyDict::new_bound(py);
    rank.set_item("score", core.rank.score)?;
    rank.set_item("confidence", core.rank.confidence)?;
    rank.set_item("band", core.rank.band.clone())?;
    rank.set_item("raw_score", core.rank.raw_score)?;
    report.set_item("rank", rank)?;

    let gate = PyDict::new_bound(py);
    gate.set_item("ok", core.gate.ok)?;
    gate.set_item("mode", core.gate.mode.clone())?;
    gate.set_item("error_count", core.gate.error_count)?;
    gate.set_item("warn_count", core.gate.warn_count)?;
    let failed = PyList::empty_bound(py);
    for aid in &core.gate.failed_audit_ids {
        failed.append(aid.clone())?;
    }
    gate.set_item("failed_audit_ids", failed)?;
    report.set_item("gate", gate)?;

    let categories = PyList::empty_bound(py);
    for cat in &core.categories {
        let d = PyDict::new_bound(py);
        d.set_item("id", cat.id.clone())?;
        d.set_item("name", cat.name.clone())?;
        d.set_item("weight", cat.weight)?;
        d.set_item("score", cat.score)?;
        d.set_item("confidence", cat.confidence)?;
        d.set_item("audit_count", cat.audit_count)?;
        d.set_item("fail_count", cat.fail_count)?;
        d.set_item("warn_count", cat.warn_count)?;
        categories.append(d)?;
    }
    report.set_item("categories", categories)?;

    let audits = PyList::empty_bound(py);
    let failed_audit_ids: std::collections::BTreeSet<String> =
        core.gate.failed_audit_ids.iter().cloned().collect();
    let mut stage_counts: std::collections::BTreeMap<String, usize> =
        std::collections::BTreeMap::new();
    let mut source_counts: std::collections::BTreeMap<String, usize> =
        std::collections::BTreeMap::new();
    let mut category_counts: std::collections::BTreeMap<String, usize> =
        std::collections::BTreeMap::new();
    let mut class_counts: std::collections::BTreeMap<String, usize> =
        std::collections::BTreeMap::new();
    let mut verdict_counts: std::collections::BTreeMap<String, usize> =
        std::collections::BTreeMap::new();
    let correlation_index = PyList::empty_bound(py);
    for audit in &core.audits {
        audits.append(pmr_core_audit_to_py(py, audit)?)?;
        *stage_counts.entry(audit.stage.clone()).or_insert(0) += 1;
        *source_counts.entry(audit.source.clone()).or_insert(0) += 1;
        *category_counts.entry(audit.category.clone()).or_insert(0) += 1;
        *class_counts.entry(audit.class_name.clone()).or_insert(0) += 1;
        *verdict_counts.entry(audit.verdict.clone()).or_insert(0) += 1;
        let include_corr = matches!(audit.verdict.as_str(), "fail" | "warn" | "manual_needed")
            || audit.fix_hint.is_some();
        if include_corr {
            let row = PyDict::new_bound(py);
            row.set_item("audit_id", audit.audit_id.clone())?;
            row.set_item("category", audit.category.clone())?;
            row.set_item("class", audit.class_name.clone())?;
            row.set_item("verdict", audit.verdict.clone())?;
            row.set_item("severity", audit.severity.clone())?;
            row.set_item("stage", audit.stage.clone())?;
            row.set_item("source", audit.source.clone())?;
            row.set_item("gate_failed", failed_audit_ids.contains(&audit.audit_id))?;
            row.set_item(
                "gate_relevant",
                matches!(audit.verdict.as_str(), "fail" | "warn"),
            )?;
            row.set_item(
                "opportunity",
                audit.fix_hint.is_some() || audit.class_name == "opportunity",
            )?;
            row.set_item("scored", audit.scored)?;
            row.set_item("has_fix_hint", audit.fix_hint.is_some())?;
            if let Some(score) = audit.score {
                row.set_item("score", score)?;
            }
            correlation_index.append(row)?;
        }
    }
    report.set_item("audits", audits)?;

    let observability = PyDict::new_bound(py);
    observability.set_item("original_audit_count", core.audits.len())?;
    observability.set_item("reported_audit_count", core.audits.len())?;
    observability.set_item("dedup_event_count", 0usize)?;
    observability.set_item("dedup_merged_audit_count", 0usize)?;
    observability.set_item("correlated_audit_count", correlation_index.len())?;
    let stage_counts_py = PyDict::new_bound(py);
    for (k, v) in &stage_counts {
        stage_counts_py.set_item(k, *v)?;
    }
    observability.set_item("stage_counts", stage_counts_py)?;
    let source_counts_py = PyDict::new_bound(py);
    for (k, v) in &source_counts {
        source_counts_py.set_item(k, *v)?;
    }
    observability.set_item("source_counts", source_counts_py)?;
    let category_counts_py = PyDict::new_bound(py);
    for (k, v) in &category_counts {
        category_counts_py.set_item(k, *v)?;
    }
    observability.set_item("category_counts", category_counts_py)?;
    let class_counts_py = PyDict::new_bound(py);
    for (k, v) in &class_counts {
        class_counts_py.set_item(k, *v)?;
    }
    observability.set_item("class_counts", class_counts_py)?;
    let verdict_counts_py = PyDict::new_bound(py);
    for (k, v) in &verdict_counts {
        verdict_counts_py.set_item(k, *v)?;
    }
    observability.set_item("verdict_counts", verdict_counts_py)?;
    observability.set_item("correlation_index", correlation_index)?;
    report.set_item("observability", observability)?;

    let manual_debt = PyDict::new_bound(py);
    manual_debt.set_item("item_count", core.manual_debt_item_count)?;
    manual_debt.set_item("high_risk_count", core.manual_debt_high_risk_count)?;
    if !core.manual_debt_items.is_empty() {
        let items = PyList::empty_bound(py);
        for item in &core.manual_debt_items {
            let d = PyDict::new_bound(py);
            d.set_item("id", item.id.clone())?;
            d.set_item("reason", item.reason.clone())?;
            d.set_item("severity", item.severity.clone())?;
            if let Some(category) = &item.category {
                d.set_item("category", category.clone())?;
            }
            items.append(d)?;
        }
        manual_debt.set_item("items", items)?;
    }
    report.set_item("manual_debt", manual_debt)?;

    let coverage = PyDict::new_bound(py);
    coverage.set_item("evaluated_audit_count", core.coverage.evaluated_audit_count)?;
    coverage.set_item("applicable_audit_count", core.coverage.applicable_audit_count)?;
    coverage.set_item("scored_audit_count", core.coverage.scored_audit_count)?;
    coverage.set_item("manual_needed_count", core.coverage.manual_needed_count)?;
    coverage.set_item(
        "not_evaluated_audit_count",
        core.coverage.not_evaluated_audit_count,
    )?;
    report.set_item("coverage", coverage)?;

    let tooling = PyDict::new_bound(py);
    tooling.set_item("fullbleed_version", env!("CARGO_PKG_VERSION"))?;
    tooling.set_item("report_schema_version", "1.0.0-draft")?;
    let contract_meta = audit_contract::metadata();
    tooling.set_item("audit_contract_id", contract_meta.contract_id)?;
    tooling.set_item("audit_contract_version", contract_meta.contract_version)?;
    tooling.set_item(
        "audit_contract_fingerprint",
        format!("sha256:{}", contract_meta.contract_fingerprint_sha256),
    )?;
    tooling.set_item(
        "audit_registry_hash",
        format!("sha256:{}", contract_meta.audit_registry_hash_sha256),
    )?;
    tooling.set_item(
        "wcag20aa_registry_hash",
        format!("sha256:{}", contract_meta.wcag20aa_registry_hash_sha256),
    )?;
    tooling.set_item(
        "section508_html_registry_hash",
        format!("sha256:{}", contract_meta.section508_html_registry_hash_sha256),
    )?;
    let dt = py.import_bound("datetime")?;
    let tz = dt.getattr("timezone")?.getattr("utc")?;
    let now = dt.getattr("datetime")?.call_method1("now", (tz,))?;
    let iso = now.call_method0("isoformat")?.extract::<String>()?;
    tooling.set_item("generated_at", iso.replace("+00:00", "Z"))?;
    report.set_item("tooling", tooling)?;

    let artifacts = PyDict::new_bound(py);
    artifacts.set_item("html_hash", format!("sha256:{}", sha256_file_hex(html_path)?))?;
    artifacts.set_item("css_hash", format!("sha256:{}", sha256_file_hex(css_path)?))?;
    artifacts.set_item("css_linked", core.facts.has_css_link)?;
    artifacts.set_item(
        "packaging_mode",
        if core.facts.has_css_link {
            "linked-css"
        } else {
            "separate-files"
        },
    )?;
    report.set_item("artifacts", artifacts)?;

    Ok(report.to_object(py))
}

#[pymethods]
impl PdfEngine {
    #[new]
    #[pyo3(
        signature = (
            page_width=None,
            page_height=None,
            margin=None,
            page_margins=None,
            font_dirs=None,
            font_files=None,
            reuse_xobjects=true,
            svg_form_xobjects=false,
            svg_raster_fallback=false,
            unicode_support=true,
            shape_text=true,
            unicode_metrics=true,
            pdf_version=None,
            pdf_profile=None,
            output_intent_icc=None,
            output_intent_identifier=None,
            output_intent_info=None,
            output_intent_components=None,
            color_space=None,
            document_lang=None,
            document_title=None,
            header_first=None,
            header_each=None,
            header_last=None,
            header_x=None,
            header_y_from_top=None,
            header_font_name=None,
            header_font_size=None,
            header_color=None,
            header_html_first=None,
            header_html_each=None,
            header_html_last=None,
            header_html_x=None,
            header_html_y_from_top=None,
            header_html_width=None,
            header_html_height=None,
            footer_first=None,
            footer_each=None,
            footer_last=None,
            footer_x=None,
            footer_y_from_bottom=None,
            footer_font_name=None,
            footer_font_size=None,
            footer_color=None,
            watermark=None,
            watermark_text=None,
            watermark_html=None,
            watermark_image=None,
            watermark_layer="overlay",
            watermark_semantics="artifact",
            watermark_opacity=0.15,
            watermark_rotation=0.0,
            watermark_font_name=None,
            watermark_font_size=None,
            watermark_color=None,
            paginated_context=None,
            template_binding=None,
            layout_strategy=None,
            accept_lazy_layout_cost=false,
            lazy_max_passes=4,
            lazy_budget_ms=50.0,
            jit_mode=None,
            debug=false,
            debug_out=None,
            perf=false,
            perf_out=None
        )
    )]
    fn new(
        page_width: Option<&Bound<'_, PyAny>>,
        page_height: Option<&Bound<'_, PyAny>>,
        margin: Option<&Bound<'_, PyAny>>,
        page_margins: Option<&Bound<'_, PyAny>>,
        font_dirs: Option<Vec<String>>,
        font_files: Option<Vec<String>>,
        reuse_xobjects: bool,
        svg_form_xobjects: bool,
        svg_raster_fallback: bool,
        unicode_support: bool,
        shape_text: bool,
        unicode_metrics: bool,
        pdf_version: Option<&Bound<'_, PyAny>>,
        pdf_profile: Option<&Bound<'_, PyAny>>,
        output_intent_icc: Option<String>,
        output_intent_identifier: Option<String>,
        output_intent_info: Option<String>,
        output_intent_components: Option<u8>,
        color_space: Option<String>,
        document_lang: Option<String>,
        document_title: Option<String>,
        header_first: Option<String>,
        header_each: Option<String>,
        header_last: Option<String>,
        header_x: Option<&Bound<'_, PyAny>>,
        header_y_from_top: Option<&Bound<'_, PyAny>>,
        header_font_name: Option<String>,
        header_font_size: Option<f32>,
        header_color: Option<String>,
        header_html_first: Option<String>,
        header_html_each: Option<String>,
        header_html_last: Option<String>,
        header_html_x: Option<&Bound<'_, PyAny>>,
        header_html_y_from_top: Option<&Bound<'_, PyAny>>,
        header_html_width: Option<&Bound<'_, PyAny>>,
        header_html_height: Option<&Bound<'_, PyAny>>,
        footer_first: Option<String>,
        footer_each: Option<String>,
        footer_last: Option<String>,
        footer_x: Option<&Bound<'_, PyAny>>,
        footer_y_from_bottom: Option<&Bound<'_, PyAny>>,
        footer_font_name: Option<String>,
        footer_font_size: Option<f32>,
        footer_color: Option<String>,
        watermark: Option<PyWatermarkSpec>,
        watermark_text: Option<String>,
        watermark_html: Option<String>,
        watermark_image: Option<String>,
        watermark_layer: &str,
        watermark_semantics: &str,
        watermark_opacity: f32,
        watermark_rotation: f32,
        watermark_font_name: Option<String>,
        watermark_font_size: Option<f32>,
        watermark_color: Option<String>,
        paginated_context: Option<HashMap<String, String>>,
        template_binding: Option<&Bound<'_, PyAny>>,
        layout_strategy: Option<String>,
        accept_lazy_layout_cost: bool,
        lazy_max_passes: usize,
        lazy_budget_ms: f64,
        jit_mode: Option<String>,
        debug: bool,
        debug_out: Option<String>,
        perf: bool,
        perf_out: Option<String>,
    ) -> PyResult<Self> {
        let mut builder = FullBleed::builder();
        let page_width = parse_py_length(page_width)?;
        let page_height = parse_py_length(page_height)?;
        if let (Some(width), Some(height)) = (page_width, page_height) {
            builder = builder.page_size(Size {
                width: Pt::from_f32(width),
                height: Pt::from_f32(height),
            });
        }
        if let Some(margin) = margin {
            if let Some(m) = parse_py_margins(Some(margin))? {
                builder = builder.margins(m);
            }
        }

        if let Some(pm) = page_margins {
            let dict = pm.downcast::<PyDict>().map_err(|_| {
                PyValueError::new_err("page_margins must be a dict like {1: {'top':'30mm',...}, 2: 24, 'n': {'top':'20mm',...}}")
            })?;

            // Determine numeric pages so we can map "n"/"each" to the last template index.
            let mut max_numeric = 1usize;
            let mut has_n = false;
            for (k, _v) in dict.iter() {
                if let Ok(i) = k.extract::<usize>() {
                    max_numeric = max_numeric.max(i.max(1));
                    continue;
                }
                if let Ok(s) = k.extract::<String>() {
                    let s = s.trim().to_ascii_lowercase();
                    if s == "n" || s == "each" {
                        has_n = true;
                        continue;
                    }
                    if let Ok(i) = s.parse::<usize>() {
                        max_numeric = max_numeric.max(i.max(1));
                        continue;
                    }
                }
            }
            let n_page = max_numeric.saturating_add(1);

            for (k, v) in dict.iter() {
                let page_number = if let Ok(i) = k.extract::<usize>() {
                    i.max(1)
                } else if let Ok(s) = k.extract::<String>() {
                    let s = s.trim().to_ascii_lowercase();
                    if s == "n" || s == "each" {
                        n_page
                    } else {
                        s.parse::<usize>().map_err(|_| {
                            PyValueError::new_err(format!(
                                "page_margins key must be an int page number or 'n'/'each', got {s:?}"
                            ))
                        })?.max(1)
                    }
                } else {
                    return Err(PyValueError::new_err(
                        "page_margins keys must be integers (1, 2, ...) or strings ('n'/'each')",
                    ));
                };

                let margins = parse_py_margins(Some(&v))?.ok_or_else(|| {
                    PyValueError::new_err("page_margins values must be a number or dict")
                })?;
                builder = builder.page_margin(page_number, margins);
            }

            let _ = has_n;
        }
        builder = builder.reuse_xobjects(reuse_xobjects);
        builder = builder.svg_form_xobjects(svg_form_xobjects);
        builder = builder.svg_raster_fallback(svg_raster_fallback);
        builder = builder.unicode_support(unicode_support);
        builder = builder.shape_text(shape_text);
        builder = builder.unicode_metrics(unicode_metrics);
        if let Some(version) = parse_pdf_version(pdf_version)? {
            builder = builder.pdf_version(version);
        }
        if let Some(profile) = parse_pdf_profile(pdf_profile)? {
            builder = builder.pdf_profile(profile);
        }
        if let Some(intent) = parse_output_intent(
            output_intent_icc,
            output_intent_identifier,
            output_intent_info,
            output_intent_components,
        )? {
            builder = builder.output_intent(intent);
        }
        if let Some(color_space) = color_space {
            builder = builder.color_space(parse_color_space(&color_space)?);
        }
        if let Some(lang) = document_lang {
            builder = builder.document_lang(lang);
        }
        if let Some(title) = document_title {
            builder = builder.document_title(title);
        }

        // Prefer HTML header if provided; otherwise fall back to plain text header.
        if header_html_first.is_some() || header_html_each.is_some() || header_html_last.is_some() {
            let hx = parse_py_length(header_html_x)?.unwrap_or(36.0);
            let hy = parse_py_length(header_html_y_from_top)?.unwrap_or(18.0);
            let hw = parse_py_length(header_html_width)?.unwrap_or(540.0);
            let hh = parse_py_length(header_html_height)?.unwrap_or(42.0);
            builder = builder.page_header_html(
                header_html_first,
                header_html_each,
                header_html_last,
                hx,
                hy,
                hw,
                hh,
            );
        } else {
            let header_x = parse_py_length(header_x)?.unwrap_or(36.0);
            let header_y_from_top = parse_py_length(header_y_from_top)?.unwrap_or(18.0);
            let header_font_name = header_font_name.unwrap_or_else(|| "Helvetica".to_string());
            let header_font_size = header_font_size.unwrap_or(9.0);
            let header_color = header_color
                .as_deref()
                .and_then(parse_color_hex)
                .unwrap_or(Color {
                    r: 0.333,
                    g: 0.333,
                    b: 0.333,
                });

            if header_first.is_some() || header_each.is_some() || header_last.is_some() {
                builder = builder.page_header(
                    header_first,
                    header_each,
                    header_last,
                    header_x,
                    header_y_from_top,
                    header_font_name,
                    header_font_size,
                    header_color,
                );
            }
        }

        let footer_x = parse_py_length(footer_x)?.unwrap_or(36.0);
        let footer_y_from_bottom = parse_py_length(footer_y_from_bottom)?.unwrap_or(24.0);
        let footer_font_name = footer_font_name.unwrap_or_else(|| "Helvetica".to_string());
        let footer_font_size = footer_font_size.unwrap_or(9.0);
        let footer_color = footer_color
            .as_deref()
            .and_then(parse_color_hex)
            .unwrap_or(Color {
                r: 0.333,
                g: 0.333,
                b: 0.333,
            });

        if footer_first.is_some() || footer_each.is_some() || footer_last.is_some() {
            builder = builder.page_footer(
                footer_first,
                footer_each,
                footer_last,
                footer_x,
                footer_y_from_bottom,
                footer_font_name,
                footer_font_size,
                footer_color,
            );
        }

        let parsed_watermark_semantics = parse_watermark_semantics(watermark_semantics)?;
        let mut watermark_spec: Option<WatermarkSpec> = None;
        if let Some(spec) = watermark {
            watermark_spec = Some(watermark_spec_from_py(&spec)?);
        } else if watermark_text.is_some() || watermark_html.is_some() || watermark_image.is_some()
        {
            let layer = parse_watermark_layer(&watermark_layer)?;
            let mut spec = if let Some(text) = watermark_text {
                WatermarkSpec::text(text)
            } else if let Some(html) = watermark_html {
                WatermarkSpec::html(html)
            } else if let Some(image) = watermark_image {
                WatermarkSpec::image(image)
            } else {
                WatermarkSpec::text(String::new())
            };
            spec.layer = layer;
            spec.semantics = parsed_watermark_semantics;
            spec.opacity = watermark_opacity.clamp(0.0, 1.0);
            spec.rotation_deg = watermark_rotation;
            if let Some(font_name) = watermark_font_name {
                spec.font_name = font_name;
            }
            if let Some(font_size) = watermark_font_size {
                spec.font_size = Pt::from_f32(font_size);
            }
            if let Some(color_str) = watermark_color {
                if let Some(color) = parse_color_hex(&color_str) {
                    spec.color = color;
                } else {
                    return Err(PyValueError::new_err(format!(
                        "Invalid watermark_color: {color_str:?}. Expected '#RRGGBB'."
                    )));
                }
            }
            watermark_spec = Some(spec);
        }
        if let Some(spec) = watermark_spec {
            builder = builder.watermark(spec);
        }

        if let Some(spec_map) = paginated_context {
            let mut ops = HashMap::new();
            for (key, op_raw) in spec_map {
                let Some(op) = crate::PaginatedContextSpec::parse_op(&op_raw) else {
                    return Err(PyValueError::new_err(format!(
                        "Invalid paginated_context op for key {key:?}: {op_raw:?}. Expected one of: 'every', 'count', 'sum', 'sum:<scale>'"
                    )));
                };
                ops.insert(key, op);
            }
            builder = builder.paginated_context(crate::PaginatedContextSpec::new(ops));
        }
        if let Some(raw) = template_binding {
            let spec = parse_template_binding_spec(raw)?;
            builder = builder.template_binding_spec(spec);
        }
        if let Some(strategy_raw) = layout_strategy {
            let strategy = parse_layout_strategy(&strategy_raw)?;
            builder = builder.layout_strategy(strategy);
        }
        builder = builder.accept_lazy_layout_cost(accept_lazy_layout_cost);
        builder = builder.lazy_layout_limits(lazy_max_passes, lazy_budget_ms);
        if let Some(mode) = jit_mode {
            let mode = mode.trim().to_ascii_lowercase();
            let jit_mode = match mode.as_str() {
                "off" => JitMode::Off,
                "plan" | "plan-only" | "plan_only" => JitMode::PlanOnly,
                "replay" | "plan-and-replay" | "plan_and_replay" => JitMode::PlanAndReplay,
                _ => {
                    return Err(PyValueError::new_err(format!(
                        "invalid jit_mode '{mode}' (expected off, plan, replay)"
                    )));
                }
            };
            builder = builder.jit_mode(jit_mode);
        }
        if debug {
            let path = debug_out.unwrap_or_else(|| "fullbleed_jit.log".to_string());
            builder = builder.debug_log(path);
        }
        if perf || perf_out.is_some() {
            let path = perf_out.unwrap_or_else(|| "fullbleed_perf.log".to_string());
            builder = builder.perf_log(path);
        }
        if let Some(dirs) = font_dirs {
            for dir in dirs {
                builder = builder.register_font_dir(dir);
            }
        }
        if let Some(files) = font_files {
            for file in files {
                builder = builder.register_font_file(file);
            }
        }
        let engine = builder.clone().build().map_err(to_py_err)?;
        Ok(Self { engine, builder })
    }

    fn register_bundle(&mut self, bundle: PyRef<'_, PyAssetBundle>) -> PyResult<()> {
        self.builder = self.builder.clone().register_bundle(bundle.bundle.clone());
        self.rebuild_from_builder()
    }

    #[getter]
    fn document_lang(&self) -> Option<String> {
        self.builder
            .document_lang_value()
            .map(std::string::ToString::to_string)
    }

    #[setter(document_lang)]
    fn set_document_lang(&mut self, value: Option<String>) -> PyResult<()> {
        self.builder = match value {
            Some(lang) => self.builder.clone().document_lang(lang),
            None => self.builder.clone().clear_document_lang(),
        };
        self.rebuild_from_builder()
    }

    #[getter]
    fn document_title(&self) -> Option<String> {
        self.builder
            .document_title_value()
            .map(std::string::ToString::to_string)
    }

    #[setter(document_title)]
    fn set_document_title(&mut self, value: Option<String>) -> PyResult<()> {
        self.builder = match value {
            Some(title) => self.builder.clone().document_title(title),
            None => self.builder.clone().clear_document_title(),
        };
        self.rebuild_from_builder()
    }

    fn document_metadata(&self, py: Python<'_>) -> PyResult<PyObject> {
        let out = PyDict::new_bound(py);
        out.set_item("document_lang", self.document_lang())?;
        out.set_item("document_title", self.document_title())?;
        Ok(out.to_object(py))
    }

    #[pyo3(signature = (html, out_path, wrap_document=true))]
    fn emit_html(&self, html: &str, out_path: &str, wrap_document: bool) -> PyResult<String> {
        self.engine
            .emit_html_artifact(html, out_path, wrap_document)
            .map_err(to_py_err)
    }

    fn emit_css(&self, css: &str, out_path: &str) -> PyResult<String> {
        self.engine
            .emit_css_artifact(css, out_path)
            .map_err(to_py_err)
    }

    #[pyo3(signature = (html, css, html_path, css_path, wrap_document=true))]
    fn emit_artifacts(
        &self,
        py: Python<'_>,
        html: &str,
        css: &str,
        html_path: &str,
        css_path: &str,
        wrap_document: bool,
    ) -> PyResult<PyObject> {
        let (html_text, css_text) = self
            .engine
            .emit_html_css_artifacts(html, css, html_path, css_path, wrap_document)
            .map_err(to_py_err)?;
        let out = PyDict::new_bound(py);
        out.set_item("html_path", html_path)?;
        out.set_item("css_path", css_path)?;
        out.set_item("html", html_text)?;
        out.set_item("css", css_text)?;
        Ok(out.to_object(py))
    }

    #[pyo3(signature = (html, css="", profile="strict", mode="error", render_preview_png_path=None, a11y_report=None, claim_evidence=None))]
    fn verify_accessibility_html(
        &self,
        py: Python<'_>,
        html: &str,
        css: &str,
        profile: &str,
        mode: &str,
        render_preview_png_path: Option<&str>,
        a11y_report: Option<PyObject>,
        claim_evidence: Option<PyObject>,
    ) -> PyResult<PyObject> {
        let core = self.engine.verify_accessibility_html_core(html, profile);

        // For in-memory HTML verification we emit a schema-shaped report without artifact hashes/paths.
        // Reuse the file-based path with temporary files to keep the report shape stable for now.
        let tmp_dir = std::env::temp_dir();
        let pid = std::process::id();
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let html_path = tmp_dir.join(format!("fullbleed_a11y_verify_{pid}_{ts}.html"));
        let css_path = tmp_dir.join(format!("fullbleed_a11y_verify_{pid}_{ts}.css"));
        std::fs::write(&html_path, html)
            .map_err(|e| PyValueError::new_err(format!("failed to write temp html for verification: {e}")))?;
        std::fs::write(&css_path, css)
            .map_err(|e| PyValueError::new_err(format!("failed to write temp css for verification: {e}")))?;
        let report = build_a11y_verify_report_py(
            py,
            &core,
            &html_path.to_string_lossy(),
            &css_path.to_string_lossy(),
            mode,
            render_preview_png_path,
            a11y_report.as_ref().map(|v| v.bind(py)),
            claim_evidence.as_ref().map(|v| v.bind(py)),
        );
        let _ = std::fs::remove_file(&html_path);
        let _ = std::fs::remove_file(&css_path);
        report
    }

    #[pyo3(signature = (html_path, css_path, profile="strict", mode="error", render_preview_png_path=None, a11y_report=None, claim_evidence=None))]
    fn verify_accessibility_artifacts(
        &self,
        py: Python<'_>,
        html_path: &str,
        css_path: &str,
        profile: &str,
        mode: &str,
        render_preview_png_path: Option<&str>,
        a11y_report: Option<PyObject>,
        claim_evidence: Option<PyObject>,
    ) -> PyResult<PyObject> {
        let html = std::fs::read_to_string(html_path).map_err(|e| {
            PyValueError::new_err(format!("failed to read html artifact for accessibility verification: {e}"))
        })?;
        // Ensure CSS file is readable/existing even though current core checks are HTML-focused.
        let _css = std::fs::read_to_string(css_path).map_err(|e| {
            PyValueError::new_err(format!("failed to read css artifact for accessibility verification: {e}"))
        })?;
        let core = self.engine.verify_accessibility_html_core(&html, profile);
        build_a11y_verify_report_py(
            py,
            &core,
            html_path,
            css_path,
            mode,
            render_preview_png_path,
            a11y_report.as_ref().map(|v| v.bind(py)),
            claim_evidence.as_ref().map(|v| v.bind(py)),
        )
    }

    #[pyo3(signature = (
        html,
        css="",
        profile="strict",
        mode="error",
        overflow_count=None,
        known_loss_count=None,
        source_page_count=None,
        render_page_count=None,
        review_queue_items=None
    ))]
    fn verify_paged_media_rank_html(
        &self,
        py: Python<'_>,
        html: &str,
        css: &str,
        profile: &str,
        mode: &str,
        overflow_count: Option<i64>,
        known_loss_count: Option<i64>,
        source_page_count: Option<i64>,
        render_page_count: Option<i64>,
        review_queue_items: Option<i64>,
    ) -> PyResult<PyObject> {
        let tmp_dir = std::env::temp_dir();
        let pid = std::process::id();
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let html_path = tmp_dir.join(format!("fullbleed_pmr_{pid}_{ts}.html"));
        let css_path = tmp_dir.join(format!("fullbleed_pmr_{pid}_{ts}.css"));
        std::fs::write(&html_path, html)
            .map_err(|e| PyValueError::new_err(format!("failed to write temp html for PMR verification: {e}")))?;
        std::fs::write(&css_path, css)
            .map_err(|e| PyValueError::new_err(format!("failed to write temp css for PMR verification: {e}")))?;
        let ctx = PmrCoreContext {
            overflow_count,
            known_loss_count,
            source_page_count,
            render_page_count,
            review_queue_items,
            html_artifact_bytes: Some(html.len() as u64),
            css_artifact_bytes: Some(css.len() as u64),
        };
        let core = self
            .engine
            .verify_paged_media_rank_html_core(html, profile, mode, &ctx);
        let report = build_pmr_report_py(
            py,
            &core,
            &html_path.to_string_lossy(),
            &css_path.to_string_lossy(),
        );
        let _ = std::fs::remove_file(&html_path);
        let _ = std::fs::remove_file(&css_path);
        report
    }

    #[pyo3(signature = (
        html_path,
        css_path,
        profile="strict",
        mode="error",
        overflow_count=None,
        known_loss_count=None,
        source_page_count=None,
        render_page_count=None,
        review_queue_items=None
    ))]
    fn verify_paged_media_rank_artifacts(
        &self,
        py: Python<'_>,
        html_path: &str,
        css_path: &str,
        profile: &str,
        mode: &str,
        overflow_count: Option<i64>,
        known_loss_count: Option<i64>,
        source_page_count: Option<i64>,
        render_page_count: Option<i64>,
        review_queue_items: Option<i64>,
    ) -> PyResult<PyObject> {
        let html = std::fs::read_to_string(html_path)
            .map_err(|e| PyValueError::new_err(format!("failed to read html artifact for PMR verification: {e}")))?;
        let css = std::fs::read_to_string(css_path)
            .map_err(|e| PyValueError::new_err(format!("failed to read css artifact for PMR verification: {e}")))?;
        let ctx = PmrCoreContext {
            overflow_count,
            known_loss_count,
            source_page_count,
            render_page_count,
            review_queue_items,
            html_artifact_bytes: Some(html.len() as u64),
            css_artifact_bytes: Some(css.len() as u64),
        };
        let core = self
            .engine
            .verify_paged_media_rank_html_core(&html, profile, mode, &ctx);
        build_pmr_report_py(py, &core, html_path, css_path)
    }

    #[pyo3(signature = (html, css, deterministic_hash=None))]
    fn render_pdf(
        &self,
        py: Python<'_>,
        html: &str,
        css: &str,
        deterministic_hash: Option<String>,
    ) -> PyResult<Py<PyBytes>> {
        let bytes = py
            .allow_threads(|| self.engine.render_to_buffer(html, css))
            .map_err(to_py_err)?;
        if let Some(path) = deterministic_hash.as_deref() {
            write_hash_file(path, &sha256_hex(&bytes))?;
        }
        Ok(PyBytes::new_bound(py, &bytes).unbind())
    }

    #[pyo3(signature = (html, css, dpi=150))]
    fn render_image_pages(
        &self,
        py: Python<'_>,
        html: &str,
        css: &str,
        dpi: u32,
    ) -> PyResult<PyObject> {
        let pages = py
            .allow_threads(|| self.engine.render_image_pages(html, css, dpi))
            .map_err(to_py_err)?;
        let out = PyList::empty_bound(py);
        for page in pages {
            out.append(PyBytes::new_bound(py, &page))?;
        }
        Ok(out.to_object(py))
    }

    #[pyo3(signature = (html, css, out_dir, dpi=150, stem=None))]
    fn render_image_pages_to_dir(
        &self,
        py: Python<'_>,
        html: &str,
        css: &str,
        out_dir: &str,
        dpi: u32,
        stem: Option<String>,
    ) -> PyResult<PyObject> {
        let stem = stem.unwrap_or_else(|| "render".to_string());
        let paths = py
            .allow_threads(|| {
                self.engine
                    .render_image_pages_to_dir(html, css, out_dir, &stem, dpi)
            })
            .map_err(to_py_err)?;
        let out = PyList::empty_bound(py);
        for path in paths {
            out.append(path.to_string_lossy().to_string())?;
        }
        Ok(out.to_object(py))
    }

    #[pyo3(signature = (pdf_path, dpi=150))]
    fn render_finalized_pdf_image_pages(
        &self,
        py: Python<'_>,
        pdf_path: &str,
        dpi: u32,
    ) -> PyResult<PyObject> {
        let pages = py
            .allow_threads(|| self.engine.render_finalized_pdf_image_pages(pdf_path, dpi))
            .map_err(to_py_err)?;
        let out = PyList::empty_bound(py);
        for page in pages {
            out.append(PyBytes::new_bound(py, &page))?;
        }
        Ok(out.to_object(py))
    }

    #[pyo3(signature = (pdf_path, out_dir, dpi=150, stem=None))]
    fn render_finalized_pdf_image_pages_to_dir(
        &self,
        py: Python<'_>,
        pdf_path: &str,
        out_dir: &str,
        dpi: u32,
        stem: Option<String>,
    ) -> PyResult<PyObject> {
        let stem = stem.unwrap_or_else(|| "render".to_string());
        let paths = py
            .allow_threads(|| {
                self.engine
                    .render_finalized_pdf_image_pages_to_dir(pdf_path, out_dir, &stem, dpi)
            })
            .map_err(to_py_err)?;
        let out = PyList::empty_bound(py);
        for path in paths {
            out.append(path.to_string_lossy().to_string())?;
        }
        Ok(out.to_object(py))
    }

    fn render_pdf_with_page_data(
        &self,
        py: Python<'_>,
        html: &str,
        css: &str,
    ) -> PyResult<(Py<PyBytes>, PyObject)> {
        let (bytes, page_data) = py
            .allow_threads(|| self.engine.render_with_page_data(html, css))
            .map_err(to_py_err)?;

        let data_obj = match page_data {
            Some(ctx) => page_data_context_to_py(py, &ctx)?,
            None => py.None(),
        };

        Ok((PyBytes::new_bound(py, &bytes).unbind(), data_obj))
    }

    fn render_pdf_with_page_data_and_glyph_report(
        &self,
        py: Python<'_>,
        html: &str,
        css: &str,
    ) -> PyResult<(Py<PyBytes>, PyObject, PyObject)> {
        let (bytes, page_data, report) = py
            .allow_threads(|| {
                self.engine
                    .render_with_page_data_and_glyph_report(html, css)
            })
            .map_err(to_py_err)?;

        let data_obj = match page_data {
            Some(ctx) => page_data_context_to_py(py, &ctx)?,
            None => py.None(),
        };
        let report_obj = glyph_report_to_py(py, &report)?;

        Ok((
            PyBytes::new_bound(py, &bytes).unbind(),
            data_obj,
            report_obj,
        ))
    }

    fn export_render_time_reading_order_trace(
        &self,
        py: Python<'_>,
        html: &str,
        css: &str,
    ) -> PyResult<PyObject> {
        let doc = py
            .allow_threads(|| self.engine.render_to_document(html, css))
            .map_err(to_py_err)?;
        build_render_time_reading_order_trace_py(py, &doc)
    }

    fn export_render_time_structure_trace(
        &self,
        py: Python<'_>,
        html: &str,
        css: &str,
    ) -> PyResult<PyObject> {
        let doc = py
            .allow_threads(|| self.engine.render_to_document(html, css))
            .map_err(to_py_err)?;
        build_render_time_structure_trace_py(py, &doc)
    }

    fn render_pdf_with_page_data_and_template_bindings(
        &self,
        py: Python<'_>,
        html: &str,
        css: &str,
    ) -> PyResult<(Py<PyBytes>, PyObject, PyObject)> {
        let (bytes, page_data, template_bindings) = py
            .allow_threads(|| {
                self.engine
                    .render_with_page_data_and_template_bindings(html, css)
            })
            .map_err(to_py_err)?;

        let data_obj = match page_data {
            Some(ctx) => page_data_context_to_py(py, &ctx)?,
            None => py.None(),
        };
        let bindings_obj = match template_bindings {
            Some(bindings) => template_binding_decisions_to_py(py, &bindings)?,
            None => py.None(),
        };

        Ok((
            PyBytes::new_bound(py, &bytes).unbind(),
            data_obj,
            bindings_obj,
        ))
    }

    fn render_pdf_with_page_data_and_template_bindings_and_glyph_report(
        &self,
        py: Python<'_>,
        html: &str,
        css: &str,
    ) -> PyResult<(Py<PyBytes>, PyObject, PyObject, PyObject)> {
        let (bytes, page_data, template_bindings, report) = py
            .allow_threads(|| {
                self.engine
                    .render_with_page_data_and_template_bindings_and_glyph_report(html, css)
            })
            .map_err(to_py_err)?;

        let data_obj = match page_data {
            Some(ctx) => page_data_context_to_py(py, &ctx)?,
            None => py.None(),
        };
        let bindings_obj = match template_bindings {
            Some(bindings) => template_binding_decisions_to_py(py, &bindings)?,
            None => py.None(),
        };
        let report_obj = glyph_report_to_py(py, &report)?;

        Ok((
            PyBytes::new_bound(py, &bytes).unbind(),
            data_obj,
            bindings_obj,
            report_obj,
        ))
    }

    #[pyo3(signature = (html, css, templates, dx=0.0, dy=0.0))]
    fn plan_template_compose(
        &self,
        py: Python<'_>,
        html: &str,
        css: &str,
        templates: Vec<(String, String)>,
        dx: f32,
        dy: f32,
    ) -> PyResult<PyObject> {
        let entries = inspect_template_catalog_entries(&templates)?;
        let mut template_page_counts: BTreeMap<String, usize> = BTreeMap::new();
        for entry in &entries {
            if !entry.issues.is_empty() {
                let issue_codes: Vec<&str> = entry.issues.iter().map(|i| i.as_str()).collect();
                return Err(PyValueError::new_err(format!(
                    "template catalog item is not composition-compatible for template_id={}: {} (issues={:?})",
                    entry.template_id, entry.pdf_path, issue_codes
                )));
            }
            template_page_counts.insert(entry.template_id.clone(), entry.report.page_count);
        }

        let (_bytes, page_data, template_bindings) = py
            .allow_threads(|| {
                self.engine
                    .render_with_page_data_and_template_bindings(html, css)
            })
            .map_err(to_py_err)?;

        let bindings = template_bindings.ok_or_else(|| {
            PyValueError::new_err(
                "template compose planning requires template_binding on PdfEngine and non-empty bindings",
            )
        })?;
        if bindings.is_empty() {
            return Err(PyValueError::new_err(
                "template compose planning requires non-empty template bindings",
            ));
        }

        let mut sorted_bindings = bindings.clone();
        sorted_bindings.sort_by_key(|item| item.page_index);
        let plan_obj = compose_plan_to_py(py, &sorted_bindings, &template_page_counts, dx, dy)?;

        let out = PyDict::new_bound(py);
        out.set_item("ok", true)?;
        out.set_item("dx", dx)?;
        out.set_item("dy", dy)?;
        let page_data_obj = match page_data {
            Some(ctx) => page_data_context_to_py(py, &ctx)?,
            None => py.None(),
        };
        out.set_item("page_data", page_data_obj)?;
        out.set_item(
            "bindings",
            template_binding_decisions_to_py(py, &sorted_bindings)?,
        )?;
        out.set_item("plan", plan_obj)?;
        out.set_item(
            "template_catalog",
            template_catalog_entries_to_py(py, &entries)?,
        )?;

        let metrics = PyDict::new_bound(py);
        metrics.set_item("pages", sorted_bindings.len())?;
        metrics.set_item("templates", template_page_counts.len())?;
        metrics.set_item("dx", dx)?;
        metrics.set_item("dy", dy)?;
        out.set_item("metrics", metrics)?;

        Ok(out.to_object(py))
    }

    #[pyo3(signature = (html, css))]
    fn render_pdf_with_glyph_report(
        &self,
        py: Python<'_>,
        html: &str,
        css: &str,
    ) -> PyResult<(Py<PyBytes>, PyObject)> {
        let (bytes, report) = py
            .allow_threads(|| self.engine.render_with_glyph_report(html, css))
            .map_err(to_py_err)?;
        let report_obj = glyph_report_to_py(py, &report)?;
        Ok((PyBytes::new_bound(py, &bytes).unbind(), report_obj))
    }

    #[pyo3(signature = (html, css, path, deterministic_hash=None))]
    fn render_pdf_to_file(
        &self,
        html: &str,
        css: &str,
        path: &str,
        deterministic_hash: Option<String>,
    ) -> PyResult<usize> {
        let written = Python::with_gil(|py| {
            py.allow_threads(|| self.engine.render_to_file(html, css, path))
                .map_err(to_py_err)
        })?;
        if let Some(hash_path) = deterministic_hash.as_deref() {
            let hash = sha256_file_hex(path)?;
            write_hash_file(hash_path, &hash)?;
        }
        Ok(written)
    }

    #[pyo3(signature = (html_list, css, deterministic_hash=None))]
    fn render_pdf_batch(
        &self,
        py: Python<'_>,
        html_list: Vec<String>,
        css: &str,
        deterministic_hash: Option<String>,
    ) -> PyResult<Py<PyBytes>> {
        let bytes = py
            .allow_threads(|| self.engine.render_many_to_buffer(&html_list, css))
            .map_err(to_py_err)?;
        if let Some(path) = deterministic_hash.as_deref() {
            write_hash_file(path, &sha256_hex(&bytes))?;
        }
        Ok(PyBytes::new_bound(py, &bytes).unbind())
    }

    #[pyo3(signature = (html_list, css, path, deterministic_hash=None))]
    fn render_pdf_batch_to_file(
        &self,
        html_list: Vec<String>,
        css: &str,
        path: &str,
        deterministic_hash: Option<String>,
    ) -> PyResult<usize> {
        let written = Python::with_gil(|py| {
            py.allow_threads(|| self.engine.render_many_to_file(&html_list, css, path))
                .map_err(to_py_err)
        })?;
        if let Some(hash_path) = deterministic_hash.as_deref() {
            let hash = sha256_file_hex(path)?;
            write_hash_file(hash_path, &hash)?;
        }
        Ok(written)
    }

    #[pyo3(signature = (jobs, deterministic_hash=None))]
    fn render_pdf_batch_with_css(
        &self,
        py: Python<'_>,
        jobs: Vec<(String, String)>,
        deterministic_hash: Option<String>,
    ) -> PyResult<Py<PyBytes>> {
        let bytes = py
            .allow_threads(|| self.engine.render_many_to_buffer_with_css(&jobs))
            .map_err(to_py_err)?;
        if let Some(path) = deterministic_hash.as_deref() {
            write_hash_file(path, &sha256_hex(&bytes))?;
        }
        Ok(PyBytes::new_bound(py, &bytes).unbind())
    }

    #[pyo3(signature = (jobs, path, deterministic_hash=None))]
    fn render_pdf_batch_with_css_to_file(
        &self,
        jobs: Vec<(String, String)>,
        path: &str,
        deterministic_hash: Option<String>,
    ) -> PyResult<usize> {
        let written = Python::with_gil(|py| {
            py.allow_threads(|| self.engine.render_many_to_file_with_css(&jobs, path))
                .map_err(to_py_err)
        })?;
        if let Some(hash_path) = deterministic_hash.as_deref() {
            let hash = sha256_file_hex(path)?;
            write_hash_file(hash_path, &hash)?;
        }
        Ok(written)
    }

    // Parallel batch (common CSS). Uses Rust threads; releases the GIL for the duration.
    #[pyo3(signature = (html_list, css, deterministic_hash=None))]
    fn render_pdf_batch_parallel(
        &self,
        py: Python<'_>,
        html_list: Vec<String>,
        css: &str,
        deterministic_hash: Option<String>,
    ) -> PyResult<Py<PyBytes>> {
        let bytes = py
            .allow_threads(|| self.engine.render_many_to_buffer_parallel(&html_list, css))
            .map_err(to_py_err)?;
        if let Some(path) = deterministic_hash.as_deref() {
            write_hash_file(path, &sha256_hex(&bytes))?;
        }
        Ok(PyBytes::new_bound(py, &bytes).unbind())
    }

    #[pyo3(signature = (html_list, css, path, deterministic_hash=None))]
    fn render_pdf_batch_to_file_parallel_with_page_data(
        &self,
        html_list: Vec<String>,
        css: &str,
        path: &str,
        deterministic_hash: Option<String>,
    ) -> PyResult<(usize, PyObject)> {
        let (bytes_written, page_data) = Python::with_gil(|py| {
            py.allow_threads(|| {
                self.engine
                    .render_many_to_file_parallel_with_page_data(&html_list, css, path)
            })
            .map_err(to_py_err)
        })?;
        if let Some(hash_path) = deterministic_hash.as_deref() {
            let hash = sha256_file_hex(path)?;
            write_hash_file(hash_path, &hash)?;
        }
        Python::with_gil(|py| {
            let list = PyList::empty_bound(py);
            for ctx in page_data {
                let obj = match ctx {
                    Some(c) => page_data_context_to_py(py, &c)?,
                    None => py.None(),
                };
                list.append(obj)?;
            }
            Ok((bytes_written, list.to_object(py)))
        })
    }

    #[pyo3(signature = (html_list, css, path, deterministic_hash=None))]
    fn render_pdf_batch_to_file_parallel(
        &self,
        html_list: Vec<String>,
        css: &str,
        path: &str,
        deterministic_hash: Option<String>,
    ) -> PyResult<usize> {
        let written = Python::with_gil(|py| {
            py.allow_threads(|| {
                self.engine
                    .render_many_to_file_parallel(&html_list, css, path)
            })
            .map_err(to_py_err)
        })?;
        if let Some(hash_path) = deterministic_hash.as_deref() {
            let hash = sha256_file_hex(path)?;
            write_hash_file(hash_path, &hash)?;
        }
        Ok(written)
    }
}

#[pymodule]
fn _fullbleed(_py: Python<'_>, module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<PdfEngine>()?;
    module.add_class::<PyAssetKind>()?;
    module.add_class::<PyAsset>()?;
    module.add_class::<PyAssetBundle>()?;
    module.add_class::<PyWatermarkSpec>()?;
    module.add_function(wrap_pyfunction!(inspect_pdf, module)?)?;
    module.add_function(wrap_pyfunction!(inspect_template_catalog, module)?)?;
    module.add_function(wrap_pyfunction!(vendored_asset, module)?)?;
    module.add_function(wrap_pyfunction!(fetch_asset, module)?)?;
    module.add_function(wrap_pyfunction!(concat_css, module)?)?;
    module.add_function(wrap_pyfunction!(audit_contract_metadata, module)?)?;
    module.add_function(wrap_pyfunction!(audit_contract_registry, module)?)?;
    module.add_function(wrap_pyfunction!(audit_contract_wcag20aa_coverage, module)?)?;
    module.add_function(wrap_pyfunction!(audit_contract_section508_html_coverage, module)?)?;
    module.add_function(wrap_pyfunction!(audit_contrast_render_png, module)?)?;
    module.add_function(wrap_pyfunction!(export_pdf_reading_order_trace, module)?)?;
    module.add_function(wrap_pyfunction!(export_pdf_structure_trace, module)?)?;
    module.add_function(wrap_pyfunction!(verify_pdf_ua_seed, module)?)?;
    module.add_function(wrap_pyfunction!(finalize_stamp_pdf, module)?)?;
    module.add_function(wrap_pyfunction!(finalize_compose_pdf, module)?)?;
    Ok(())
}

fn to_py_err(err: FullBleedError) -> PyErr {
    PyValueError::new_err(err.to_string())
}

fn pdf_asset_inspect_err_to_py(err: crate::PdfInspectError) -> PyErr {
    match err.code {
        crate::PdfInspectErrorCode::PdfParseFailed => {
            PyValueError::new_err(format!("invalid pdf data: {}", err.message))
        }
        crate::PdfInspectErrorCode::PdfEncryptedUnsupported => {
            PyValueError::new_err("encrypted pdf assets are not supported")
        }
        crate::PdfInspectErrorCode::PdfEmptyOrNoPages => {
            PyValueError::new_err("invalid pdf data: pdf has no pages")
        }
        crate::PdfInspectErrorCode::PdfIoError => PyValueError::new_err(err.message),
    }
}

fn pdf_inspect_err_to_py(err: crate::PdfInspectError) -> PyErr {
    PyValueError::new_err(format!("{}: {}", err.code.as_str(), err.message))
}
