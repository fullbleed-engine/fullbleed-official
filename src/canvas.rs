use crate::flowable::PaintFilterSpec;
use crate::types::{Color, MixBlendMode, Pt, Rect, Shading, Size};

pub const META_FLOWABLE_BBOX_KEY: &str = "__fb_bbox";
pub const META_HTML_SCROLLABLE_RIGHT_KEY: &str = "__fb_html_scrollable_right";
pub const META_PAGINATION_EVENT_KEY: &str = "__fb_pagination_event";
pub const META_DIAGNOSTIC_SCOPE_BEGIN_KEY: &str = "__fb_diag_scope_begin";
pub const META_DIAGNOSTIC_SCOPE_END_KEY: &str = "__fb_diag_scope_end";

#[derive(Debug, Clone)]
pub enum Command {
    SaveState,
    RestoreState,
    Translate(Pt, Pt),
    /// Translate around a point expressed in FullBleed's top-down page space.
    /// PDF emission converts the y coordinate to bottom-up space; raster/JIT
    /// consumers keep it top-down. `inverse` emits the return translation.
    CssTransformOrigin {
        x: Pt,
        y: Pt,
        inverse: bool,
    },
    Scale(f32, f32),
    Rotate(f32),
    ConcatMatrix {
        a: f32,
        b: f32,
        c: f32,
        d: f32,
        e: Pt,
        f: Pt,
    },
    // Non-rendered metadata used for page-aware reporting. Ignored by the PDF renderer.
    Meta {
        key: String,
        value: String,
    },
    SetFillColor(Color),
    SetStrokeColor(Color),
    SetLineWidth(Pt),
    SetLineCap(u8),
    SetLineJoin(u8),
    SetMiterLimit(Pt),
    SetDash {
        pattern: Vec<Pt>,
        phase: Pt,
    },
    // Applies both fill and stroke alpha (ca/CA). Values outside 0..1 are clamped.
    SetOpacity {
        fill: f32,
        stroke: f32,
    },
    SetBlendMode {
        mode: MixBlendMode,
    },
    ApplyBackdropFilter {
        x: Pt,
        y: Pt,
        width: Pt,
        height: Pt,
        radius: Pt,
        filter: PaintFilterSpec,
    },
    SetFontName(String),
    SetFontSize(Pt),
    SetTextRenderingMode(u8),
    ClipRect {
        x: Pt,
        y: Pt,
        width: Pt,
        height: Pt,
    },
    // Clip to the current path (W/W* n). The current path is consumed.
    ClipPath {
        evenodd: bool,
    },
    // Paint a shading (/<name> sh). Usually used with ClipPath.
    ShadingFill(Shading),
    MoveTo {
        x: Pt,
        y: Pt,
    },
    LineTo {
        x: Pt,
        y: Pt,
    },
    CurveTo {
        x1: Pt,
        y1: Pt,
        x2: Pt,
        y2: Pt,
        x: Pt,
        y: Pt,
    },
    ClosePath,
    Fill,
    FillEvenOdd,
    Stroke,
    FillStroke,
    FillStrokeEvenOdd,
    DrawString {
        x: Pt,
        y: Pt,
        text: String,
    },
    // Raster-focused command: draw text with an explicit PDF-space linear transform.
    DrawStringTransformed {
        x: Pt,
        y: Pt,
        text: String,
        m00: f32,
        m01: f32,
        m10: f32,
        m11: f32,
    },
    // Explicit glyph run used by the raster backend and by PDF Type 3 outline fonts.
    DrawGlyphRun {
        x: Pt,
        y: Pt,
        glyph_ids: Vec<u16>,
        advances: Vec<(Pt, Pt)>,
        m00: f32,
        m01: f32,
        m10: f32,
        m11: f32,
    },
    DrawRect {
        x: Pt,
        y: Pt,
        width: Pt,
        height: Pt,
    },
    DrawImage {
        x: Pt,
        y: Pt,
        width: Pt,
        height: Pt,
        resource_id: String,
        interpolate: bool,
    },
    DefineForm {
        resource_id: String,
        width: Pt,
        height: Pt,
        commands: Vec<Command>,
    },
    DefineIsolatedForm {
        resource_id: String,
        width: Pt,
        height: Pt,
        commands: Vec<Command>,
    },
    DrawForm {
        x: Pt,
        y: Pt,
        width: Pt,
        height: Pt,
        resource_id: String,
    },
    DrawFilteredForm {
        x: Pt,
        y: Pt,
        width: Pt,
        height: Pt,
        resource_id: String,
        filter: PaintFilterSpec,
    },
    BeginTag {
        role: String,
        mcid: Option<u32>,
        alt: Option<String>,
        scope: Option<String>,
        table_id: Option<u32>,
        col_index: Option<u16>,
        group_only: bool,
    },
    EndTag,
    BeginArtifact {
        subtype: Option<String>,
    },
    BeginOptionalContent {
        name: String,
    },
    EndMarkedContent,
}

#[derive(Debug, Clone)]
pub struct Page {
    pub commands: Vec<Command>,
}

impl Page {
    fn new() -> Self {
        Self {
            commands: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Document {
    pub page_size: Size,
    pub pages: Vec<Page>,
}

#[derive(Debug, Clone)]
struct GraphicsState {
    fill_color: Color,
    stroke_color: Color,
    line_width: Pt,
    line_cap: u8,
    line_join: u8,
    blend_mode: MixBlendMode,
    font_size: Pt,
    font_name: String,
    text_rendering_mode: u8,
}

pub struct Canvas {
    page_size: Size,
    pages: Vec<Page>,
    current: Page,
    state_stack: Vec<GraphicsState>,
    current_state: GraphicsState,
    current_mcid: u32,
    // Nearest positioned-ancestor containing block stack for out-of-flow absolute placement.
    abs_containing_block_stack: Vec<Rect>,
}

impl Canvas {
    pub fn new(page_size: Size) -> Self {
        Self {
            page_size,
            pages: Vec::new(),
            current: Page::new(),
            state_stack: Vec::new(),
            current_state: GraphicsState {
                fill_color: Color::BLACK,
                stroke_color: Color::BLACK,
                line_width: Pt::from_f32(1.0),
                line_cap: 0,
                line_join: 0,
                blend_mode: MixBlendMode::Normal,
                font_size: Pt::from_f32(12.0),
                font_name: "Helvetica".to_string(),
                text_rendering_mode: 0,
            },
            current_mcid: 0,
            abs_containing_block_stack: Vec::new(),
        }
    }

    pub fn page_size(&self) -> Size {
        self.page_size
    }

    pub fn push_abs_containing_block(&mut self, rect: Rect) {
        self.abs_containing_block_stack.push(rect);
    }

    pub fn pop_abs_containing_block(&mut self) {
        self.abs_containing_block_stack.pop();
    }

    pub fn current_abs_containing_block(&self) -> Option<Rect> {
        self.abs_containing_block_stack.last().copied()
    }

    pub fn save_state(&mut self) {
        self.state_stack.push(self.current_state.clone());
        self.current.commands.push(Command::SaveState);
    }

    pub fn restore_state(&mut self) {
        if let Some(state) = self.state_stack.pop() {
            let previous = self.current_state.clone();
            self.current_state = state;
            self.current.commands.push(Command::RestoreState);
            // PDF q/Q restores graphics paint parameters, but the text state is
            // maintained separately. Re-emit changed text parameters so a
            // scoped font draw cannot leak into the following text run.
            if self.current_state.font_name != previous.font_name {
                self.current
                    .commands
                    .push(Command::SetFontName(self.current_state.font_name.clone()));
            }
            if self.current_state.font_size != previous.font_size {
                self.current
                    .commands
                    .push(Command::SetFontSize(self.current_state.font_size));
            }
            if self.current_state.text_rendering_mode != previous.text_rendering_mode {
                self.current.commands.push(Command::SetTextRenderingMode(
                    self.current_state.text_rendering_mode,
                ));
            }
        }
    }

    pub fn translate(&mut self, x: Pt, y: Pt) {
        self.current.commands.push(Command::Translate(x, y));
    }

    pub fn translate_css_transform_origin(&mut self, x: Pt, y: Pt, inverse: bool) {
        self.current
            .commands
            .push(Command::CssTransformOrigin { x, y, inverse });
    }

    pub fn scale(&mut self, x: f32, y: f32) {
        self.current.commands.push(Command::Scale(x, y));
    }

    pub fn rotate(&mut self, angle_radians: f32) {
        self.current.commands.push(Command::Rotate(angle_radians));
    }

    pub fn concat_matrix(&mut self, a: f32, b: f32, c: f32, d: f32, e: Pt, f: Pt) {
        self.current
            .commands
            .push(Command::ConcatMatrix { a, b, c, d, e, f });
    }

    pub fn record_flowable_bounds(&mut self, rect: Rect) {
        let value = format!(
            "{},{},{},{}",
            rect.x.to_milli_i64(),
            rect.y.to_milli_i64(),
            rect.width.to_milli_i64(),
            rect.height.to_milli_i64()
        );
        self.current.commands.push(Command::Meta {
            key: META_FLOWABLE_BBOX_KEY.to_string(),
            value,
        });
    }

    pub fn record_html_scrollable_right(&mut self, right: Pt) {
        self.current.commands.push(Command::Meta {
            key: META_HTML_SCROLLABLE_RIGHT_KEY.to_string(),
            value: right.to_milli_i64().to_string(),
        });
    }

    pub fn meta(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.current.commands.push(Command::Meta {
            key: key.into(),
            value: value.into(),
        });
    }

    pub fn set_fill_color(&mut self, color: Color) {
        if self.current_state.fill_color == color {
            return;
        }
        self.current_state.fill_color = color;
        self.current.commands.push(Command::SetFillColor(color));
    }

    pub fn set_stroke_color(&mut self, color: Color) {
        if self.current_state.stroke_color == color {
            return;
        }
        self.current_state.stroke_color = color;
        self.current.commands.push(Command::SetStrokeColor(color));
    }

    pub fn set_line_width(&mut self, width: Pt) {
        let width = if width < Pt::ZERO { Pt::ZERO } else { width };
        if self.current_state.line_width == width {
            return;
        }
        self.current_state.line_width = width;
        self.current.commands.push(Command::SetLineWidth(width));
    }

    pub fn set_line_cap(&mut self, cap: u8) {
        if self.current_state.line_cap == cap {
            return;
        }
        self.current_state.line_cap = cap;
        self.current.commands.push(Command::SetLineCap(cap));
    }

    pub fn set_line_join(&mut self, join: u8) {
        if self.current_state.line_join == join {
            return;
        }
        self.current_state.line_join = join;
        self.current.commands.push(Command::SetLineJoin(join));
    }

    pub fn set_miter_limit(&mut self, limit: Pt) {
        let limit = if limit < Pt::ZERO { Pt::ZERO } else { limit };
        self.current.commands.push(Command::SetMiterLimit(limit));
    }

    pub fn set_dash(&mut self, pattern: Vec<Pt>, phase: Pt) {
        self.current
            .commands
            .push(Command::SetDash { pattern, phase });
    }

    pub fn set_opacity(&mut self, fill: f32, stroke: f32) {
        self.current.commands.push(Command::SetOpacity {
            fill: fill.clamp(0.0, 1.0),
            stroke: stroke.clamp(0.0, 1.0),
        });
    }

    pub fn set_blend_mode(&mut self, mode: MixBlendMode) {
        if self.current_state.blend_mode == mode {
            return;
        }
        self.current_state.blend_mode = mode;
        self.current.commands.push(Command::SetBlendMode { mode });
    }

    pub fn apply_backdrop_filter(
        &mut self,
        x: Pt,
        y: Pt,
        width: Pt,
        height: Pt,
        radius: Pt,
        filter: PaintFilterSpec,
    ) {
        self.current.commands.push(Command::ApplyBackdropFilter {
            x,
            y,
            width,
            height,
            radius,
            filter,
        });
    }

    pub fn set_font_name(&mut self, name: &str) {
        if self.current_state.font_name == name {
            return;
        }
        self.current_state.font_name = name.to_string();
        self.current
            .commands
            .push(Command::SetFontName(self.current_state.font_name.clone()));
    }

    pub fn set_font_size(&mut self, size: Pt) {
        if self.current_state.font_size == size {
            return;
        }
        self.current_state.font_size = size;
        self.current.commands.push(Command::SetFontSize(size));
    }

    pub fn set_text_rendering_mode(&mut self, mode: u8) {
        let mode = mode.min(7);
        if self.current_state.text_rendering_mode == mode {
            return;
        }
        self.current_state.text_rendering_mode = mode;
        self.current
            .commands
            .push(Command::SetTextRenderingMode(mode));
    }

    pub fn clip_rect(&mut self, x: Pt, y: Pt, width: Pt, height: Pt) {
        self.current.commands.push(Command::ClipRect {
            x,
            y,
            width,
            height,
        });
    }

    pub fn clip_path(&mut self, evenodd: bool) {
        self.current.commands.push(Command::ClipPath { evenodd });
    }

    pub fn shading_fill(&mut self, shading: Shading) {
        self.current.commands.push(Command::ShadingFill(shading));
    }

    pub fn move_to(&mut self, x: Pt, y: Pt) {
        self.current.commands.push(Command::MoveTo { x, y });
    }

    pub fn line_to(&mut self, x: Pt, y: Pt) {
        self.current.commands.push(Command::LineTo { x, y });
    }

    pub fn curve_to(&mut self, x1: Pt, y1: Pt, x2: Pt, y2: Pt, x: Pt, y: Pt) {
        self.current.commands.push(Command::CurveTo {
            x1,
            y1,
            x2,
            y2,
            x,
            y,
        });
    }

    pub fn close_path(&mut self) {
        self.current.commands.push(Command::ClosePath);
    }

    pub fn fill(&mut self) {
        self.current.commands.push(Command::Fill);
    }

    pub fn fill_evenodd(&mut self) {
        self.current.commands.push(Command::FillEvenOdd);
    }

    pub fn stroke(&mut self) {
        self.current.commands.push(Command::Stroke);
    }

    pub fn fill_stroke(&mut self) {
        self.current.commands.push(Command::FillStroke);
    }

    pub fn fill_stroke_evenodd(&mut self) {
        self.current.commands.push(Command::FillStrokeEvenOdd);
    }

    pub fn draw_string(&mut self, x: Pt, y: Pt, text: impl Into<String>) {
        self.current.commands.push(Command::DrawString {
            x,
            y,
            text: text.into(),
        });
    }

    pub(crate) fn draw_glyph_run(
        &mut self,
        x: Pt,
        baseline_y_from_top: Pt,
        glyph_ids: Vec<u16>,
        advances: Vec<(Pt, Pt)>,
    ) {
        self.current.commands.push(Command::DrawGlyphRun {
            x,
            y: baseline_y_from_top,
            glyph_ids,
            advances,
            m00: 1.0,
            m01: 0.0,
            m10: 0.0,
            m11: 1.0,
        });
    }

    pub fn draw_string_synthetic_bold(
        &mut self,
        x: Pt,
        y: Pt,
        text: impl Into<String>,
        stroke_width: Pt,
    ) {
        self.save_state();
        self.set_stroke_color(self.current_state.fill_color);
        self.set_line_width(stroke_width.max(Pt::ZERO));
        self.set_text_rendering_mode(2);
        self.draw_string(x, y, text);
        self.restore_state();
    }

    pub fn draw_string_synthetic_italic(
        &mut self,
        x: Pt,
        y: Pt,
        text: impl Into<String>,
        shear: f32,
    ) {
        let baseline_y = self.page_size.height - y - self.current_state.font_size;
        self.current.commands.push(Command::DrawStringTransformed {
            x,
            y: baseline_y,
            text: text.into(),
            m00: 1.0,
            m01: 0.0,
            m10: shear,
            m11: 1.0,
        });
    }

    pub fn draw_string_synthetic_bold_italic(
        &mut self,
        x: Pt,
        y: Pt,
        text: impl Into<String>,
        stroke_width: Pt,
        shear: f32,
    ) {
        self.save_state();
        self.set_stroke_color(self.current_state.fill_color);
        self.set_line_width(stroke_width.max(Pt::ZERO));
        self.set_text_rendering_mode(2);
        self.draw_string_synthetic_italic(x, y, text, shear);
        self.restore_state();
    }

    pub fn draw_rect(&mut self, x: Pt, y: Pt, width: Pt, height: Pt) {
        self.current.commands.push(Command::DrawRect {
            x,
            y,
            width,
            height,
        });
    }

    pub fn draw_image(
        &mut self,
        x: Pt,
        y: Pt,
        width: Pt,
        height: Pt,
        resource_id: impl Into<String>,
    ) {
        self.draw_image_with_interpolation(x, y, width, height, resource_id, true);
    }

    pub fn draw_image_with_interpolation(
        &mut self,
        x: Pt,
        y: Pt,
        width: Pt,
        height: Pt,
        resource_id: impl Into<String>,
        interpolate: bool,
    ) {
        self.current.commands.push(Command::DrawImage {
            x,
            y,
            width,
            height,
            resource_id: resource_id.into(),
            interpolate,
        });
    }

    pub fn define_form(
        &mut self,
        resource_id: impl Into<String>,
        width: Pt,
        height: Pt,
        commands: Vec<Command>,
    ) {
        self.current.commands.push(Command::DefineForm {
            resource_id: resource_id.into(),
            width,
            height,
            commands,
        });
    }

    pub fn define_isolated_form(
        &mut self,
        resource_id: impl Into<String>,
        width: Pt,
        height: Pt,
        commands: Vec<Command>,
    ) {
        self.current.commands.push(Command::DefineIsolatedForm {
            resource_id: resource_id.into(),
            width,
            height,
            commands,
        });
    }

    pub fn draw_form(
        &mut self,
        x: Pt,
        y: Pt,
        width: Pt,
        height: Pt,
        resource_id: impl Into<String>,
    ) {
        self.current.commands.push(Command::DrawForm {
            x,
            y,
            width,
            height,
            resource_id: resource_id.into(),
        });
    }

    pub fn draw_filtered_form(
        &mut self,
        x: Pt,
        y: Pt,
        width: Pt,
        height: Pt,
        resource_id: impl Into<String>,
        filter: PaintFilterSpec,
    ) {
        self.current.commands.push(Command::DrawFilteredForm {
            x,
            y,
            width,
            height,
            resource_id: resource_id.into(),
            filter,
        });
    }

    pub fn show_page(&mut self) {
        let current = std::mem::replace(&mut self.current, Page::new());
        self.pages.push(current);
        self.state_stack.clear();
        self.current_state = GraphicsState {
            fill_color: Color::BLACK,
            stroke_color: Color::BLACK,
            line_width: Pt::from_f32(1.0),
            line_cap: 0,
            line_join: 0,
            blend_mode: MixBlendMode::Normal,
            font_size: Pt::from_f32(12.0),
            font_name: "Helvetica".to_string(),
            text_rendering_mode: 0,
        };
        self.current_mcid = 0;
    }

    pub fn begin_tag(
        &mut self,
        role: impl Into<String>,
        alt: Option<String>,
        scope: Option<String>,
        table_id: Option<u32>,
        col_index: Option<u16>,
        group_only: bool,
    ) -> Option<u32> {
        let mcid = if group_only {
            None
        } else {
            let mcid = self.current_mcid;
            self.current_mcid = self.current_mcid.saturating_add(1);
            Some(mcid)
        };
        self.current.commands.push(Command::BeginTag {
            role: role.into(),
            mcid,
            alt,
            scope,
            table_id,
            col_index,
            group_only,
        });
        mcid
    }

    pub fn end_tag(&mut self) {
        self.current.commands.push(Command::EndTag);
    }

    pub fn begin_artifact(&mut self, subtype: Option<String>) {
        self.current
            .commands
            .push(Command::BeginArtifact { subtype });
    }

    pub fn begin_optional_content(&mut self, name: impl Into<String>) {
        self.current
            .commands
            .push(Command::BeginOptionalContent { name: name.into() });
    }

    pub fn end_marked_content(&mut self) {
        self.current.commands.push(Command::EndMarkedContent);
    }

    pub fn current_command_count(&self) -> usize {
        self.current.commands.len()
    }

    pub fn is_current_empty(&self) -> bool {
        self.current.commands.is_empty()
    }

    pub fn finish(mut self) -> Document {
        if !self.current.commands.is_empty() || self.pages.is_empty() {
            self.show_page();
        }
        Document {
            page_size: self.page_size,
            pages: self.pages,
        }
    }

    pub fn finish_without_show(self) -> Document {
        Document {
            page_size: self.page_size,
            pages: self.pages,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synthetic_bold_keeps_one_extractable_text_command() {
        let mut canvas = Canvas::new(Size {
            width: Pt::from_f32(100.0),
            height: Pt::from_f32(100.0),
        });
        canvas.set_fill_color(Color::rgb(0.4, 0.0, 0.0));
        canvas.draw_string_synthetic_bold(
            Pt::from_f32(10.0),
            Pt::from_f32(20.0),
            "Bold",
            Pt::from_f32(0.75),
        );
        let document = canvas.finish();
        let commands = &document.pages[0].commands;
        assert_eq!(
            commands
                .iter()
                .filter(|command| matches!(command, Command::DrawString { .. }))
                .count(),
            1
        );
        assert!(
            commands
                .iter()
                .any(|command| { matches!(command, Command::SetTextRenderingMode(2)) })
        );
        assert!(matches!(
            commands.last(),
            Some(Command::SetTextRenderingMode(0))
        ));
    }

    #[test]
    fn synthetic_italic_anchors_a_pdf_space_shear_at_the_baseline() {
        let mut canvas = Canvas::new(Size {
            width: Pt::from_f32(100.0),
            height: Pt::from_f32(100.0),
        });
        canvas.set_font_size(Pt::from_f32(20.0));
        canvas.draw_string_synthetic_italic(Pt::from_f32(10.0), Pt::from_f32(30.0), "Italic", 0.25);
        let document = canvas.finish();

        assert!(matches!(
            document.pages[0].commands.last(),
            Some(Command::DrawStringTransformed {
                x,
                y,
                m00,
                m01,
                m10,
                m11,
                ..
            }) if *x == Pt::from_f32(10.0)
                && *y == Pt::from_f32(50.0)
                && *m00 == 1.0
                && *m01 == 0.0
                && *m10 == 0.25
                && *m11 == 1.0
        ));
    }

    #[test]
    fn restore_state_reselects_changed_text_parameters() {
        let mut canvas = Canvas::new(Size {
            width: Pt::from_f32(100.0),
            height: Pt::from_f32(100.0),
        });
        canvas.set_font_name("Main");
        canvas.set_font_size(Pt::from_f32(20.0));
        canvas.save_state();
        canvas.set_font_name("Annotation");
        canvas.set_font_size(Pt::from_f32(10.0));
        canvas.restore_state();
        let document = canvas.finish();
        let commands = &document.pages[0].commands;
        let suffix = &commands[commands.len() - 3..];
        assert!(matches!(suffix[0], Command::RestoreState));
        assert!(matches!(
            &suffix[1],
            Command::SetFontName(name) if name == "Main"
        ));
        assert!(matches!(
            &suffix[2],
            Command::SetFontSize(size) if *size == Pt::from_f32(20.0)
        ));
    }
}
