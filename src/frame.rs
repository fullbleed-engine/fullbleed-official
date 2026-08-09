use crate::canvas::Canvas;
use crate::flowable::{
    BreakInside, Flowable, PageFootnoteEntry, draw_page_footnotes, page_footnote_height,
    partition_page_footnotes_for_max_height,
};
use crate::types::{Pt, Rect, Size};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddDisposition {
    Placed,
    Split,
    Overflow,
}

impl AddDisposition {
    pub fn as_str(self) -> &'static str {
        match self {
            AddDisposition::Placed => "placed",
            AddDisposition::Split => "split",
            AddDisposition::Overflow => "overflow",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AddTrace {
    pub disposition: AddDisposition,
    pub reason: &'static str,
    pub avail_width: Pt,
    pub avail_height: Pt,
    pub frame_rect: Rect,
    pub cursor_y_before: Pt,
    pub wrapped_size: Size,
    pub placed_rect: Option<Rect>,
}

pub enum AddResult {
    Placed(AddTrace),
    Split(Box<dyn Flowable>, AddTrace),
    Overflow(Box<dyn Flowable>, AddTrace),
}

pub struct Frame {
    rect: Rect,
    cursor_y: Pt,
    footnote_reserved: Pt,
    deferred_footnotes: Vec<PageFootnoteEntry>,
}

impl Frame {
    pub fn new(rect: Rect) -> Self {
        Self {
            rect: rect.quantized(),
            cursor_y: Pt::ZERO,
            footnote_reserved: Pt::ZERO,
            deferred_footnotes: Vec::new(),
        }
    }

    pub fn remaining_height(&self) -> Pt {
        (self.rect.height - self.cursor_y - self.footnote_reserved).max(Pt::ZERO)
    }

    pub fn rect(&self) -> Rect {
        self.rect
    }

    pub fn is_empty(&self) -> bool {
        self.cursor_y <= Pt::ZERO
    }

    pub(crate) fn take_deferred_footnotes(&mut self) -> Vec<PageFootnoteEntry> {
        std::mem::take(&mut self.deferred_footnotes)
    }

    fn place_footnotes(&mut self, entries: &[PageFootnoteEntry], height: Pt, canvas: &mut Canvas) {
        if entries.is_empty() || height <= Pt::ZERO {
            return;
        }
        let y = self.rect.y + self.rect.height - self.footnote_reserved - height;
        draw_page_footnotes(entries, canvas, self.rect.x, y, self.rect.width, height);
        self.footnote_reserved += height;
    }

    fn draw_in_fragmentainer(
        &self,
        flowable: &dyn Flowable,
        canvas: &mut Canvas,
        y: Pt,
        avail_width: Pt,
        avail_height: Pt,
    ) {
        // Expose page-area bounds to effect compilers without clipping ordinary
        // CSS overflow. Browser print clips completed filter surfaces at the
        // fragmentainer, while text/background/transform overflow can paint
        // outside the page area.
        canvas.record_html_page_area(self.rect);
        canvas.push_fragmentainer(self.rect);
        flowable.draw(canvas, self.rect.x, y, avail_width, avail_height);
        canvas.pop_fragmentainer();
    }

    pub fn add(&mut self, flowable: Box<dyn Flowable>, canvas: &mut Canvas) -> AddResult {
        debug_assert!(
            self.deferred_footnotes.is_empty(),
            "deferred footnotes must be consumed after every frame placement"
        );
        let avail_width = self.rect.width;
        let avail_height = self.remaining_height();
        let cursor_y_before = self.cursor_y;
        if avail_height <= Pt::ZERO {
            return AddResult::Overflow(
                flowable,
                AddTrace {
                    disposition: AddDisposition::Overflow,
                    reason: "no_remaining_height",
                    avail_width,
                    avail_height,
                    frame_rect: self.rect,
                    cursor_y_before,
                    wrapped_size: Size {
                        width: Pt::ZERO,
                        height: Pt::ZERO,
                    },
                    placed_rect: None,
                },
            );
        }

        let pagination = flowable.pagination();
        let size = flowable.wrap(avail_width, avail_height);
        let all_footnotes = flowable.page_footnotes();
        let (footnotes, deferred_footnotes) =
            partition_page_footnotes_for_max_height(&all_footnotes, avail_width);
        let footnote_height = page_footnote_height(&footnotes, avail_width);
        let normal_avail_height = (avail_height - footnote_height).max(Pt::ZERO);
        if flowable.is_monolithic_replaced()
            && size.height > normal_avail_height
            && size.height <= self.rect.height
            && !self.is_empty()
        {
            return AddResult::Overflow(
                flowable,
                AddTrace {
                    disposition: AddDisposition::Overflow,
                    reason: "monolithic_replaced_move",
                    avail_width,
                    avail_height,
                    frame_rect: self.rect,
                    cursor_y_before,
                    wrapped_size: size,
                    placed_rect: None,
                },
            );
        }
        if matches!(
            pagination.break_inside,
            BreakInside::Avoid | BreakInside::AvoidPage
        ) && size.height > normal_avail_height
            && size.height <= self.rect.height
        {
            let can_move = !self.is_empty();
            if can_move {
                return AddResult::Overflow(
                    flowable,
                    AddTrace {
                        disposition: AddDisposition::Overflow,
                        reason: "avoid_page_move",
                        avail_width,
                        avail_height,
                        frame_rect: self.rect,
                        cursor_y_before,
                        wrapped_size: size,
                        placed_rect: None,
                    },
                );
            }
        }

        if size.height <= normal_avail_height {
            let rect = Rect {
                x: self.rect.x,
                y: self.rect.y + self.cursor_y,
                width: size.width,
                height: size.height,
            };
            self.draw_in_fragmentainer(
                flowable.as_ref(),
                canvas,
                self.rect.y + self.cursor_y,
                avail_width,
                normal_avail_height,
            );
            canvas.record_flowable_bounds(rect);
            self.cursor_y = self.cursor_y + size.height;
            self.place_footnotes(&footnotes, footnote_height, canvas);
            self.deferred_footnotes.extend(deferred_footnotes);
            return AddResult::Placed(AddTrace {
                disposition: AddDisposition::Placed,
                reason: "fits_in_remaining_height",
                avail_width,
                avail_height,
                frame_rect: self.rect,
                cursor_y_before,
                wrapped_size: size,
                placed_rect: Some(rect),
            });
        }

        if let Some((first, second)) = flowable.split(avail_width, avail_height) {
            let all_first_footnotes = first.page_footnotes();
            let (first_footnotes, first_deferred_footnotes) =
                partition_page_footnotes_for_max_height(&all_first_footnotes, avail_width);
            let first_footnote_height = page_footnote_height(&first_footnotes, avail_width);
            let first_avail_height = (avail_height - first_footnote_height).max(Pt::ZERO);
            let first_size = first.wrap(avail_width, first_avail_height);
            if first_size.height > Pt::ZERO && first_size.height <= first_avail_height {
                let rect = Rect {
                    x: self.rect.x,
                    y: self.rect.y + self.cursor_y,
                    width: first_size.width,
                    height: first_size.height,
                };
                self.draw_in_fragmentainer(
                    first.as_ref(),
                    canvas,
                    self.rect.y + self.cursor_y,
                    avail_width,
                    first_avail_height,
                );
                canvas.record_flowable_bounds(rect);
                self.cursor_y = self.cursor_y + first_size.height;
                self.place_footnotes(&first_footnotes, first_footnote_height, canvas);
                self.deferred_footnotes.extend(first_deferred_footnotes);
                return AddResult::Split(
                    second,
                    AddTrace {
                        disposition: AddDisposition::Split,
                        reason: "split_to_fit",
                        avail_width,
                        avail_height,
                        frame_rect: self.rect,
                        cursor_y_before,
                        wrapped_size: size,
                        placed_rect: Some(rect),
                    },
                );
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
            self.draw_in_fragmentainer(
                flowable.as_ref(),
                canvas,
                self.rect.y + self.cursor_y,
                avail_width,
                avail_height,
            );
            canvas.record_flowable_bounds(rect);
            self.cursor_y = self.rect.height;
            return AddResult::Placed(AddTrace {
                disposition: AddDisposition::Placed,
                reason: "forced_unsplittable_full_frame",
                avail_width,
                avail_height,
                frame_rect: self.rect,
                cursor_y_before,
                wrapped_size: size,
                placed_rect: Some(rect),
            });
        }

        AddResult::Overflow(
            flowable,
            AddTrace {
                disposition: AddDisposition::Overflow,
                reason: "unsplittable_overflow",
                avail_width,
                avail_height,
                frame_rect: self.rect,
                cursor_y_before,
                wrapped_size: size,
                placed_rect: None,
            },
        )
    }
}
