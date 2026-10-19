use super::*;

fn pt(value: i32) -> Pt {
    Pt::from_i32(value)
}
fn size() -> Size {
    Size {
        width: pt(100),
        height: pt(40),
    }
}
fn rect(x: i32, y: i32, width: i32, height: i32) -> Command {
    Command::DrawRect {
        x: pt(x),
        y: pt(y),
        width: pt(width),
        height: pt(height),
    }
}
fn run(commands: &[Command]) -> Result<Option<Bounds>, Unknown> {
    measure(
        commands,
        size(),
        &FontRegistry::new(),
        &PdfOptions::default(),
    )
}
fn extent(commands: &[Command], expected: [f64; 4]) {
    let bounds = run(commands).unwrap().unwrap();
    for (actual, expected) in [bounds.left, bounds.bottom, bounds.right, bounds.top]
        .into_iter()
        .zip(expected)
    {
        assert!(
            (actual - expected).abs() < 0.000001,
            "{bounds:?}, expected {expected}"
        );
    }
}
fn draw_form(name: &str, x: i32) -> Command {
    Command::DrawForm {
        resource_id: name.into(),
        x: pt(x),
        y: pt(0),
        width: pt(20),
        height: pt(20),
    }
}
fn form(name: &str, commands: Vec<Command>) -> Command {
    Command::DefineForm {
        resource_id: name.into(),
        width: pt(20),
        height: pt(20),
        commands,
    }
}

#[test]
fn empty_is_distinct_from_unknown_and_finite() {
    assert_eq!(run(&[]).unwrap(), None);
    assert!(
        run(&[rect(0, 0, 100, 40)])
            .unwrap()
            .unwrap()
            .fits(pt(100), pt(40))
    );
    assert!(
        !run(&[rect(0, 0, 101, 40)])
            .unwrap()
            .unwrap()
            .fits(pt(100), pt(40))
    );
    let unknown = run(&[draw_form("missing", 0)]).unwrap_err();
    assert_eq!(unknown.reason, "missing_form");
    assert_eq!(unknown.command, 0);
}

#[test]
fn transforms_use_pdf_coordinates_and_composition_order() {
    extent(&[rect(10, 5, 20, 10)], [10.0, 25.0, 30.0, 35.0]);
    extent(
        &[Command::Translate(pt(7), pt(3)), rect(10, 5, 20, 10)],
        [17.0, 22.0, 37.0, 32.0],
    );
    extent(
        &[
            Command::Translate(pt(70), pt(0)),
            Command::Scale(-2.0, 1.0),
            rect(10, 5, 20, 10),
        ],
        [10.0, 25.0, 50.0, 35.0],
    );
    extent(
        &[
            Command::ConcatMatrix {
                a: 1.0,
                b: 0.0,
                c: 0.5,
                d: 1.0,
                e: pt(7),
                f: pt(3),
            },
            rect(10, 5, 20, 10),
        ],
        [-0.5, 22.0, 24.5, 32.0],
    );
}

#[test]
fn rotation_about_css_origin_can_reveal_overflow() {
    let commands = [
        Command::CssTransformOrigin {
            x: pt(50),
            y: pt(20),
            inverse: false,
        },
        Command::Rotate(std::f32::consts::FRAC_PI_2),
        Command::CssTransformOrigin {
            x: pt(50),
            y: pt(20),
            inverse: true,
        },
        rect(0, 10, 100, 20),
    ];
    let bounds = run(&commands).unwrap().unwrap();
    assert!(!bounds.fits(pt(100), pt(40)));
    // A quarter turn exchanges the extents.
    assert!((bounds.right - bounds.left - 20.0).abs() < 0.01);
    assert!((bounds.top - bounds.bottom - 100.0).abs() < 0.01);
}

#[test]
fn authored_clip_is_distinct_from_form_bbox() {
    let clip = Command::ClipRect {
        x: pt(0),
        y: pt(0),
        width: pt(20),
        height: pt(20),
    };
    assert_eq!(
        run(&[form("f", vec![rect(0, 0, 30, 20)]), draw_form("f", 0)])
            .unwrap_err()
            .reason,
        "nested_form_clip"
    );
    extent(
        &[form("f", vec![clip, rect(0, 0, 30, 20)]), draw_form("f", 0)],
        [0.0, 20.0, 20.0, 40.0],
    );
}

#[test]
fn repeated_and_nested_forms_keep_placement_and_trace() {
    extent(
        &[
            form("f", vec![rect(0, 0, 20, 20)]),
            draw_form("f", 0),
            draw_form("f", 30),
        ],
        [0.0, 20.0, 50.0, 40.0],
    );
    let error = run(&[
        form("outer", vec![draw_form("missing", 0)]),
        draw_form("outer", 0),
    ])
    .unwrap_err();
    assert_eq!(error.forms, vec!["outer"]);
    assert_eq!(error.reason, "missing_form");
    assert_eq!(
        run(&[
            form("loop", vec![draw_form("loop", 0)]),
            draw_form("loop", 0)
        ])
        .unwrap_err()
        .reason,
        "recursive_form"
    );
}

#[test]
fn form_redefinition_cannot_change_an_earlier_measurement() {
    let definition = form("f", vec![rect(0, 0, 20, 20)]);
    assert!(run(&[definition.clone(), definition, draw_form("f", 0)]).is_ok());
    assert_eq!(
        run(&[
            form("f", vec![rect(0, 0, 20, 20)]),
            form("f", vec![rect(0, 0, 10, 10)])
        ])
        .unwrap_err()
        .reason,
        "resource_redefinition"
    );
}

#[test]
fn current_path_is_not_saved_by_graphics_state() {
    extent(
        &[
            Command::MoveTo { x: pt(1), y: pt(2) },
            Command::SaveState,
            Command::Translate(pt(10), pt(0)),
            Command::LineTo { x: pt(2), y: pt(5) },
            Command::RestoreState,
            Command::Fill,
        ],
        [1.0, 35.0, 12.0, 38.0],
    );
}

#[test]
fn clipping_consumes_path_and_q_restores_clip() {
    let clip = Command::ClipRect {
        x: pt(10),
        y: pt(10),
        width: pt(20),
        height: pt(20),
    };
    assert_eq!(run(&[clip.clone(), Command::Fill]).unwrap(), None);
    extent(
        &[
            Command::SaveState,
            clip.clone(),
            rect(0, 0, 100, 40),
            Command::RestoreState,
            rect(80, 5, 10, 10),
        ],
        [10.0, 10.0, 90.0, 35.0],
    );
    assert_eq!(
        run(&[
            clip,
            Command::ClipPath { evenodd: true },
            rect(0, 0, 100, 40)
        ])
        .unwrap(),
        None
    );
}

#[test]
fn rectangle_operators_append_to_the_current_path() {
    extent(
        &[Command::MoveTo { x: pt(1), y: pt(1) }, rect(10, 10, 10, 10)],
        [1.0, 20.0, 20.0, 39.0],
    );
    extent(
        &[
            Command::MoveTo { x: pt(1), y: pt(1) },
            Command::ClipRect {
                x: pt(10),
                y: pt(10),
                width: pt(10),
                height: pt(10),
            },
            rect(0, 0, 100, 40),
        ],
        [1.0, 20.0, 20.0, 39.0],
    );
}

#[test]
fn stroke_bounds_include_caps_and_miters_but_reject_hairlines() {
    let path = [
        Command::MoveTo {
            x: pt(10),
            y: pt(20),
        },
        Command::LineTo {
            x: pt(90),
            y: pt(20),
        },
        Command::Stroke,
    ];
    let bounds = run(&path).unwrap().unwrap();
    assert!(
        bounds.left <= 5.0 && bounds.right >= 95.0 && bounds.bottom <= 15.0 && bounds.top >= 25.0
    );
    let mut hairline = vec![Command::SetLineWidth(Pt::ZERO)];
    hairline.extend(path);
    assert_eq!(run(&hairline).unwrap_err().reason, "hairline_stroke");
}

#[test]
fn transparent_fill_does_not_hide_visible_stroke() {
    assert_eq!(
        run(&[
            Command::SetOpacity {
                fill: 0.0,
                stroke: 1.0
            },
            rect(0, 0, 100, 40)
        ])
        .unwrap(),
        None
    );
    assert!(
        run(&[
            Command::SetOpacity {
                fill: 0.0,
                stroke: 1.0
            },
            Command::MoveTo {
                x: pt(10),
                y: pt(20)
            },
            Command::LineTo {
                x: pt(90),
                y: pt(20)
            },
            Command::FillStroke
        ])
        .unwrap()
        .is_some()
    );
}

#[test]
fn malformed_and_unbounded_programs_fail_closed() {
    assert_eq!(
        run(&[Command::RestoreState]).unwrap_err().reason,
        "graphics_state_underflow"
    );
    assert_eq!(
        run(&[Command::SaveState]).unwrap_err().reason,
        "unbalanced_graphics_state"
    );
    assert_eq!(
        run(&[Command::Scale(f32::NAN, 1.0)]).unwrap_err().reason,
        "nonfinite_transform"
    );
    assert_eq!(
        run(&vec![
            Command::Meta {
                key: "x".into(),
                value: "y".into()
            };
            100_001
        ])
        .unwrap_err()
        .reason,
        "command_budget"
    );
}

fn fonts() -> FontRegistry {
    let mut registry = FontRegistry::new();
    registry
        .register_bytes(
            include_bytes!("../preview_fonts/FullbleedPreviewSans-Regular.ttf").to_vec(),
            Some("FullbleedPreviewSans-Regular.ttf"),
        )
        .unwrap();
    registry
}

#[test]
fn embedded_text_accounts_for_descenders_and_positioning() {
    let registry = fonts();
    let commands = [
        Command::SetFontName("Fullbleed Preview Sans".into()),
        Command::SetFontSize(pt(12)),
        Command::DrawString {
            x: pt(4),
            y: pt(4),
            text: "Agj ffi A\u{301}".into(),
        },
    ];
    let bounds = measure(&commands, size(), &registry, &PdfOptions::default())
        .unwrap()
        .unwrap();
    assert!(
        bounds.bottom < 24.0,
        "descenders must pass below the baseline: {bounds:?}"
    );
    assert!(bounds.fits(pt(100), pt(40)));
    let mut moved = vec![Command::Translate(pt(100), pt(0))];
    moved.extend(commands);
    assert!(
        !measure(&moved, size(), &registry, &PdfOptions::default())
            .unwrap()
            .unwrap()
            .fits(pt(100), pt(40))
    );
}

#[test]
fn unresolved_or_unembedded_text_does_not_claim_fit() {
    let text = Command::DrawString {
        x: pt(4),
        y: pt(4),
        text: "text".into(),
    };
    assert_eq!(
        run(&[text.clone()]).unwrap_err().reason,
        "unresolved_paint_font"
    );
    let registry = fonts();
    let options = PdfOptions {
        unicode_support: false,
        ..PdfOptions::default()
    };
    assert_eq!(
        measure(
            &[
                Command::SetFontName("Fullbleed Preview Sans".into()),
                text.clone()
            ],
            size(),
            &registry,
            &options
        )
        .unwrap_err()
        .reason,
        "unsupported_paint_font_encoding"
    );
    assert_eq!(
        run(&[Command::SetTextRenderingMode(3), text]).unwrap(),
        None
    );
}

#[test]
fn isolated_group_resets_inner_alpha_instead_of_losing_visible_strokes() {
    let commands = vec![
        Command::MoveTo {
            x: pt(6),
            y: pt(10),
        },
        Command::LineTo {
            x: pt(14),
            y: pt(10),
        },
        Command::Stroke,
    ];
    let definition = Command::DefineIsolatedForm {
        resource_id: "group".into(),
        width: pt(20),
        height: pt(20),
        commands,
    };
    let ink = run(&[
        Command::SetOpacity {
            fill: 1.0,
            stroke: 0.0,
        },
        definition,
        draw_form("group", 0),
    ])
    .unwrap()
    .unwrap();
    assert!(ink.left <= 1.0 && ink.right >= 19.0);
}

#[test]
fn large_offscreen_transforms_cannot_pass_a_finite_sampling_window() {
    for scale in [f32::MAX, -f32::MAX] {
        let bounds = run(&[Command::Scale(scale, 1.0), rect(10, 5, 20, 10)])
            .unwrap()
            .unwrap();
        assert!(!bounds.fits(pt(100), pt(40)));
    }
    let mut program = vec![Command::Scale(f32::MAX, f32::MAX); 32];
    program.push(rect(10, 5, 20, 10));
    assert_eq!(run(&program).unwrap_err().reason, "nonfinite_geometry");
}

#[test]
fn invisible_fill_does_not_suppress_a_stroked_text_unknown() {
    let error = run(&[
        Command::SetOpacity {
            fill: 0.0,
            stroke: 1.0,
        },
        Command::SetTextRenderingMode(1),
        Command::DrawString {
            x: pt(0),
            y: pt(0),
            text: "A".into(),
        },
    ])
    .unwrap_err();
    assert_eq!(error.reason, "stroked_text");
}

#[test]
fn unreadable_glyph_is_unknown_while_a_bounded_empty_program_is_empty() {
    let mut bytes = include_bytes!("../preview_fonts/FullbleedPreviewSans-Regular.ttf").to_vec();
    let (gid, position) = {
        let face = crate::sfnt::Face::parse(&bytes, 0).unwrap();
        let gid = face.glyph_index(u32::from('A')).unwrap();
        let loca = face.table(*b"loca").unwrap();
        let head = face.table(*b"head").unwrap();
        let index = usize::from(gid.0);
        let offset = if i16::from_be_bytes(head[50..52].try_into().unwrap()) == 1 {
            u32::from_be_bytes(loca[index * 4..index * 4 + 4].try_into().unwrap()) as usize
        } else {
            usize::from(u16::from_be_bytes(
                loca[index * 2..index * 2 + 2].try_into().unwrap(),
            )) * 2
        };
        let glyf = face.table(*b"glyf").unwrap();
        (
            gid,
            glyf.as_ptr() as usize - bytes.as_ptr() as usize + offset,
        )
    };
    // Make a bounded but unreadable composite: its component glyph is absent.
    bytes[position..position + 2].copy_from_slice(&(-1_i16).to_be_bytes());
    bytes[position + 10..position + 14].copy_from_slice(&[0, 0, 255, 255]);
    let mut registry = FontRegistry::new();
    let name = registry.register_bytes(bytes, Some("fixture.ttf")).unwrap();
    assert!(registry.glyph_outline_for_id(&name, gid.0).is_none());
    assert!(!registry.glyph_program_is_empty(&name, gid.0));
    let text = |text: &str| {
        [
            Command::SetFontName(name.clone()),
            Command::DrawString {
                x: pt(4),
                y: pt(4),
                text: text.into(),
            },
        ]
    };
    assert_eq!(
        measure(&text("A"), size(), &registry, &PdfOptions::default())
            .unwrap_err()
            .reason,
        "unresolved_glyph_outline"
    );
    assert_eq!(
        measure(&text(" "), size(), &registry, &PdfOptions::default()).unwrap(),
        None
    );
}
