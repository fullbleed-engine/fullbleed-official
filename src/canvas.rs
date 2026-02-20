use crate::types::{Color, Pt, Rect, Shading, Size};

#[derive(Debug, Clone)]
pub enum Command {
    SaveState,
    RestoreState,
    Translate(Pt, Pt),
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
    SetFontName(String),
    SetFontSize(Pt),
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
    // Raster-only command emitted by PDF parser for precise Type0/CID text rendering.
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
    },
    DefineForm {
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
    font_size: Pt,
    font_name: String,
}

pub struct Canvas {
    page_size: Size,
    pages: Vec<Page>,
    current: Page,
    state_stack: Vec<GraphicsState>,
    current_state: GraphicsState,
    current_mcid: u32,
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
                font_size: Pt::from_f32(12.0),
                font_name: "Helvetica".to_string(),
            },
            current_mcid: 0,
        }
    }

    pub fn page_size(&self) -> Size {
        self.page_size
    }

    pub fn save_state(&mut self) {
        self.state_stack.push(self.current_state.clone());
        self.current.commands.push(Command::SaveState);
    }

    pub fn restore_state(&mut self) {
        if let Some(state) = self.state_stack.pop() {
            self.current_state = state;
            self.current.commands.push(Command::RestoreState);
        }
    }

    pub fn translate(&mut self, x: Pt, y: Pt) {
        self.current.commands.push(Command::Translate(x, y));
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
            key: "__fb_bbox".to_string(),
            value,
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
        self.current.commands.push(Command::DrawImage {
            x,
            y,
            width,
            height,
            resource_id: resource_id.into(),
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
            font_size: Pt::from_f32(12.0),
            font_name: "Helvetica".to_string(),
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
