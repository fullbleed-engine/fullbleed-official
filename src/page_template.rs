use crate::Canvas;
use crate::doc_context::DocContext;
use crate::frame::Frame;
use crate::types::{Rect, Size};
use std::sync::Arc;

#[derive(Clone, Copy)]
pub struct FrameSpec {
    pub rect: Rect,
}

pub type OnPageCallback = Arc<dyn Fn(&mut Canvas, &DocContext) + Send + Sync>;

#[derive(Clone)]
pub struct PageTemplate {
    pub name: String,
    pub page_size: Size,
    frames: Vec<FrameSpec>,
    on_page: Option<OnPageCallback>,
}

impl PageTemplate {
    pub fn new(name: impl Into<String>, page_size: Size) -> Self {
        Self {
            name: name.into(),
            page_size: page_size.quantized(),
            frames: Vec::new(),
            on_page: None,
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

    pub fn on_page(&self) -> Option<&OnPageCallback> {
        self.on_page.as_ref()
    }

    pub fn instantiate_frames(&self) -> Vec<Frame> {
        self.frames
            .iter()
            .map(|spec| Frame::new(spec.rect))
            .collect()
    }
}
