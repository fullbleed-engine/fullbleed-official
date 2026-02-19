#![allow(unsafe_op_in_unsafe_fn)]

use crate::assets::is_supported_font_path;
use crate::{
    Asset, AssetBundle, AssetKind, Color, ColorSpace, FullBleed, FullBleedBuilder, FullBleedError,
    GlyphCoverageReport, JitMode, Margins, OutputIntent, PageDataContext, PageDataValue,
    PdfProfile, PdfVersion, Pt, Size, WatermarkLayer, WatermarkSemantics, WatermarkSpec,
    composition_compatibility_issues, inspect_pdf_bytes, inspect_pdf_path,
    require_pdf_composition_compatibility,
};
use base64::Engine;
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
        self.engine = self.builder.clone().build().map_err(to_py_err)?;
        Ok(())
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
