use crate::Document;
use crate::debug::DebugLogger;
use crate::error::FullBleedError;
use crate::finalize::{
    PageBindingDecision, TemplateBindingSpec, resolve_template_bindings_for_document,
};
use crate::font::FontRegistry;
use crate::jit::{self, DocPlan, JitMode};
use crate::page_data::{PageDataContext, PaginatedContextSpec, compute_page_data_context};
use std::sync::Arc;

pub struct PlannedDoc {
    pub page_data: Option<PageDataContext>,
    pub template_bindings: Option<Vec<PageBindingDecision>>,
    pub overlay: Option<Document>,
    pub background: Option<Document>,
    pub plan: Option<DocPlan>,
}

pub fn plan_document_with_overlay<F>(
    doc_id: usize,
    base: &Document,
    paginated_context: Option<&PaginatedContextSpec>,
    template_binding_spec: Option<&TemplateBindingSpec>,
    page_data_override: Option<PageDataContext>,
    debug: Option<Arc<DebugLogger>>,
    jit_mode: JitMode,
    font_registry: Option<&FontRegistry>,
    overlay_builder: F,
) -> Result<PlannedDoc, FullBleedError>
where
    F: FnOnce(Option<&PageDataContext>) -> (Option<Document>, Option<Document>),
{
    let page_data = match page_data_override {
        Some(ctx) => Some(ctx),
        None => paginated_context.map(|spec| compute_page_data_context(base, spec)),
    };
    let template_bindings = match template_binding_spec {
        Some(spec) => Some(resolve_template_bindings_for_document(base, spec)?),
        None => None,
    };
    let (overlay, background) = overlay_builder(page_data.as_ref());

    let plan = match jit_mode {
        JitMode::Off => None,
        JitMode::PlanOnly | JitMode::PlanAndReplay => Some(jit::plan_document_with_overlay(
            doc_id,
            base,
            background.as_ref(),
            overlay.as_ref(),
            page_data.clone(),
            debug,
            font_registry,
        )),
    };

    Ok(PlannedDoc {
        page_data,
        template_bindings,
        overlay,
        background,
        plan,
    })
}
