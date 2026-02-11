#[derive(Debug, Clone, Default)]
pub struct PageMetrics {
    pub page_number: usize,
    pub render_ms: f64,
    pub command_count: usize,
    pub flowable_count: usize,
    pub content_bytes: usize,
}

#[derive(Debug, Clone, Default)]
pub struct DocumentMetrics {
    pub pages: Vec<PageMetrics>,
    pub total_render_ms: f64,
    pub total_bytes: usize,
}
