use crate::canvas::Canvas;
use crate::flowable::{BreakInside, Flowable};
use crate::types::{Pt, Rect};

pub enum AddResult {
    Placed,
    Split(Box<dyn Flowable>),
    Overflow(Box<dyn Flowable>),
}

pub struct Frame {
    rect: Rect,
    cursor_y: Pt,
}

impl Frame {
    pub fn new(rect: Rect) -> Self {
        Self {
            rect: rect.quantized(),
            cursor_y: Pt::ZERO,
        }
    }

    pub fn remaining_height(&self) -> Pt {
        (self.rect.height - self.cursor_y).max(Pt::ZERO)
    }

    pub fn rect(&self) -> Rect {
        self.rect
    }

    pub fn is_empty(&self) -> bool {
        self.cursor_y <= Pt::ZERO
    }

    pub fn add(&mut self, flowable: Box<dyn Flowable>, canvas: &mut Canvas) -> AddResult {
        let avail_width = self.rect.width;
        let avail_height = self.remaining_height();
        if avail_height <= Pt::ZERO {
            return AddResult::Overflow(flowable);
        }

        let pagination = flowable.pagination();
        let mut size = flowable.wrap(avail_width, avail_height);
        size.width = size.width;
        size.height = size.height;
        if matches!(
            pagination.break_inside,
            BreakInside::Avoid | BreakInside::AvoidPage
        ) && size.height > avail_height
            && size.height <= self.rect.height
        {
            let can_move = !self.is_empty();
            if can_move {
                return AddResult::Overflow(flowable);
            }
        }

        if size.height <= avail_height {
            let rect = Rect {
                x: self.rect.x,
                y: self.rect.y + self.cursor_y,
                width: size.width,
                height: size.height,
            };
            flowable.draw(
                canvas,
                self.rect.x,
                self.rect.y + self.cursor_y,
                avail_width,
                avail_height,
            );
            canvas.record_flowable_bounds(rect);
            self.cursor_y = self.cursor_y + size.height;
            return AddResult::Placed;
        }

        if let Some((first, second)) = flowable.split(avail_width, avail_height) {
            let first_size = first.wrap(avail_width, avail_height);
            if first_size.height > Pt::ZERO && first_size.height <= avail_height {
                let rect = Rect {
                    x: self.rect.x,
                    y: self.rect.y + self.cursor_y,
                    width: first_size.width,
                    height: first_size.height,
                };
                first.draw(
                    canvas,
                    self.rect.x,
                    self.rect.y + self.cursor_y,
                    avail_width,
                    avail_height,
                );
                canvas.record_flowable_bounds(rect);
                self.cursor_y = self.cursor_y + first_size.height;
                return AddResult::Split(second);
            }
        }

        // Fallback: if this flowable is taller than a full page and cannot be split,
        // place it on the current page to avoid a hard failure. This mirrors browser
        // behavior for overfull blocks and keeps pagination moving forward.
        if self.is_empty() {
            let rect = Rect {
                x: self.rect.x,
                y: self.rect.y + self.cursor_y,
                width: avail_width,
                height: avail_height,
            };
            flowable.draw(
                canvas,
                self.rect.x,
                self.rect.y + self.cursor_y,
                avail_width,
                avail_height,
            );
            canvas.record_flowable_bounds(rect);
            self.cursor_y = self.rect.height;
            return AddResult::Placed;
        }

        AddResult::Overflow(flowable)
    }
}
