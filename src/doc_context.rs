use crate::style::CssPageContentPosition;
use crate::types::Pt;
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RunningElementValue {
    pub resource_id: String,
    pub width: Pt,
    pub height: Pt,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct GeneratedContentValues<T> {
    pub start: Option<T>,
    pub first: Option<T>,
    pub last: Option<T>,
    pub first_except: Option<T>,
}

impl<T> GeneratedContentValues<T> {
    fn select(&self, position: &CssPageContentPosition) -> Option<&T> {
        match position {
            CssPageContentPosition::Start => self.start.as_ref(),
            CssPageContentPosition::First => self.first.as_ref(),
            CssPageContentPosition::Last => self.last.as_ref(),
            CssPageContentPosition::FirstExcept => self.first_except.as_ref(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct DocContext {
    pub page_number: usize,
    pub total_pages: usize,
    pub page_counter: i32,
    pub template_name: String,
    running_elements: HashMap<String, GeneratedContentValues<RunningElementValue>>,
    named_strings: HashMap<String, GeneratedContentValues<String>>,
}

impl DocContext {
    pub fn new(page_number: usize, template_name: impl Into<String>) -> Self {
        Self {
            page_number,
            total_pages: page_number,
            page_counter: i32::try_from(page_number).unwrap_or(i32::MAX),
            template_name: template_name.into(),
            running_elements: HashMap::new(),
            named_strings: HashMap::new(),
        }
    }

    pub(crate) fn finalized(
        page_number: usize,
        total_pages: usize,
        page_counter: i32,
        template_name: impl Into<String>,
        running_elements: HashMap<String, GeneratedContentValues<RunningElementValue>>,
        named_strings: HashMap<String, GeneratedContentValues<String>>,
    ) -> Self {
        Self {
            page_number,
            total_pages,
            page_counter,
            template_name: template_name.into(),
            running_elements,
            named_strings,
        }
    }

    pub(crate) fn running_element(
        &self,
        name: &str,
        position: &CssPageContentPosition,
    ) -> Option<&RunningElementValue> {
        self.running_elements.get(name)?.select(position)
    }

    pub(crate) fn named_string(
        &self,
        name: &str,
        position: &CssPageContentPosition,
    ) -> Option<&str> {
        self.named_strings
            .get(name)?
            .select(position)
            .map(String::as_str)
    }
}
