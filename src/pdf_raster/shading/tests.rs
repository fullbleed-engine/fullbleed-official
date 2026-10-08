use super::*;
use crate::pdf_native::dictionary;

fn exponential(c0: Vec<f32>, c1: Vec<f32>, exponent: f32) -> LoObject {
    dictionary! {
        "FunctionType" => 2,
        "Domain" => vec![0.into(), 1.into()],
        "C0" => c0.into_iter().map(LoObject::from).collect::<Vec<_>>(),
        "C1" => c1.into_iter().map(LoObject::from).collect::<Vec<_>>(),
        "N" => exponent,
    }
    .into()
}

fn red_blue() -> LoObject {
    exponential(vec![1.0, 0.0, 0.0], vec![0.0, 0.0, 1.0], 1.0)
}

fn axial(function: LoObject, coords: [f32; 4], extend: [bool; 2]) -> LoDictionary {
    dictionary! {
        "ShadingType" => 2,
        "ColorSpace" => "DeviceRGB",
        "Coords" => coords.into_iter().map(LoObject::from).collect::<Vec<_>>(),
        "Function" => function,
        "Extend" => extend.into_iter().map(LoObject::Boolean).collect::<Vec<_>>(),
    }
}

fn pdf(mut doc: LoDocument, resources: LoDictionary, content: &str) -> Vec<u8> {
    let pages = doc.new_object_id();
    let stream = doc.add_object(LoStream::new(
        LoDictionary::new(),
        content.as_bytes().to_vec(),
    ));
    let page = doc.add_object(dictionary! {
        "Type"=>"Page","Parent"=>pages,"MediaBox"=>vec![0.into(),0.into(),100.into(),100.into()],
        "Resources"=>resources,"Contents"=>stream,
    });
    doc.objects.insert(
        pages,
        dictionary! {"Type"=>"Pages","Kids"=>vec![page.into()],"Count"=>1}.into(),
    );
    let root = doc.add_object(dictionary! {"Type"=>"Catalog","Pages"=>pages});
    doc.trailer.set("Root", root);
    doc.compress();
    let mut bytes = Vec::new();
    doc.save_to(&mut bytes).unwrap();
    bytes
}

fn image(bytes: &[u8]) -> crate::image_native::RgbaImage {
    let png = pdf_bytes_to_png_pages(bytes, 72, None, false).expect("finalized gradient preview");
    assert_eq!(png.len(), 1);
    crate::image_native::load_from_memory(&png[0])
        .unwrap()
        .into_rgba8()
}

fn simple(shading: LoDictionary, content: &str) -> crate::image_native::RgbaImage {
    image(&pdf(
        LoDocument::with_version("1.7"),
        dictionary! {"Shading"=>dictionary! {"G"=>shading}},
        content,
    ))
}

fn color(image: &crate::image_native::RgbaImage, x: u32, y: u32, expected: [u8; 3], tolerance: u8) {
    let actual = image.get_pixel(x, y).0;
    assert!(
        actual[..3]
            .iter()
            .zip(expected)
            .all(|(a, b)| a.abs_diff(b) <= tolerance),
        "pixel ({x},{y}): {actual:?}, expected {expected:?}"
    );
}

#[test]
fn finalized_axial_obeys_clip_bbox_and_nonextended_domain() {
    let mut shading = axial(red_blue(), [20.0, 0.0, 80.0, 0.0], [false, false]);
    shading.set("BBox", vec![0.into(), 20.into(), 100.into(), 80.into()]);
    // Background is ignored by `sh`, so the nonextended regions stay white.
    shading.set("Background", vec![0.into(), 1.into(), 0.into()]);
    let image = simple(shading, "0 0 70 100 re W n /G sh");
    color(&image, 10, 50, [255, 255, 255], 0);
    color(&image, 50, 10, [255, 255, 255], 0);
    color(&image, 75, 50, [255, 255, 255], 0);
    color(&image, 50, 50, [125, 0, 130], 2);
    color(&image, 21, 50, [249, 0, 6], 2);
}

#[test]
fn finalized_axial_extends_across_the_clip_after_translation_and_scaling() {
    let shading = axial(red_blue(), [0.0, 0.0, 100.0, 0.0], [true, true]);
    let image = simple(shading, "0 0 100 100 re W n 0.5 0 0 0.5 50 10 cm /G sh");
    // Extending colors must not be clipped to a CTM-transformed page rectangle.
    color(&image, 10, 50, [255, 0, 0], 0);
    color(&image, 75, 50, [125, 0, 130], 2);
    color(&image, 99, 5, [3, 0, 252], 2);
}

#[test]
fn finalized_axial_shear_preserves_the_gradient_coordinate_field() {
    let image = simple(
        axial(red_blue(), [0.0, 0.0, 100.0, 0.0], [true, true]),
        "2 0 1 1 0 0 cm /G sh",
    );
    color(&image, 70, 50, [228, 0, 27], 2);
    color(&image, 20, 50, [255, 0, 0], 0);
}

#[test]
fn finalized_gradient_preserves_stitched_hard_stops_and_narrow_bands() {
    let constant = |rgb: Vec<f32>| exponential(rgb.clone(), rgb, 1.0);
    let function = dictionary! {
        "FunctionType"=>3,"Domain"=>vec![0.into(),1.into()],
        "Functions"=>vec![constant(vec![1.0,0.0,0.0]),constant(vec![0.0,1.0,0.0]),constant(vec![0.0,0.0,1.0])],
        "Bounds"=>vec![0.49.into(),0.51.into()],"Encode"=>vec![0.into(),1.into(),0.into(),1.into(),0.into(),1.into()],
    };
    let image = simple(
        axial(function.into(), [0.0, 0.0, 100.0, 0.0], [true, true]),
        "/G sh",
    );
    color(&image, 48, 50, [255, 0, 0], 0);
    color(&image, 49, 50, [0, 255, 0], 0);
    color(&image, 50, 50, [0, 255, 0], 0);
    color(&image, 51, 50, [0, 0, 255], 0);
}

#[test]
fn finalized_gradient_evaluates_exponents_function_arrays_and_encode() {
    let functions = LoObject::Array(vec![
        exponential(vec![0.0], vec![1.0], 2.0),
        exponential(vec![0.0], vec![0.0], 1.0),
        exponential(vec![1.0], vec![0.0], 1.0),
    ]);
    let image = simple(
        axial(functions, [0.0, 0.0, 100.0, 0.0], [true, true]),
        "/G sh",
    );
    color(&image, 50, 50, [65, 0, 126], 2);
    let function = dictionary! {"FunctionType"=>3,"Domain"=>vec![2.into(),4.into()],"Functions"=>vec![red_blue()],"Bounds"=>Vec::<LoObject>::new(),"Encode"=>vec![1.into(),0.into()]};
    let mut shading = axial(function.into(), [0.0, 0.0, 100.0, 0.0], [true, true]);
    shading.set("Domain", vec![2.into(), 4.into()]);
    let image = simple(shading, "/G sh");
    color(&image, 25, 50, [65, 0, 190], 2);
}

#[test]
fn finalized_radial_preserves_nonzero_start_radius() {
    let shading = dictionary! {"ShadingType"=>3,"ColorSpace"=>"DeviceRGB","Coords"=>vec![50.into(),50.into(),10.into(),50.into(),50.into(),40.into()],"Function"=>red_blue(),"Extend"=>vec![true.into(),true.into()]};
    let image = simple(shading, "/G sh");
    color(&image, 50, 50, [255, 0, 0], 0);
    color(&image, 75, 50, [123, 0, 132], 3);
    color(&image, 95, 50, [0, 0, 255], 0);
}

#[test]
fn finalized_gradient_resources_are_inherited_by_forms_with_a_bbox() {
    let mut doc = LoDocument::with_version("1.7");
    let form=doc.add_object(LoStream::new(dictionary! {"Type"=>"XObject","Subtype"=>"Form","BBox"=>vec![20.into(),20.into(),80.into(),80.into()],"Matrix"=>vec![1.into(),0.into(),0.into(),1.into(),5.into(),0.into()]},b"/G sh".to_vec()));
    let resources = dictionary! {"Shading"=>dictionary! {"G"=>axial(red_blue(),[0.0,0.0,100.0,0.0],[true,true])},"XObject"=>dictionary! {"Form"=>form}};
    let image = image(&pdf(doc, resources, "/Form Do 0 1 0 rg 0 0 10 10 re f"));
    color(&image, 10, 50, [255, 255, 255], 0);
    color(&image, 50, 50, [139, 0, 116], 2);
    color(&image, 90, 50, [255, 255, 255], 0);
    color(&image, 5, 95, [0, 255, 0], 0);
}

#[test]
fn finalized_html_gradient_keeps_the_real_chart_fill() {
    let engine = crate::FullBleed::builder().build().unwrap();
    let pdf=engine.render_to_buffer("<div></div>","@page {size:100pt 100pt;margin:10pt} div {width:80pt;height:50pt;background:linear-gradient(180deg,#d9bc77,#a9b884)}").unwrap();
    let image = image(&pdf);
    color(&image, 50, 20, [207, 187, 122], 3);
    color(&image, 50, 50, [178, 185, 130], 3);
    color(&image, 50, 80, [255, 255, 255], 0);
}

#[test]
fn finalized_html_gradient_respects_the_luminosity_soft_mask() {
    let engine = crate::FullBleed::builder().build().unwrap();
    let pdf=engine.render_to_buffer("<div></div>","@page {size:100pt 100pt;margin:0} body {background:#00ff00} div {width:100pt;height:100pt;background:linear-gradient(90deg,rgba(255,0,0,.5),rgba(0,0,255,.5))}").unwrap();
    let image = image(&pdf);
    color(&image, 50, 50, [63, 128, 64], 3);
}

#[test]
fn finalized_gradient_rejects_recursive_functions_without_hanging() {
    let mut doc = LoDocument::with_version("1.7");
    let id = doc.new_object_id();
    doc.objects.insert(id,dictionary! {"FunctionType"=>3,"Domain"=>vec![0.into(),1.into()],"Functions"=>vec![id.into()],"Bounds"=>Vec::<LoObject>::new(),"Encode"=>vec![0.into(),1.into()]}.into());
    let resources = dictionary! {"Shading"=>dictionary! {"G"=>axial(id.into(),[0.0,0.0,100.0,0.0],[true,true])}};
    let bytes = pdf(doc, resources, "/G sh");
    let error = pdf_bytes_to_png_pages(&bytes, 72, None, false)
        .unwrap_err()
        .to_string();
    assert!(error.contains("function nesting"), "{error}");
}

#[test]
fn finalized_shading_preserves_the_current_path_and_fill_color() {
    let image = simple(
        axial(red_blue(), [0.0, 0.0, 100.0, 0.0], [true, true]),
        "0 1 0 rg 10 10 20 20 re /G sh f",
    );
    color(&image, 20, 80, [0, 255, 0], 0);
    color(&image, 50, 50, [126, 0, 129], 2);
}

#[test]
fn finalized_hard_radial_gradient_retains_its_rings() {
    let engine = crate::FullBleed::builder().build().unwrap();
    let pdf=engine.render_to_buffer("<div></div>","@page {size:100pt 100pt;margin:0} div {width:100pt;height:100pt;background:radial-gradient(circle 40pt at 50pt 50pt,red 0%,red 50%,blue 50%,blue 100%)}").unwrap();
    let image = image(&pdf);
    color(&image, 50, 50, [255, 0, 0], 0);
    color(&image, 65, 50, [255, 0, 0], 0);
    color(&image, 80, 50, [0, 0, 255], 0);
}

fn masked(subtype: &str, mask_content: &str, content: &str) -> crate::image_native::RgbaImage {
    masked_with_form(subtype, mask_content, "/G sh", content)
}

fn masked_with_form(
    subtype: &str,
    mask_content: &str,
    paint_content: &str,
    content: &str,
) -> crate::image_native::RgbaImage {
    let mut doc = LoDocument::with_version("1.7");
    let gray = axial(
        exponential(vec![0.0, 0.0, 0.0], vec![1.0, 1.0, 1.0], 1.0),
        [0.0, 0.0, 100.0, 0.0],
        [true, true],
    );
    let form=doc.add_object(LoStream::new(dictionary! {
        "Type"=>"XObject","Subtype"=>"Form","BBox"=>vec![0.into(),0.into(),100.into(),100.into()],
        "Group"=>dictionary! {"S"=>"Transparency","CS"=>"DeviceRGB","I"=>true},
        "Resources"=>dictionary! {"Shading"=>dictionary! {"M"=>gray},"ExtGState"=>dictionary! {"Half"=>dictionary! {"ca"=>0.5}}},
    },mask_content.as_bytes().to_vec()));
    let blue = axial(
        exponential(vec![0.0, 0.0, 1.0], vec![0.0, 0.0, 1.0], 1.0),
        [0.0, 0.0, 100.0, 0.0],
        [true, true],
    );
    let paint = doc.add_object(LoStream::new(
        dictionary! {
            "Type"=>"XObject","Subtype"=>"Form","BBox"=>vec![0.into(),0.into(),100.into(),100.into()],
            "Group"=>dictionary! {"S"=>"Transparency","CS"=>"DeviceRGB","I"=>true},
            "Resources"=>dictionary! {"Shading"=>dictionary! {"G"=>blue.clone()}},
        },
        paint_content.as_bytes().to_vec(),
    ));
    let resources = dictionary! {
        "Shading"=>dictionary! {"G"=>blue},
        "XObject"=>dictionary! {"Paint"=>paint},
        "ExtGState"=>dictionary! {
            "Half"=>dictionary! {"ca"=>0.5},
            "Mask"=>dictionary! {"SMask"=>dictionary! {"S"=>LoObject::Name(subtype.as_bytes().to_vec()),"G"=>form}},
            "Clear"=>dictionary! {"SMask"=>"None"},
        },
    };
    image(&pdf(doc, resources, content))
}

#[test]
fn finalized_masked_group_preserves_empty_regions_around_path_paint() {
    let image = masked_with_form(
        "Luminosity",
        "1 1 1 rg 0 0 30 100 re f 70 0 30 100 re f",
        "0 0 1 rg 0 0 100 100 re f",
        "/Mask gs /Paint Do",
    );
    color(&image, 10, 50, [0, 0, 255], 0);
    color(&image, 50, 50, [255, 255, 255], 0);
    color(&image, 90, 50, [0, 0, 255], 0);
}

#[test]
fn finalized_masked_group_does_not_apply_the_mask_twice_to_a_shading() {
    let image = masked_with_form(
        "Alpha",
        "/Half gs 0 0 0 rg 0 0 100 100 re f",
        "/G sh",
        "/Mask gs /Paint Do",
    );
    color(&image, 50, 50, [127, 127, 255], 1);
}

#[test]
fn finalized_masked_group_applies_parent_alpha_once_after_overlapping_members() {
    let image = masked_with_form(
        "Luminosity",
        "1 1 1 rg 0 0 100 100 re f",
        "0 0 1 rg 0 0 70 100 re f 30 0 70 100 re f",
        "/Half gs /Mask gs /Paint Do",
    );
    for x in [10, 50, 90] {
        color(&image, x, 50, [127, 127, 255], 1);
    }
}

#[test]
fn finalized_masked_group_preserves_the_mask_ctm_and_caller_clip() {
    let image = masked_with_form(
        "Luminosity",
        "/M sh",
        "0 0 1 rg 0 0 100 100 re f",
        "0 0 70 100 re W n /Mask gs 1 0 0 1 20 0 cm /Paint Do",
    );
    color(&image, 10, 50, [255, 255, 255], 0);
    color(&image, 25, 50, [190, 190, 255], 2);
    color(&image, 60, 50, [100, 100, 255], 2);
    color(&image, 80, 50, [255, 255, 255], 0);
}

#[test]
fn finalized_soft_mask_uses_pdf_luminosity_and_preserves_partial_extgstate() {
    let image = masked(
        "Luminosity",
        "1 0 0 rg 0 0 100 100 re f",
        "/Half gs /Mask gs /G sh",
    );
    // Red luminosity .3, multiplied once by the .5 constant opacity.
    color(&image, 50, 50, [217, 217, 255], 1);
}

#[test]
fn finalized_alpha_mask_uses_alpha_instead_of_color() {
    let image = masked(
        "Alpha",
        "/Half gs 0 0 0 rg 0 0 100 100 re f",
        "/Mask gs /G sh",
    );
    color(&image, 50, 50, [127, 127, 255], 1);
}

#[test]
fn finalized_soft_mask_keeps_the_ctm_from_its_gs_operator() {
    let image = masked("Luminosity", "/M sh", "/Mask gs 1 0 0 1 20 0 cm /G sh");
    color(&image, 25, 50, [190, 190, 255], 2);
    color(&image, 75, 50, [62, 62, 255], 2);
}

#[test]
fn finalized_soft_mask_can_be_cleared_and_restored_with_q_q() {
    let image = masked(
        "Luminosity",
        "/M sh",
        "/Mask gs q /Clear gs 0 0 30 100 re W n /G sh Q q 30 0 30 100 re W n /G sh Q /Clear gs 60 0 40 100 re W n /G sh",
    );
    color(&image, 10, 50, [0, 0, 255], 0);
    color(&image, 45, 50, [139, 139, 255], 2);
    color(&image, 80, 50, [0, 0, 255], 0);
}

#[test]
fn finalized_varying_alpha_gradient_keeps_premultiplied_interpolation() {
    let engine = crate::FullBleed::builder().build().unwrap();
    let pdf=engine.render_to_buffer("<div></div>","@page {size:100pt 100pt;margin:0} body {background:#00ff00} div {width:100pt;height:100pt;background:linear-gradient(90deg,rgba(255,0,0,0),rgba(0,0,255,1))}").unwrap();
    let image = image(&pdf);
    color(&image, 25, 50, [0, 190, 65], 2);
    color(&image, 75, 50, [0, 62, 193], 2);
}

#[test]
fn finalized_gradient_reports_unsupported_function_types() {
    let function = dictionary! {"FunctionType"=>4,"Domain"=>vec![0.into(),1.into()]};
    let bytes = pdf(
        LoDocument::with_version("1.7"),
        dictionary! {"Shading"=>dictionary! {"G"=>axial(function.into(),[0.0,0.0,100.0,0.0],[true,true])}},
        "/G sh",
    );
    let error = pdf_bytes_to_png_pages(&bytes, 72, None, false)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("unsupported gradient FunctionType 4"),
        "{error}"
    );
}

#[test]
fn finalized_radial_cone_uses_the_last_circle_and_leaves_outside_unpainted() {
    let shading = dictionary! {"ShadingType"=>3,"ColorSpace"=>"DeviceRGB","Coords"=>vec![20.into(),50.into(),0.into(),80.into(),50.into(),15.into()],"Function"=>red_blue(),"Extend"=>vec![true.into(),true.into()]};
    let image = simple(shading, "/G sh");
    color(&image, 35, 50, [167, 0, 88], 2);
    color(&image, 50, 50, [82, 0, 173], 2);
    color(&image, 75, 50, [0, 0, 255], 0);
    color(&image, 50, 20, [255, 255, 255], 0);
}
