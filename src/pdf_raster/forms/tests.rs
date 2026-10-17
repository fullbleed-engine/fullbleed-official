use super::*;
use crate::pdf_native::dictionary;

fn group(doc: &mut LoDocument, resources: LoDictionary, content: &str) -> ObjectId {
    let attributes = doc.add_object(dictionary! {
        "S" => "Transparency", "I" => true, "K" => false,
    });
    doc.add_object(LoStream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Form", "FormType" => 1,
            "BBox" => vec![0.into(), 0.into(), 80.into(), 80.into()],
            "Group" => attributes, "Resources" => resources,
        },
        content.as_bytes().to_vec(),
    ))
}

fn image(
    mut doc: LoDocument,
    resources: LoDictionary,
    content: &str,
) -> crate::image_native::RgbaImage {
    let pages = doc.new_object_id();
    let stream = doc.add_object(LoStream::new(
        LoDictionary::new(),
        content.as_bytes().to_vec(),
    ));
    let page = doc.add_object(dictionary! {
        "Type" => "Page", "Parent" => pages, "MediaBox" => vec![0.into(), 0.into(), 100.into(), 100.into()],
        "Resources" => resources, "Contents" => stream,
    });
    doc.objects.insert(
        pages,
        dictionary! {"Type" => "Pages", "Kids" => vec![page.into()], "Count" => 1}.into(),
    );
    let root = doc.add_object(dictionary! {"Type" => "Catalog", "Pages" => pages});
    doc.trailer.set("Root", root);
    doc.compress();
    let mut bytes = Vec::new();
    doc.save_to(&mut bytes).unwrap();
    let png =
        pdf_bytes_to_png_pages(&bytes, 72, None, false).expect("finalized isolated-group preview");
    assert_eq!(
        png,
        pdf_bytes_to_png_pages(&bytes, 72, None, false).unwrap()
    );
    assert_eq!(png.len(), 1);
    crate::image_native::load_from_memory(&png[0])
        .unwrap()
        .into_rgba8()
}

fn resources(form: ObjectId) -> LoDictionary {
    dictionary! {
        "XObject" => dictionary! {"Group" => form},
        // A transparency group is composited using the nonstroking constant,
        // even when its members consist entirely of strokes (ISO 32000-1 11.6.6).
        "ExtGState" => dictionary! {"Half" => dictionary! {"ca" => 0.5, "CA" => 0.1}},
    }
}

fn color(image: &crate::image_native::RgbaImage, x: u32, y: u32, expected: [u8; 3]) {
    let actual = image.get_pixel(x, y).0;
    assert!(
        actual[..3]
            .iter()
            .zip(expected)
            .all(|(a, b)| a.abs_diff(b) <= 1),
        "pixel ({x},{y}): {actual:?}, expected {expected:?}"
    );
}

const OVERLAP: &str = "1 0 0 rg 0 0 80 80 re f 0 0 80 80 re f";

#[test]
fn finalized_isolated_group_composites_overlapping_members_once() {
    let mut doc = LoDocument::with_version("1.7");
    let form = group(&mut doc, LoDictionary::new(), OVERLAP);
    let result = image(doc, resources(form), "/Half gs /Group Do");
    color(&result, 40, 60, [255, 128, 128]);
    color(&result, 90, 60, [255, 255, 255]);
}

#[test]
fn finalized_isolated_group_keeps_member_alpha_separate() {
    let mut doc = LoDocument::with_version("1.7");
    let form = group(
        &mut doc,
        dictionary! {"ExtGState" => dictionary! {"Member" => dictionary! {"ca" => 0.5}}},
        &format!("/Member gs {OVERLAP}"),
    );
    let result = image(doc, resources(form), "/Half gs /Group Do");
    // Two half-opaque members yield 0.75 inside the group, then 0.375 on white.
    color(&result, 40, 60, [255, 159, 159]);
}

#[test]
fn finalized_isolated_group_uses_fill_alpha_for_stroked_members() {
    let mut doc = LoDocument::with_version("1.7");
    let form = group(
        &mut doc,
        LoDictionary::new(),
        "1 0 0 RG 30 w 0 40 m 80 40 l S 0 40 m 80 40 l S",
    );
    let result = image(doc, resources(form), "/Half gs /Group Do");
    color(&result, 40, 60, [255, 128, 128]);
}

#[test]
fn finalized_nested_isolated_groups_apply_each_alpha_once() {
    let mut doc = LoDocument::with_version("1.7");
    let inner = group(&mut doc, LoDictionary::new(), OVERLAP);
    let outer = group(&mut doc, resources(inner), "/Half gs /Group Do");
    let result = image(doc, resources(outer), "/Half gs /Group Do");
    color(&result, 40, 60, [255, 191, 191]);
}

#[test]
fn finalized_isolated_group_preserves_clip_transform_and_caller_state() {
    let mut doc = LoDocument::with_version("1.7");
    let form = group(&mut doc, LoDictionary::new(), OVERLAP);
    let result = image(
        doc,
        resources(form),
        "q 10 10 40 40 re W n /Half gs 1 0 0 1 20 20 cm /Group Do Q 0 0 1 rg 70 0 20 20 re f",
    );
    color(&result, 30, 70, [255, 128, 128]);
    color(&result, 55, 70, [255, 255, 255]);
    color(&result, 15, 70, [255, 255, 255]);
    color(&result, 80, 90, [0, 0, 255]);
}

#[test]
fn finalized_isolated_group_inherits_paint_without_inheriting_alpha() {
    let mut doc = LoDocument::with_version("1.7");
    let form = group(
        &mut doc,
        LoDictionary::new(),
        "0 0 80 80 re f 0 0 80 80 re f",
    );
    let result = image(doc, resources(form), "0 0 1 rg /Half gs /Group Do");
    color(&result, 40, 60, [128, 128, 255]);
}
