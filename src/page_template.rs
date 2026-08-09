use crate::Canvas;
use crate::doc_context::DocContext;
use crate::frame::Frame;
use crate::types::{PagePresentation, Rect, Size};
use std::sync::Arc;

#[derive(Clone, Copy)]
pub struct FrameSpec {
    pub rect: Rect,
}

pub type OnPageCallback = Arc<dyn Fn(&mut Canvas, &DocContext) + Send + Sync>;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum PageSelector {
    Sequence,
    Any,
    First,
    Left,
    Right,
    BlankLeft,
    BlankRight,
    NamedAny(u64),
    NamedFirst(u64),
    NamedLeft(u64),
    NamedRight(u64),
    NamedBlankLeft(u64),
    NamedBlankRight(u64),
}

#[derive(Clone)]
pub struct PageTemplate {
    pub name: String,
    pub page_size: Size,
    frames: Vec<FrameSpec>,
    on_page: Option<OnPageCallback>,
    on_page_finalize: Option<OnPageCallback>,
    page_counter_reset: Option<i32>,
    page_counter_increment: Option<i32>,
    page_presentation: PagePresentation,
    selector: PageSelector,
}

impl PageTemplate {
    pub fn new(name: impl Into<String>, page_size: Size) -> Self {
        Self {
            name: name.into(),
            page_size: page_size.quantized(),
            frames: Vec::new(),
            on_page: None,
            on_page_finalize: None,
            page_counter_reset: None,
            page_counter_increment: None,
            page_presentation: PagePresentation::default(),
            selector: PageSelector::Sequence,
        }
    }

    pub fn with_frame(mut self, rect: Rect) -> Self {
        self.frames.push(FrameSpec {
            rect: rect.quantized(),
        });
        self
    }

    pub fn set_on_page<F>(mut self, callback: F) -> Self
    where
        F: Fn(&mut Canvas, &DocContext) + Send + Sync + 'static,
    {
        self.on_page = Some(Arc::new(callback));
        self
    }

    pub(crate) fn append_on_page<F>(mut self, callback: F) -> Self
    where
        F: Fn(&mut Canvas, &DocContext) + Send + Sync + 'static,
    {
        let previous = self.on_page.take();
        self.on_page = Some(Arc::new(move |canvas, context| {
            if let Some(previous) = previous.as_ref() {
                previous(canvas, context);
            }
            callback(canvas, context);
        }));
        self
    }

    pub fn on_page(&self) -> Option<&OnPageCallback> {
        self.on_page.as_ref()
    }

    pub(crate) fn append_on_page_finalize<F>(mut self, callback: F) -> Self
    where
        F: Fn(&mut Canvas, &DocContext) + Send + Sync + 'static,
    {
        let previous = self.on_page_finalize.take();
        self.on_page_finalize = Some(Arc::new(move |canvas, context| {
            if let Some(previous) = previous.as_ref() {
                previous(canvas, context);
            }
            callback(canvas, context);
        }));
        self
    }

    pub(crate) fn on_page_finalize(&self) -> Option<&OnPageCallback> {
        self.on_page_finalize.as_ref()
    }

    pub(crate) fn with_page_counter(mut self, reset: Option<i32>, increment: Option<i32>) -> Self {
        self.page_counter_reset = reset;
        self.page_counter_increment = increment;
        self
    }

    pub(crate) fn page_counter_reset(&self) -> Option<i32> {
        self.page_counter_reset
    }

    pub(crate) fn page_counter_increment(&self) -> Option<i32> {
        self.page_counter_increment
    }

    pub(crate) fn with_page_presentation(mut self, presentation: PagePresentation) -> Self {
        self.page_presentation = presentation;
        self
    }

    pub(crate) fn page_presentation(&self) -> PagePresentation {
        self.page_presentation
    }

    pub(crate) fn with_page_selector(mut self, selector: PageSelector) -> Self {
        self.selector = selector;
        self
    }

    pub(crate) fn page_selector(&self) -> PageSelector {
        self.selector
    }

    pub(crate) fn primary_frame_rect(&self) -> Option<Rect> {
        self.frames.first().map(|frame| frame.rect)
    }

    pub fn instantiate_frames(&self) -> Vec<Frame> {
        self.frames
            .iter()
            .map(|spec| Frame::new(spec.rect))
            .collect()
    }
}
