use super::*;
use crate::python_abi::{self, FromPyObject, IntoPyValue, ffi};
use std::ffi::{CStr, c_void};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr;
use std::sync::{Arc, Mutex, MutexGuard, TryLockError};

const ENGINE_CAPSULE: &[u8] = b"fullbleed.PdfEngine\0";
const COMPILED_CAPSULE: &[u8] = b"fullbleed.CompiledDocument\0";
const ASSET_CAPSULE: &[u8] = b"fullbleed.Asset\0";
const BUNDLE_CAPSULE: &[u8] = b"fullbleed.AssetBundle\0";

fn expect_arity(payload: &Bound<'_, PyAny>, expected: isize) -> PyResult<()> {
    python_abi::sequence_length(payload, expected)
}

fn argument<'py, T: FromPyObject>(payload: &Bound<'py, PyAny>, index: isize) -> PyResult<T> {
    python_abi::sequence_item(payload, index)?.extract()
}

fn argument_object<'py>(payload: &Bound<'py, PyAny>, index: isize) -> PyResult<Bound<'py, PyAny>> {
    python_abi::sequence_item(payload, index)
}

fn optional_argument<T: FromPyObject>(
    payload: &Bound<'_, PyAny>,
    index: isize,
) -> PyResult<Option<T>> {
    let value = argument_object(payload, index)?;
    if value.is_none() {
        Ok(None)
    } else {
        value.extract().map(Some)
    }
}

fn optional_object(payload: &Bound<'_, PyAny>, index: isize) -> PyResult<Option<PyObject>> {
    let value = argument_object(payload, index)?;
    if value.is_none() {
        Ok(None)
    } else {
        Ok(Some(value.unbind()))
    }
}

fn optional_bound<'py>(
    payload: &Bound<'py, PyAny>,
    index: isize,
) -> PyResult<Option<Bound<'py, PyAny>>> {
    let value = argument_object(payload, index)?;
    if value.is_none() {
        Ok(None)
    } else {
        Ok(Some(value))
    }
}

fn dict_required<T: FromPyObject>(dict: &Bound<'_, PyDict>, key: &str) -> PyResult<T> {
    dict.get_item(key)?
        .ok_or_else(|| PyValueError::new_err(format!("missing required field {key:?}")))?
        .extract()
}

fn dict_optional<T: FromPyObject>(dict: &Bound<'_, PyDict>, key: &str) -> PyResult<Option<T>> {
    let Some(value) = dict.get_item(key)? else {
        return Ok(None);
    };
    if value.is_none() {
        Ok(None)
    } else {
        value.extract().map(Some)
    }
}

fn watermark_from_payload(value: Option<Bound<'_, PyAny>>) -> PyResult<Option<PyWatermarkSpec>> {
    let Some(value) = value else {
        return Ok(None);
    };
    let dict = value
        .downcast::<PyDict>()
        .map_err(|_| PyValueError::new_err("watermark must be a WatermarkSpec instance or None"))?;
    Ok(Some(PyWatermarkSpec::new(
        dict_required(dict, "kind")?,
        dict_required(dict, "value")?,
        &dict_required::<String>(dict, "layer")?,
        dict_optional(dict, "semantics")?,
        dict_required(dict, "opacity")?,
        dict_required(dict, "rotation_deg")?,
        dict_optional(dict, "font_name")?,
        dict_optional(dict, "font_size")?,
        dict_optional(dict, "color")?,
    )))
}

fn capsule<T>(
    value: T,
    name: &'static [u8],
    destructor: unsafe extern "C" fn(*mut ffi::PyObject),
) -> PyResult<PyObject> {
    let boxed = Box::into_raw(Box::new(Mutex::new(value)));
    let object = unsafe {
        ffi::PyCapsule_New(
            boxed.cast::<c_void>(),
            name.as_ptr().cast(),
            Some(destructor),
        )
    };
    if object.is_null() {
        unsafe { drop(Box::from_raw(boxed)) };
        return Err(PyErr::fetch());
    }
    unsafe { Bound::<PyAny>::from_owned_ptr(object) }.map(Bound::unbind)
}

unsafe fn capsule_lock<'a, T>(
    object: &'a Bound<'_, PyAny>,
    name: &'static [u8],
) -> PyResult<MutexGuard<'a, T>> {
    let pointer = unsafe { ffi::PyCapsule_GetPointer(object.as_ptr(), name.as_ptr().cast()) };
    if pointer.is_null() {
        return Err(PyErr::fetch());
    }
    match unsafe { &*pointer.cast::<Mutex<T>>() }.try_lock() {
        Ok(guard) => Ok(guard),
        Err(TryLockError::WouldBlock) => Err(PyErr::runtime_error(
            "Fullbleed native object is already in use by another call",
        )),
        Err(TryLockError::Poisoned(_)) => Err(PyErr::runtime_error(
            "Fullbleed native object is unavailable after a previous panic",
        )),
    }
}

unsafe extern "C" fn drop_engine(capsule: *mut ffi::PyObject) {
    let pointer = unsafe { ffi::PyCapsule_GetPointer(capsule, ENGINE_CAPSULE.as_ptr().cast()) };
    if pointer.is_null() {
        drop(PyErr::fetch());
    } else {
        unsafe { drop(Box::from_raw(pointer.cast::<Mutex<PdfEngine>>())) };
    }
}

unsafe extern "C" fn drop_compiled(capsule: *mut ffi::PyObject) {
    let pointer = unsafe { ffi::PyCapsule_GetPointer(capsule, COMPILED_CAPSULE.as_ptr().cast()) };
    if pointer.is_null() {
        drop(PyErr::fetch());
    } else {
        unsafe {
            drop(Box::from_raw(
                pointer.cast::<Mutex<Arc<crate::CompiledDocument>>>(),
            ))
        };
    }
}

unsafe extern "C" fn drop_asset(capsule: *mut ffi::PyObject) {
    let pointer = unsafe { ffi::PyCapsule_GetPointer(capsule, ASSET_CAPSULE.as_ptr().cast()) };
    if pointer.is_null() {
        drop(PyErr::fetch());
    } else {
        unsafe { drop(Box::from_raw(pointer.cast::<Mutex<PyAsset>>())) };
    }
}

unsafe extern "C" fn drop_bundle(capsule: *mut ffi::PyObject) {
    let pointer = unsafe { ffi::PyCapsule_GetPointer(capsule, BUNDLE_CAPSULE.as_ptr().cast()) };
    if pointer.is_null() {
        drop(PyErr::fetch());
    } else {
        unsafe { drop(Box::from_raw(pointer.cast::<Mutex<PyAssetBundle>>())) };
    }
}

fn dispatch_free_function(
    py: Python<'_>,
    operation: &str,
    payload: &Bound<'_, PyAny>,
) -> PyResult<PyObject> {
    match operation {
        "build_features" => {
            expect_arity(payload, 0)?;
            build_features(py)
        }
        "inspect_pdf" => {
            expect_arity(payload, 1)?;
            let path: String = argument(payload, 0)?;
            inspect_pdf(py, &path)
        }
        "extract_pdf_page_texts" => {
            expect_arity(payload, 1)?;
            let path: String = argument(payload, 0)?;
            extract_pdf_page_texts(py, &path)
        }
        "export_pdf_reading_order_trace" => {
            expect_arity(payload, 1)?;
            let path: String = argument(payload, 0)?;
            export_pdf_reading_order_trace(py, &path)
        }
        "export_pdf_structure_trace" => {
            expect_arity(payload, 1)?;
            let path: String = argument(payload, 0)?;
            export_pdf_structure_trace(py, &path)
        }
        "verify_pdf_ua_seed" => {
            expect_arity(payload, 2)?;
            let path: String = argument(payload, 0)?;
            let mode: String = argument(payload, 1)?;
            verify_pdf_ua_seed(py, &path, &mode)
        }
        "inspect_template_catalog" => {
            expect_arity(payload, 1)?;
            inspect_template_catalog(py, argument(payload, 0)?)
        }
        "vendored_asset" => {
            expect_arity(payload, 5)?;
            let source: String = argument(payload, 0)?;
            let kind: String = argument(payload, 1)?;
            let asset = vendored_asset(
                py,
                &source,
                &kind,
                optional_argument(payload, 2)?,
                argument(payload, 3)?,
                argument(payload, 4)?,
            )?;
            capsule(asset, ASSET_CAPSULE, drop_asset)
        }
        "fetch_asset" => {
            expect_arity(payload, 1)?;
            let url: String = argument(payload, 0)?;
            fetch_asset(py, &url).map(IntoPyValue::into_py_value)?
        }
        "concat_css" => {
            expect_arity(payload, 1)?;
            concat_css(argument(payload, 0)?).and_then(IntoPyValue::into_py_value)
        }
        "finalize_stamp_pdf" => {
            expect_arity(payload, 6)?;
            let template: String = argument(payload, 0)?;
            let overlay: String = argument(payload, 1)?;
            let out: String = argument(payload, 2)?;
            finalize_stamp_pdf(
                &template,
                &overlay,
                &out,
                optional_argument(payload, 3)?,
                argument(payload, 4)?,
                argument(payload, 5)?,
            )
        }
        "finalize_compose_pdf" => {
            expect_arity(payload, 5)?;
            let overlay: String = argument(payload, 2)?;
            let out: String = argument(payload, 3)?;
            let annotation_mode: Option<String> = optional_argument(payload, 4)?;
            finalize_compose_pdf(
                argument(payload, 0)?,
                argument(payload, 1)?,
                &overlay,
                &out,
                annotation_mode.as_deref(),
            )
        }
        "audit_contract_metadata" => {
            expect_arity(payload, 0)?;
            audit_contract_metadata(py)
        }
        "audit_contract_registry" => {
            expect_arity(payload, 1)?;
            let name: String = argument(payload, 0)?;
            audit_contract_registry(&name).and_then(IntoPyValue::into_py_value)
        }
        "audit_contract_wcag20aa_coverage" => {
            expect_arity(payload, 1)?;
            let findings = argument_object(payload, 0)?;
            audit_contract_wcag20aa_coverage(py, &findings)
        }
        "audit_contract_section508_html_coverage" => {
            expect_arity(payload, 1)?;
            let findings = argument_object(payload, 0)?;
            audit_contract_section508_html_coverage(py, &findings)
        }
        "audit_contrast_render_png" => {
            expect_arity(payload, 1)?;
            let path: String = argument(payload, 0)?;
            audit_contrast_render_png(py, &path)
        }
        "audit_sparse_page_visual_pair" => {
            expect_arity(payload, 2)?;
            let source: String = argument(payload, 0)?;
            let render: String = argument(payload, 1)?;
            audit_sparse_page_visual_pair(py, &source, &render)
        }
        _ => Err(PyValueError::new_err(format!(
            "unknown FullBleed native operation: {operation}"
        ))),
    }
}

fn dispatch_asset(
    py: Python<'_>,
    handle: &Bound<'_, PyAny>,
    operation: &str,
    payload: &Bound<'_, PyAny>,
) -> PyResult<PyObject> {
    let asset = unsafe { capsule_lock::<PyAsset>(handle, ASSET_CAPSULE)? };
    match operation {
        "Asset.info" => {
            expect_arity(payload, 0)?;
            asset.info(py)
        }
        _ => Err(PyValueError::new_err(format!(
            "unknown Asset operation: {operation}"
        ))),
    }
}

fn dispatch_bundle(
    py: Python<'_>,
    handle: &Bound<'_, PyAny>,
    operation: &str,
    payload: &Bound<'_, PyAny>,
) -> PyResult<PyObject> {
    let mut bundle = unsafe { capsule_lock::<PyAssetBundle>(handle, BUNDLE_CAPSULE)? };
    match operation {
        "AssetBundle.add" => {
            expect_arity(payload, 1)?;
            let asset_handle = argument_object(payload, 0)?;
            let asset = unsafe { capsule_lock::<PyAsset>(&asset_handle, ASSET_CAPSULE)? };
            bundle.add(&asset);
            ().into_py_value()
        }
        "AssetBundle.add_file" => {
            expect_arity(payload, 5)?;
            let path: String = argument(payload, 0)?;
            let kind: String = argument(payload, 1)?;
            let asset = bundle.add_file(
                py,
                &path,
                &kind,
                optional_argument(payload, 2)?,
                argument(payload, 3)?,
                argument(payload, 4)?,
            )?;
            capsule(asset, ASSET_CAPSULE, drop_asset)
        }
        "AssetBundle.css" => {
            expect_arity(payload, 0)?;
            bundle.css().into_py_value()
        }
        "AssetBundle.assets_info" => {
            expect_arity(payload, 0)?;
            bundle.assets_info(py)
        }
        _ => Err(PyValueError::new_err(format!(
            "unknown AssetBundle operation: {operation}"
        ))),
    }
}

fn new_engine(payload: &Bound<'_, PyAny>) -> PyResult<PyObject> {
    expect_arity(payload, 66)?;
    let page_width = optional_bound(payload, 0)?;
    let page_height = optional_bound(payload, 1)?;
    let margin = optional_bound(payload, 2)?;
    let page_margins = optional_bound(payload, 3)?;
    let pdf_version = optional_bound(payload, 12)?;
    let pdf_profile = optional_bound(payload, 13)?;
    let header_x = optional_bound(payload, 24)?;
    let header_y = optional_bound(payload, 25)?;
    let header_html_x = optional_bound(payload, 32)?;
    let header_html_y = optional_bound(payload, 33)?;
    let header_html_width = optional_bound(payload, 34)?;
    let header_html_height = optional_bound(payload, 35)?;
    let footer_x = optional_bound(payload, 39)?;
    let footer_y = optional_bound(payload, 40)?;
    let watermark = watermark_from_payload(optional_bound(payload, 44)?)?;
    let template_binding = optional_bound(payload, 56)?;
    let watermark_layer: String = argument(payload, 48)?;
    let watermark_semantics: String = argument(payload, 49)?;

    let engine = PdfEngine::new(
        page_width.as_ref(),
        page_height.as_ref(),
        margin.as_ref(),
        page_margins.as_ref(),
        optional_argument(payload, 4)?,
        optional_argument(payload, 5)?,
        argument(payload, 6)?,
        argument(payload, 7)?,
        argument(payload, 8)?,
        argument(payload, 9)?,
        argument(payload, 10)?,
        argument(payload, 11)?,
        pdf_version.as_ref(),
        pdf_profile.as_ref(),
        optional_argument(payload, 14)?,
        optional_argument(payload, 15)?,
        optional_argument(payload, 16)?,
        optional_argument(payload, 17)?,
        optional_argument(payload, 18)?,
        optional_argument(payload, 19)?,
        optional_argument(payload, 20)?,
        optional_argument(payload, 21)?,
        optional_argument(payload, 22)?,
        optional_argument(payload, 23)?,
        header_x.as_ref(),
        header_y.as_ref(),
        optional_argument(payload, 26)?,
        optional_argument(payload, 27)?,
        optional_argument(payload, 28)?,
        optional_argument(payload, 29)?,
        optional_argument(payload, 30)?,
        optional_argument(payload, 31)?,
        header_html_x.as_ref(),
        header_html_y.as_ref(),
        header_html_width.as_ref(),
        header_html_height.as_ref(),
        optional_argument(payload, 36)?,
        optional_argument(payload, 37)?,
        optional_argument(payload, 38)?,
        footer_x.as_ref(),
        footer_y.as_ref(),
        optional_argument(payload, 41)?,
        optional_argument(payload, 42)?,
        optional_argument(payload, 43)?,
        watermark,
        optional_argument(payload, 45)?,
        optional_argument(payload, 46)?,
        optional_argument(payload, 47)?,
        &watermark_layer,
        &watermark_semantics,
        argument(payload, 50)?,
        argument(payload, 51)?,
        optional_argument(payload, 52)?,
        optional_argument(payload, 53)?,
        optional_argument(payload, 54)?,
        optional_argument(payload, 55)?,
        template_binding.as_ref(),
        optional_argument(payload, 57)?,
        argument(payload, 58)?,
        argument(payload, 59)?,
        argument(payload, 60)?,
        optional_argument(payload, 61)?,
        argument(payload, 62)?,
        optional_argument(payload, 63)?,
        argument(payload, 64)?,
        optional_argument(payload, 65)?,
    )?;
    capsule(engine, ENGINE_CAPSULE, drop_engine)
}

fn dispatch_engine(
    py: Python<'_>,
    handle: &Bound<'_, PyAny>,
    operation: &str,
    payload: &Bound<'_, PyAny>,
) -> PyResult<PyObject> {
    let mut engine = unsafe { capsule_lock::<PdfEngine>(handle, ENGINE_CAPSULE)? };
    match operation {
        "PdfEngine.register_bundle" => {
            expect_arity(payload, 1)?;
            let bundle_handle = argument_object(payload, 0)?;
            let bundle = unsafe { capsule_lock::<PyAssetBundle>(&bundle_handle, BUNDLE_CAPSULE)? };
            engine.register_bundle(&bundle)?.into_py_value()
        }
        "PdfEngine.get_document_lang" => engine.document_lang().into_py_value(),
        "PdfEngine.set_document_lang" => {
            expect_arity(payload, 1)?;
            engine
                .set_document_lang(optional_argument(payload, 0)?)?
                .into_py_value()
        }
        "PdfEngine.get_document_title" => engine.document_title().into_py_value(),
        "PdfEngine.set_document_title" => {
            expect_arity(payload, 1)?;
            engine
                .set_document_title(optional_argument(payload, 0)?)?
                .into_py_value()
        }
        "PdfEngine.get_document_css_href" => engine.document_css_href().into_py_value(),
        "PdfEngine.set_document_css_href" => {
            expect_arity(payload, 1)?;
            engine
                .set_document_css_href(optional_argument(payload, 0)?)?
                .into_py_value()
        }
        "PdfEngine.get_document_css_source_path" => {
            engine.document_css_source_path().into_py_value()
        }
        "PdfEngine.set_document_css_source_path" => {
            expect_arity(payload, 1)?;
            engine
                .set_document_css_source_path(optional_argument(payload, 0)?)?
                .into_py_value()
        }
        "PdfEngine.get_document_css_media" => engine.document_css_media().into_py_value(),
        "PdfEngine.set_document_css_media" => {
            expect_arity(payload, 1)?;
            engine
                .set_document_css_media(optional_argument(payload, 0)?)?
                .into_py_value()
        }
        "PdfEngine.get_document_css_required" => engine.document_css_required().into_py_value(),
        "PdfEngine.set_document_css_required" => {
            expect_arity(payload, 1)?;
            engine
                .set_document_css_required(argument(payload, 0)?)?
                .into_py_value()
        }
        "PdfEngine.document_metadata" => {
            expect_arity(payload, 0)?;
            engine.document_metadata(py)
        }
        "PdfEngine.emit_html" => {
            expect_arity(payload, 3)?;
            let html: String = argument(payload, 0)?;
            let path: String = argument(payload, 1)?;
            engine
                .emit_html(&html, &path, argument(payload, 2)?)
                .and_then(IntoPyValue::into_py_value)
        }
        "PdfEngine.emit_css" => {
            expect_arity(payload, 2)?;
            let css: String = argument(payload, 0)?;
            let path: String = argument(payload, 1)?;
            engine
                .emit_css(&css, &path)
                .and_then(IntoPyValue::into_py_value)
        }
        "PdfEngine.emit_artifacts" => {
            expect_arity(payload, 5)?;
            let html: String = argument(payload, 0)?;
            let css: String = argument(payload, 1)?;
            let html_path: String = argument(payload, 2)?;
            let css_path: String = argument(payload, 3)?;
            engine.emit_artifacts(
                py,
                &html,
                &css,
                &html_path,
                &css_path,
                argument(payload, 4)?,
            )
        }
        "PdfEngine.verify_accessibility_html" => {
            expect_arity(payload, 9)?;
            let html: String = argument(payload, 0)?;
            let css: String = argument(payload, 1)?;
            let profile: String = argument(payload, 2)?;
            let mode: String = argument(payload, 3)?;
            let preview: Option<String> = optional_argument(payload, 4)?;
            engine.verify_accessibility_html(
                py,
                &html,
                &css,
                &profile,
                &mode,
                preview.as_deref(),
                optional_object(payload, 5)?,
                optional_object(payload, 6)?,
                optional_object(payload, 7)?,
                optional_object(payload, 8)?,
            )
        }
        "PdfEngine.verify_accessibility_artifacts" => {
            expect_arity(payload, 9)?;
            let html: String = argument(payload, 0)?;
            let css: String = argument(payload, 1)?;
            let profile: String = argument(payload, 2)?;
            let mode: String = argument(payload, 3)?;
            let preview: Option<String> = optional_argument(payload, 4)?;
            engine.verify_accessibility_artifacts(
                py,
                &html,
                &css,
                &profile,
                &mode,
                preview.as_deref(),
                optional_object(payload, 5)?,
                optional_object(payload, 6)?,
                optional_object(payload, 7)?,
                optional_object(payload, 8)?,
            )
        }
        "PdfEngine.verify_paged_media_rank_html" => {
            expect_arity(payload, 11)?;
            let html: String = argument(payload, 0)?;
            let css: String = argument(payload, 1)?;
            let profile: String = argument(payload, 2)?;
            let mode: String = argument(payload, 3)?;
            engine.verify_paged_media_rank_html(
                py,
                &html,
                &css,
                &profile,
                &mode,
                optional_argument(payload, 4)?,
                optional_argument(payload, 5)?,
                optional_argument(payload, 6)?,
                optional_argument(payload, 7)?,
                optional_argument(payload, 8)?,
                optional_object(payload, 9)?,
                optional_object(payload, 10)?,
            )
        }
        "PdfEngine.verify_paged_media_rank_artifacts" => {
            expect_arity(payload, 11)?;
            let html: String = argument(payload, 0)?;
            let css: String = argument(payload, 1)?;
            let profile: String = argument(payload, 2)?;
            let mode: String = argument(payload, 3)?;
            engine.verify_paged_media_rank_artifacts(
                py,
                &html,
                &css,
                &profile,
                &mode,
                optional_argument(payload, 4)?,
                optional_argument(payload, 5)?,
                optional_argument(payload, 6)?,
                optional_argument(payload, 7)?,
                optional_argument(payload, 8)?,
                optional_object(payload, 9)?,
                optional_object(payload, 10)?,
            )
        }
        "PdfEngine.compile_pdf" => {
            expect_arity(payload, 2)?;
            let html: String = argument(payload, 0)?;
            let css: String = argument(payload, 1)?;
            let compiled = py
                .allow_threads(|| engine.engine.compile_document(&html, &css))
                .map_err(to_py_err)?;
            capsule(Arc::new(compiled), COMPILED_CAPSULE, drop_compiled)
        }
        "PdfEngine.render_pdf" => {
            expect_arity(payload, 3)?;
            let html: String = argument(payload, 0)?;
            let css: String = argument(payload, 1)?;
            engine
                .render_pdf(py, &html, &css, optional_argument(payload, 2)?)
                .and_then(IntoPyValue::into_py_value)
        }
        "PdfEngine.render_image_pages" => {
            expect_arity(payload, 3)?;
            let html: String = argument(payload, 0)?;
            let css: String = argument(payload, 1)?;
            engine.render_image_pages(py, &html, &css, argument(payload, 2)?)
        }
        "PdfEngine.render_image_pages_to_dir" => {
            expect_arity(payload, 5)?;
            let html: String = argument(payload, 0)?;
            let css: String = argument(payload, 1)?;
            let out: String = argument(payload, 2)?;
            engine.render_image_pages_to_dir(
                py,
                &html,
                &css,
                &out,
                argument(payload, 3)?,
                optional_argument(payload, 4)?,
            )
        }
        "PdfEngine.render_finalized_pdf_image_pages" => {
            expect_arity(payload, 2)?;
            let path: String = argument(payload, 0)?;
            engine.render_finalized_pdf_image_pages(py, &path, argument(payload, 1)?)
        }
        "PdfEngine.render_finalized_pdf_image_pages_to_dir" => {
            expect_arity(payload, 4)?;
            let path: String = argument(payload, 0)?;
            let out: String = argument(payload, 1)?;
            engine.render_finalized_pdf_image_pages_to_dir(
                py,
                &path,
                &out,
                argument(payload, 2)?,
                optional_argument(payload, 3)?,
            )
        }
        "PdfEngine.render_pdf_with_page_data" => {
            expect_arity(payload, 2)?;
            let html: String = argument(payload, 0)?;
            let css: String = argument(payload, 1)?;
            engine
                .render_pdf_with_page_data(py, &html, &css)
                .and_then(IntoPyValue::into_py_value)
        }
        "PdfEngine.render_pdf_with_page_data_and_glyph_report" => {
            expect_arity(payload, 2)?;
            let html: String = argument(payload, 0)?;
            let css: String = argument(payload, 1)?;
            engine
                .render_pdf_with_page_data_and_glyph_report(py, &html, &css)
                .and_then(IntoPyValue::into_py_value)
        }
        "PdfEngine.export_render_time_reading_order_trace"
        | "PdfEngine.export_render_time_structure_trace"
        | "PdfEngine.export_render_time_font_resolution_trace"
        | "PdfEngine.export_render_time_asset_resolution_trace"
        | "PdfEngine.export_render_time_pagination_trace"
        | "PdfEngine.export_render_time_typography_drift_trace"
        | "PdfEngine.export_render_time_region_text_alignment_trace" => {
            expect_arity(payload, 2)?;
            let html: String = argument(payload, 0)?;
            let css: String = argument(payload, 1)?;
            match operation {
                "PdfEngine.export_render_time_reading_order_trace" => {
                    engine.export_render_time_reading_order_trace(py, &html, &css)
                }
                "PdfEngine.export_render_time_structure_trace" => {
                    engine.export_render_time_structure_trace(py, &html, &css)
                }
                "PdfEngine.export_render_time_font_resolution_trace" => {
                    engine.export_render_time_font_resolution_trace(py, &html, &css)
                }
                "PdfEngine.export_render_time_asset_resolution_trace" => {
                    engine.export_render_time_asset_resolution_trace(py, &html, &css)
                }
                "PdfEngine.export_render_time_pagination_trace" => {
                    engine.export_render_time_pagination_trace(py, &html, &css)
                }
                "PdfEngine.export_render_time_typography_drift_trace" => {
                    engine.export_render_time_typography_drift_trace(py, &html, &css)
                }
                _ => engine.export_render_time_region_text_alignment_trace(py, &html, &css),
            }
        }
        "PdfEngine.render_pdf_with_page_data_and_template_bindings" => {
            expect_arity(payload, 2)?;
            let html: String = argument(payload, 0)?;
            let css: String = argument(payload, 1)?;
            engine
                .render_pdf_with_page_data_and_template_bindings(py, &html, &css)
                .and_then(IntoPyValue::into_py_value)
        }
        "PdfEngine.render_pdf_with_page_data_and_template_bindings_and_glyph_report" => {
            expect_arity(payload, 2)?;
            let html: String = argument(payload, 0)?;
            let css: String = argument(payload, 1)?;
            engine
                .render_pdf_with_page_data_and_template_bindings_and_glyph_report(py, &html, &css)
                .and_then(IntoPyValue::into_py_value)
        }
        "PdfEngine.plan_template_compose" => {
            expect_arity(payload, 5)?;
            let html: String = argument(payload, 0)?;
            let css: String = argument(payload, 1)?;
            engine.plan_template_compose(
                py,
                &html,
                &css,
                argument(payload, 2)?,
                argument(payload, 3)?,
                argument(payload, 4)?,
            )
        }
        "PdfEngine.render_pdf_with_glyph_report" => {
            expect_arity(payload, 2)?;
            let html: String = argument(payload, 0)?;
            let css: String = argument(payload, 1)?;
            engine
                .render_pdf_with_glyph_report(py, &html, &css)
                .and_then(IntoPyValue::into_py_value)
        }
        "PdfEngine.render_pdf_with_glyph_report_and_render_time_reading_order_trace" => {
            expect_arity(payload, 2)?;
            let html: String = argument(payload, 0)?;
            let css: String = argument(payload, 1)?;
            engine
                .render_pdf_with_glyph_report_and_render_time_reading_order_trace(py, &html, &css)
                .and_then(IntoPyValue::into_py_value)
        }
        "PdfEngine.render_pdf_to_file" => {
            expect_arity(payload, 4)?;
            let html: String = argument(payload, 0)?;
            let css: String = argument(payload, 1)?;
            let path: String = argument(payload, 2)?;
            engine
                .render_pdf_to_file(&html, &css, &path, optional_argument(payload, 3)?)
                .and_then(IntoPyValue::into_py_value)
        }
        "PdfEngine.render_pdf_batch" | "PdfEngine.render_pdf_batch_parallel" => {
            expect_arity(payload, 3)?;
            let html: Vec<String> = argument(payload, 0)?;
            let css: String = argument(payload, 1)?;
            let hash = optional_argument(payload, 2)?;
            if operation.ends_with("_parallel") {
                engine
                    .render_pdf_batch_parallel(py, html, &css, hash)
                    .and_then(IntoPyValue::into_py_value)
            } else {
                engine
                    .render_pdf_batch(py, html, &css, hash)
                    .and_then(IntoPyValue::into_py_value)
            }
        }
        "PdfEngine.render_pdf_batch_to_file" | "PdfEngine.render_pdf_batch_to_file_parallel" => {
            expect_arity(payload, 4)?;
            let html: Vec<String> = argument(payload, 0)?;
            let css: String = argument(payload, 1)?;
            let path: String = argument(payload, 2)?;
            let hash = optional_argument(payload, 3)?;
            if operation.ends_with("_parallel") {
                engine
                    .render_pdf_batch_to_file_parallel(html, &css, &path, hash)
                    .and_then(IntoPyValue::into_py_value)
            } else {
                engine
                    .render_pdf_batch_to_file(html, &css, &path, hash)
                    .and_then(IntoPyValue::into_py_value)
            }
        }
        "PdfEngine.render_pdf_batch_with_css" => {
            expect_arity(payload, 2)?;
            engine
                .render_pdf_batch_with_css(
                    py,
                    argument(payload, 0)?,
                    optional_argument(payload, 1)?,
                )
                .and_then(IntoPyValue::into_py_value)
        }
        "PdfEngine.render_pdf_batch_with_css_to_file" => {
            expect_arity(payload, 3)?;
            let path: String = argument(payload, 1)?;
            engine
                .render_pdf_batch_with_css_to_file(
                    argument(payload, 0)?,
                    &path,
                    optional_argument(payload, 2)?,
                )
                .and_then(IntoPyValue::into_py_value)
        }
        "PdfEngine.render_pdf_batch_to_file_parallel_with_page_data" => {
            expect_arity(payload, 4)?;
            let html: Vec<String> = argument(payload, 0)?;
            let css: String = argument(payload, 1)?;
            let path: String = argument(payload, 2)?;
            engine
                .render_pdf_batch_to_file_parallel_with_page_data(
                    html,
                    &css,
                    &path,
                    optional_argument(payload, 3)?,
                )
                .and_then(IntoPyValue::into_py_value)
        }
        _ => Err(PyValueError::new_err(format!(
            "unknown PdfEngine operation: {operation}"
        ))),
    }
}

fn dispatch_compiled(
    py: Python<'_>,
    handle: &Bound<'_, PyAny>,
    operation: &str,
    payload: &Bound<'_, PyAny>,
) -> PyResult<PyObject> {
    // Clone the immutable Arc while holding the capsule lock, then release the lock before the
    // GIL. Separate Python threads may therefore render one compiled document concurrently.
    let compiled = {
        let guard =
            unsafe { capsule_lock::<Arc<crate::CompiledDocument>>(handle, COMPILED_CAPSULE)? };
        guard.clone()
    };
    match operation {
        "CompiledDocument.stats" => {
            expect_arity(payload, 0)?;
            let out = PyDict::new(py);
            out.set_item("page_count", compiled.page_count())?;
            out.set_item("command_count", compiled.command_count())?;
            out.set_item("compile_ms", compiled.compile_time_ms())?;
            out.set_item("binding_slot_count", compiled.binding_slots().len())?;
            out.set_item("binding_slots", PyList::new(py, compiled.binding_slots())?)?;
            Ok(out.unbind().into_any())
        }
        "CompiledDocument.render_pdf" => {
            expect_arity(payload, 1)?;
            let bytes = py
                .allow_threads(|| compiled.render_to_buffer())
                .map_err(to_py_err)?;
            if let Some(path) = optional_argument::<String>(payload, 0)? {
                write_hash_file(&path, &sha256_hex(&bytes))?;
            }
            Ok(PyBytes::new(py, &bytes).unbind().into_any())
        }
        "CompiledDocument.render_pdf_to_file" => {
            expect_arity(payload, 2)?;
            let path: String = argument(payload, 0)?;
            let written = py
                .allow_threads(|| compiled.render_to_file(&path))
                .map_err(to_py_err)?;
            if let Some(hash_path) = optional_argument::<String>(payload, 1)? {
                write_hash_file(&hash_path, &sha256_file_hex(&path)?)?;
            }
            written.into_py_value()
        }
        "CompiledDocument.render_pdf_batch" => {
            expect_arity(payload, 2)?;
            let copies: usize = argument(payload, 0)?;
            let bytes = py
                .allow_threads(|| compiled.render_many_to_buffer(copies))
                .map_err(to_py_err)?;
            if let Some(path) = optional_argument::<String>(payload, 1)? {
                write_hash_file(&path, &sha256_hex(&bytes))?;
            }
            Ok(PyBytes::new(py, &bytes).unbind().into_any())
        }
        "CompiledDocument.render_pdf_bindings" => {
            expect_arity(payload, 2)?;
            let bindings: HashMap<String, Vec<String>> = argument(payload, 0)?;
            let bytes = py
                .allow_threads(|| compiled.render_bindings_to_buffer(&bindings))
                .map_err(to_py_err)?;
            if let Some(path) = optional_argument::<String>(payload, 1)? {
                write_hash_file(&path, &sha256_hex(&bytes))?;
            }
            Ok(PyBytes::new(py, &bytes).unbind().into_any())
        }
        "CompiledDocument.render_pdf_bindings_to_file" => {
            expect_arity(payload, 3)?;
            let bindings: HashMap<String, Vec<String>> = argument(payload, 0)?;
            let path: String = argument(payload, 1)?;
            let written = py
                .allow_threads(|| compiled.render_bindings_to_file(&bindings, &path))
                .map_err(to_py_err)?;
            if let Some(hash_path) = optional_argument::<String>(payload, 2)? {
                write_hash_file(&hash_path, &sha256_file_hex(&path)?)?;
            }
            written.into_py_value()
        }
        _ => Err(PyValueError::new_err(format!(
            "unknown CompiledDocument operation: {operation}"
        ))),
    }
}

fn dispatch(
    py: Python<'_>,
    handle: &Bound<'_, PyAny>,
    operation: &str,
    payload: &Bound<'_, PyAny>,
) -> PyResult<PyObject> {
    match operation {
        "AssetBundle.new" => {
            expect_arity(payload, 0)?;
            capsule(PyAssetBundle::new(), BUNDLE_CAPSULE, drop_bundle)
        }
        "PdfEngine.new" => new_engine(payload),
        operation if operation.starts_with("AssetBundle.") => {
            dispatch_bundle(py, handle, operation, payload)
        }
        operation if operation.starts_with("Asset.") => {
            dispatch_asset(py, handle, operation, payload)
        }
        operation if operation.starts_with("PdfEngine.") => {
            dispatch_engine(py, handle, operation, payload)
        }
        operation if operation.starts_with("CompiledDocument.") => {
            dispatch_compiled(py, handle, operation, payload)
        }
        _ => dispatch_free_function(py, operation, payload),
    }
}

unsafe extern "C" fn native_dispatch(
    _self_object: *mut ffi::PyObject,
    args: *mut ffi::PyObject,
) -> *mut ffi::PyObject {
    let result = catch_unwind(AssertUnwindSafe(|| {
        Python::with_gil(|py| {
            let count = unsafe { python_abi::tuple_argument_count(args)? };
            if count != 3 {
                return Err(PyErr::type_error(format!(
                    "_dispatch expected 3 arguments, got {count}"
                )));
            }
            let handle = unsafe { python_abi::tuple_argument(args, 0)? };
            let operation = unsafe { python_abi::tuple_argument(args, 1)? }.extract::<String>()?;
            let payload = unsafe { python_abi::tuple_argument(args, 2)? };
            dispatch(py, &handle, &operation, &payload)
        })
    }));
    match result {
        Ok(result) => unsafe { python_abi::result_to_raw(result) },
        Err(_) => unsafe {
            python_abi::result_to_raw::<PyObject>(Err(PyErr::runtime_error(
                "panic in FullBleed's native Python boundary",
            )))
        },
    }
}

unsafe extern "C" fn execute_module(_module: *mut ffi::PyObject) -> i32 {
    0
}

fn python_runtime_at_least(required_major: u32, required_minor: u32) -> bool {
    let version_pointer = unsafe { ffi::Py_GetVersion() };
    if version_pointer.is_null() {
        return false;
    }
    let Ok(version) = unsafe { CStr::from_ptr(version_pointer) }.to_str() else {
        return false;
    };
    let mut parts = version.split('.');
    let Some(major) = parts.next().and_then(|part| part.parse::<u32>().ok()) else {
        return false;
    };
    let Some(minor) = parts.next().and_then(|part| part.parse::<u32>().ok()) else {
        return false;
    };
    (major, minor) >= (required_major, required_minor)
}

fn initialize_module() -> PyResult<*mut ffi::PyObject> {
    let methods = Box::leak(Box::new([
        ffi::PyMethodDef {
            ml_name: c"_dispatch".as_ptr(),
            ml_meth: Some(native_dispatch),
            ml_flags: ffi::METH_VARARGS,
            ml_doc: c"Internal stable-ABI dispatcher for the FullBleed Python facade.".as_ptr(),
        },
        ffi::PyMethodDef {
            ml_name: ptr::null(),
            ml_meth: None,
            ml_flags: 0,
            ml_doc: ptr::null(),
        },
    ]));
    let mut slots = vec![ffi::PyModuleDefSlot {
        slot: ffi::PY_MOD_EXEC,
        value: execute_module as *const () as *mut c_void,
    }];
    // The wheel targets Python 3.10's stable ABI, where slot 3 is unknown. Add
    // the multiple-interpreters declaration only when the runtime supports it.
    if python_runtime_at_least(3, 12) {
        slots.push(ffi::PyModuleDefSlot {
            slot: ffi::PY_MOD_MULTIPLE_INTERPRETERS,
            value: ffi::PY_MOD_PER_INTERPRETER_GIL_SUPPORTED as *mut c_void,
        });
    }
    slots.push(ffi::PyModuleDefSlot {
        slot: 0,
        value: ptr::null_mut(),
    });
    let slots = Box::leak(slots.into_boxed_slice());
    let definition = Box::leak(Box::new(ffi::PyModuleDef {
        m_base: ffi::PyModuleDefBase {
            ob_base: ffi::PyObjectHead {
                ob_refcnt: 1,
                ob_type: ptr::null_mut(),
            },
            m_init: ptr::null_mut(),
            m_index: 0,
            m_copy: ptr::null_mut(),
        },
        m_name: c"_fullbleed".as_ptr(),
        m_doc: c"Fullbleed's dependency-free CPython stable-ABI boundary.".as_ptr(),
        m_size: 0,
        m_methods: methods.as_mut_ptr(),
        m_slots: slots.as_mut_ptr(),
        m_traverse: ptr::null_mut(),
        m_clear: ptr::null_mut(),
        m_free: ptr::null_mut(),
    }));
    let definition_pointer = unsafe { ffi::PyModuleDef_Init(definition) };
    if definition_pointer.is_null() {
        Err(PyErr::fetch())
    } else {
        Ok(definition_pointer)
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyInit__fullbleed() -> *mut ffi::PyObject {
    let result = catch_unwind(AssertUnwindSafe(initialize_module));
    match result {
        Ok(Ok(definition)) => definition,
        Ok(Err(error)) => {
            unsafe { error.restore() };
            ptr::null_mut()
        }
        Err(_) => unsafe {
            python_abi::result_to_raw::<PyObject>(Err(PyErr::runtime_error(
                "panic while initializing FullBleed's native Python module",
            )))
        },
    }
}
