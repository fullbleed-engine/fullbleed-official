use crate::{
    Command, Document, FullBleedError, PdfInspectError, PdfInspectErrorCode,
    composition_compatibility_issues, inspect_pdf_path,
};
use lopdf::{
    Document as LoDocument, Object as LoObject, ObjectId as LoObjectId, Stream as LoStream,
    dictionary,
};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

pub const META_PAGE_TEMPLATE_KEY: &str = "fb.page_template";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BindingSource {
    Feature,
    PageTemplate,
    Default,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageBindingDecision {
    pub page_index: usize,
    pub page_template_name: Option<String>,
    pub feature_hits: Vec<String>,
    pub template_id: String,
    pub source: BindingSource,
}

#[derive(Debug, Clone)]
pub struct TemplateBindingSpec {
    pub default_template_id: Option<String>,
    pub by_page_template: BTreeMap<String, String>,
    pub by_feature: BTreeMap<String, String>,
    pub feature_prefix: String,
}

impl Default for TemplateBindingSpec {
    fn default() -> Self {
        Self {
            default_template_id: None,
            by_page_template: BTreeMap::new(),
            by_feature: BTreeMap::new(),
            feature_prefix: "fb.feature.".to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateAsset {
    pub template_id: String,
    pub pdf_path: PathBuf,
    pub sha256: Option<String>,
    pub page_count: Option<usize>,
}

#[derive(Debug, Clone, Default)]
pub struct TemplateCatalog {
    pub by_id: BTreeMap<String, TemplateAsset>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinalizeStampSummary {
    pub pages_written: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ComposePagePlan {
    pub template_id: String,
    pub template_page_index: usize,
    pub overlay_page_index: usize,
    pub dx: f32,
    pub dy: f32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinalizeComposeSummary {
    pub pages_written: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ComposeAnnotationMode {
    None,
    #[default]
    LinkOnly,
    CarryWidgets,
}

fn lopdf_err(err: lopdf::Error) -> FullBleedError {
    FullBleedError::InvalidConfiguration(format!("pdf compose error: {err}"))
}

fn inspect_err_to_finalize_err(err: PdfInspectError) -> FullBleedError {
    match err.code {
        PdfInspectErrorCode::PdfParseFailed | PdfInspectErrorCode::PdfIoError => {
            FullBleedError::InvalidConfiguration(format!("pdf compose error: {}", err.message))
        }
        PdfInspectErrorCode::PdfEncryptedUnsupported => {
            FullBleedError::InvalidConfiguration("pdf compose error: encrypted pdf".to_string())
        }
        PdfInspectErrorCode::PdfEmptyOrNoPages => {
            FullBleedError::InvalidConfiguration("pdf compose error: pdf has no pages".to_string())
        }
    }
}

fn preflight_finalize_pdf(
    path: &std::path::Path,
    role: &str,
) -> Result<crate::PdfInspectReport, FullBleedError> {
    let report = inspect_pdf_path(path).map_err(inspect_err_to_finalize_err)?;
    let issues = composition_compatibility_issues(&report);
    for issue in issues {
        match issue {
            PdfInspectErrorCode::PdfEncryptedUnsupported => {
                return Err(FullBleedError::InvalidConfiguration(format!(
                    "{role} PDF is encrypted"
                )));
            }
            PdfInspectErrorCode::PdfEmptyOrNoPages => {
                return Err(FullBleedError::InvalidConfiguration(format!(
                    "pdf compose error: {role} PDF has no pages"
                )));
            }
            PdfInspectErrorCode::PdfParseFailed | PdfInspectErrorCode::PdfIoError => {}
        }
    }
    Ok(report)
}

fn page_box(page: &lopdf::Dictionary) -> Vec<LoObject> {
    if let Ok(arr) = page.get(b"CropBox").and_then(LoObject::as_array) {
        return arr.clone();
    }
    if let Ok(arr) = page.get(b"MediaBox").and_then(LoObject::as_array) {
        return arr.clone();
    }
    vec![0.into(), 0.into(), 612.into(), 792.into()]
}

fn page_resources_object(doc: &LoDocument, page: &lopdf::Dictionary) -> LoObject {
    match page.get(b"Resources") {
        Ok(obj) => match obj {
            LoObject::Reference(id) => doc
                .get_object(*id)
                .map(|o| o.clone())
                .unwrap_or_else(|_| LoObject::Dictionary(lopdf::Dictionary::new())),
            LoObject::Dictionary(d) => LoObject::Dictionary(d.clone()),
            _ => LoObject::Dictionary(lopdf::Dictionary::new()),
        },
        Err(_) => LoObject::Dictionary(lopdf::Dictionary::new()),
    }
}

fn page_resources_dict(page: &lopdf::Dictionary, doc: &LoDocument) -> lopdf::Dictionary {
    match page.get(b"Resources") {
        Ok(LoObject::Dictionary(d)) => d.clone(),
        Ok(LoObject::Reference(id)) => doc
            .get_object(*id)
            .ok()
            .and_then(|o| o.as_dict().ok())
            .cloned()
            .unwrap_or_default(),
        _ => lopdf::Dictionary::new(),
    }
}

fn page_xobject_dict(resources: &lopdf::Dictionary, doc: &LoDocument) -> lopdf::Dictionary {
    match resources.get(b"XObject") {
        Ok(LoObject::Dictionary(d)) => d.clone(),
        Ok(LoObject::Reference(id)) => doc
            .get_object(*id)
            .ok()
            .and_then(|o| o.as_dict().ok())
            .cloned()
            .unwrap_or_default(),
        _ => lopdf::Dictionary::new(),
    }
}

fn resolve_finalize_object<'a>(
    doc: &'a LoDocument,
    mut obj: &'a LoObject,
) -> Result<&'a LoObject, FullBleedError> {
    loop {
        match obj {
            LoObject::Reference(id) => {
                obj = doc.get_object(*id).map_err(lopdf_err)?;
            }
            _ => return Ok(obj),
        }
    }
}

fn object_is_name(doc: &LoDocument, obj: &LoObject, expected: &[u8]) -> bool {
    let Ok(resolved) = resolve_finalize_object(doc, obj) else {
        return false;
    };
    let Ok(name) = resolved.as_name() else {
        return false;
    };
    name == expected || name == format!("/{}", String::from_utf8_lossy(expected)).as_bytes()
}

fn annotation_allowed_for_mode(
    mode: ComposeAnnotationMode,
    doc: &LoDocument,
    annot_dict: &lopdf::Dictionary,
) -> bool {
    let subtype_obj = match annot_dict.get(b"Subtype") {
        Ok(obj) => obj,
        Err(_) => return false,
    };
    match mode {
        ComposeAnnotationMode::None => false,
        ComposeAnnotationMode::LinkOnly => object_is_name(doc, subtype_obj, b"Link"),
        ComposeAnnotationMode::CarryWidgets => {
            object_is_name(doc, subtype_obj, b"Link")
                || object_is_name(doc, subtype_obj, b"Widget")
        }
    }
}

fn clone_template_annotations_for_output_page(
    doc: &mut LoDocument,
    template_page: &lopdf::Dictionary,
    output_page_id: LoObjectId,
    mode: ComposeAnnotationMode,
) -> Result<Option<LoObject>, FullBleedError> {
    if mode == ComposeAnnotationMode::None {
        return Ok(None);
    }

    let annots_obj = match template_page.get(b"Annots") {
        Ok(obj) => obj,
        Err(_) => return Ok(None),
    };

    let annots_arr = match resolve_finalize_object(doc, annots_obj)? {
        LoObject::Array(arr) => arr.clone(),
        _ => return Ok(None),
    };

    let mut out_annots: Vec<LoObject> = Vec::new();
    for annot in annots_arr {
        let annot_dict = match resolve_finalize_object(doc, &annot)? {
            LoObject::Dictionary(d) => d.clone(),
            LoObject::Stream(s) => s.dict.clone(),
            _ => continue,
        };

        if !annotation_allowed_for_mode(mode, doc, &annot_dict) {
            continue;
        }

        let mut cloned = annot_dict;
        cloned.set("P", LoObject::Reference(output_page_id));
        let new_annot_id = doc.add_object(LoObject::Dictionary(cloned));
        out_annots.push(LoObject::Reference(new_annot_id));
    }

    if out_annots.is_empty() {
        Ok(None)
    } else {
        Ok(Some(LoObject::Array(out_annots)))
    }
}

fn import_document_objects(
    dst: &mut LoDocument,
    mut src: LoDocument,
) -> Result<Vec<LoObjectId>, FullBleedError> {
    let start_id = dst.max_id + 1;
    src.renumber_objects_with(start_id);
    let page_ids: Vec<LoObjectId> = src.get_pages().values().copied().collect();
    if src.max_id > dst.max_id {
        dst.max_id = src.max_id;
    }
    dst.objects.extend(src.objects);
    Ok(page_ids)
}

pub fn stamp_overlay_on_template_pdf(
    template_pdf: &std::path::Path,
    overlay_pdf: &std::path::Path,
    out_pdf: &std::path::Path,
    page_map: Option<&[(usize, usize)]>,
    dx: f32,
    dy: f32,
) -> Result<FinalizeStampSummary, FullBleedError> {
    let template_meta = preflight_finalize_pdf(template_pdf, "template")?;
    let overlay_meta = preflight_finalize_pdf(overlay_pdf, "overlay")?;

    let mut template = LoDocument::load(template_pdf).map_err(lopdf_err)?;
    let mut overlay = LoDocument::load(overlay_pdf).map_err(lopdf_err)?;

    let template_pages = template.get_pages();
    let template_count = template_meta.page_count;
    let overlay_count = overlay_meta.page_count;
    let mapping = match page_map {
        Some(v) => v.to_vec(),
        None => default_page_map(template_count, overlay_count)?,
    };
    validate_page_map(&mapping, template_count, overlay_count)?;

    let start_id = template.max_id + 1;
    overlay.renumber_objects_with(start_id);
    let overlay_pages = overlay.get_pages();
    if overlay.max_id > template.max_id {
        template.max_id = overlay.max_id;
    }
    template.objects.extend(overlay.objects);

    let template_ids: Vec<LoObjectId> = template_pages.values().copied().collect();
    let overlay_ids: Vec<LoObjectId> = overlay_pages.values().copied().collect();

    for (out_idx, (tpl_i, ovl_i)) in mapping.iter().enumerate() {
        let template_page_id = template_ids[*tpl_i];
        let overlay_page_id = overlay_ids[*ovl_i];

        let overlay_page = template
            .get_object(overlay_page_id)
            .and_then(LoObject::as_dict)
            .map_err(lopdf_err)?
            .clone();
        let overlay_content = template
            .get_page_content(overlay_page_id)
            .map_err(lopdf_err)?;
        let bbox = page_box(&overlay_page);
        let overlay_resources = page_resources_object(&template, &overlay_page);

        let form_stream = LoStream::new(
            dictionary! {
                "Type" => "XObject",
                "Subtype" => "Form",
                "FormType" => 1,
                "BBox" => LoObject::Array(bbox),
                "Resources" => overlay_resources,
            },
            overlay_content,
        );
        let form_id = template.add_object(form_stream);
        let form_name = format!("FB_OVL_{}", out_idx + 1);

        let page_dict = template
            .get_object(template_page_id)
            .and_then(LoObject::as_dict)
            .map_err(lopdf_err)?
            .clone();
        let mut resources = page_resources_dict(&page_dict, &template);
        let mut xobjects = page_xobject_dict(&resources, &template);
        xobjects.set(form_name.as_bytes().to_vec(), LoObject::Reference(form_id));
        resources.set("XObject", LoObject::Dictionary(xobjects));

        {
            let page_mut = template
                .get_object_mut(template_page_id)
                .and_then(LoObject::as_dict_mut)
                .map_err(lopdf_err)?;
            page_mut.set("Resources", LoObject::Dictionary(resources));
        }

        let do_content = format!("q 1 0 0 1 {} {} cm /{} Do Q\n", dx, dy, form_name).into_bytes();
        template
            .add_page_contents(template_page_id, do_content)
            .map_err(lopdf_err)?;
    }

    template.prune_objects();
    template.renumber_objects();
    template.compress();
    template.save(out_pdf)?;

    Ok(FinalizeStampSummary {
        pages_written: mapping.len(),
    })
}

pub fn compose_overlay_with_template_catalog(
    catalog: &TemplateCatalog,
    overlay_pdf: &std::path::Path,
    out_pdf: &std::path::Path,
    plan: &[ComposePagePlan],
) -> Result<FinalizeComposeSummary, FullBleedError> {
    compose_overlay_with_template_catalog_with_annotation_mode(
        catalog,
        overlay_pdf,
        out_pdf,
        plan,
        ComposeAnnotationMode::default(),
    )
}

pub fn compose_overlay_with_template_catalog_with_annotation_mode(
    catalog: &TemplateCatalog,
    overlay_pdf: &std::path::Path,
    out_pdf: &std::path::Path,
    plan: &[ComposePagePlan],
    annotation_mode: ComposeAnnotationMode,
) -> Result<FinalizeComposeSummary, FullBleedError> {
    if plan.is_empty() {
        return Err(FullBleedError::InvalidConfiguration(
            "compose plan cannot be empty".to_string(),
        ));
    }
    if catalog.by_id.is_empty() {
        return Err(FullBleedError::InvalidConfiguration(
            "template catalog cannot be empty".to_string(),
        ));
    }

    let mut composed = LoDocument::with_version("1.7");
    let mut template_pages_by_id: BTreeMap<String, Vec<LoObjectId>> = BTreeMap::new();

    for (template_id, asset) in &catalog.by_id {
        let report = preflight_finalize_pdf(&asset.pdf_path, "template")?;
        if let Some(expected) = asset.page_count {
            if expected != report.page_count {
                return Err(FullBleedError::InvalidConfiguration(format!(
                    "template page count mismatch for template_id={}: expected {} found {}",
                    template_id, expected, report.page_count
                )));
            }
        }

        let src = LoDocument::load(&asset.pdf_path).map_err(lopdf_err)?;
        let page_ids = import_document_objects(&mut composed, src)?;
        if let Some(expected) = asset.page_count {
            if expected != page_ids.len() {
                return Err(FullBleedError::InvalidConfiguration(format!(
                    "template page count mismatch for template_id={}: expected {} found {}",
                    template_id,
                    expected,
                    page_ids.len()
                )));
            }
        }
        template_pages_by_id.insert(template_id.clone(), page_ids);
    }

    preflight_finalize_pdf(overlay_pdf, "overlay")?;
    let overlay_src = LoDocument::load(overlay_pdf).map_err(lopdf_err)?;
    let overlay_pages = import_document_objects(&mut composed, overlay_src)?;

    for (idx, item) in plan.iter().enumerate() {
        let Some(template_pages) = template_pages_by_id.get(&item.template_id) else {
            return Err(FullBleedError::InvalidConfiguration(format!(
                "plan item {} references unknown template_id: {}",
                idx, item.template_id
            )));
        };
        if item.template_page_index >= template_pages.len() {
            return Err(FullBleedError::InvalidConfiguration(format!(
                "plan item {} template_page out of range: {} (allowed 0..{})",
                idx,
                item.template_page_index,
                template_pages.len().saturating_sub(1)
            )));
        }
        if item.overlay_page_index >= overlay_pages.len() {
            return Err(FullBleedError::InvalidConfiguration(format!(
                "plan item {} overlay_page out of range: {} (allowed 0..{})",
                idx,
                item.overlay_page_index,
                overlay_pages.len().saturating_sub(1)
            )));
        }
    }

    let pages_id = composed.new_object_id();
    let mut kids: Vec<LoObject> = Vec::with_capacity(plan.len());

    for (idx, item) in plan.iter().enumerate() {
        let template_page_id = template_pages_by_id[&item.template_id][item.template_page_index];
        let overlay_page_id = overlay_pages[item.overlay_page_index];

        let template_page = composed
            .get_object(template_page_id)
            .and_then(LoObject::as_dict)
            .map_err(lopdf_err)?
            .clone();
        let overlay_page = composed
            .get_object(overlay_page_id)
            .and_then(LoObject::as_dict)
            .map_err(lopdf_err)?
            .clone();

        let template_content = composed
            .get_page_content(template_page_id)
            .map_err(lopdf_err)?;
        let overlay_content = composed
            .get_page_content(overlay_page_id)
            .map_err(lopdf_err)?;
        let template_bbox = page_box(&template_page);
        let overlay_bbox = page_box(&overlay_page);
        let template_resources = page_resources_object(&composed, &template_page);
        let overlay_resources = page_resources_object(&composed, &overlay_page);

        let template_form_id = composed.add_object(LoStream::new(
            dictionary! {
                "Type" => "XObject",
                "Subtype" => "Form",
                "FormType" => 1,
                "BBox" => LoObject::Array(template_bbox.clone()),
                "Resources" => template_resources,
            },
            template_content,
        ));
        let overlay_form_id = composed.add_object(LoStream::new(
            dictionary! {
                "Type" => "XObject",
                "Subtype" => "Form",
                "FormType" => 1,
                "BBox" => LoObject::Array(overlay_bbox),
                "Resources" => overlay_resources,
            },
            overlay_content,
        ));

        let page_content = format!(
            "q 1 0 0 1 0 0 cm /FB_TPL_{} Do Q\nq 1 0 0 1 {} {} cm /FB_OVL_{} Do Q\n",
            idx + 1,
            item.dx,
            item.dy,
            idx + 1
        )
        .into_bytes();
        let page_content_id = composed.add_object(LoStream::new(dictionary! {}, page_content));

        let page_id = composed.new_object_id();
        let mut out_page = dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "Contents" => page_content_id,
            "Resources" => dictionary! {
                "XObject" => dictionary! {
                    format!("FB_TPL_{}", idx + 1) => template_form_id,
                    format!("FB_OVL_{}", idx + 1) => overlay_form_id,
                },
            },
            "MediaBox" => LoObject::Array(template_bbox),
        };
        if let Some(annots) = clone_template_annotations_for_output_page(
            &mut composed,
            &template_page,
            page_id,
            annotation_mode,
        )?
        {
            out_page.set("Annots", annots);
        }
        composed.objects.insert(page_id, LoObject::Dictionary(out_page));
        kids.push(LoObject::Reference(page_id));
    }

    composed.objects.insert(
        pages_id,
        LoObject::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => kids,
            "Count" => plan.len() as i64,
        }),
    );

    let catalog_id = composed.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => pages_id,
    });
    composed.trailer.set("Root", catalog_id);
    composed.prune_objects();
    composed.renumber_objects();
    composed.compress();
    composed.save(out_pdf)?;

    Ok(FinalizeComposeSummary {
        pages_written: plan.len(),
    })
}

impl TemplateCatalog {
    pub fn insert(&mut self, asset: TemplateAsset) -> Result<(), FullBleedError> {
        if asset.template_id.trim().is_empty() {
            return Err(FullBleedError::InvalidConfiguration(
                "template_id cannot be empty".to_string(),
            ));
        }
        if self.by_id.contains_key(&asset.template_id) {
            return Err(FullBleedError::InvalidConfiguration(format!(
                "duplicate template_id in catalog: {}",
                asset.template_id
            )));
        }
        self.by_id.insert(asset.template_id.clone(), asset);
        Ok(())
    }

    pub fn get(&self, template_id: &str) -> Option<&TemplateAsset> {
        self.by_id.get(template_id)
    }
}

fn is_truthy_flag(value: &str) -> bool {
    let v = value.trim().to_ascii_lowercase();
    matches!(v.as_str(), "" | "1" | "true" | "yes" | "on")
}

pub fn collect_page_feature_flags(doc: &Document, feature_prefix: &str) -> Vec<BTreeSet<String>> {
    doc.pages
        .iter()
        .map(|page| {
            let mut features = BTreeSet::new();
            for cmd in &page.commands {
                let Command::Meta { key, value } = cmd else {
                    continue;
                };
                if !key.starts_with(feature_prefix) {
                    continue;
                }
                let feature_name = key[feature_prefix.len()..].trim();
                if feature_name.is_empty() {
                    continue;
                }
                if is_truthy_flag(value) {
                    features.insert(feature_name.to_string());
                }
            }
            features
        })
        .collect()
}

pub fn collect_page_template_names(doc: &Document, template_key: &str) -> Vec<Option<String>> {
    doc.pages
        .iter()
        .map(|page| {
            for cmd in &page.commands {
                let Command::Meta { key, value } = cmd else {
                    continue;
                };
                if key == template_key && !value.trim().is_empty() {
                    return Some(value.clone());
                }
            }
            None
        })
        .collect()
}

pub fn resolve_template_bindings(
    spec: &TemplateBindingSpec,
    page_template_names: &[Option<String>],
    page_features: &[BTreeSet<String>],
) -> Result<Vec<PageBindingDecision>, FullBleedError> {
    if page_template_names.len() != page_features.len() {
        return Err(FullBleedError::InvalidConfiguration(format!(
            "finalize binding mismatch: page_template_names={} page_features={}",
            page_template_names.len(),
            page_features.len()
        )));
    }

    let mut out = Vec::with_capacity(page_template_names.len());

    for idx in 0..page_template_names.len() {
        let template_name = page_template_names[idx].clone();
        let features = &page_features[idx];

        // Feature binding has highest precedence. If multiple matched features map to different
        // template IDs for the same page, fail fast.
        let mut matched_features: Vec<&str> = features
            .iter()
            .filter_map(|f| spec.by_feature.get_key_value(f).map(|(k, _)| k.as_str()))
            .collect();
        matched_features.sort_unstable();

        if !matched_features.is_empty() {
            let mut matched_template_ids: BTreeSet<&str> = BTreeSet::new();
            for feature in &matched_features {
                if let Some(template_id) = spec.by_feature.get(*feature) {
                    matched_template_ids.insert(template_id.as_str());
                }
            }
            if matched_template_ids.len() > 1 {
                return Err(FullBleedError::InvalidConfiguration(format!(
                    "ambiguous feature bindings on page {}: features={:?} template_ids={:?}",
                    idx + 1,
                    matched_features,
                    matched_template_ids
                )));
            }
            let template_id = matched_template_ids
                .iter()
                .next()
                .ok_or_else(|| {
                    FullBleedError::InvalidConfiguration(format!(
                        "internal error resolving feature binding on page {}",
                        idx + 1
                    ))
                })?
                .to_string();
            out.push(PageBindingDecision {
                page_index: idx,
                page_template_name: template_name,
                feature_hits: matched_features.iter().map(|s| s.to_string()).collect(),
                template_id,
                source: BindingSource::Feature,
            });
            continue;
        }

        if let Some(name) = template_name.as_deref() {
            if let Some(template_id) = spec.by_page_template.get(name) {
                out.push(PageBindingDecision {
                    page_index: idx,
                    page_template_name: template_name,
                    feature_hits: Vec::new(),
                    template_id: template_id.clone(),
                    source: BindingSource::PageTemplate,
                });
                continue;
            }
        }

        if let Some(template_id) = spec.default_template_id.as_deref() {
            out.push(PageBindingDecision {
                page_index: idx,
                page_template_name: template_name,
                feature_hits: Vec::new(),
                template_id: template_id.to_string(),
                source: BindingSource::Default,
            });
            continue;
        }

        return Err(FullBleedError::InvalidConfiguration(format!(
            "no template binding for page {} (template_name={:?})",
            idx + 1,
            page_template_names[idx]
        )));
    }

    Ok(out)
}

pub fn resolve_template_bindings_for_document(
    doc: &Document,
    spec: &TemplateBindingSpec,
) -> Result<Vec<PageBindingDecision>, FullBleedError> {
    let page_template_names = collect_page_template_names(doc, META_PAGE_TEMPLATE_KEY);
    let page_features = collect_page_feature_flags(doc, &spec.feature_prefix);
    resolve_template_bindings(spec, &page_template_names, &page_features)
}

pub fn default_page_map(
    template_pages: usize,
    overlay_pages: usize,
) -> Result<Vec<(usize, usize)>, FullBleedError> {
    if template_pages != overlay_pages {
        return Err(FullBleedError::InvalidConfiguration(format!(
            "template/overlay page count mismatch without explicit page map (template={}, overlay={})",
            template_pages, overlay_pages
        )));
    }
    Ok((0..template_pages).map(|i| (i, i)).collect())
}

pub fn validate_page_map(
    page_map: &[(usize, usize)],
    template_pages: usize,
    overlay_pages: usize,
) -> Result<(), FullBleedError> {
    for (idx, (tpl_i, ovl_i)) in page_map.iter().enumerate() {
        if *tpl_i >= template_pages {
            return Err(FullBleedError::InvalidConfiguration(format!(
                "page_map item {} template index out of range: {} (allowed 0..{})",
                idx,
                tpl_i,
                template_pages.saturating_sub(1)
            )));
        }
        if *ovl_i >= overlay_pages {
            return Err(FullBleedError::InvalidConfiguration(format!(
                "page_map item {} overlay index out of range: {} (allowed 0..{})",
                idx,
                ovl_i,
                overlay_pages.saturating_sub(1)
            )));
        }
    }
    Ok(())
}

pub fn validate_bindings_against_catalog(
    bindings: &[PageBindingDecision],
    catalog: &TemplateCatalog,
) -> Result<(), FullBleedError> {
    for decision in bindings {
        if catalog.get(&decision.template_id).is_none() {
            return Err(FullBleedError::InvalidConfiguration(format!(
                "missing template_id in catalog for page {}: {}",
                decision.page_index + 1,
                decision.template_id
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Page, Size};
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    use std::io::Write;

    fn make_single_page_pdf(path: &std::path::Path, text: &str) {
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
        doc.save(path).expect("save");
    }

    fn make_single_page_pdf_with_link(path: &std::path::Path, text: &str) {
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
        let action_id = doc.add_object(dictionary! {
            "S" => "URI",
            "URI" => "https://www.example.com",
        });
        let link_annot_id = doc.add_object(dictionary! {
            "Type" => "Annot",
            "Subtype" => "Link",
            "Rect" => vec![72.into(), 700.into(), 240.into(), 720.into()],
            "Border" => vec![0.into(), 0.into(), 0.into()],
            "A" => action_id,
        });
        let page_id = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "Contents" => content_id,
            "Resources" => resources_id,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
            "Annots" => vec![LoObject::Reference(link_annot_id)],
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
        doc.save(path).expect("save");
    }

    fn make_single_page_pdf_with_link_and_widget(path: &std::path::Path, text: &str) {
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
        let action_id = doc.add_object(dictionary! {
            "S" => "URI",
            "URI" => "https://www.example.com",
        });
        let link_annot_id = doc.add_object(dictionary! {
            "Type" => "Annot",
            "Subtype" => "Link",
            "Rect" => vec![72.into(), 700.into(), 240.into(), 720.into()],
            "Border" => vec![0.into(), 0.into(), 0.into()],
            "A" => action_id,
        });
        let widget_annot_id = doc.add_object(dictionary! {
            "Type" => "Annot",
            "Subtype" => "Widget",
            "FT" => "Tx",
            "T" => "Field1",
            "Rect" => vec![72.into(), 660.into(), 240.into(), 684.into()],
            "F" => 4,
        });
        let page_id = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "Contents" => content_id,
            "Resources" => resources_id,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
            "Annots" => vec![LoObject::Reference(link_annot_id), LoObject::Reference(widget_annot_id)],
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
        doc.save(path).expect("save");
    }

    fn page_annotation_count_by_subtype(
        doc: &LoDocument,
        page_id: LoObjectId,
        subtype: &[u8],
    ) -> usize {
        let page = doc
            .get_object(page_id)
            .and_then(LoObject::as_dict)
            .expect("page dict");
        let annots_obj = match page.get(b"Annots") {
            Ok(obj) => obj,
            Err(_) => return 0,
        };
        let annots = match annots_obj {
            LoObject::Array(arr) => arr.clone(),
            LoObject::Reference(id) => doc
                .get_object(*id)
                .and_then(LoObject::as_array)
                .expect("annots by ref")
                .clone(),
            _ => return 0,
        };
        let mut out = 0usize;
        for annot in annots {
            let annot_dict = match annot {
                LoObject::Reference(id) => doc
                    .get_object(id)
                    .and_then(LoObject::as_dict)
                    .expect("annot dict by ref")
                    .clone(),
                LoObject::Dictionary(d) => d,
                _ => continue,
            };
            if let Ok(subtype_obj) = annot_dict.get(b"Subtype") {
                if super::object_is_name(doc, subtype_obj, subtype) {
                    out += 1;
                }
            }
        }
        out
    }

    fn pdf_structural_signature(path: &std::path::Path) -> u64 {
        let doc = LoDocument::load(path).expect("load pdf");
        let pages: Vec<(u32, LoObjectId)> = doc.get_pages().into_iter().collect();
        let mut hasher = DefaultHasher::new();
        pages.len().hash(&mut hasher);
        for (page_no, page_id) in pages {
            page_no.hash(&mut hasher);
            let content = doc.get_page_content(page_id).expect("page content");
            content.len().hash(&mut hasher);
            content.hash(&mut hasher);
        }
        hasher.finish()
    }

    #[test]
    fn collect_page_feature_flags_reads_meta_prefix() {
        let doc = Document {
            page_size: Size::a4(),
            pages: vec![
                Page {
                    commands: vec![
                        Command::Meta {
                            key: "fb.feature.legacy".to_string(),
                            value: "0".to_string(),
                        },
                        Command::Meta {
                            key: "fb.feature.i9".to_string(),
                            value: "true".to_string(),
                        },
                    ],
                },
                Page {
                    commands: vec![Command::Meta {
                        key: "fb.feature.w2".to_string(),
                        value: "1".to_string(),
                    }],
                },
            ],
        };

        let out = collect_page_feature_flags(&doc, "fb.feature.");
        assert_eq!(out.len(), 2);
        assert!(out[0].contains("i9"));
        assert!(!out[0].contains("legacy"));
        assert!(out[1].contains("w2"));
    }

    #[test]
    fn resolve_template_bindings_precedence_feature_then_template_then_default() {
        let mut spec = TemplateBindingSpec {
            default_template_id: Some("tpl-default".to_string()),
            ..TemplateBindingSpec::default()
        };
        spec.by_page_template
            .insert("page_1".to_string(), "tpl-page-1".to_string());
        spec.by_feature
            .insert("vip".to_string(), "tpl-vip".to_string());

        let page_names = vec![
            Some("page_1".to_string()),
            Some("page_2".to_string()),
            Some("page_3".to_string()),
        ];
        let mut f0 = BTreeSet::new();
        f0.insert("vip".to_string());
        let features = vec![f0, BTreeSet::new(), BTreeSet::new()];

        let out = resolve_template_bindings(&spec, &page_names, &features).expect("bindings");
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].template_id, "tpl-vip");
        assert_eq!(out[0].source, BindingSource::Feature);
        assert_eq!(out[1].template_id, "tpl-default");
        assert_eq!(out[1].source, BindingSource::Default);
        assert_eq!(out[2].template_id, "tpl-default");
        assert_eq!(out[2].source, BindingSource::Default);
    }

    #[test]
    fn resolve_template_bindings_fails_on_ambiguous_feature_mappings() {
        let mut spec = TemplateBindingSpec::default();
        spec.by_feature.insert("a".to_string(), "tpl-a".to_string());
        spec.by_feature.insert("b".to_string(), "tpl-b".to_string());
        let mut features = BTreeSet::new();
        features.insert("a".to_string());
        features.insert("b".to_string());
        let err = resolve_template_bindings(&spec, &[None], &[features]).expect_err("ambiguous");
        assert!(
            err.to_string()
                .contains("ambiguous feature bindings on page 1")
        );
    }

    #[test]
    fn collect_page_template_names_uses_first_matching_meta_key() {
        let doc = Document {
            page_size: Size::a4(),
            pages: vec![Page {
                commands: vec![
                    Command::Meta {
                        key: META_PAGE_TEMPLATE_KEY.to_string(),
                        value: "page_1".to_string(),
                    },
                    Command::Meta {
                        key: META_PAGE_TEMPLATE_KEY.to_string(),
                        value: "page_ignored".to_string(),
                    },
                ],
            }],
        };
        let out = collect_page_template_names(&doc, META_PAGE_TEMPLATE_KEY);
        assert_eq!(out, vec![Some("page_1".to_string())]);
    }

    #[test]
    fn resolve_template_bindings_for_document_uses_template_meta_and_features() {
        let doc = Document {
            page_size: Size::a4(),
            pages: vec![
                Page {
                    commands: vec![
                        Command::Meta {
                            key: META_PAGE_TEMPLATE_KEY.to_string(),
                            value: "page_1".to_string(),
                        },
                        Command::Meta {
                            key: "fb.feature.vip".to_string(),
                            value: "1".to_string(),
                        },
                    ],
                },
                Page {
                    commands: vec![Command::Meta {
                        key: META_PAGE_TEMPLATE_KEY.to_string(),
                        value: "page_2".to_string(),
                    }],
                },
            ],
        };
        let mut spec = TemplateBindingSpec {
            default_template_id: Some("tpl-default".to_string()),
            ..TemplateBindingSpec::default()
        };
        spec.by_page_template
            .insert("page_2".to_string(), "tpl-p2".to_string());
        spec.by_feature
            .insert("vip".to_string(), "tpl-vip".to_string());
        let out = resolve_template_bindings_for_document(&doc, &spec).expect("bindings");
        assert_eq!(out[0].template_id, "tpl-vip");
        assert_eq!(out[1].template_id, "tpl-p2");
    }

    #[test]
    fn default_page_map_requires_equal_page_counts() {
        let err = default_page_map(2, 3).expect_err("must fail");
        assert!(err.to_string().contains("page count mismatch"));
        let ok = default_page_map(2, 2).expect("ok");
        assert_eq!(ok, vec![(0, 0), (1, 1)]);
    }

    #[test]
    fn validate_page_map_fails_for_out_of_range_indices() {
        let err = validate_page_map(&[(0, 3)], 1, 1).expect_err("must fail");
        assert!(err.to_string().contains("overlay index out of range"));
        let err = validate_page_map(&[(2, 0)], 1, 1).expect_err("must fail");
        assert!(err.to_string().contains("template index out of range"));
    }

    #[test]
    fn template_catalog_rejects_duplicate_ids() {
        let mut cat = TemplateCatalog::default();
        cat.insert(TemplateAsset {
            template_id: "tpl".to_string(),
            pdf_path: PathBuf::from("a.pdf"),
            sha256: None,
            page_count: None,
        })
        .expect("insert");
        let err = cat
            .insert(TemplateAsset {
                template_id: "tpl".to_string(),
                pdf_path: PathBuf::from("b.pdf"),
                sha256: None,
                page_count: None,
            })
            .expect_err("dup");
        assert!(err.to_string().contains("duplicate template_id"));
    }

    #[test]
    fn validate_bindings_against_catalog_fails_on_missing_template_id() {
        let mut cat = TemplateCatalog::default();
        cat.insert(TemplateAsset {
            template_id: "tpl-a".to_string(),
            pdf_path: PathBuf::from("a.pdf"),
            sha256: None,
            page_count: None,
        })
        .expect("insert");
        let bindings = vec![PageBindingDecision {
            page_index: 0,
            page_template_name: Some("Page1".to_string()),
            feature_hits: Vec::new(),
            template_id: "tpl-missing".to_string(),
            source: BindingSource::Default,
        }];
        let err = validate_bindings_against_catalog(&bindings, &cat).expect_err("missing");
        assert!(
            err.to_string()
                .contains("missing template_id in catalog for page 1")
        );
    }

    #[test]
    fn stamp_overlay_on_template_pdf_smoke() {
        use std::fs;
        let temp_dir = std::env::temp_dir().join(format!(
            "fullbleed_finalize_smoke_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        fs::create_dir_all(&temp_dir).expect("mkdir");
        let template_path = temp_dir.join("template.pdf");
        let overlay_path = temp_dir.join("overlay.pdf");
        let out_path = temp_dir.join("out.pdf");

        make_single_page_pdf(&template_path, "TEMPLATE");
        make_single_page_pdf(&overlay_path, "OVERLAY");
        let summary =
            stamp_overlay_on_template_pdf(&template_path, &overlay_path, &out_path, None, 0.0, 0.0)
                .expect("stamp");
        assert_eq!(summary.pages_written, 1);
        let out = LoDocument::load(&out_path).expect("load out");
        assert_eq!(out.get_pages().len(), 1);
    }

    #[test]
    fn compose_overlay_with_template_catalog_smoke() {
        use std::fs;
        let temp_dir = std::env::temp_dir().join(format!(
            "fullbleed_finalize_compose_smoke_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        fs::create_dir_all(&temp_dir).expect("mkdir");
        let tpl_a = temp_dir.join("tpl_a.pdf");
        let tpl_b = temp_dir.join("tpl_b.pdf");
        let overlay = temp_dir.join("overlay.pdf");
        let out_path = temp_dir.join("out_compose.pdf");

        make_single_page_pdf(&tpl_a, "TEMPLATE_A");
        make_single_page_pdf(&tpl_b, "TEMPLATE_B");
        make_single_page_pdf(&overlay, "OVERLAY");

        let mut catalog = TemplateCatalog::default();
        catalog
            .insert(TemplateAsset {
                template_id: "a".to_string(),
                pdf_path: tpl_a.clone(),
                sha256: None,
                page_count: Some(1),
            })
            .expect("catalog a");
        catalog
            .insert(TemplateAsset {
                template_id: "b".to_string(),
                pdf_path: tpl_b.clone(),
                sha256: None,
                page_count: Some(1),
            })
            .expect("catalog b");

        let plan = vec![
            ComposePagePlan {
                template_id: "a".to_string(),
                template_page_index: 0,
                overlay_page_index: 0,
                dx: 0.0,
                dy: 0.0,
            },
            ComposePagePlan {
                template_id: "b".to_string(),
                template_page_index: 0,
                overlay_page_index: 0,
                dx: 0.0,
                dy: 0.0,
            },
        ];

        let summary = compose_overlay_with_template_catalog(&catalog, &overlay, &out_path, &plan)
            .expect("compose");
        assert_eq!(summary.pages_written, 2);
        let out = LoDocument::load(&out_path).expect("load out");
        assert_eq!(out.get_pages().len(), 2);
    }

    #[test]
    fn compose_overlay_preserves_template_link_annotations() {
        use std::fs;
        let temp_dir = std::env::temp_dir().join(format!(
            "fullbleed_finalize_compose_link_annots_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        fs::create_dir_all(&temp_dir).expect("mkdir");
        let tpl = temp_dir.join("tpl_link.pdf");
        let overlay = temp_dir.join("overlay.pdf");
        let out_path = temp_dir.join("out_compose.pdf");

        make_single_page_pdf_with_link(&tpl, "TEMPLATE_WITH_LINK");
        make_single_page_pdf(&overlay, "OVERLAY");

        let mut catalog = TemplateCatalog::default();
        catalog
            .insert(TemplateAsset {
                template_id: "tpl".to_string(),
                pdf_path: tpl.clone(),
                sha256: None,
                page_count: Some(1),
            })
            .expect("catalog");
        let plan = vec![ComposePagePlan {
            template_id: "tpl".to_string(),
            template_page_index: 0,
            overlay_page_index: 0,
            dx: 0.0,
            dy: 0.0,
        }];

        let summary = compose_overlay_with_template_catalog(&catalog, &overlay, &out_path, &plan)
            .expect("compose");
        assert_eq!(summary.pages_written, 1);

        let out = LoDocument::load(&out_path).expect("load out");
        let out_page_id = *out
            .get_pages()
            .values()
            .next()
            .expect("output page id");
        let out_page = out
            .get_object(out_page_id)
            .and_then(LoObject::as_dict)
            .expect("output page dict");
        let annots_obj = out_page.get(b"Annots").expect("annots present");
        let annots = match annots_obj {
            LoObject::Array(arr) => arr.clone(),
            LoObject::Reference(id) => out
                .get_object(*id)
                .and_then(LoObject::as_array)
                .expect("annots array by ref")
                .clone(),
            _ => panic!("unexpected Annots object"),
        };
        assert!(!annots.is_empty(), "expected at least one link annotation");
        let first = annots.first().expect("first annot");
        let first_dict = match first {
            LoObject::Reference(id) => out
                .get_object(*id)
                .and_then(LoObject::as_dict)
                .expect("annot dict"),
            LoObject::Dictionary(d) => d,
            _ => panic!("unexpected annot entry"),
        };
        let subtype = first_dict
            .get(b"Subtype")
            .and_then(LoObject::as_name)
            .expect("Subtype");
        assert_eq!(subtype, b"Link");
    }

    #[test]
    fn compose_overlay_annotation_mode_none_omits_template_annotations() {
        use std::fs;
        let temp_dir = std::env::temp_dir().join(format!(
            "fullbleed_finalize_compose_ann_mode_none_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        fs::create_dir_all(&temp_dir).expect("mkdir");
        let tpl = temp_dir.join("tpl_link.pdf");
        let overlay = temp_dir.join("overlay.pdf");
        let out_path = temp_dir.join("out_compose.pdf");
        make_single_page_pdf_with_link(&tpl, "TEMPLATE_WITH_LINK");
        make_single_page_pdf(&overlay, "OVERLAY");

        let mut catalog = TemplateCatalog::default();
        catalog
            .insert(TemplateAsset {
                template_id: "tpl".to_string(),
                pdf_path: tpl.clone(),
                sha256: None,
                page_count: Some(1),
            })
            .expect("catalog");
        let plan = vec![ComposePagePlan {
            template_id: "tpl".to_string(),
            template_page_index: 0,
            overlay_page_index: 0,
            dx: 0.0,
            dy: 0.0,
        }];
        compose_overlay_with_template_catalog_with_annotation_mode(
            &catalog,
            &overlay,
            &out_path,
            &plan,
            ComposeAnnotationMode::None,
        )
        .expect("compose");

        let out = LoDocument::load(&out_path).expect("load out");
        let out_page_id = *out
            .get_pages()
            .values()
            .next()
            .expect("output page id");
        assert_eq!(page_annotation_count_by_subtype(&out, out_page_id, b"Link"), 0);
        assert_eq!(page_annotation_count_by_subtype(&out, out_page_id, b"Widget"), 0);
    }

    #[test]
    fn compose_overlay_annotation_mode_carry_widgets_preserves_widget_annotations() {
        use std::fs;
        let temp_dir = std::env::temp_dir().join(format!(
            "fullbleed_finalize_compose_ann_mode_widgets_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        fs::create_dir_all(&temp_dir).expect("mkdir");
        let tpl = temp_dir.join("tpl_link_widget.pdf");
        let overlay = temp_dir.join("overlay.pdf");
        let out_path = temp_dir.join("out_compose.pdf");
        make_single_page_pdf_with_link_and_widget(&tpl, "TEMPLATE_WITH_LINK_WIDGET");
        make_single_page_pdf(&overlay, "OVERLAY");

        let mut catalog = TemplateCatalog::default();
        catalog
            .insert(TemplateAsset {
                template_id: "tpl".to_string(),
                pdf_path: tpl.clone(),
                sha256: None,
                page_count: Some(1),
            })
            .expect("catalog");
        let plan = vec![ComposePagePlan {
            template_id: "tpl".to_string(),
            template_page_index: 0,
            overlay_page_index: 0,
            dx: 0.0,
            dy: 0.0,
        }];
        compose_overlay_with_template_catalog_with_annotation_mode(
            &catalog,
            &overlay,
            &out_path,
            &plan,
            ComposeAnnotationMode::CarryWidgets,
        )
        .expect("compose");

        let out = LoDocument::load(&out_path).expect("load out");
        let out_page_id = *out
            .get_pages()
            .values()
            .next()
            .expect("output page id");
        assert_eq!(page_annotation_count_by_subtype(&out, out_page_id, b"Link"), 1);
        assert_eq!(page_annotation_count_by_subtype(&out, out_page_id, b"Widget"), 1);
    }

    #[test]
    fn compose_overlay_rejects_malformed_overlay_pdf() {
        use std::fs;
        let temp_dir = std::env::temp_dir().join(format!(
            "fullbleed_finalize_compose_malformed_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        fs::create_dir_all(&temp_dir).expect("mkdir");
        let tpl = temp_dir.join("tpl.pdf");
        let overlay = temp_dir.join("overlay_bad.pdf");
        let out_path = temp_dir.join("out.pdf");
        make_single_page_pdf(&tpl, "TEMPLATE");
        let mut f = fs::File::create(&overlay).expect("create");
        f.write_all(b"this is not a pdf").expect("write");

        let mut catalog = TemplateCatalog::default();
        catalog
            .insert(TemplateAsset {
                template_id: "tpl".to_string(),
                pdf_path: tpl.clone(),
                sha256: None,
                page_count: Some(1),
            })
            .expect("catalog");
        let plan = vec![ComposePagePlan {
            template_id: "tpl".to_string(),
            template_page_index: 0,
            overlay_page_index: 0,
            dx: 0.0,
            dy: 0.0,
        }];

        let err = compose_overlay_with_template_catalog(&catalog, &overlay, &out_path, &plan)
            .expect_err("must fail malformed overlay");
        assert!(err.to_string().contains("pdf compose error"));
    }

    #[test]
    fn compose_overlay_structural_signature_is_deterministic() {
        use std::fs;
        let temp_dir = std::env::temp_dir().join(format!(
            "fullbleed_finalize_compose_determinism_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        fs::create_dir_all(&temp_dir).expect("mkdir");
        let tpl = temp_dir.join("tpl.pdf");
        let overlay = temp_dir.join("overlay.pdf");
        let out_a = temp_dir.join("out_a.pdf");
        let out_b = temp_dir.join("out_b.pdf");
        make_single_page_pdf(&tpl, "TEMPLATE");
        make_single_page_pdf(&overlay, "OVERLAY");

        let mut catalog = TemplateCatalog::default();
        catalog
            .insert(TemplateAsset {
                template_id: "tpl".to_string(),
                pdf_path: tpl,
                sha256: None,
                page_count: Some(1),
            })
            .expect("catalog");

        let plan = vec![
            ComposePagePlan {
                template_id: "tpl".to_string(),
                template_page_index: 0,
                overlay_page_index: 0,
                dx: 0.0,
                dy: 0.0,
            },
            ComposePagePlan {
                template_id: "tpl".to_string(),
                template_page_index: 0,
                overlay_page_index: 0,
                dx: 0.0,
                dy: 0.0,
            },
        ];

        compose_overlay_with_template_catalog(&catalog, &overlay, &out_a, &plan)
            .expect("compose a");
        compose_overlay_with_template_catalog(&catalog, &overlay, &out_b, &plan)
            .expect("compose b");

        let sig_a = pdf_structural_signature(&out_a);
        let sig_b = pdf_structural_signature(&out_b);
        assert_eq!(sig_a, sig_b, "structural signatures should match");
    }
}
