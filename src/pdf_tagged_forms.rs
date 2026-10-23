//! Content-stream ownership for semantic Form XObjects.
//!
//! A form's MCIDs are local to its stream. Each semantic placement has its own
//! form and parent-tree entry; images, fonts and artifact forms still reuse the
//! ordinary resource caches.
use super::*;

#[derive(Debug, Clone, Copy, Default)]
pub(super) enum TagContentStream {
    #[default]
    Page,
    Form {
        object_id: usize,
        parent_tree_key: usize,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tagged_forms_redeclaration_refreshes_specialized_aliases() {
        let size = Size::a4();
        let commands = |text: &str| {
            let mut canvas = crate::Canvas::new(size);
            canvas.draw_string(Pt::ZERO, Pt::from_f32(18.0), text);
            canvas.finish().pages.remove(0).commands
        };
        let mut bytes = Vec::new();
        let mut writer =
            PdfStreamWriter::new(&mut bytes, size, None, PdfOptions::default(), None, None)
                .unwrap();
        writer.declare_form_definition("local", size.width, size.height, &commands("ALPHA"), false);
        let first = writer
            .ensure_registered_form_with_filter_offset("local", (Pt::ZERO, Pt::ZERO))
            .unwrap();
        writer.declare_form_definition("local", size.width, size.height, &commands("BRAVO"), false);
        let second = writer
            .ensure_registered_form_with_filter_offset("local", (Pt::ZERO, Pt::ZERO))
            .unwrap();
        assert_ne!(
            first, second,
            "specialized aliases must follow the current declaration"
        );
    }

    #[test]
    fn tagged_forms_keep_recipient_content_when_resource_names_repeat() {
        let make_document = |text: &str| {
            let size = Size::a4();
            let mut content = crate::Canvas::new(size);
            content.begin_tag("P", None, None, None, None, false);
            content.draw_string(Pt::from_f32(12.0), Pt::from_f32(18.0), text);
            content.end_tag();
            let mut canvas = crate::Canvas::new(size);
            canvas.define_form(
                "recipient",
                size.width,
                size.height,
                content.finish().pages.remove(0).commands,
            );
            canvas.draw_form(Pt::ZERO, Pt::ZERO, size.width, size.height, "recipient");
            canvas.finish()
        };
        let mut bytes = Vec::new();
        let mut options = PdfOptions::default();
        options.pdf_profile = PdfProfile::Tagged;
        let mut writer =
            PdfStreamWriter::new(&mut bytes, Size::a4(), None, options, None, None).unwrap();
        writer.add_document(0, &make_document("ALPHA")).unwrap();
        writer.add_document(1, &make_document("BRAVO")).unwrap();
        writer.finish().unwrap();
        drop(writer);
        let pdf = crate::pdf_native::Document::load_mem(&bytes).unwrap();
        let content = pdf
            .objects
            .values()
            .filter_map(|object| object.as_stream().ok())
            .filter(|stream| stream.dict.get(b"StructParents").is_ok())
            .map(|stream| String::from_utf8(stream.get_plain_content().unwrap()).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(content.len(), 2);
        assert!(content[0].contains("ALPHA"));
        assert!(
            content[1].contains("BRAVO"),
            "each recipient must retain its own form content"
        );
        assert!(!content[1].contains("ALPHA"));
    }

    #[test]
    fn tagged_forms_specialize_each_placement_and_reuse_artifact_paint() {
        let size = Size::a4();
        let mut content = crate::Canvas::new(size);
        content.begin_tag("P", None, None, None, None, false);
        content.draw_string(Pt::from_f32(12.0), Pt::from_f32(18.0), "Reusable content");
        content.end_tag();
        let commands = content.finish().pages.remove(0).commands;
        let mut canvas = crate::Canvas::new(size);
        canvas.define_form("shared", size.width, size.height, commands);
        canvas.begin_tag("Div", None, None, None, None, true);
        for y in [0.0, 30.0] {
            canvas.draw_form(Pt::ZERO, Pt::from_f32(y), size.width, size.height, "shared");
        }
        canvas.end_tag();
        canvas.begin_artifact(None);
        for y in [60.0, 90.0] {
            canvas.draw_form(Pt::ZERO, Pt::from_f32(y), size.width, size.height, "shared");
        }
        canvas.end_marked_content();
        let document = canvas.finish();
        let mut options = PdfOptions::default();
        options.pdf_profile = PdfProfile::Tagged;
        let bytes =
            document_to_pdf_with_metrics_and_registry(&document, None, None, &options).unwrap();
        let pdf = crate::pdf_native::Document::load_mem(&bytes).unwrap();
        let forms: Vec<_> = pdf
            .objects
            .values()
            .filter_map(|object| object.as_stream().ok())
            .filter(|stream| {
                stream.dict.get(b"Subtype").and_then(|v| v.as_name()).ok() == Some(b"Form")
            })
            .collect();
        assert_eq!(
            forms.len(),
            3,
            "two semantic placements and one shared artifact form"
        );
        assert_eq!(
            forms
                .iter()
                .filter(|form| form.dict.get(b"StructParents").is_ok())
                .count(),
            2
        );
        assert_eq!(
            String::from_utf8_lossy(&bytes)
                .matches("/Type /MCR /Stm ")
                .count(),
            2
        );
        if let Some(directory) = std::env::var_os("FULLBLEED_LINK_TEST_OUTPUT") {
            let directory = std::path::PathBuf::from(directory);
            std::fs::create_dir_all(&directory).unwrap();
            std::fs::write(directory.join("forms-repeated-placement-tagged.pdf"), bytes).unwrap();
        }
    }

    #[test]
    fn tagged_forms_preserve_opacity_targets_nested_structure_and_copies() {
        for (profile_name, profile) in [("ua1", PdfProfile::PdfUa1), ("ua2", PdfProfile::PdfUa2)] {
            let engine = crate::FullBleed::builder()
                .register_font_file("python/fullbleed_assets/fonts/NotoSans-Regular.ttf")
                .pdf_profile(profile)
                .document_lang("en-US")
                .document_title("Semantic opacity forms")
                .build()
                .unwrap();
            for (name, body) in [
                (
                    "target",
                    r##"<h1>Report</h1><p><a href="#target">Details</a></p><h2 id="target" style="opacity:0.6">Target heading</h2>"##,
                ),
                (
                    "link",
                    r##"<h1>Report</h1><p><a href="https://example.invalid/report" style="display:inline-block;opacity:0.6">Read report</a></p>"##,
                ),
                (
                    "nested",
                    r##"<h1>Report</h1><p><a href="#target">Details</a></p><div style="opacity:0.7"><h2 id="target">Target heading</h2><div style="opacity:0.5"><p>Nested text</p><p><a href="https://example.invalid/report">Read report</a></p></div></div>"##,
                ),
            ] {
                let compiled = engine.compile_document(body, "@page{size:300pt 400pt;margin:16pt}body{font-family:'Noto Sans';font-size:12pt;line-height:18pt}h1,h2{font-weight:normal}p{margin:0 0 12pt}").unwrap();
                for copies in [1, 2] {
                    let bytes = compiled.render_many_to_buffer(copies).unwrap();
                    assert_eq!(bytes, compiled.render_many_to_buffer(copies).unwrap());
                    let pdf = crate::pdf_native::Document::load_mem(&bytes).unwrap();
                    let forms: Vec<_> = pdf
                        .objects
                        .iter()
                        .filter_map(|(id, object)| {
                            let stream = object.as_stream().ok()?;
                            stream
                                .dict
                                .get(b"StructParents")
                                .ok()
                                .map(|key| (*id, key.as_i64().unwrap()))
                        })
                        .collect();
                    assert!(
                        forms.len() >= copies,
                        "{profile_name}/{name}: missing semantic forms"
                    );
                    let unique: BTreeSet<_> = forms.iter().map(|(_, key)| key).collect();
                    assert_eq!(
                        unique.len(),
                        forms.len(),
                        "each semantic placement owns a parent-tree slot"
                    );
                    assert!(String::from_utf8_lossy(&bytes).contains("/Type /MCR /Stm "));
                    if let Some(directory) = std::env::var_os("FULLBLEED_LINK_TEST_OUTPUT") {
                        let directory = std::path::PathBuf::from(directory);
                        std::fs::create_dir_all(&directory).unwrap();
                        std::fs::write(
                            directory.join(format!("forms-{name}-{profile_name}-{copies}.pdf")),
                            &bytes,
                        )
                        .unwrap();
                    }
                }
            }
        }
    }
}

pub(super) struct FormTagContext {
    pub content_stream: TagContentStream,
    pub parent: Option<usize>,
    pub destinations: Vec<(usize, String)>,
    pub group_scope: Option<usize>,
}

impl<W: Write> PdfStreamWriter<'_, W> {
    pub(super) fn page_parent_tree_key(&mut self, page: usize) -> usize {
        while self.page_parent_tree_keys.len() <= page {
            self.page_parent_tree_keys.push(self.next_parent_tree_key);
            self.next_parent_tree_key += 1;
        }
        self.page_parent_tree_keys[page]
    }

    pub(super) fn form_has_semantic_content(&self, resource: &str) -> io::Result<bool> {
        fn visit<'a>(
            resource: &'a str,
            definitions: &mut HashMap<&'a str, &'a [Command]>,
            active: &mut Vec<&'a str>,
        ) -> io::Result<bool> {
            if active.len() >= 64 || active.contains(&resource) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "cyclic or excessive semantic form nesting",
                ));
            }
            let Some(&commands) = definitions.get(resource) else {
                return Ok(false);
            };
            active.push(resource);
            let mut artifacts = 0usize;
            let mut marked = Vec::new();
            for command in commands {
                match command {
                    Command::DefineForm {
                        resource_id,
                        commands,
                        ..
                    }
                    | Command::DefineIsolatedForm {
                        resource_id,
                        commands,
                        ..
                    } => {
                        definitions.insert(resource_id, commands);
                    }
                    Command::BeginArtifact { .. } => {
                        artifacts += 1;
                        marked.push(true);
                    }
                    Command::BeginOptionalContent { .. } => marked.push(false),
                    Command::EndMarkedContent => {
                        if marked.pop() == Some(true) {
                            artifacts = artifacts.saturating_sub(1);
                        }
                    }
                    Command::BeginTag { .. } | Command::BeginTagActualText { .. }
                        if artifacts == 0 =>
                    {
                        active.pop();
                        return Ok(true);
                    }
                    Command::DrawForm { resource_id, .. }
                    | Command::DrawMaskedForm { resource_id, .. }
                        if artifacts == 0 =>
                    {
                        if visit(resource_id, definitions, active)? {
                            active.pop();
                            return Ok(true);
                        }
                    }
                    _ => {}
                }
            }
            active.pop();
            Ok(false)
        }
        let mut definitions = self
            .form_definition_map
            .iter()
            .map(|(key, (_, _, commands))| (key.as_str(), commands.as_slice()))
            .collect();
        visit(resource, &mut definitions, &mut Vec::new())
    }

    pub(super) fn ensure_semantic_form(
        &mut self,
        resource: &str,
        page_index: usize,
        parent: Option<usize>,
        destinations: Vec<(usize, String)>,
        parent_group_scope: Option<usize>,
        filter_offset: Option<(Pt, Pt)>,
    ) -> io::Result<Option<String>> {
        if self.active_semantic_forms.len() >= 64
            || self
                .active_semantic_forms
                .iter()
                .any(|active| active == resource)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "cyclic or excessive semantic form nesting",
            ));
        }
        let Some((width, height, commands)) = self.form_definition_map.get(resource).cloned()
        else {
            return Ok(None);
        };
        let object_id = self.alloc_ids(1);
        // Collected inline fragments retain their originating stream's logical
        // groups. Independently placed forms always get a distinct identity.
        let group_scope = if matches!(commands.first(), Some(Command::Meta { key, value })
            if key == crate::inline_opacity::SHARE_GROUP_SCOPE && value == "parent")
        {
            parent_group_scope
        } else {
            Some(object_id)
        };
        let parent_tree_key = self.next_parent_tree_key;
        self.next_parent_tree_key += 1;
        self.active_semantic_forms.push(resource.to_owned());
        let content = self.render_commands_with_filter_offset(
            &commands,
            height,
            Some(page_index),
            filter_offset,
            &[],
            None,
            Some(FormTagContext {
                content_stream: TagContentStream::Form {
                    object_id,
                    parent_tree_key,
                },
                parent,
                destinations,
                group_scope,
            }),
        );
        self.active_semantic_forms.pop();
        let content = content?;
        let name = format!("Fm{}", self.next_form_index);
        self.next_form_index += 1;
        let group = if self
            .form_isolated_map
            .get(resource)
            .copied()
            .unwrap_or(false)
        {
            " /Group << /S /Transparency /I true /K false >>"
        } else {
            ""
        };
        let dict = format!(
            "/Type /XObject /Subtype /Form /FormType 1 /BBox [0 0 {} {}] /Resources {} 0 R /StructParents {}{}",
            fmt_pt(width),
            fmt_pt(height),
            PDF_RESOURCES_ID,
            parent_tree_key,
            group,
        );
        self.write_content_stream_object(object_id, &dict, content.as_bytes())?;
        self.form_resources.push((name.clone(), object_id));
        self.form_size_map
            .insert(resource.to_owned(), Size { width, height });
        Ok(Some(name))
    }
}
