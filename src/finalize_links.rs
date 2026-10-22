//! Navigation carried through page composition. Destinations are resolved in
//! their source document, then mapped to real pages in the new output tree.
use super::*;

#[path = "finalize_clip.rs"]
mod clip;

fn invalid(message: &str) -> FullBleedError {
    FullBleedError::InvalidConfiguration(format!("pdf compose navigation: {message}"))
}

#[derive(Default)]
pub(super) struct NamedDestinations {
    legacy: BTreeMap<Vec<u8>, LoObject>,
    tree: BTreeMap<Vec<u8>, LoObject>,
}

impl NamedDestinations {
    pub(super) fn read(doc: &LoDocument) -> Result<Self, FullBleedError> {
        let root = resolve_finalize_object(doc, doc.trailer.get(b"Root").map_err(pdf_err)?)?
            .as_dict()
            .map_err(pdf_err)?;
        let mut out = Self::default();
        if let Ok(dests) = root.get(b"Dests") {
            for (key, value) in resolve_finalize_object(doc, dests)?
                .as_dict()
                .map_err(pdf_err)?
                .iter()
            {
                out.legacy.insert(key.clone(), value.clone());
            }
        }
        if let Ok(names) = root.get(b"Names") {
            let names = resolve_finalize_object(doc, names)?
                .as_dict()
                .map_err(pdf_err)?;
            if let Ok(dests) = names.get(b"Dests") {
                out.read_tree(doc, dests, &mut BTreeSet::new(), 0)?;
            }
        }
        Ok(out)
    }

    fn read_tree(
        &mut self,
        doc: &LoDocument,
        node: &LoObject,
        seen: &mut BTreeSet<LoObjectId>,
        depth: usize,
    ) -> Result<(), FullBleedError> {
        if depth > 64 {
            return Err(invalid("destination name tree is too deep"));
        }
        if let LoObject::Reference(id) = node {
            if !seen.insert(*id) {
                return Err(invalid(
                    "destination name tree contains a cycle or repeated node",
                ));
            }
        }
        let node = resolve_finalize_object(doc, node)?
            .as_dict()
            .map_err(pdf_err)?;
        if let Ok(names) = node.get(b"Names") {
            let names = resolve_finalize_object(doc, names)?
                .as_array()
                .map_err(pdf_err)?;
            if names.len() % 2 != 0 {
                return Err(invalid("destination name tree has an unpaired entry"));
            }
            for pair in names.chunks_exact(2) {
                let key = resolve_finalize_object(doc, &pair[0])?
                    .as_str()
                    .map_err(pdf_err)?;
                self.tree.insert(key.to_vec(), pair[1].clone());
            }
        }
        if let Ok(kids) = node.get(b"Kids") {
            for kid in resolve_finalize_object(doc, kids)?
                .as_array()
                .map_err(pdf_err)?
            {
                self.read_tree(doc, kid, seen, depth + 1)?;
            }
        }
        Ok(())
    }
}

pub(super) struct ImportedDocument {
    pub pages: Vec<LoObjectId>,
    pub destinations: NamedDestinations,
}

#[derive(Clone, Copy)]
pub(super) struct Placement {
    pub output_page: LoObjectId,
    pub dx: f32,
    pub dy: f32,
    pub occurrence: usize,
}

pub(super) type PagePlacements = BTreeMap<LoObjectId, Vec<Placement>>;

pub(super) fn register_placement(
    pages: &mut PagePlacements,
    source_page: LoObjectId,
    output_page: LoObjectId,
    dx: f32,
    dy: f32,
) -> Placement {
    let copies = pages.entry(source_page).or_default();
    let placement = Placement {
        output_page,
        dx,
        dy,
        occurrence: copies.len(),
    };
    copies.push(placement);
    placement
}

struct Context<'a> {
    source_page: LoObjectId,
    placement: Placement,
    pages: &'a PagePlacements,
    destinations: &'a NamedDestinations,
    preserve_semantic_destinations: bool,
}

impl Context<'_> {
    fn target(&self, page: LoObjectId) -> Result<Placement, FullBleedError> {
        if page == self.source_page {
            return Ok(self.placement);
        }
        let targets = self
            .pages
            .get(&page)
            .ok_or_else(|| invalid("local destination page is not included in the compose plan"))?;
        // Repeated documents follow occurrence order: copy N links to copy N.
        // A destination included only once is shared by all source occurrences.
        targets
            .get(if targets.len() == 1 {
                0
            } else {
                self.placement.occurrence
            })
            .copied()
            .ok_or_else(|| invalid("local destination has no matching output occurrence"))
    }

    fn destination(&self, doc: &LoDocument, value: &LoObject) -> Result<LoObject, FullBleedError> {
        let mut current = value;
        for _ in 0..64 {
            current = resolve_finalize_object(doc, current)?;
            match current {
                LoObject::Name(name) => {
                    current = self.destinations.legacy.get(name).ok_or_else(|| {
                        invalid("named destination is not defined in its source document")
                    })?
                }
                LoObject::String(name, _) => {
                    current = self.destinations.tree.get(name).ok_or_else(|| {
                        invalid("named destination is not defined in its source document")
                    })?
                }
                LoObject::Dictionary(dict) => current = dict.get(b"D").map_err(pdf_err)?,
                LoObject::Array(array) => {
                    let mut out = array.clone();
                    let page = out
                        .first()
                        .ok_or_else(|| invalid("empty local destination"))?
                        .as_reference()
                        .map_err(pdf_err)?;
                    let target = self.target(page)?;
                    let kind = out
                        .get(1)
                        .ok_or_else(|| invalid("local destination has no view type"))?
                        .as_name()
                        .map_err(pdf_err)?;
                    let allow_null = kind != b"FitR";
                    let (size, coordinates): (usize, &[(usize, f32)]) = match kind {
                        b"XYZ" => (5, &[(2, target.dx), (3, target.dy)]),
                        b"Fit" | b"FitB" => (2, &[]),
                        b"FitH" | b"FitBH" => (3, &[(2, target.dy)]),
                        b"FitV" | b"FitBV" => (3, &[(2, target.dx)]),
                        b"FitR" => (
                            6,
                            &[
                                (2, target.dx),
                                (3, target.dy),
                                (4, target.dx),
                                (5, target.dy),
                            ],
                        ),
                        _ => return Err(invalid("unsupported local destination view type")),
                    };
                    if out.len() != size {
                        return Err(invalid("local destination has an invalid coordinate count"));
                    }
                    out[0] = LoObject::Reference(target.output_page);
                    for &(index, delta) in coordinates {
                        out[index] = translate_number(doc, &out[index], delta, allow_null)?;
                    }
                    return Ok(LoObject::Array(out));
                }
                _ => return Err(invalid("invalid local destination")),
            }
        }
        Err(invalid(
            "local destination contains a cycle or exceeds the nesting limit",
        ))
    }

    fn action(
        &self,
        doc: &LoDocument,
        value: &LoObject,
        depth: usize,
    ) -> Result<LoObject, FullBleedError> {
        if depth > 64 {
            return Err(invalid(
                "action chain contains a cycle or exceeds the nesting limit",
            ));
        }
        let mut action = resolve_finalize_object(doc, value)?
            .as_dict()
            .map_err(pdf_err)?
            .clone();
        if action
            .get(b"S")
            .is_ok_and(|s| object_is_name(doc, s, b"GoTo"))
        {
            action.set(
                "D",
                self.destination(doc, action.get(b"D").map_err(pdf_err)?)?,
            );
            if !self.preserve_semantic_destinations {
                // Imported overlay structures have no owner in the output.
                action.remove(b"SD");
            }
        }
        if let Ok(next) = action.get(b"Next") {
            let next = match resolve_finalize_object(doc, next)? {
                LoObject::Array(items) => LoObject::Array(
                    items
                        .iter()
                        .map(|item| self.action(doc, item, depth + 1))
                        .collect::<Result<_, _>>()?,
                ),
                _ => self.action(doc, next, depth + 1)?,
            };
            action.set("Next", next);
        }
        Ok(LoObject::Dictionary(action))
    }
}

fn translate_number(
    doc: &LoDocument,
    value: &LoObject,
    delta: f32,
    allow_null: bool,
) -> Result<LoObject, FullBleedError> {
    let value = resolve_finalize_object(doc, value)?;
    if allow_null && matches!(value, LoObject::Null) {
        return Ok(LoObject::Null);
    }
    let translated = value.as_float().map_err(pdf_err)? + delta;
    if !translated.is_finite() {
        return Err(invalid(
            "annotation or destination coordinate is not finite",
        ));
    }
    Ok(if delta == 0.0 {
        value.clone()
    } else {
        LoObject::Real(translated)
    })
}

pub(super) fn clone_annotations(
    doc: &mut LoDocument,
    source_page: LoObjectId,
    placement: Placement,
    mode: ComposeAnnotationMode,
    pages: &PagePlacements,
    destinations: &NamedDestinations,
    output_box: &[LoObject],
) -> Result<Vec<LoObject>, FullBleedError> {
    if mode == ComposeAnnotationMode::None {
        return Ok(Vec::new());
    }
    let page = doc
        .get_object(source_page)
        .and_then(LoObject::as_dict)
        .map_err(pdf_err)?;
    let Ok(annots) = page.get(b"Annots") else {
        return Ok(Vec::new());
    };
    let annots = resolve_finalize_object(doc, annots)?
        .as_array()
        .map_err(pdf_err)?
        .clone();
    let context = Context {
        source_page,
        placement,
        pages,
        destinations,
        preserve_semantic_destinations: false,
    };
    let mut source_clip = clip::bounds(doc, &page_box(doc, source_page))?;
    for i in 0..4 {
        source_clip[i] += if i % 2 == 0 {
            placement.dx as f64
        } else {
            placement.dy as f64
        };
    }
    let output_clip = clip::bounds(doc, output_box)?;
    let mut output = Vec::new();
    for annot in annots {
        let mut cloned = match resolve_finalize_object(doc, &annot)? {
            LoObject::Dictionary(dict) => dict.clone(),
            LoObject::Stream(stream) => stream.dict.clone(),
            _ => continue,
        };
        if !annotation_allowed_for_mode(mode, doc, &cloned) {
            continue;
        }
        for key in [b"Rect".as_slice(), b"QuadPoints".as_slice()] {
            if let Ok(value) = cloned.get(key) {
                let values = resolve_finalize_object(doc, value)?
                    .as_array()
                    .map_err(pdf_err)?;
                if (key == b"Rect" && values.len() != 4)
                    || (key == b"QuadPoints" && (values.is_empty() || values.len() % 8 != 0))
                {
                    return Err(invalid("annotation has an invalid geometry array"));
                }
                let translated = values
                    .iter()
                    .enumerate()
                    .map(|(i, value)| {
                        translate_number(
                            doc,
                            value,
                            if i % 2 == 0 {
                                placement.dx
                            } else {
                                placement.dy
                            },
                            false,
                        )
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                cloned.set(key, LoObject::Array(translated));
            }
        }
        if !clip::annotation(doc, &mut cloned, source_clip, output_clip)? {
            continue;
        }
        if let Ok(dest) = cloned.get(b"Dest") {
            cloned.set("Dest", context.destination(doc, dest)?);
        }
        if let Ok(action) = cloned.get(b"A") {
            cloned.set("A", context.action(doc, action, 0)?);
        }
        if let Ok(actions) = cloned.get(b"AA") {
            let actions = resolve_finalize_object(doc, actions)?
                .as_dict()
                .map_err(pdf_err)?;
            let mut translated = LoDictionary::new();
            for (key, action) in actions.iter() {
                translated.set(key, context.action(doc, action, 0)?);
            }
            cloned.set("AA", translated);
        }
        cloned.remove(b"StructParent");
        cloned.set("P", LoObject::Reference(placement.output_page));
        let id = doc.add_object(cloned);
        output.push(LoObject::Reference(id));
    }
    Ok(output)
}

/// Normalize the retained template's names without replacing its annotation
/// objects, structure owners, or semantic destinations. Readers otherwise can
/// conflate a legacy name and a byte-string name-tree entry with the same text.
pub(super) fn normalize_template_navigation(doc: &mut LoDocument) -> Result<(), FullBleedError> {
    let page_ids: Vec<_> = doc.get_pages().into_values().collect();
    let destinations = NamedDestinations::read(doc)?;
    let mut pages = PagePlacements::new();
    for &page in &page_ids {
        register_placement(&mut pages, page, page, 0.0, 0.0);
    }
    for page_id in page_ids {
        let page = doc
            .get_object(page_id)
            .and_then(LoObject::as_dict)
            .map_err(pdf_err)?;
        let Ok(entries) = page.get(b"Annots") else {
            continue;
        };
        let mut entries = resolve_finalize_object(doc, entries)?
            .as_array()
            .map_err(pdf_err)?
            .clone();
        let context = Context {
            source_page: page_id,
            placement: pages[&page_id][0],
            pages: &pages,
            destinations: &destinations,
            preserve_semantic_destinations: true,
        };
        for entry in &mut entries {
            let mut target = &*entry;
            let mut target_id = None;
            let mut seen = BTreeSet::new();
            while let LoObject::Reference(id) = target {
                if !seen.insert(*id) {
                    return Err(invalid("cyclic template annotation reference"));
                }
                target_id = Some(*id);
                target = doc.get_object(*id).map_err(pdf_err)?;
            }
            let mut annotation = match target {
                LoObject::Dictionary(value) => value.clone(),
                LoObject::Stream(value) => value.dict.clone(),
                _ => continue,
            };
            if !annotation_allowed_for_mode(ComposeAnnotationMode::LinkOnly, doc, &annotation) {
                continue;
            }
            if let Ok(destination) = annotation.get(b"Dest") {
                annotation.set("Dest", context.destination(doc, destination)?);
            }
            if let Ok(action) = annotation.get(b"A") {
                annotation.set("A", context.action(doc, action, 0)?);
            }
            if let Ok(actions) = annotation.get(b"AA") {
                let actions = resolve_finalize_object(doc, actions)?
                    .as_dict()
                    .map_err(pdf_err)?;
                let mut normalized = LoDictionary::new();
                for (key, action) in actions.iter() {
                    normalized.set(key, context.action(doc, action, 0)?);
                }
                annotation.set("AA", normalized);
            }
            if let Some(id) = target_id {
                doc.objects.insert(id, LoObject::Dictionary(annotation));
            } else {
                *entry = LoObject::Dictionary(annotation);
            }
        }
        doc.get_object_mut(page_id)
            .and_then(LoObject::as_dict_mut)
            .map_err(pdf_err)?
            .set("Annots", LoObject::Array(entries));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pdf_native::StringFormat;

    fn string(value: &str) -> LoObject {
        LoObject::String(value.as_bytes().to_vec(), StringFormat::Literal)
    }

    fn fixture(path: &std::path::Path, label: &str) {
        let mut doc = LoDocument::with_version("1.7");
        let parent = doc.new_object_id();
        let pages = [doc.new_object_id(), doc.new_object_id()];
        for (index, &page) in pages.iter().enumerate() {
            let cross = vec![pages[1 - index].into(), "FitH".into(), 70.into()];
            let actions = [
                (
                    "uri",
                    dictionary! {"A" => dictionary! {
                        "S" => "URI", "URI" => string(&format!("https://example.invalid/{label}/{index}")),
                        "Next" => vec![LoObject::Dictionary(dictionary! {"S" => "GoTo", "D" => cross})],
                    }},
                ),
                (
                    "self",
                    dictionary! {"Dest" => vec![page.into(), "XYZ".into(), 20.into(), 30.into(), LoObject::Null]},
                ),
                (
                    "tree",
                    dictionary! {"A" => dictionary! {"S" => "GoTo", "D" => string("details"), "SD" => parent}},
                ),
                (
                    "legacy",
                    dictionary! {"Dest" => LoObject::Name(b"details".to_vec())},
                ),
                (
                    "remote",
                    dictionary! {"A" => dictionary! {"S" => "GoToR", "D" => vec![0.into(), "Fit".into()], "F" => string("other.pdf")}},
                ),
            ];
            let mut annots = Vec::new();
            for (kind, mut annotation) in actions {
                annotation.set("Type", "Annot");
                annotation.set("Subtype", "Link");
                annotation.set("Contents", string(&format!("{label}-{kind}")));
                annotation.set("Rect", vec![10.into(), 20.into(), 50.into(), 40.into()]);
                annotation.set(
                    "QuadPoints",
                    vec![
                        10.into(),
                        40.into(),
                        50.into(),
                        40.into(),
                        50.into(),
                        20.into(),
                        10.into(),
                        20.into(),
                    ],
                );
                annotation.set("Border", vec![0.into(), 0.into(), 0.into()]);
                annotation.set("P", page);
                annotation.set("StructParent", 0);
                annots.push(doc.add_object(annotation).into());
            }
            annots.push(
                doc.add_object(dictionary! {
                    "Type" => "Annot", "Subtype" => "Widget", "FT" => "Tx", "T" => string(label),
                    "Rect" => vec![10.into(), 60.into(), 50.into(), 80.into()], "P" => page,
                })
                .into(),
            );
            let content = doc.add_object(LoStream::new(
                dictionary! {},
                if label == "template" {
                    b"0 0.5 0 rg 10 20 40 20 re f".to_vec()
                } else {
                    b"0.2 0.2 0.8 rg 10 20 40 20 re f".to_vec()
                },
            ));
            doc.objects.insert(
                page,
                dictionary! {
                    "Type" => "Page", "Parent" => parent, "Contents" => content,
                    "MediaBox" => vec![0.into(), 0.into(), 200.into(), 200.into()],
                    "Resources" => dictionary! {}, "Annots" => annots,
                }
                .into(),
            );
        }
        doc.objects.insert(parent, dictionary! {"Type" => "Pages", "Kids" => vec![pages[0].into(), pages[1].into()], "Count" => 2}.into());
        let named = doc.add_object(dictionary! {
            "D" => vec![pages[1].into(), "XYZ".into(), 80.into(), 90.into(), 2.into()],
        });
        let leaf = doc.add_object(dictionary! {"Names" => vec![string("details"), named.into()]});
        let root = doc.add_object(dictionary! {
            "Type" => "Catalog", "Pages" => parent,
            "Names" => dictionary! {"Dests" => dictionary! {"Kids" => vec![leaf.into()]}},
            "Dests" => dictionary! {"details" => vec![pages[0].into(), "FitR".into(), 10.into(), 20.into(), 50.into(), 60.into()]},
        });
        doc.trailer.set("Root", root);
        doc.save(path).unwrap();
    }

    fn workspace(name: &str) -> PathBuf {
        let root = std::env::var_os("FULLBLEED_COMPOSE_TEST_OUTPUT")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                std::env::temp_dir().join(format!("fullbleed-compose-{}", std::process::id()))
            });
        let path = root.join(name);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn setup(path: &std::path::Path) -> (TemplateCatalog, PathBuf) {
        let template = path.join("template.pdf");
        let overlay = path.join("overlay.pdf");
        fixture(&template, "template");
        fixture(&overlay, "overlay");
        let mut catalog = TemplateCatalog::default();
        catalog
            .insert(TemplateAsset {
                template_id: "tpl".into(),
                pdf_path: template,
                sha256: None,
                page_count: Some(2),
            })
            .unwrap();
        (catalog, overlay)
    }

    fn numbers(values: &LoObject) -> Vec<f32> {
        values
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_float().unwrap())
            .collect()
    }

    #[test]
    fn compose_navigation_preserves_links_offsets_named_destinations_and_repeated_pages() {
        let dir = workspace("links");
        let (catalog, overlay) = setup(&dir);
        let plan: Vec<_> = [
            (1, 12.0, 18.0),
            (0, -8.0, 22.0),
            (1, 30.0, -4.0),
            (0, 40.0, 70.0),
        ]
        .into_iter()
        .map(|(index, dx, dy)| ComposePagePlan {
            template_id: "tpl".into(),
            template_page_index: index,
            overlay_page_index: index,
            dx,
            dy,
        })
        .collect();
        for (mode, template_links, widgets) in [
            (ComposeAnnotationMode::LinkOnly, 5, 0),
            (ComposeAnnotationMode::None, 0, 0),
            (ComposeAnnotationMode::CarryWidgets, 5, 1),
        ] {
            let out = dir.join(format!("{mode:?}.pdf"));
            compose_overlay_with_template_catalog_with_annotation_mode(
                &catalog, &overlay, &out, &plan, mode,
            )
            .unwrap();
            let replay = dir.join(format!("{mode:?}-replay.pdf"));
            compose_overlay_with_template_catalog_with_annotation_mode(
                &catalog, &overlay, &replay, &plan, mode,
            )
            .unwrap();
            assert_eq!(std::fs::read(&out).unwrap(), std::fs::read(replay).unwrap());
            let doc = LoDocument::load(out).unwrap();
            let pages: Vec<_> = doc.get_pages().into_values().collect();
            assert_eq!(pages.len(), 4);
            for (index, &page) in pages.iter().enumerate() {
                let page_dict = doc.get_object(page).unwrap().as_dict().unwrap();
                let annots = page_dict.get(b"Annots").unwrap().as_array().unwrap();
                assert_eq!(annots.len(), template_links + 5 + widgets);
                let mut widget_count = 0;
                for annotation in annots {
                    let annotation = resolve_finalize_object(&doc, annotation)
                        .unwrap()
                        .as_dict()
                        .unwrap();
                    assert_eq!(annotation.get(b"P").unwrap().as_reference().unwrap(), page);
                    assert!(annotation.get(b"StructParent").is_err());
                    if object_is_name(&doc, annotation.get(b"Subtype").unwrap(), b"Widget") {
                        widget_count += 1;
                        continue;
                    }
                    let label =
                        std::str::from_utf8(annotation.get(b"Contents").unwrap().as_str().unwrap())
                            .unwrap();
                    let is_overlay = label.starts_with("overlay-");
                    let (dx, dy) = if is_overlay {
                        (plan[index].dx, plan[index].dy)
                    } else {
                        (0.0, 0.0)
                    };
                    assert_eq!(
                        numbers(annotation.get(b"Rect").unwrap()),
                        vec![10.0 + dx, 20.0 + dy, 50.0 + dx, 40.0 + dy]
                    );
                    assert_eq!(
                        numbers(annotation.get(b"QuadPoints").unwrap()),
                        vec![
                            10.0 + dx,
                            40.0 + dy,
                            50.0 + dx,
                            40.0 + dy,
                            50.0 + dx,
                            20.0 + dy,
                            10.0 + dx,
                            20.0 + dy
                        ]
                    );
                    let kind = label.split_once('-').unwrap().1;
                    let mut target_index = index;
                    let destination = match kind {
                        "self" => annotation.get(b"Dest").unwrap(),
                        "legacy" => {
                            target_index = index / 2 * 2 + 1;
                            annotation.get(b"Dest").unwrap()
                        }
                        "tree" => {
                            target_index = index / 2 * 2;
                            let action = annotation.get(b"A").unwrap().as_dict().unwrap();
                            assert!(action.get(b"SD").is_err());
                            action.get(b"D").unwrap()
                        }
                        "uri" => {
                            let action = annotation.get(b"A").unwrap().as_dict().unwrap();
                            assert_eq!(
                                action.get(b"URI").unwrap().as_str().unwrap(),
                                format!(
                                    "https://example.invalid/{}/{}",
                                    if is_overlay { "overlay" } else { "template" },
                                    plan[index].overlay_page_index
                                )
                                .as_bytes()
                            );
                            target_index = index ^ 1;
                            action.get(b"Next").unwrap().as_array().unwrap()[0]
                                .as_dict()
                                .unwrap()
                                .get(b"D")
                                .unwrap()
                        }
                        "remote" => {
                            let action = annotation.get(b"A").unwrap().as_dict().unwrap();
                            assert_eq!(
                                action.get(b"D").unwrap(),
                                &LoObject::Array(vec![0.into(), "Fit".into()])
                            );
                            continue;
                        }
                        _ => panic!("unexpected link"),
                    }
                    .as_array()
                    .unwrap();
                    assert_eq!(destination[0].as_reference().unwrap(), pages[target_index]);
                    let (tx, ty) = if is_overlay {
                        (plan[target_index].dx, plan[target_index].dy)
                    } else {
                        (0.0, 0.0)
                    };
                    match kind {
                        "self" => {
                            assert_eq!(destination[2].as_float().unwrap(), 20.0 + tx);
                            assert_eq!(destination[3].as_float().unwrap(), 30.0 + ty);
                            assert_eq!(destination[4], LoObject::Null);
                        }
                        "tree" => {
                            assert_eq!(destination[2].as_float().unwrap(), 80.0 + tx);
                            assert_eq!(destination[3].as_float().unwrap(), 90.0 + ty);
                            assert_eq!(destination[4], 2.into());
                        }
                        "legacy" => assert_eq!(
                            numbers(&LoObject::Array(destination[2..].to_vec())),
                            vec![10.0 + tx, 20.0 + ty, 50.0 + tx, 60.0 + ty]
                        ),
                        "uri" => assert_eq!(destination[2].as_float().unwrap(), 70.0 + ty),
                        _ => unreachable!(),
                    }
                }
                assert_eq!(widget_count, widgets);
            }
        }
    }

    #[test]
    fn compose_navigation_rejects_missing_pages_and_nonfinite_offsets_before_writing() {
        let dir = workspace("errors");
        let (catalog, overlay) = setup(&dir);
        let output = dir.join("keep.pdf");
        std::fs::write(&output, b"existing-output").unwrap();
        let mut plan = vec![ComposePagePlan {
            template_id: "tpl".into(),
            template_page_index: 0,
            overlay_page_index: 0,
            dx: 0.0,
            dy: 0.0,
        }];
        let error =
            compose_overlay_with_template_catalog(&catalog, &overlay, &output, &plan).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("not included in the compose plan"),
            "{error}"
        );
        plan[0].dx = f32::NAN;
        let error =
            compose_overlay_with_template_catalog(&catalog, &overlay, &output, &plan).unwrap_err();
        assert!(
            error.to_string().contains("non-finite placement"),
            "{error}"
        );
        assert_eq!(std::fs::read(output).unwrap(), b"existing-output");
    }

    #[test]
    fn compose_and_stamp_clip_link_activation_without_rescaling_appearance() {
        for kind in [
            "plain",
            "appearance",
            "extension",
            "states",
            "border",
            "rounded",
            "beveled",
            "inset",
            "underline",
            "inactive-artwork",
            "outside",
        ] {
            let dir = workspace(&format!("crop-{kind}"));
            let (catalog, overlay) = setup(&dir);
            let template_path = &catalog.by_id["tpl"].pdf_path;
            let mut template = LoDocument::load(template_path).unwrap();
            let empty = template.add_object(LoStream::new(dictionary! {}, Vec::new()));
            for page in template.get_pages().into_values() {
                let page = template
                    .get_object_mut(page)
                    .unwrap()
                    .as_dict_mut()
                    .unwrap();
                page.remove(b"Annots");
                page.set("Contents", empty);
            }
            template.save(template_path).unwrap();
            let mut source = LoDocument::load(&overlay).unwrap();
            let page_id = *source.get_pages().values().next().unwrap();
            let page = source.get_object(page_id).unwrap().as_dict().unwrap();
            let reference = page.get(b"Annots").unwrap().as_array().unwrap()[0].clone();
            let mut annotation = resolve_finalize_object(&source, &reference)
                .unwrap()
                .as_dict()
                .unwrap()
                .clone();
            let action = annotation.get(b"A").unwrap().as_dict().unwrap().clone();
            let mut action = action;
            action.remove(b"Next");
            annotation.set("A", action);
            let appearance = source.add_object(LoStream::new(dictionary! {
                "Type" => "XObject", "Subtype" => "Form", "BBox" => vec![0.into(),0.into(),40.into(),20.into()],
                "Matrix" => vec![0.into(),1.into(),(-1).into(),0.into(),20.into(),0.into()],
                "Resources" => dictionary! {},
            },b"1 0 0 rg 0 0 20 20 re f 0 1 1 rg 20 0 20 20 re f 0 1 0 rg 0 0 40 5 re f".to_vec()));
            match kind {
                "appearance" | "extension" | "inactive-artwork" => annotation.set(
                    "AP",
                    dictionary! {"N" => appearance, "R" => appearance, "D" => appearance},
                ),
                "states" => {
                    annotation.set(
                        "AP",
                        dictionary! {"N" => dictionary! {"on" => appearance}, "R" => appearance},
                    );
                    annotation.set("AS", "on");
                }
                "border" | "beveled" | "inset" | "underline" => {
                    let style = match kind {
                        "border" => "D",
                        "beveled" => "B",
                        "inset" => "I",
                        _ => "U",
                    };
                    annotation.set(
                        "BS",
                        dictionary! {"W" => 2, "S" => style, "D" => vec![3.into(),2.into()]},
                    );
                    annotation.set("C", vec![1.into(), 0.into(), 0.into()]);
                }
                "rounded" => annotation.set("Border", vec![6.into(), 4.into(), 2.into()]),
                _ => (),
            }
            if kind == "extension" {
                let mut appearance_dictionary =
                    annotation.get(b"AP").unwrap().as_dict().unwrap().clone();
                appearance_dictionary.set("FBExtension", 42);
                annotation.set("AP", appearance_dictionary);
            }
            if kind == "inactive-artwork" {
                annotation.set(
                    "QuadPoints",
                    vec![
                        10.into(),
                        20.into(),
                        20.into(),
                        20.into(),
                        20.into(),
                        40.into(),
                        10.into(),
                        40.into(),
                    ],
                );
            }
            let annot_id = source.add_object(annotation);
            let crop = source.add_object(LoObject::Array(vec![
                (if kind == "outside" { 60 } else { 30 }).into(),
                0.into(),
                200.into(),
                200.into(),
            ]));
            let page = source
                .get_object_mut(page_id)
                .unwrap()
                .as_dict_mut()
                .unwrap();
            page.set("Annots", vec![annot_id.into()]);
            page.set("CropBox", crop);
            source.save(&overlay).unwrap();
            let plan = [ComposePagePlan {
                template_id: "tpl".into(),
                template_page_index: 0,
                overlay_page_index: 0,
                dx: 12.0,
                dy: 18.0,
            }];
            for method in ["compose", "stamp"] {
                let path = dir.join(format!("{method}.pdf"));
                if method == "compose" {
                    compose_overlay_with_template_catalog_with_annotation_mode(
                        &catalog,
                        &overlay,
                        &path,
                        &plan,
                        ComposeAnnotationMode::None,
                    )
                    .unwrap();
                } else {
                    stamp_overlay_on_template_pdf(
                        template_path,
                        &overlay,
                        &path,
                        Some(&[(0, 0)]),
                        12.0,
                        18.0,
                    )
                    .unwrap();
                }
                let output = LoDocument::load(path).unwrap();
                let page_id = *output.get_pages().values().next().unwrap();
                let page = output.get_object(page_id).unwrap().as_dict().unwrap();
                if kind == "outside" {
                    assert!(page.get(b"Annots").is_err());
                    continue;
                }
                let annots = page.get(b"Annots").unwrap().as_array().unwrap();
                assert_eq!(annots.len(), 1);
                let annotation = resolve_finalize_object(&output, &annots[0])
                    .unwrap()
                    .as_dict()
                    .unwrap();
                assert_eq!(
                    numbers(annotation.get(b"Rect").unwrap()),
                    vec![42.0, 38.0, 62.0, 58.0]
                );
                let is_inactive = kind == "inactive-artwork";
                assert_eq!(
                    annotation.get(b"Subtype").unwrap().as_name().unwrap(),
                    if is_inactive {
                        b"Stamp".as_slice()
                    } else {
                        b"Link".as_slice()
                    }
                );
                assert_eq!(annotation.get(b"A").is_err(), is_inactive);
                if !is_inactive {
                    let points = numbers(annotation.get(b"QuadPoints").unwrap());
                    assert!(
                        points
                            .chunks_exact(2)
                            .all(|p| p[0] >= 42.0 && p[0] <= 62.0 && p[1] >= 38.0 && p[1] <= 58.0)
                    );
                }
                let appearances = annotation.get(b"AP").unwrap().as_dict().unwrap();
                if kind == "extension" {
                    assert_eq!(
                        appearances.get(b"FBExtension").unwrap(),
                        &LoObject::Integer(42)
                    );
                }
                let mut normal =
                    resolve_finalize_object(&output, appearances.get(b"N").unwrap()).unwrap();
                if let LoObject::Dictionary(states) = normal {
                    normal = resolve_finalize_object(&output, states.get(b"on").unwrap()).unwrap();
                }
                assert_eq!(
                    numbers(normal.as_stream().unwrap().dict.get(b"BBox").unwrap()),
                    vec![42.0, 38.0, 62.0, 58.0]
                );
            }
        }
    }

    #[test]
    fn stamp_navigation_preserves_template_and_overlay_links_with_repeated_placements() {
        let dir = workspace("stamp");
        let (catalog, overlay) = setup(&dir);
        let template = &catalog.by_id["tpl"].pdf_path;
        let mapping = [(0, 0), (0, 1), (0, 0), (1, 1)];
        let out = dir.join("stamp.pdf");
        let replay = dir.join("stamp-replay.pdf");
        for path in [&out, &replay] {
            stamp_overlay_on_template_pdf(template, &overlay, path, Some(&mapping), 12.0, 18.0)
                .unwrap();
        }
        assert_eq!(std::fs::read(&out).unwrap(), std::fs::read(replay).unwrap());
        let doc = LoDocument::load(out).unwrap();
        let pages: Vec<_> = doc.get_pages().into_values().collect();
        assert_eq!(pages.len(), 2);
        for (index, &page_id) in pages.iter().enumerate() {
            let page = doc.get_object(page_id).unwrap().as_dict().unwrap();
            let annots = page.get(b"Annots").unwrap().as_array().unwrap();
            assert_eq!(annots.len(), if index == 0 { 21 } else { 11 });
            let mut overlay_uri_targets = Vec::new();
            let mut template_links = 0;
            for reference in annots {
                let annotation = resolve_finalize_object(&doc, reference)
                    .unwrap()
                    .as_dict()
                    .unwrap();
                if object_is_name(&doc, annotation.get(b"Subtype").unwrap(), b"Widget") {
                    continue;
                }
                let label = annotation.get(b"Contents").unwrap().as_str().unwrap();
                assert_eq!(
                    annotation.get(b"P").unwrap().as_reference().unwrap(),
                    page_id
                );
                if label.starts_with(b"template-") {
                    template_links += 1;
                    assert_eq!(
                        annotation.get(b"StructParent").unwrap(),
                        &LoObject::Integer(0)
                    );
                    if label == b"template-legacy" {
                        assert_eq!(
                            annotation.get(b"Dest").unwrap().as_array().unwrap()[0]
                                .as_reference()
                                .unwrap(),
                            pages[0]
                        );
                    }
                    if label == b"template-tree" {
                        let action = annotation.get(b"A").unwrap().as_dict().unwrap();
                        assert_eq!(
                            action.get(b"D").unwrap().as_array().unwrap()[0]
                                .as_reference()
                                .unwrap(),
                            pages[1]
                        );
                        assert!(action.get(b"SD").is_ok());
                    }
                    assert_eq!(
                        numbers(annotation.get(b"Rect").unwrap()),
                        vec![10.0, 20.0, 50.0, 40.0]
                    );
                    continue;
                }
                assert_eq!(
                    numbers(annotation.get(b"Rect").unwrap()),
                    vec![22.0, 38.0, 62.0, 58.0]
                );
                if label == b"overlay-uri" {
                    let action = annotation.get(b"A").unwrap().as_dict().unwrap();
                    let next = &action.get(b"Next").unwrap().as_array().unwrap()[0];
                    let destination = next
                        .as_dict()
                        .unwrap()
                        .get(b"D")
                        .unwrap()
                        .as_array()
                        .unwrap();
                    overlay_uri_targets.push(destination[0].as_reference().unwrap());
                    assert_eq!(destination[2].as_float().unwrap(), 88.0);
                }
            }
            assert_eq!(template_links, 5);
            assert_eq!(
                overlay_uri_targets,
                if index == 0 {
                    vec![pages[0], pages[0], pages[1]]
                } else {
                    vec![pages[0]]
                }
            );
        }
    }

    #[test]
    fn compose_navigation_destination_views_preserve_nulls_and_translate_target_coordinates() {
        let doc = LoDocument::with_version("1.7");
        let names = NamedDestinations::default();
        let current = Placement {
            output_page: (11, 0),
            dx: 2.0,
            dy: 3.0,
            occurrence: 0,
        };
        let target = Placement {
            output_page: (12, 0),
            dx: 12.0,
            dy: 18.0,
            occurrence: 0,
        };
        let pages = BTreeMap::from([((1, 0), vec![current]), ((2, 0), vec![target])]);
        let context = Context {
            source_page: (1, 0),
            placement: current,
            pages: &pages,
            destinations: &names,
            preserve_semantic_destinations: false,
        };
        for (kind, values, expected) in [
            (
                "XYZ",
                vec![LoObject::Null, 50.into(), 2.into()],
                vec![LoObject::Null, 68.into(), 2.into()],
            ),
            ("Fit", vec![], vec![]),
            ("FitB", vec![], vec![]),
            ("FitH", vec![50.into()], vec![68.into()]),
            ("FitBH", vec![LoObject::Null], vec![LoObject::Null]),
            ("FitV", vec![50.into()], vec![62.into()]),
            ("FitBV", vec![LoObject::Null], vec![LoObject::Null]),
            (
                "FitR",
                vec![1.into(), 2.into(), 3.into(), 4.into()],
                vec![13.into(), 20.into(), 15.into(), 22.into()],
            ),
        ] {
            let mut destination = vec![LoObject::Reference((2, 0)), kind.into()];
            destination.extend(values);
            let result = context
                .destination(&doc, &LoObject::Array(destination))
                .unwrap();
            let result = result.as_array().unwrap();
            assert_eq!(result[0], LoObject::Reference((12, 0)));
            for (actual, expected) in result[2..].iter().zip(expected) {
                if expected == LoObject::Null {
                    assert_eq!(*actual, expected);
                } else {
                    assert_eq!(actual.as_float().unwrap(), expected.as_float().unwrap());
                }
            }
        }
    }

    #[test]
    fn compose_navigation_rejects_cyclic_objects_names_and_actions() {
        let mut doc = LoDocument::with_version("1.7");
        let cycle = doc.new_object_id();
        doc.objects.insert(cycle, LoObject::Reference(cycle));
        assert!(resolve_finalize_object(&doc, &LoObject::Reference(cycle)).is_err());
        let mut names = NamedDestinations::default();
        names.tree.insert(b"cycle".to_vec(), string("cycle"));
        let placement = Placement {
            output_page: (11, 0),
            dx: 0.0,
            dy: 0.0,
            occurrence: 0,
        };
        let pages = BTreeMap::from([((10, 0), vec![placement])]);
        let context = Context {
            source_page: (10, 0),
            placement,
            pages: &pages,
            destinations: &names,
            preserve_semantic_destinations: false,
        };
        assert!(
            context
                .destination(&doc, &string("cycle"))
                .unwrap_err()
                .to_string()
                .contains("cycle")
        );
        let action = doc.new_object_id();
        doc.objects.insert(action, dictionary! {"S" => "URI", "URI" => string("https://example.invalid"), "Next" => action}.into());
        assert!(
            context
                .action(&doc, &LoObject::Reference(action), 0)
                .unwrap_err()
                .to_string()
                .contains("cycle")
        );
    }
}
