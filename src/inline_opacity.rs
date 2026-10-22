//! Page-fragment compositing for non-atomic inline elements.
//!
//! Layout retains one source identity across runs, line breaks and page splits.
//! Only after the page's paint order is known do we collect its fragments into
//! an isolated form. Graphics state stays in the original stream as well, since
//! subsequent text may inherit a font or color set by a collected fragment.
use crate::canvas::{
    Command, META_DIAGNOSTIC_SCOPE_BEGIN_KEY, META_DIAGNOSTIC_SCOPE_END_KEY,
    META_READING_TEXT_BEGIN_KEY, META_READING_TEXT_END_KEY,
};
use crate::types::{Pt, Size};

pub(crate) const BEGIN: &str = "__fb_inline_opacity_begin";
pub(crate) const END: &str = "__fb_inline_opacity_end";
pub(crate) const GROUP_SOURCE: &str = "__fb_inline_group_source";
pub(crate) const SHARE_GROUP_SCOPE: &str = "__fb_inline_group_scope";

#[derive(Debug, Clone, Copy)]
pub(crate) struct InlineOpacity {
    pub id: usize,
    pub alpha: f32,
}

#[derive(Clone, Copy)]
struct Scope {
    begin: usize,
    end: usize,
    graphics: bool,
}

fn boundary(command: &Command) -> Option<(usize, bool)> {
    match command {
        Command::SaveState => Some((0, true)),
        Command::RestoreState => Some((0, false)),
        Command::BeginTag { .. } | Command::BeginTagActualText { .. } => Some((1, true)),
        Command::EndTag => Some((1, false)),
        Command::BeginArtifact { .. } | Command::BeginOptionalContent { .. } => Some((2, true)),
        Command::EndMarkedContent => Some((2, false)),
        Command::Meta { key, .. } => match key.as_str() {
            META_DIAGNOSTIC_SCOPE_BEGIN_KEY => Some((3, true)),
            META_DIAGNOSTIC_SCOPE_END_KEY => Some((3, false)),
            META_READING_TEXT_BEGIN_KEY => Some((4, true)),
            META_READING_TEXT_END_KEY => Some((4, false)),
            BEGIN => Some((5, true)),
            END => Some((5, false)),
            _ => None,
        },
        _ => None,
    }
}

fn graphics(command: &Command) -> bool {
    matches!(
        command,
        Command::SaveState
            | Command::RestoreState
            | Command::Translate(..)
            | Command::CssTransformOrigin { .. }
            | Command::Scale(..)
            | Command::Rotate(..)
            | Command::ConcatMatrix { .. }
            | Command::SetFillColor(..)
            | Command::SetStrokeColor(..)
            | Command::SetLineWidth(..)
            | Command::SetLineCap(..)
            | Command::SetLineJoin(..)
            | Command::SetMiterLimit(..)
            | Command::SetDash { .. }
            | Command::SetOpacity { .. }
            | Command::SetBlendMode { .. }
            | Command::SetFontName(..)
            | Command::SetFontSize(..)
            | Command::SetTextRenderingMode(..)
            | Command::ClipRect { .. }
            | Command::DefineForm { .. }
            | Command::DefineIsolatedForm { .. }
    )
}

fn begin(command: &Command) -> Option<InlineOpacity> {
    let Command::Meta { key, value } = command else {
        return None;
    };
    if key != BEGIN {
        return None;
    }
    let (id, alpha) = value.split_once(':')?;
    Some(InlineOpacity {
        id: id.parse().ok()?,
        alpha: alpha.parse().ok()?,
    })
}

/// Independent graphics, tags and diagnostic stacks may cross one another;
/// pairing each kind independently avoids imposing a fictitious nesting rule.
fn scopes(commands: &[Command]) -> Vec<Scope> {
    let mut stacks: [Vec<usize>; 6] = std::array::from_fn(|_| Vec::new());
    let mut result = Vec::new();
    for (index, command) in commands.iter().enumerate() {
        if let Some((kind, opening)) = boundary(command) {
            if opening {
                stacks[kind].push(index);
            } else if let Some(begin) = stacks[kind].pop() {
                result.push(Scope {
                    begin,
                    end: index,
                    graphics: kind == 0,
                });
            }
        }
    }
    result
}

// Suspend the scopes opened by the retained prefix, then replay only their
// state after painting the form. Painted text and paths must not be repeated.
fn retained_prefix_state(
    commands: &[Command],
    scopes: &[Scope],
    keep: &[u8],
    start: usize,
    first: usize,
) -> Option<(Vec<Command>, Vec<Command>)> {
    let mut active: Vec<_> = scopes
        .iter()
        .filter(|scope| scope.begin >= start && scope.begin < first && scope.end >= first)
        .collect();
    active.sort_by_key(|scope| scope.begin);
    let is_active = |index| active.iter().any(|scope| scope.begin == index);
    let mut close = Vec::new();
    for scope in active.iter().rev() {
        close.push(match &commands[scope.begin] {
            Command::SaveState => Command::RestoreState,
            Command::BeginTag {
                group_only: true, ..
            } => Command::EndTag,
            Command::BeginArtifact { .. } | Command::BeginOptionalContent { .. } => {
                Command::EndMarkedContent
            }
            Command::Meta { key, value } if key == META_DIAGNOSTIC_SCOPE_BEGIN_KEY => {
                Command::Meta {
                    key: META_DIAGNOSTIC_SCOPE_END_KEY.into(),
                    value: value.clone(),
                }
            }
            // The authoring path places effect boundaries outside text leaves.
            // Do not duplicate a low-level MCID or ActualText sequence if a
            // manually constructed stream instead cuts through one.
            _ => return None,
        });
    }
    let active_owner = |index, diagnostic| {
        scopes.iter().filter(|scope| scope.begin < index && scope.end >= index)
            .filter(|scope| if diagnostic {
                matches!(&commands[scope.begin], Command::Meta { key, .. } if key == META_DIAGNOSTIC_SCOPE_BEGIN_KEY)
            } else {
                matches!(&commands[scope.begin], Command::BeginTag { .. } | Command::BeginTagActualText { .. })
            })
            .max_by_key(|scope| scope.begin)
            .is_some_and(|scope| is_active(scope.begin))
    };
    let mut replay = Vec::new();
    for index in start..first {
        if keep[index] & 2 == 0 {
            continue;
        }
        let command = &commands[index];
        let state = graphics(command)
            || is_active(index)
            || match command {
                Command::MoveTo { .. }
                | Command::LineTo { .. }
                | Command::CurveTo { .. }
                | Command::ClosePath
                | Command::ClipPath { .. } => keep[index] == 3,
                Command::Meta { key, .. } if key == GROUP_SOURCE => {
                    index > 0 && is_active(index - 1)
                }
                Command::Meta { key, .. }
                    if matches!(
                        key.as_str(),
                        crate::link_semantics::META_LINK_ID
                            | crate::link_semantics::META_LINK_TARGET
                            | crate::link_semantics::META_LINK_LABEL
                            | crate::link_semantics::META_DESTINATION
                            | crate::link_semantics::META_NAVIGATION_SUPPRESSED
                    ) =>
                {
                    active_owner(index, true)
                }
                Command::Meta { key, .. }
                    if matches!(
                        key.as_str(),
                        crate::canvas::META_LIST_NUMBERING_KEY
                            | crate::canvas::META_HTML_FIGURE_GROUP_KEY
                    ) =>
                {
                    active_owner(index, false)
                }
                _ => false,
            };
        if state {
            replay.push(command.clone());
        }
    }
    Some((close, replay))
}

pub(crate) fn resolve(mut commands: Vec<Command>, size: Size) -> Vec<Command> {
    if !commands.iter().any(|command| begin(command).is_some()) {
        return commands;
    }
    let namespace =
        crate::running_layout::form_id("inline-groups", size.width, size.height, &commands);
    let mut identified = Vec::with_capacity(commands.len());
    for (index, command) in commands.iter().enumerate() {
        identified.push(command.clone());
        if matches!(
            command,
            Command::BeginTag {
                group_only: true,
                ..
            }
        ) && !matches!(commands.get(index + 1), Some(Command::Meta { key, .. }) if key == GROUP_SOURCE)
        {
            identified.push(Command::Meta {
                key: GROUP_SOURCE.into(),
                value: format!("{namespace}:{index}"),
            });
        }
    }
    commands = identified;
    while let Some(effect) = commands.iter().find_map(begin) {
        commands = resolve_group(commands, size, effect);
    }
    commands
}

fn resolve_group(mut commands: Vec<Command>, size: Size, effect: InlineOpacity) -> Vec<Command> {
    let mut active = Vec::new();
    let mut selected = vec![false; commands.len()];
    for (index, command) in commands.iter().enumerate() {
        if let Some(group) = begin(command) {
            active.push(group.id);
        }
        selected[index] = active.contains(&effect.id);
        if matches!(command, Command::Meta { key, .. } if key == END) {
            active.pop();
        }
    }
    let start = selected.iter().position(|value| *value).unwrap();
    let first_selected = start;
    let end = selected.iter().rposition(|value| *value).unwrap() + 1;
    let scopes = scopes(&commands);
    let (mut start, mut end) = (start, end);
    loop {
        let previous = (start, end);
        for scope in &scopes {
            if scope.begin < start && scope.end >= start && scope.end < end {
                start = scope.begin;
            }
            if scope.begin >= start && scope.begin < end && scope.end >= end {
                end = scope.end + 1;
            }
        }
        if previous == (start, end) {
            break;
        }
    }

    // Bit 1 retains the command in the form; bit 2 in the original stream.
    let mut keep: Vec<u8> = commands
        .iter()
        .enumerate()
        .map(|(index, command)| {
            if graphics(command) {
                3
            } else if selected[index] {
                1
            } else {
                2
            }
        })
        .collect();
    let mut pending_path = Vec::new();
    for (index, command) in commands.iter().enumerate() {
        match command {
            Command::MoveTo { .. }
            | Command::LineTo { .. }
            | Command::CurveTo { .. }
            | Command::ClosePath => pending_path.push(index),
            Command::ClipPath { .. } => {
                keep[index] = 3;
                for point in pending_path.drain(..) {
                    keep[point] = 3;
                }
            }
            Command::Fill
            | Command::FillEvenOdd
            | Command::Stroke
            | Command::FillStroke
            | Command::FillStrokeEvenOdd => {
                for point in pending_path.drain(..) {
                    keep[point] = keep[index];
                }
            }
            _ => {}
        }
    }
    let mut form_content = vec![0usize; commands.len() + 1];
    let mut other_content = vec![0usize; commands.len() + 1];
    for (index, command) in commands.iter().enumerate() {
        let meaningful = !graphics(command)
            && !matches!(command, Command::Meta { key, .. } if key == GROUP_SOURCE || key == SHARE_GROUP_SCOPE)
            && (boundary(command).is_none()
                || matches!(command, Command::BeginTagActualText { .. }));
        form_content[index + 1] = form_content[index] + usize::from(meaningful && selected[index]);
        other_content[index + 1] =
            other_content[index] + usize::from(meaningful && !selected[index]);
    }
    for scope in &scopes {
        if scope.begin < start || scope.end >= end {
            continue;
        }
        let form = scope.graphics || form_content[scope.end + 1] > form_content[scope.begin];
        let other = scope.graphics || other_content[scope.end + 1] > other_content[scope.begin];
        let flags = u8::from(form) | (u8::from(other) << 1);
        keep[scope.begin] = flags;
        keep[scope.end] = flags;
        if matches!(commands.get(scope.begin + 1), Some(Command::Meta { key, .. }) if key == GROUP_SOURCE)
        {
            keep[scope.begin + 1] = flags;
        }
    }
    let mut form = Vec::new();
    let mut remainder = Vec::new();
    let prefix_state = retained_prefix_state(&commands, &scopes, &keep, start, first_selected);
    let mut prefix_len = 0;
    for (index, command) in commands.drain(start..end).enumerate() {
        let index = index + start;
        let own_marker = begin(&command).is_some_and(|group| group.id == effect.id)
            || matches!(&command, Command::Meta { key, value } if key == END && value == &effect.id.to_string());
        if own_marker {
            continue;
        }
        if keep[index] & 1 != 0 {
            form.push(command.clone());
        }
        if keep[index] & 2 != 0 {
            if index < first_selected {
                prefix_len += 1;
            }
            remainder.push(command);
        }
    }
    let mut form = resolve(form, size);
    form.insert(
        0,
        Command::Meta {
            key: SHARE_GROUP_SCOPE.into(),
            value: "parent".into(),
        },
    );
    let resource_id =
        crate::running_layout::form_id("inline-opacity", size.width, size.height, &form);
    let mut replacement = vec![Command::DefineIsolatedForm {
        resource_id: resource_id.clone(),
        width: size.width,
        height: size.height,
        commands: form,
    }];
    let paint = vec![
        Command::SaveState,
        Command::SetOpacity {
            fill: effect.alpha,
            stroke: effect.alpha,
        },
        Command::DrawForm {
            x: Pt::ZERO,
            y: Pt::ZERO,
            width: size.width,
            height: size.height,
            resource_id,
        },
        Command::RestoreState,
    ];
    if let Some((close, replay)) = prefix_state.filter(|_| prefix_len > 0) {
        let suffix = remainder.split_off(prefix_len);
        replacement.push(Command::SaveState);
        replacement.extend(remainder);
        replacement.extend(close);
        replacement.push(Command::RestoreState);
        replacement.extend(paint);
        replacement.extend(replay);
        replacement.extend(suffix);
    } else {
        replacement.extend(paint);
        replacement.extend(remainder);
    }
    commands.splice(start..start, replacement);
    commands
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Canvas, FullBleed, PdfProfile};

    fn retain(name: &str, bytes: &[u8]) {
        retain_in("inline-opacity", name, bytes);
    }

    fn retain_in(folder: &str, name: &str, bytes: &[u8]) {
        if let Some(directory) = std::env::var_os("FULLBLEED_LINK_TEST_OUTPUT") {
            let directory = std::path::PathBuf::from(directory).join(folder);
            std::fs::create_dir_all(&directory).unwrap();
            std::fs::write(directory.join(name), bytes).unwrap();
        }
    }

    #[test]
    fn inline_opacity_anonymous_whitespace_does_not_interrupt_block_margins() {
        let engine = FullBleed::builder().build().unwrap();
        let css = "@page{size:220pt 240pt;margin:20pt}body{font-family:Helvetica;font-size:12pt;line-height:18pt}.block{margin:18pt 0;padding:10pt;border:1pt solid black}";
        let first = r#"<div class="block">First</div>"#;
        let second = r#"<div class="block">Second</div>"#;
        let compact = engine
            .render_to_buffer(&format!("{first}{second}"), css)
            .unwrap();
        retain_in("block-whitespace", "compact.pdf", &compact);
        for (index, separator) in [" ", "\n  ", "<!-- formatting -->\n  ", "<span> </span>"]
            .into_iter()
            .enumerate()
        {
            let indented = engine
                .render_to_buffer(&format!("{first}{separator}{second}"), css)
                .unwrap();
            retain_in(
                "block-whitespace",
                &format!("separated-{index}.pdf"),
                &indented,
            );
            assert_eq!(compact, indented, "Formatting separator {separator:?}");
        }
        let preserved = format!("<section style=\"white-space:pre\">{first}\n{second}</section>");
        let no_break = format!("<section style=\"white-space:pre\">{first}{second}</section>");
        assert_ne!(
            engine.render_to_buffer(&preserved, css).unwrap(),
            engine.render_to_buffer(&no_break, css).unwrap()
        );
        let inline_first = r#"<span style="display:inline-block;width:20pt">A</span>"#;
        let inline_second = r#"<span style="display:inline-block;width:20pt">B</span>"#;
        let inline_compact = engine
            .render_to_buffer(&format!("{inline_first}{inline_second}"), css)
            .unwrap();
        let inline_spaced = engine
            .render_to_buffer(&format!("{inline_first} {inline_second}"), css)
            .unwrap();
        retain_in("block-whitespace", "inline-compact.pdf", &inline_compact);
        retain_in("block-whitespace", "inline-spaced.pdf", &inline_spaced);
        assert_ne!(
            inline_compact, inline_spaced,
            "Inline spaces retain their advance"
        );
    }

    #[test]
    fn inline_opacity_block_text_uses_its_emitted_semantic_owner() {
        let text = r#"Before <span class="fade">Rich text</span> after."#;
        let mut cases = Vec::new();
        for (context, start, end) in [
            ("root", "<div>", "</div>"),
            ("paragraph", "<p>", "</p>"),
            ("heading", "<h1>Document</h1><h2>", "</h2>"),
            (
                "link",
                r#"<a href="https://example.invalid/report" style="display:block">"#,
                "</a>",
            ),
        ] {
            for tag in ["span", "i", "div"] {
                cases.push((
                    format!("{context}-{tag}"),
                    format!("{start}<{tag} class=\"box\">{text}</{tag}>{end}"),
                ));
            }
        }
        for (name, start, end) in [
            (
                "contents-paragraph",
                r#"<p style="display:contents"><span class="box">"#,
                "</span></p>",
            ),
            (
                "flex-paragraph",
                r#"<p style="display:flex"><span>"#,
                "</span></p>",
            ),
            (
                "flow-root-span",
                r#"<div><span style="display:flow-root">"#,
                "</span></div>",
            ),
            (
                "atomic-span",
                r#"<div><span style="display:inline-block">"#,
                "</span></div>",
            ),
            (
                "nested-block-span",
                r#"<div>Lead <span class="box">First <span class="box">"#,
                "</span> Last</span> tail</div>",
            ),
            (
                "definition-label",
                r#"<dl><dt><span class="box">"#,
                "</span></dt><dd>Definition</dd></dl>",
            ),
            (
                "table-cell",
                r#"<table><tr><th scope="col">Header</th></tr><tr><td><span class="box">"#,
                "</span></td></tr></table>",
            ),
        ] {
            cases.push((name.to_owned(), format!("{start}{text}{end}")));
        }
        for (profile_name, profile) in [("ua1", PdfProfile::PdfUa1), ("ua2", PdfProfile::PdfUa2)] {
            let engine = FullBleed::builder()
                .register_font_file("python/fullbleed_assets/fonts/NotoSans-Regular.ttf")
                .pdf_profile(profile)
                .document_lang("en-US")
                .document_title("Block text ownership")
                .build()
                .unwrap();
            for (name, body) in &cases {
                for alpha in [1.0, 0.5] {
                    let css = format!(
                        "@page{{size:300pt 300pt;margin:16pt}}body{{font-family:'Noto Sans';font-size:12pt;line-height:18pt}}h1,h2{{font-weight:normal}}p{{margin:0}}.box{{display:block}}.fade{{opacity:{alpha}}}"
                    );
                    let bytes = engine.render_to_buffer(body, &css).unwrap();
                    assert_eq!(bytes, engine.render_to_buffer(body, &css).unwrap());
                    retain_in(
                        "block-text-owners",
                        &format!("{name}-{profile_name}-{alpha}.pdf"),
                        &bytes,
                    );
                    let pdf = crate::pdf_native::Document::load_mem(&bytes).unwrap();
                    let mut roles = Vec::new();
                    for dict in pdf
                        .objects
                        .values()
                        .filter_map(|object| object.as_dict().ok())
                    {
                        let Ok(role) = dict.get(b"S").and_then(|value| value.as_name()) else {
                            continue;
                        };
                        roles.push(role);
                        let Some(parent) = dict
                            .get(b"P")
                            .and_then(|value| value.as_reference())
                            .ok()
                            .and_then(|id| pdf.objects.get(&id))
                            .and_then(|object| object.as_dict().ok())
                        else {
                            continue;
                        };
                        let Ok(parent_role) = parent.get(b"S").and_then(|value| value.as_name())
                        else {
                            continue;
                        };
                        assert!(
                            !(role == b"Span" && parent_role == b"Document"),
                            "Unowned inline text: {name}/{profile_name}/{alpha}"
                        );
                        assert!(
                            !(role == b"P"
                                && matches!(parent_role, b"P" | b"H1" | b"H2" | b"Link" | b"Lbl")),
                            "Nested paragraph in existing inline owner: {name}/{profile_name}/{alpha}"
                        );
                    }
                    if name.starts_with("heading-") {
                        assert_eq!(roles.iter().filter(|role| **role == b"H2").count(), 1);
                    } else if name.starts_with("root-")
                        || matches!(
                            name.as_str(),
                            "contents-paragraph"
                                | "flow-root-span"
                                | "atomic-span"
                                | "flex-paragraph"
                        )
                    {
                        assert_eq!(
                            roles.iter().filter(|role| **role == b"P").count(),
                            1,
                            "{name}/{profile_name}/{alpha}"
                        );
                    } else if name == "nested-block-span" {
                        assert_eq!(
                            roles.iter().filter(|role| **role == b"P").count(),
                            5,
                            "{profile_name}/{alpha}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn inline_opacity_anonymous_text_owners_do_not_reparent_figures() {
        for (profile_name, profile) in [("ua1", PdfProfile::PdfUa1), ("ua2", PdfProfile::PdfUa2)] {
            let engine = FullBleed::builder()
                .register_font_file("python/fullbleed_assets/fonts/NotoSans-Regular.ttf")
                .pdf_profile(profile)
                .document_lang("en-US")
                .document_title("Figure and caption ownership")
                .build()
                .unwrap();
            let image = r#"<img width="40" height="30" alt="Red tile" src="data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAIAAAD91JpzAAAAEElEQVR4nGP8zwACTGCSAQANHQEDgslx/wAAAABJRU5ErkJggg=="/>"#;
            for (name, body, expected_role) in [
                (
                    "single",
                    format!("<figure>{image}<figcaption>First caption.</figcaption></figure>"),
                    b"Figure".as_slice(),
                ),
                (
                    "before",
                    format!("<figure><figcaption>Caption first.</figcaption>{image}</figure>"),
                    b"Figure".as_slice(),
                ),
                (
                    "multiple",
                    format!(
                        "<figure>{image} {image}<figcaption>Related illustrations.</figcaption></figure>"
                    ),
                    b"Sect".as_slice(),
                ),
                (
                    "faded-image",
                    format!(
                        "<figure>{}<figcaption>Transparent image.</figcaption></figure>",
                        image.replace("<img ", "<img style=\"opacity:0.5\" ")
                    ),
                    b"Figure".as_slice(),
                ),
            ] {
                let css = "@page{size:220pt 180pt;margin:12pt}body{font-family:'Noto Sans';font-size:12pt;line-height:18pt}";
                let bytes = engine.render_to_buffer(&body, css).unwrap();
                assert_eq!(bytes, engine.render_to_buffer(&body, css).unwrap());
                let pdf = crate::pdf_native::Document::load_mem(&bytes).unwrap();
                let captions: Vec<_> = pdf
                    .objects
                    .values()
                    .filter_map(|object| object.as_dict().ok())
                    .filter(|dict| {
                        dict.get(b"S").and_then(|value| value.as_name()).ok() == Some(b"Caption")
                    })
                    .collect();
                assert_eq!(captions.len(), 1);
                let parent_id = captions[0].get(b"P").unwrap().as_reference().unwrap();
                let parent = pdf.objects[&parent_id].as_dict().unwrap();
                assert_eq!(
                    parent.get(b"S").unwrap().as_name().unwrap(),
                    expected_role,
                    "{profile_name}/{name}"
                );
                let roles: Vec<_> = pdf
                    .objects
                    .values()
                    .filter_map(|object| object.as_dict().ok())
                    .filter(|dict| {
                        dict.get(b"P").and_then(|value| value.as_reference()).ok()
                            == Some(parent_id)
                    })
                    .filter_map(|dict| dict.get(b"S").and_then(|value| value.as_name()).ok())
                    .collect();
                assert!(
                    !roles.contains(&b"P".as_slice()),
                    "An image-only row must not gain a paragraph: {name}"
                );
                if expected_role == b"Figure" {
                    assert_eq!(parent.get(b"Alt").unwrap().as_str().unwrap(), b"Red tile");
                }
                retain_in(
                    "figure-ownership",
                    &format!("{name}-{profile_name}.pdf"),
                    &bytes,
                );
            }
        }
    }

    #[test]
    fn inline_opacity_text_measurement_keeps_shaped_and_legacy_caches_distinct() {
        let engine = FullBleed::builder()
            .register_font_file("python/fullbleed_assets/fonts/NotoSans-Regular.ttf")
            .build()
            .unwrap();
        let registry = &engine.font_registry;
        let size = Pt::from_f32(12.0);
        let options = crate::text_shape::ShapeOptions::default();
        let estimated = registry.measure_text_width("Noto Sans", size, "Before");
        let shaped = registry.measure_shaped_text_width("Noto Sans", size, "Before", options);
        assert!(shaped < estimated, "this fixture has real kerning");
        assert_eq!(
            estimated,
            registry.measure_text_width("Noto Sans", size, "Before")
        );
        assert_eq!(
            shaped,
            registry.measure_shaped_text_width("Noto Sans", size, "Before", options)
        );
        let no_kerning = crate::text_shape::ShapeOptions {
            kerning: false,
            ..options
        };
        assert_eq!(
            estimated,
            registry.measure_shaped_text_width("Noto Sans", size, "Before", no_kerning)
        );
    }

    #[test]
    fn inline_opacity_combines_fragments_and_preserves_inherited_state() {
        let size = Size {
            width: Pt::from_f32(120.0),
            height: Pt::from_f32(120.0),
        };
        let mut canvas = Canvas::new(size);
        let effect = InlineOpacity { id: 1, alpha: 0.5 };
        canvas.begin_tag("P", None, None, None, None, true);
        canvas.begin_inline_opacity(effect);
        canvas.set_fill_color(crate::Color::rgb(1.0, 0.0, 0.0));
        canvas.draw_rect(
            Pt::from_f32(10.0),
            Pt::from_f32(10.0),
            Pt::from_f32(40.0),
            Pt::from_f32(30.0),
        );
        canvas.end_inline_opacity(effect);
        // These independent graphics and structure scopes intentionally cross.
        canvas.save_state();
        canvas.begin_tag("Span", None, None, None, None, false);
        canvas.set_fill_color(crate::Color::rgb(0.0, 0.0, 1.0));
        canvas.draw_rect(
            Pt::from_f32(80.0),
            Pt::from_f32(10.0),
            Pt::from_f32(20.0),
            Pt::from_f32(20.0),
        );
        canvas.restore_state();
        canvas.end_tag();
        canvas.begin_inline_opacity(effect);
        canvas.draw_rect(
            Pt::from_f32(30.0),
            Pt::from_f32(20.0),
            Pt::from_f32(40.0),
            Pt::from_f32(30.0),
        );
        canvas.end_inline_opacity(effect);
        // The red state set by the group also applies to this opaque sibling.
        canvas.draw_rect(
            Pt::from_f32(10.0),
            Pt::from_f32(70.0),
            Pt::from_f32(20.0),
            Pt::from_f32(20.0),
        );
        canvas.end_tag();
        let document = canvas.finish();
        let commands = &document.pages[0].commands;
        assert!(!format!("{commands:?}").contains(BEGIN));
        assert_eq!(
            commands
                .iter()
                .filter(|c| matches!(c, Command::DrawForm { .. }))
                .count(),
            1
        );
        let pdf = crate::pdf::document_to_pdf_with_metrics_and_registry(
            &document,
            None,
            None,
            &crate::PdfOptions::default(),
        )
        .unwrap();
        retain("canvas-overlap.pdf", &pdf);
    }

    #[test]
    fn inline_opacity_retained_prefix_keeps_clip_transform_and_paint_order() {
        let size = Size {
            width: Pt::from_f32(120.0),
            height: Pt::from_f32(120.0),
        };
        for path_clip in [false, true] {
            let mut canvas = Canvas::new(size);
            let effect = InlineOpacity { id: 1, alpha: 0.5 };
            canvas.begin_tag("P", None, None, None, None, true);
            canvas.save_state();
            canvas.translate(Pt::from_f32(10.0), Pt::from_f32(10.0));
            if path_clip {
                canvas.move_to(Pt::ZERO, Pt::ZERO);
                canvas.line_to(Pt::from_f32(50.0), Pt::ZERO);
                canvas.line_to(Pt::from_f32(50.0), Pt::from_f32(40.0));
                canvas.line_to(Pt::ZERO, Pt::from_f32(40.0));
                canvas.close_path();
                canvas.clip_path(false);
            } else {
                canvas.clip_rect(Pt::ZERO, Pt::ZERO, Pt::from_f32(50.0), Pt::from_f32(40.0));
            }
            // This earlier sibling must paint before the transparent red group.
            canvas.set_fill_color(crate::Color::rgb(0.0, 0.0, 1.0));
            canvas.draw_rect(Pt::ZERO, Pt::ZERO, Pt::from_f32(50.0), Pt::from_f32(40.0));
            canvas.begin_inline_opacity(effect);
            canvas.set_fill_color(crate::Color::rgb(1.0, 0.0, 0.0));
            canvas.draw_rect(
                Pt::from_f32(20.0),
                Pt::ZERO,
                Pt::from_f32(60.0),
                Pt::from_f32(40.0),
            );
            canvas.end_inline_opacity(effect);
            canvas.restore_state();
            canvas.end_tag();
            canvas.begin_tag("P", None, None, None, None, true);
            canvas.begin_inline_opacity(effect);
            canvas.set_fill_color(crate::Color::rgb(1.0, 0.0, 0.0));
            canvas.draw_rect(
                Pt::from_f32(80.0),
                Pt::from_f32(10.0),
                Pt::from_f32(20.0),
                Pt::from_f32(20.0),
            );
            canvas.end_inline_opacity(effect);
            // The later sibling inherits red, but neither opacity nor clipping.
            canvas.draw_rect(
                Pt::from_f32(80.0),
                Pt::from_f32(50.0),
                Pt::from_f32(20.0),
                Pt::from_f32(20.0),
            );
            canvas.end_tag();
            let document = canvas.finish();
            let options = crate::PdfOptions::default();
            let render = || {
                crate::pdf::document_to_pdf_with_metrics_and_registry(
                    &document, None, None, &options,
                )
                .unwrap()
            };
            let pdf = render();
            assert_eq!(pdf, render());
            retain(
                if path_clip {
                    "canvas-prefix-path.pdf"
                } else {
                    "canvas-prefix-rect.pdf"
                },
                &pdf,
            );
        }
    }

    #[test]
    fn inline_opacity_group_identity_is_local_to_each_reusable_placement() {
        for (name, profile) in [("ua1", PdfProfile::PdfUa1), ("ua2", PdfProfile::PdfUa2)] {
            let engine = FullBleed::builder()
                .register_font_file("python/fullbleed_assets/fonts/NotoSans-Regular.ttf")
                .pdf_profile(profile)
                .document_lang("en-US")
                .document_title("Repeated logical paragraphs")
                .build()
                .unwrap();
            let mut content = engine.render_to_document(
                r#"<div>Before <span style="opacity:0.5">First <div>Block inside an inline</div> Last</span> After</div>"#,
                "@page{size:220pt 180pt;margin:12pt}body{font-family:'Noto Sans';font-size:12pt;line-height:18pt}",
            ).unwrap();
            assert_eq!(content.pages.len(), 1);
            let content_size = content.page_size;
            let page = content.pages.remove(0);
            let size = Size {
                width: Pt::from_f32(220.0),
                height: Pt::from_f32(360.0),
            };
            let mut canvas = Canvas::new(size);
            canvas.define_form(
                "paragraphs",
                content_size.width,
                content_size.height,
                page.commands,
            );
            for y in [0.0, 180.0] {
                canvas.draw_form(
                    Pt::ZERO,
                    Pt::from_f32(y),
                    content_size.width,
                    content_size.height,
                    "paragraphs",
                );
            }
            let document = canvas.finish();
            let render = || {
                crate::pdf::document_to_pdf_with_metrics_and_registry(
                    &document,
                    None,
                    Some(&engine.font_registry),
                    &engine.pdf_options,
                )
                .unwrap()
            };
            let bytes = render();
            assert_eq!(bytes, render());
            let pdf = crate::pdf_native::Document::load_mem(&bytes).unwrap();
            let paragraphs = pdf
                .objects
                .values()
                .filter_map(|object| object.as_dict().ok())
                .filter(|dict| dict.get(b"S").and_then(|value| value.as_name()).ok() == Some(b"P"))
                .count();
            assert_eq!(
                paragraphs, 6,
                "Each placement keeps three independent logical paragraphs"
            );
            retain(&format!("repeated-{name}-half.pdf"), &bytes);
        }
    }

    #[test]
    fn inline_opacity_html_fragments_keep_layout_links_and_nested_groups() {
        let cases = [
            ("block-text", r#"<div>Before <span class="fade">Rich text</span> after.</div>"#.to_string()),
            ("block-quote", r#"<blockquote>Before <span class="fade">Rich quote</span> after.</blockquote>"#.to_string()),
            ("styled", r#"<p>Before <span class="fade">A long inline span with <b style="font-weight:bold">bold words</b>, <i style="font-style:italic">italic words</i>, and ordinary text that wraps across several lines.</span> After.</p>"#.to_string()),
            ("bidi", r#"<p style="direction:rtl;unicode-bidi:bidi-override">A <span class="fade">B C <b style="font-weight:bold">D E</b> F G H I J K L M N O P Q R S T U V W X Y Z</span> 1234</p>"#.to_string()),
            ("raster", r#"<p><a class="fade" href="https://example.invalid/image">Image <img alt="Red tile" width="40" height="30" src="data:image/png;base64,ENCODED"/> and text</a></p><p>After</p>"#.replace("ENCODED", "iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAIAAAD91JpzAAAAEElEQVR4nGP8zwACTGCSAQANHQEDgslx/wAAAABJRU5ErkJggg==")),
            ("overlap", r#"<p><span class="fade"><i class="tile"></i><i class="tile second"></i></span></p><p>After</p>"#.to_string()),
            ("lines", r#"<p class="tight"><span class="fade"><b>Overlap</b><br><b>Overlap</b><br><b>Overlap</b></span></p><p>After</p>"#.to_string()),
            ("nested", r#"<p><span class="fade"><i class="tile"></i><span style="opacity:0.5"><i class="tile"></i></span></span></p><p>After</p>"#.to_string()),
            ("wrapping", r#"<p>Before <span class="fade">A long inline span with <b>bold words</b>, <i>italic words</i>, and ordinary text that wraps across several lines without becoming an atomic box.</span> After.</p>"#.to_string()),
            ("pages", format!("<p>Before <span class=\"fade\">{}</span> After.</p>", "One two three four five six seven eight. ".repeat(30))),
            ("links", r##"<h1 id="target">Target</h1><p><a class="fade" href="#target">Read <span>details</span><svg width="36" height="24" viewBox="0 0 36 24" role="img" aria-label="Red icon"><rect width="36" height="24" fill="red"/></svg></a></p><p><a class="fade" href="https://example.invalid/report">External report</a></p>"##.to_string()),
            ("blocks", r#"<div>Before <span class="fade">First <div>Block inside an inline</div> Last</span> After</div>"#.to_string()),
            ("blocks-unwrapped", r#"<div>Before First<div>Block inside an inline</div>Last After</div>"#.to_string()),
            ("clipped", r#"<div class="clip"><p><span class="fade">A <i class="tile"></i><i class="tile second"></i> clipped inline <b>content</b> group</span></p></div><p>After</p>"#.to_string()),
        ];
        for (profile_name, profile) in [
            ("none", PdfProfile::None),
            ("ua1", PdfProfile::PdfUa1),
            ("ua2", PdfProfile::PdfUa2),
        ] {
            let engine = FullBleed::builder()
                .register_font_file("python/fullbleed_assets/fonts/NotoSans-Regular.ttf")
                .pdf_profile(profile)
                .document_lang("en-US")
                .document_title("Inline opacity regression")
                .build()
                .unwrap();
            for (name, html) in &cases {
                for (alpha_name, alpha) in [("zero", "0"), ("half", "0.5"), ("opaque", "1")] {
                    let css = format!(
                        "@page{{size:220pt 180pt;margin:12pt}}body{{font-family:'Noto Sans';font-size:12pt;line-height:18pt}}p{{margin:0 0 12pt}}h1{{font-size:18pt;font-weight:normal}}.fade{{opacity:{alpha}}}.tile{{display:inline-block;width:40pt;height:30pt;background:red}}.second{{margin-left:-20pt}}.tight{{line-height:8pt}}.tight b{{font-size:28pt;color:red}}.clip{{width:100pt;height:40pt;overflow:hidden;transform:translate(3pt,2pt)}}"
                    );
                    let bytes = engine.render_to_buffer(html, &css).unwrap_or_else(|error| {
                        panic!("{name}-{profile_name}-{alpha_name}: {error}")
                    });
                    assert_eq!(
                        bytes,
                        engine.render_to_buffer(html, &css).unwrap(),
                        "deterministic {name}"
                    );
                    retain(&format!("{name}-{profile_name}-{alpha_name}.pdf"), &bytes);
                    if profile_name == "none" {
                        let document = engine.render_to_document(html, &css).unwrap();
                        retain(
                            &format!("{name}-{alpha_name}.commands.txt"),
                            format!("{document:#?}").as_bytes(),
                        );
                        retain(&format!("{name}-{alpha_name}.html"), html.as_bytes());
                        retain(&format!("{name}-{alpha_name}.css"), css.as_bytes());
                    }
                }
            }
        }
    }
}
