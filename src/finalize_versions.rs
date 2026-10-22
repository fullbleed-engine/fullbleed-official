//! Preserve the declarations needed to interpret imported page content.
use super::*;

fn invalid(message: &str) -> FullBleedError {
    FullBleedError::InvalidConfiguration(format!("pdf compose version: {message}"))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Version(u8, u8);

impl Version {
    fn parse(value: &[u8]) -> Result<Self, FullBleedError> {
        match value {
            [major, b'.', minor] if major.is_ascii_digit() && minor.is_ascii_digit() => {
                Ok(Self(major - b'0', minor - b'0'))
            }
            _ => Err(invalid("expected a version in major.minor form")),
        }
    }

    fn text(self) -> String {
        format!("{}.{}", self.0, self.1)
    }
}

/// Materialize bounded extension declarations before the input object numbers
/// change. All objects in an output Extensions dictionary must be direct.
fn direct_extension(
    doc: &LoDocument,
    value: &LoObject,
    depth: usize,
    remaining: &mut usize,
) -> Result<LoObject, FullBleedError> {
    if depth > 64 || *remaining == 0 {
        return Err(invalid("extension declarations are too deep or too large"));
    }
    *remaining -= 1;
    Ok(match resolve_finalize_object(doc, value)? {
        LoObject::Array(values) => LoObject::Array(
            values
                .iter()
                .map(|value| direct_extension(doc, value, depth + 1, remaining))
                .collect::<Result<_, _>>()?,
        ),
        LoObject::Dictionary(values) => {
            let mut result = LoDictionary::new();
            for (key, value) in values.iter() {
                result.set(key, direct_extension(doc, value, depth + 1, remaining)?);
            }
            LoObject::Dictionary(result)
        }
        LoObject::Stream(_) => {
            return Err(invalid("extension declarations cannot contain streams"));
        }
        // Literal and hexadecimal strings with identical bytes declare the
        // same requirement; keep their encoding while normalizing PDF syntax.
        LoObject::String(value, _) => {
            LoObject::String(value.clone(), crate::pdf_native::StringFormat::Literal)
        }
        value => value.clone(),
    })
}

pub(super) struct Requirements {
    version: Version,
    extensions: BTreeMap<Vec<u8>, Vec<LoObject>>,
    arrays: BTreeSet<Vec<u8>>,
    extension_type: bool,
}

impl Requirements {
    pub(super) fn new(version: &str) -> Result<Self, FullBleedError> {
        Ok(Self {
            version: Version::parse(version.as_bytes())?,
            extensions: BTreeMap::new(),
            arrays: BTreeSet::new(),
            extension_type: false,
        })
    }

    pub(super) fn read(doc: &LoDocument) -> Result<Self, FullBleedError> {
        let mut result = Self::new(&doc.version)?;
        result.include(doc)?;
        Ok(result)
    }

    pub(super) fn include(&mut self, doc: &LoDocument) -> Result<(), FullBleedError> {
        self.version = self.version.max(Version::parse(doc.version.as_bytes())?);
        let catalog = resolve_finalize_object(doc, doc.trailer.get(b"Root").map_err(pdf_err)?)?
            .as_dict()
            .map_err(pdf_err)?;
        if let Ok(version) = catalog.get(b"Version") {
            let name = resolve_finalize_object(doc, version)?
                .as_name()
                .map_err(pdf_err)?;
            self.version = self.version.max(Version::parse(name)?);
        }
        if let Ok(extensions) = catalog.get(b"Extensions") {
            self.version = self.version.max(Version(1, 7));
            let mut remaining = 65_536;
            let extensions = direct_extension(doc, extensions, 0, &mut remaining)?;
            for (prefix, declaration) in extensions.as_dict().map_err(pdf_err)?.iter() {
                if prefix == b"Type" {
                    if declaration.as_name().map_err(pdf_err)? != b"Extensions" {
                        return Err(invalid("invalid Extensions dictionary Type"));
                    }
                    self.extension_type = true;
                    continue;
                }
                let declarations = match declaration {
                    LoObject::Array(values) => {
                        self.arrays.insert(prefix.clone());
                        self.version = self.version.max(Version(2, 0));
                        values.as_slice()
                    }
                    LoObject::Dictionary(_) => std::slice::from_ref(declaration),
                    _ => {
                        return Err(invalid(
                            "expected a developer extension dictionary or array",
                        ));
                    }
                };
                let output = self.extensions.entry(prefix.clone()).or_default();
                for declaration in declarations {
                    let fields = declaration.as_dict().map_err(pdf_err)?;
                    let base = fields
                        .get(b"BaseVersion")
                        .and_then(LoObject::as_name)
                        .map_err(pdf_err)?;
                    fields
                        .get(b"ExtensionLevel")
                        .and_then(LoObject::as_i64)
                        .map_err(pdf_err)?;
                    self.version = self.version.max(Version::parse(base)?).max(Version(1, 7));
                    if !output.contains(declaration) {
                        output.push(declaration.clone());
                    }
                }
                // Distinct extensions from the same developer need an array;
                // selecting the largest level would lose independent requirements.
                if output.len() > 1 {
                    self.arrays.insert(prefix.clone());
                    self.version = self.version.max(Version(2, 0));
                }
            }
        }
        Ok(())
    }

    pub(super) fn apply(&self, doc: &mut LoDocument) -> Result<(), FullBleedError> {
        let mut catalog_id = doc
            .trailer
            .get(b"Root")
            .and_then(LoObject::as_reference)
            .map_err(pdf_err)?;
        let mut seen = BTreeSet::new();
        loop {
            if !seen.insert(catalog_id) {
                return Err(invalid("cyclic catalog reference"));
            }
            match doc.get_object(catalog_id).map_err(pdf_err)? {
                LoObject::Reference(next) => catalog_id = *next,
                _ => break,
            }
        }
        let catalog = doc
            .get_object_mut(catalog_id)
            .and_then(LoObject::as_dict_mut)
            .map_err(pdf_err)?;
        if catalog.get(b"Version").is_ok() {
            catalog.set("Version", LoObject::Name(self.version.text().into_bytes()));
        }
        if !self.extensions.is_empty() || self.extension_type {
            let mut extensions = LoDictionary::new();
            if self.extension_type {
                extensions.set("Type", "Extensions");
            }
            for (prefix, declarations) in &self.extensions {
                let value = if self.arrays.contains(prefix) {
                    LoObject::Array(declarations.clone())
                } else {
                    declarations[0].clone()
                };
                extensions.set(prefix, value);
            }
            catalog.set("Extensions", extensions);
        }
        doc.version = self.version.text();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pdf_native::StringFormat;

    fn fixture(header: &str, catalog_version: Option<&str>) -> LoDocument {
        let mut doc = LoDocument::with_version(header);
        let pages = doc.new_object_id();
        let transparent = header != "1.3" || catalog_version.is_some();
        let content = if transparent {
            b"q /Alpha gs 0 0 1 rg 20 20 50 30 re f Q\n".as_slice()
        } else {
            b"q 0 0 1 rg 20 20 50 30 re f Q\n".as_slice()
        };
        let content = doc.add_object(LoStream::new(dictionary! {}, content.to_vec()));
        let resources = if transparent {
            dictionary! {"ExtGState" => dictionary! {
                "Alpha" => dictionary! {"Type" => "ExtGState", "ca" => 0.4_f32}
            }}
        } else {
            LoDictionary::new()
        };
        let page = doc.add_object(dictionary! {
            "Type" => "Page", "Parent" => pages,
            "MediaBox" => vec![0.into(), 0.into(), 200.into(), 200.into()], "Contents" => content,
            "Resources" => resources,
        });
        doc.objects.insert(
            pages,
            dictionary! {"Type" => "Pages", "Kids" => vec![page.into()], "Count" => 1}.into(),
        );
        let mut catalog = dictionary! {"Type" => "Catalog", "Pages" => pages};
        if let Some(version) = catalog_version {
            catalog.set("Version", version);
        }
        let root = doc.add_object(catalog);
        doc.trailer.set("Root", root);
        doc
    }

    fn catalog(doc: &LoDocument) -> &LoDictionary {
        resolve_finalize_object(doc, doc.trailer.get(b"Root").unwrap())
            .unwrap()
            .as_dict()
            .unwrap()
    }

    fn catalog_mut(doc: &mut LoDocument) -> &mut LoDictionary {
        let root = doc.trailer.get(b"Root").unwrap().as_reference().unwrap();
        doc.get_object_mut(root).unwrap().as_dict_mut().unwrap()
    }

    fn output_dir(name: &str) -> PathBuf {
        let base = std::env::var_os("FULLBLEED_COMPOSE_TEST_OUTPUT")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                std::env::temp_dir().join(format!("fullbleed-version-tests-{}", std::process::id()))
            });
        let dir = base.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn run_both(
        template: &LoDocument,
        overlay: &LoDocument,
        dir: &std::path::Path,
    ) -> (LoDocument, LoDocument) {
        let template_path = dir.join("template.pdf");
        let overlay_path = dir.join("overlay.pdf");
        template.clone().save(&template_path).unwrap();
        overlay.clone().save(&overlay_path).unwrap();
        let catalog = TemplateCatalog {
            by_id: BTreeMap::from([(
                "template".into(),
                TemplateAsset {
                    template_id: "template".into(),
                    pdf_path: template_path.clone(),
                    sha256: None,
                    page_count: Some(1),
                },
            )]),
        };
        for suffix in ["", "-replay"] {
            stamp_overlay_on_template_pdf(
                &template_path,
                &overlay_path,
                &dir.join(format!("stamp{suffix}.pdf")),
                None,
                0.,
                0.,
            )
            .unwrap();
            compose_overlay_with_template_catalog(
                &catalog,
                &overlay_path,
                &dir.join(format!("compose{suffix}.pdf")),
                &[ComposePagePlan {
                    template_id: "template".into(),
                    template_page_index: 0,
                    overlay_page_index: 0,
                    dx: 0.,
                    dy: 0.,
                }],
            )
            .unwrap();
        }
        for method in ["stamp", "compose"] {
            assert_eq!(
                std::fs::read(dir.join(format!("{method}.pdf"))).unwrap(),
                std::fs::read(dir.join(format!("{method}-replay.pdf"))).unwrap()
            );
        }
        (
            LoDocument::load(dir.join("stamp.pdf")).unwrap(),
            LoDocument::load(dir.join("compose.pdf")).unwrap(),
        )
    }

    #[test]
    fn imported_versions_preserve_headers_and_catalog_upgrades() {
        let cases = [
            ("1.3", None, "1.4", None, "1.4", "1.7"),
            ("1.7", None, "1.7", Some("2.0"), "2.0", "2.0"),
            ("1.3", Some("2.0"), "1.4", None, "2.0", "2.0"),
            ("2.0", None, "1.7", None, "2.0", "2.0"),
            ("2.0", Some("1.4"), "1.7", None, "2.0", "2.0"),
            ("1.7", Some("1.3"), "1.4", Some("1.6"), "1.7", "1.7"),
        ];
        for (i, (th, tc, oh, oc, stamp_version, compose_version)) in cases.into_iter().enumerate() {
            let (stamp, compose) = run_both(
                &fixture(th, tc),
                &fixture(oh, oc),
                &output_dir(&format!("versions-{i}")),
            );
            assert_eq!(stamp.version, stamp_version);
            assert_eq!(compose.version, compose_version);
            if tc.is_some() {
                assert_eq!(
                    catalog(&stamp).get(b"Version").unwrap().as_name().unwrap(),
                    stamp_version.as_bytes()
                );
            }
            for output in [&stamp, &compose] {
                let page = *output.get_pages().values().next().unwrap();
                let resources = page_resources_dict(output, page);
                let forms = page_xobject_dict(&resources, output);
                assert!(forms.iter().any(|(_, object)| {
                    let form = resolve_finalize_object(output, object)
                        .unwrap()
                        .as_stream()
                        .unwrap();
                    let resources =
                        resolve_finalize_object(output, form.dict.get(b"Resources").unwrap())
                            .unwrap()
                            .as_dict()
                            .unwrap();
                    let Ok(states) = resources.get(b"ExtGState") else {
                        return false;
                    };
                    resolve_finalize_object(output, states)
                        .unwrap()
                        .as_dict()
                        .unwrap()
                        .iter()
                        .any(|(_, state)| {
                            resolve_finalize_object(output, state)
                                .unwrap()
                                .as_dict()
                                .unwrap()
                                .get(b"ca")
                                .is_ok_and(|alpha| *alpha == LoObject::Real(0.4))
                        })
                }));
            }
        }
    }

    fn extension(level: i64, format: StringFormat) -> LoObject {
        dictionary! {"Type" => "DeveloperExtensions", "BaseVersion" => "1.7", "ExtensionLevel" => level,
            "URL" => LoObject::String(format!("https://example.invalid/extension/{level}").into_bytes(), format),
            "ExtensionRevision" => LoObject::String(b"2026".to_vec(), format),
            "Custom" => vec![LoObject::Boolean(true), LoObject::Integer(9)]}.into()
    }

    #[test]
    fn extension_requirements_merge_without_discarding_declarations() {
        let mut template = fixture("1.7", None);
        catalog_mut(&mut template).set(
            "Extensions",
            dictionary! {
                "Type" => "Extensions", "TEST" => extension(1, StringFormat::Literal),
                "SOLO" => extension(7, StringFormat::Literal),
            },
        );
        for kind in ["duplicate", "distinct", "array"] {
            let mut overlay = fixture("1.7", None);
            let declaration = match kind {
                "duplicate" => extension(1, StringFormat::Hexadecimal),
                "distinct" => extension(2, StringFormat::Literal),
                _ => LoObject::Array(vec![
                    extension(1, StringFormat::Hexadecimal),
                    extension(2, StringFormat::Literal),
                ]),
            };
            let mut imported = dictionary! {"TEST" => declaration};
            if kind == "array" {
                imported.set("SINGLE", vec![extension(8, StringFormat::Literal)]);
            }
            catalog_mut(&mut overlay).set("Extensions", imported);
            let (stamp, compose) = run_both(
                &template,
                &overlay,
                &output_dir(&format!("versions-extensions-{kind}")),
            );
            for output in [&stamp, &compose] {
                assert_eq!(
                    output.version,
                    if kind == "duplicate" { "1.7" } else { "2.0" }
                );
                let extensions = catalog(output)
                    .get(b"Extensions")
                    .unwrap()
                    .as_dict()
                    .unwrap();
                assert_eq!(
                    extensions.get(b"Type").unwrap().as_name().unwrap(),
                    b"Extensions"
                );
                let expected = if kind == "duplicate" {
                    extension(1, StringFormat::Literal)
                } else {
                    LoObject::Array(vec![
                        extension(1, StringFormat::Literal),
                        extension(2, StringFormat::Literal),
                    ])
                };
                assert_eq!(extensions.get(b"TEST").unwrap(), &expected);
                assert_eq!(
                    extensions.get(b"SOLO").unwrap(),
                    &extension(7, StringFormat::Literal)
                );
                if kind == "array" {
                    assert_eq!(
                        extensions.get(b"SINGLE").unwrap(),
                        &LoObject::Array(vec![extension(8, StringFormat::Literal)])
                    );
                }
            }
        }
    }

    #[test]
    fn version_declarations_reject_cycles_and_invalid_values_before_writing() {
        let dir = output_dir("versions-errors");
        let template = dir.join("template.pdf");
        fixture("1.7", None).save(&template).unwrap();
        let templates = TemplateCatalog {
            by_id: BTreeMap::from([(
                "template".into(),
                TemplateAsset {
                    template_id: "template".into(),
                    pdf_path: template.clone(),
                    sha256: None,
                    page_count: Some(1),
                },
            )]),
        };
        let mut bad_version = fixture("1.7", Some("bogus"));
        let mut wrong_type = fixture("1.7", None);
        catalog_mut(&mut wrong_type).set("Version", 2);
        let mut cyclic = fixture("1.7", None);
        let cycle = cyclic.new_object_id();
        cyclic
            .objects
            .insert(cycle, LoObject::Array(vec![cycle.into()]));
        catalog_mut(&mut cyclic).set("Extensions", dictionary! {"TEST" => cycle});
        for (i, input) in [&mut bad_version, &mut wrong_type, &mut cyclic]
            .into_iter()
            .enumerate()
        {
            let overlay = dir.join(format!("overlay-{i}.pdf"));
            input.save(&overlay).unwrap();
            let output = dir.join(format!("keep-{i}.pdf"));
            std::fs::write(&output, b"unchanged").unwrap();
            assert!(
                stamp_overlay_on_template_pdf(&template, &overlay, &output, None, 0., 0.).is_err()
            );
            assert_eq!(std::fs::read(&output).unwrap(), b"unchanged");
            assert!(
                compose_overlay_with_template_catalog(
                    &templates,
                    &overlay,
                    &output,
                    &[ComposePagePlan {
                        template_id: "template".into(),
                        template_page_index: 0,
                        overlay_page_index: 0,
                        dx: 0.,
                        dy: 0.
                    }]
                )
                .is_err()
            );
            assert_eq!(std::fs::read(&output).unwrap(), b"unchanged");
        }
    }

    #[test]
    fn unused_templates_do_not_add_output_version_or_extension_requirements() {
        let dir = output_dir("versions-unused");
        let mut unused = fixture("2.0", None);
        catalog_mut(&mut unused).set(
            "Extensions",
            dictionary! {"UNUSED" => extension(1, StringFormat::Literal)},
        );
        let mut templates = TemplateCatalog::default();
        for (name, mut document) in [("used", fixture("1.7", None)), ("unused", unused)] {
            let path = dir.join(format!("{name}.pdf"));
            document.save(&path).unwrap();
            templates.by_id.insert(
                name.into(),
                TemplateAsset {
                    template_id: name.into(),
                    pdf_path: path,
                    sha256: None,
                    page_count: Some(1),
                },
            );
        }
        let overlay = dir.join("overlay.pdf");
        fixture("1.7", None).save(&overlay).unwrap();
        for suffix in ["", "-replay"] {
            compose_overlay_with_template_catalog(
                &templates,
                &overlay,
                &dir.join(format!("compose{suffix}.pdf")),
                &[ComposePagePlan {
                    template_id: "used".into(),
                    template_page_index: 0,
                    overlay_page_index: 0,
                    dx: 0.,
                    dy: 0.,
                }],
            )
            .unwrap();
        }
        assert_eq!(
            std::fs::read(dir.join("compose.pdf")).unwrap(),
            std::fs::read(dir.join("compose-replay.pdf")).unwrap()
        );
        let output = LoDocument::load(dir.join("compose.pdf")).unwrap();
        assert_eq!(output.version, "1.7");
        assert!(catalog(&output).get(b"Extensions").is_err());
        for suffix in ["", "-replay"] {
            let path = dir.join(format!("stamp-empty{suffix}.pdf"));
            let summary = stamp_overlay_on_template_pdf(
                &dir.join("used.pdf"),
                &dir.join("unused.pdf"),
                &path,
                Some(&[]),
                0.,
                0.,
            )
            .unwrap();
            assert_eq!(summary.pages_written, 0);
            let output = LoDocument::load(&path).unwrap();
            assert_eq!(output.version, "1.7");
            assert_eq!(output.get_pages().len(), 1);
            assert!(catalog(&output).get(b"Extensions").is_err());
        }
        assert_eq!(
            std::fs::read(dir.join("stamp-empty.pdf")).unwrap(),
            std::fs::read(dir.join("stamp-empty-replay.pdf")).unwrap()
        );
    }
}
