#[derive(Debug, Clone)]
pub struct DocContext {
    pub page_number: usize,
    pub template_name: String,
}

impl DocContext {
    pub fn new(page_number: usize, template_name: impl Into<String>) -> Self {
        Self {
            page_number,
            template_name: template_name.into(),
        }
    }
}
