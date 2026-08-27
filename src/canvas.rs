use crate::flowable::{MaskComposite, MaskMode, PaintFilterSpec};
use crate::types::{
    Color, MixBlendMode, PageOrientation, PagePresentation, Pt, Rect, Shading, Size,
};

pub const META_FLOWABLE_BBOX_KEY: &str = "__fb_bbox";
pub const META_HTML_CANVAS_BACKGROUND_KEY: &str = "__fb_html_canvas_background";
pub const META_HTML_PAGE_AREA_KEY: &str = "__fb_html_page_area";
pub const META_HTML_SCROLLABLE_BOTTOM_KEY: &str = "__fb_html_scrollable_bottom";
pub const META_HTML_SCROLLABLE_RIGHT_KEY: &str = "__fb_html_scrollable_right";
pub const META_HTML_SCROLLABLE_TOP_KEY: &str = "__fb_html_scrollable_top";
pub const META_PAGINATION_EVENT_KEY: &str = "__fb_pagination_event";
pub const META_PAGE_SIZE_KEY: &str = "__fb_page_size";
pub const META_PAGE_PRESENTATION_KEY: &str = "__fb_page_presentation";
pub const META_RUNNING_ELEMENT_PREFIX: &str = "__fb_running_element:";
pub const META_NAMED_STRING_PREFIX: &str = "__fb_named_string:";
pub const META_DIAGNOSTIC_SCOPE_BEGIN_KEY: &str = "__fb_diag_scope_begin";
pub const META_DIAGNOSTIC_SCOPE_END_KEY: &str = "__fb_diag_scope_end";

/// A compact projective vector transform retained only while the display list
/// is being compiled. Geometry is lowered to ordinary PDF path commands at
/// record time, so linked documents do not need a raster surface or a custom
/// runtime to execute CSS 3D transforms.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ProjectiveTransform {
    matrix: [[f64; 4]; 4],
}

impl ProjectiveTransform {
    pub(crate) fn identity() -> Self {
        Self {
            matrix: [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
        }
    }

    pub(crate) fn multiply(self, rhs: Self) -> Self {
        let mut matrix = [[0.0; 4]; 4];
        for (row, values) in matrix.iter_mut().enumerate() {
            for (column, value) in values.iter_mut().enumerate() {
                *value = (0..4)
                    .map(|index| self.matrix[row][index] * rhs.matrix[index][column])
                    .sum();
            }
        }
        Self { matrix }
    }

    pub(crate) fn translation(x: Pt, y: Pt, z: Pt) -> Self {
        let mut matrix = Self::identity();
        matrix.matrix[0][3] = x.to_f32() as f64;
        matrix.matrix[1][3] = y.to_f32() as f64;
        matrix.matrix[2][3] = z.to_f32() as f64;
        matrix
    }

    pub(crate) fn scale(x: f32, y: f32, z: f32) -> Self {
        let mut matrix = Self::identity();
        matrix.matrix[0][0] = x as f64;
        matrix.matrix[1][1] = y as f64;
        matrix.matrix[2][2] = z as f64;
        matrix
    }

    pub(crate) fn rotate_axis(x: f32, y: f32, z: f32, radians: f32) -> Self {
        let magnitude = ((x as f64).powi(2) + (y as f64).powi(2) + (z as f64).powi(2)).sqrt();
        if magnitude <= f64::EPSILON || !magnitude.is_finite() || !radians.is_finite() {
            return Self::identity();
        }
        let x = x as f64 / magnitude;
        let y = y as f64 / magnitude;
        let z = z as f64 / magnitude;
        let cosine = (radians as f64).cos();
        let sine = (radians as f64).sin();
        let one_minus_cosine = 1.0 - cosine;
        Self {
            matrix: [
                [
                    cosine + x * x * one_minus_cosine,
                    x * y * one_minus_cosine - z * sine,
                    x * z * one_minus_cosine + y * sine,
                    0.0,
                ],
                [
                    y * x * one_minus_cosine + z * sine,
                    cosine + y * y * one_minus_cosine,
                    y * z * one_minus_cosine - x * sine,
                    0.0,
                ],
                [
                    z * x * one_minus_cosine - y * sine,
                    z * y * one_minus_cosine + x * sine,
                    cosine + z * z * one_minus_cosine,
                    0.0,
                ],
                [0.0, 0.0, 0.0, 1.0],
            ],
        }
    }

    pub(crate) fn perspective(distance: Pt) -> Self {
        let distance = distance.to_f32() as f64;
        if !distance.is_finite() || distance <= 0.0 {
            return Self::identity();
        }
        let mut matrix = Self::identity();
        matrix.matrix[3][2] = -1.0 / distance;
        matrix
    }

    pub(crate) fn from_css_matrix3d(values: [f32; 16]) -> Self {
        let px = 0.75f64;
        Self {
            matrix: [
                [
                    values[0] as f64,
                    values[4] as f64,
                    values[8] as f64,
                    values[12] as f64 * px,
                ],
                [
                    values[1] as f64,
                    values[5] as f64,
                    values[9] as f64,
                    values[13] as f64 * px,
                ],
                [
                    values[2] as f64,
                    values[6] as f64,
                    values[10] as f64,
                    values[14] as f64 * px,
                ],
                [
                    values[3] as f64,
                    values[7] as f64,
                    values[11] as f64,
                    values[15] as f64,
                ],
            ],
        }
    }

    pub(crate) fn map_point(self, x: Pt, y: Pt) -> (Pt, Pt) {
        let x = x.to_f32() as f64;
        let y = y.to_f32() as f64;
        let out_x = self.matrix[0][0] * x + self.matrix[0][1] * y + self.matrix[0][3];
        let out_y = self.matrix[1][0] * x + self.matrix[1][1] * y + self.matrix[1][3];
        let w = self.matrix[3][0] * x + self.matrix[3][1] * y + self.matrix[3][3];
        let divisor = if w.is_finite() && w.abs() > 1.0e-9 {
            w
        } else {
            1.0
        };
        (
            Pt::from_f32((out_x / divisor) as f32),
            Pt::from_f32((out_y / divisor) as f32),
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct PerspectiveContext {
    pub origin_x: Pt,
    pub origin_y: Pt,
    pub distance: Pt,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledMaskLayer {
    pub resource_id: String,
    pub mode: MaskMode,
    pub composite: MaskComposite,
}

/// The visible portion of an image, expressed in the image command's target
/// coordinate space. Backends use this to discard fully hidden source pixels
/// before interpolation while retaining the outer clip for partial pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ImageSourceClip {
    pub left: Pt,
    pub top: Pt,
    pub right: Pt,
    pub bottom: Pt,
    pub snap_target_origin_to_css_pixel: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ResolvedImageSourceCrop {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub source_width: u32,
    pub source_height: u32,
}

impl ImageSourceClip {
    pub(crate) fn resolve(
        self,
        target_width: Pt,
        target_height: Pt,
        source_width: u32,
        source_height: u32,
    ) -> Option<ResolvedImageSourceCrop> {
        fn source_edge(value: Pt, target: Pt, source: u32, ceil: bool) -> u32 {
            let target = target.to_milli_i64().max(0) as i128;
            if target == 0 || source == 0 {
                return 0;
            }
            let value = value.to_milli_i64().clamp(0, target as i64) as i128;
            let numerator = value.saturating_mul(source as i128);
            let scaled = if ceil {
                numerator.saturating_add(target - 1) / target
            } else {
                numerator / target
            };
            scaled.clamp(0, source as i128) as u32
        }

        if target_width <= Pt::ZERO
            || target_height <= Pt::ZERO
            || source_width == 0
            || source_height == 0
        {
            return None;
        }
        let x0 = source_edge(self.left, target_width, source_width, false);
        let y0 = source_edge(self.top, target_height, source_height, false);
        let x1 = source_edge(self.right, target_width, source_width, true);
        let y1 = source_edge(self.bottom, target_height, source_height, true);
        if x0 >= x1 || y0 >= y1 {
            return None;
        }
        if x0 == 0 && y0 == 0 && x1 == source_width && y1 == source_height {
            return None;
        }
        Some(ResolvedImageSourceCrop {
            x: x0,
            y: y0,
            width: x1 - x0,
            height: y1 - y0,
            source_width,
            source_height,
        })
    }

    pub(crate) fn snap_target_rect(self, rect: (Pt, Pt, Pt, Pt)) -> (Pt, Pt, Pt, Pt) {
        fn round_to_css_pixel(value: Pt) -> Pt {
            let milli = value.to_milli_i64();
            let rounded = if milli >= 0 {
                ((milli + 375) / 750) * 750
            } else {
                ((milli - 375) / 750) * 750
            };
            Pt::from_milli_i64(rounded)
        }

        if self.snap_target_origin_to_css_pixel {
            (
                round_to_css_pixel(rect.0),
                round_to_css_pixel(rect.1),
                rect.2,
                rect.3,
            )
        } else {
            rect
        }
    }
}

impl ResolvedImageSourceCrop {
    pub(crate) fn target_rect(self, x: Pt, y: Pt, width: Pt, height: Pt) -> (Pt, Pt, Pt, Pt) {
        fn scaled(value: Pt, numerator: u32, denominator: u32) -> Pt {
            if denominator == 0 {
                return Pt::ZERO;
            }
            let product = (value.to_milli_i64() as i128).saturating_mul(numerator as i128);
            let denominator = denominator as i128;
            let rounded = if product >= 0 {
                product.saturating_add(denominator / 2) / denominator
            } else {
                -((-product).saturating_add(denominator / 2) / denominator)
            };
            Pt::from_milli_i64(rounded.clamp(i64::MIN as i128, i64::MAX as i128) as i64)
        }

        (
            x + scaled(width, self.x, self.source_width),
            y + scaled(height, self.y, self.source_height),
            scaled(width, self.width, self.source_width),
            scaled(height, self.height, self.source_height),
        )
    }
}

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
    /// Reusable vector-shader path for browser-synthesized bold. The PDF
    /// backend compiles each unique glyph into a shared Type 3 program instead
    /// of serializing the same expanded outline at every text occurrence.
    DrawSyntheticBoldGlyphRun {
        x: Pt,
        y: Pt,
        glyph_ids: Vec<u16>,
        advances: Vec<(Pt, Pt)>,
        offsets: Vec<(Pt, Pt)>,
        stroke_width: Pt,
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
        source_clip: Option<ImageSourceClip>,
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
        css_shadow: bool,
    },
    /// Applies reusable compiled mask forms to a reusable source form.  The
    /// raster and PDF backends execute the layer program without re-running
    /// layout, which keeps variable-data bindings on the compiled path.
    DrawMaskedForm {
        x: Pt,
        y: Pt,
        width: Pt,
        height: Pt,
        resource_id: String,
        layers: Vec<CompiledMaskLayer>,
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
    /// A tagged marked-content span whose accessible replacement is present
    /// only in the semantic tree. This is deliberately distinct from `alt`:
    /// `/ActualText` is the PDF contract for replacement text, while `/Alt`
    /// remains the alternate-description contract used by figures.
    BeginTagActualText {
        role: String,
        mcid: u32,
        actual_text: String,
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

/// Physical and logical geometry derived from the immutable per-page display
/// list. Layout remains in trim coordinates while PDF and raster consumers
/// apply the same presentation transform for bleed, marks, and orientation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct PageGeometry {
    pub(crate) logical_size: Size,
    pub(crate) media_size: Size,
    pub(crate) presentation: PagePresentation,
}

impl PageGeometry {
    pub(crate) fn for_page(page: &Page, fallback: Size) -> Self {
        let logical_size = page
            .commands
            .iter()
            .rev()
            .find_map(|command| match command {
                Command::Meta { key, value } if key == META_PAGE_SIZE_KEY => {
                    let (width, height) = value.split_once(',')?;
                    Some(Size {
                        width: Pt::from_milli_i64(width.parse().ok()?),
                        height: Pt::from_milli_i64(height.parse().ok()?),
                    })
                }
                _ => None,
            })
            .unwrap_or(fallback)
            .quantized();
        let presentation = page
            .commands
            .iter()
            .rev()
            .find_map(|command| match command {
                Command::Meta { key, value } if key == META_PAGE_PRESENTATION_KEY => {
                    PagePresentation::decode(value)
                }
                _ => None,
            })
            .unwrap_or_default();
        let extent = presentation.media_extent();
        let unrotated = Size {
            width: logical_size.width + extent + extent,
            height: logical_size.height + extent + extent,
        }
        .quantized();
        let media_size = match presentation.orientation {
            PageOrientation::Upright => unrotated,
            PageOrientation::RotateLeft | PageOrientation::RotateRight => Size {
                width: unrotated.height,
                height: unrotated.width,
            },
        }
        .quantized();
        Self {
            logical_size,
            media_size,
            presentation,
        }
    }
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
    projective_transform: Option<ProjectiveTransform>,
}

pub struct Canvas {
    initial_page_size: Size,
    page_size: Size,
    pages: Vec<Page>,
    current: Page,
    state_stack: Vec<GraphicsState>,
    current_state: GraphicsState,
    current_mcid: u32,
    // Nearest positioned-ancestor containing block stack for out-of-flow absolute placement.
    abs_containing_block_stack: Vec<Rect>,
    // Raster effects are compositor layers. A normal container can defer a
    // child's completed layer until its vector content has been emitted while
    // keeping the layer inside the container's clip/opacity/transform state.
    compositor_scopes: Vec<Vec<(i32, Vec<Command>)>>,
    fragmentainer_stack: Vec<Rect>,
    perspective_context_stack: Vec<PerspectiveContext>,
    // Descendants of an inline-axis overflow clip do not contribute to the
    // document scrollable width used by the final HTML page-fit pass. Keep
    // this as compiler state instead of emitting then attempting to recover
    // clip ancestry from the PDF command stream.
    html_scrollable_right_clip_depth: usize,
    html_scrollable_bottom_clip_depth: usize,
    html_scrollable_top_clip_depth: usize,
}

impl Canvas {
    pub fn new(page_size: Size) -> Self {
        Self {
            initial_page_size: page_size,
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
                projective_transform: None,
            },
            current_mcid: 0,
            abs_containing_block_stack: Vec::new(),
            compositor_scopes: Vec::new(),
            fragmentainer_stack: Vec::new(),
            perspective_context_stack: Vec::new(),
            html_scrollable_right_clip_depth: 0,
            html_scrollable_bottom_clip_depth: 0,
            html_scrollable_top_clip_depth: 0,
        }
    }

    pub fn page_size(&self) -> Size {
        self.page_size
    }

    /// Select the physical size for the current page. The metadata is ignored
    /// by layout/raster consumers and consumed by the PDF emitter when a
    /// document mixes default and named-page geometries.
    pub fn set_page_size(&mut self, page_size: Size) {
        self.page_size = page_size.quantized();
        self.current.commands.push(Command::Meta {
            key: META_PAGE_SIZE_KEY.to_string(),
            value: format!(
                "{},{}",
                self.page_size.width.to_milli_i64(),
                self.page_size.height.to_milli_i64()
            ),
        });
    }

    /// Attach physical sheet presentation to the current immutable page plan.
    /// Layout remains in trim-box coordinates; the PDF linker consumes this
    /// compact descriptor once when emitting the page stream and page boxes.
    pub(crate) fn set_page_presentation(&mut self, presentation: PagePresentation) {
        self.current.commands.push(Command::Meta {
            key: META_PAGE_PRESENTATION_KEY.to_string(),
            value: presentation.encode(),
        });
    }

    /// Reinitialize an uncommitted page after its named-page selector becomes
    /// known from the first in-flow box.
    pub fn restart_current_page(&mut self, page_size: Size) {
        self.current = Page::new();
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
            projective_transform: None,
        };
        self.current_mcid = 0;
        self.abs_containing_block_stack.clear();
        self.compositor_scopes.clear();
        self.fragmentainer_stack.clear();
        self.perspective_context_stack.clear();
        self.html_scrollable_right_clip_depth = 0;
        self.html_scrollable_bottom_clip_depth = 0;
        self.html_scrollable_top_clip_depth = 0;
        self.set_page_size(page_size);
    }

    pub(crate) fn push_fragmentainer(&mut self, rect: Rect) {
        self.fragmentainer_stack.push(rect);
    }

    pub(crate) fn pop_fragmentainer(&mut self) {
        self.fragmentainer_stack.pop();
    }

    pub(crate) fn current_fragmentainer(&self) -> Option<Rect> {
        self.fragmentainer_stack.last().copied()
    }

    pub(crate) fn push_perspective_context(&mut self, context: PerspectiveContext) {
        self.perspective_context_stack.push(context);
    }

    pub(crate) fn pop_perspective_context(&mut self) {
        self.perspective_context_stack.pop();
    }

    pub(crate) fn current_perspective_context(&self) -> Option<PerspectiveContext> {
        self.perspective_context_stack.last().copied()
    }

    pub(crate) fn apply_projective_transform(&mut self, transform: ProjectiveTransform) {
        self.current_state.projective_transform = Some(
            self.current_state
                .projective_transform
                .map(|current| current.multiply(transform))
                .unwrap_or(transform),
        );
    }

    fn project_point(&self, x: Pt, y: Pt) -> (Pt, Pt) {
        self.current_state
            .projective_transform
            .map(|transform| transform.map_point(x, y))
            .unwrap_or((x, y))
    }

    pub(crate) fn begin_compositor_scope(&mut self) {
        self.compositor_scopes.push(Vec::new());
    }

    pub(crate) fn defer_compositor_commands_since(&mut self, command_index: usize) {
        self.defer_compositor_commands_since_at_z(command_index, 0);
    }

    pub(crate) fn defer_compositor_commands_since_at_z(
        &mut self,
        command_index: usize,
        z_index: i32,
    ) {
        let Some(scope) = self.compositor_scopes.last_mut() else {
            return;
        };
        if command_index >= self.current.commands.len() {
            return;
        }
        scope.push((
            z_index,
            self.current.commands.drain(command_index..).collect(),
        ));
    }

    pub(crate) fn current_compositor_layer_count(&self) -> usize {
        self.compositor_scopes.last().map_or(0, Vec::len)
    }

    pub(crate) fn retag_compositor_layers_since(&mut self, layer_index: usize, z_index: i32) {
        let Some(scope) = self.compositor_scopes.last_mut() else {
            return;
        };
        for (layer_z, _) in scope.iter_mut().skip(layer_index) {
            *layer_z = z_index;
        }
    }

    pub(crate) fn end_compositor_scope(&mut self) {
        let Some(mut layers) = self.compositor_scopes.pop() else {
            return;
        };
        layers.sort_by_key(|(z_index, _)| *z_index);
        for (_, commands) in layers {
            self.current.commands.extend(commands);
        }
    }

    pub(crate) fn end_compositor_scope_to_parent(&mut self) {
        let Some(layers) = self.compositor_scopes.pop() else {
            return;
        };
        if let Some(parent) = self.compositor_scopes.last_mut() {
            parent.extend(layers);
            return;
        }
        let mut layers = layers;
        layers.sort_by_key(|(z_index, _)| *z_index);
        for (_, commands) in layers {
            self.current.commands.extend(commands);
        }
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
        if let Some(transform) = self.current_state.projective_transform.as_mut() {
            *transform = transform.multiply(ProjectiveTransform::translation(x, y, Pt::ZERO));
            return;
        }
        self.current.commands.push(Command::Translate(x, y));
    }

    pub fn translate_css_transform_origin(&mut self, x: Pt, y: Pt, inverse: bool) {
        if let Some(transform) = self.current_state.projective_transform.as_mut() {
            let direction = if inverse { -1 } else { 1 };
            *transform = transform.multiply(ProjectiveTransform::translation(
                x * direction,
                y * direction,
                Pt::ZERO,
            ));
            return;
        }
        self.current
            .commands
            .push(Command::CssTransformOrigin { x, y, inverse });
    }

    pub fn scale(&mut self, x: f32, y: f32) {
        if let Some(transform) = self.current_state.projective_transform.as_mut() {
            *transform = transform.multiply(ProjectiveTransform::scale(x, y, 1.0));
            return;
        }
        self.current.commands.push(Command::Scale(x, y));
    }

    pub fn rotate(&mut self, angle_radians: f32) {
        if let Some(transform) = self.current_state.projective_transform.as_mut() {
            *transform = transform.multiply(ProjectiveTransform::rotate_axis(
                0.0,
                0.0,
                1.0,
                angle_radians,
            ));
            return;
        }
        self.current.commands.push(Command::Rotate(angle_radians));
    }

    pub fn concat_matrix(&mut self, a: f32, b: f32, c: f32, d: f32, e: Pt, f: Pt) {
        if let Some(transform) = self.current_state.projective_transform.as_mut() {
            let affine = ProjectiveTransform {
                matrix: [
                    [a as f64, c as f64, 0.0, e.to_f32() as f64],
                    [b as f64, d as f64, 0.0, f.to_f32() as f64],
                    [0.0, 0.0, 1.0, 0.0],
                    [0.0, 0.0, 0.0, 1.0],
                ],
            };
            *transform = transform.multiply(affine);
            return;
        }
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
        if self.html_scrollable_right_clip_depth > 0 {
            return;
        }
        self.current.commands.push(Command::Meta {
            key: META_HTML_SCROLLABLE_RIGHT_KEY.to_string(),
            value: right.to_milli_i64().to_string(),
        });
    }

    pub(crate) fn record_html_page_area(&mut self, rect: Rect) {
        self.current.commands.push(Command::Meta {
            key: META_HTML_PAGE_AREA_KEY.to_string(),
            value: format!(
                "{},{},{},{}",
                rect.x.to_milli_i64(),
                rect.y.to_milli_i64(),
                rect.width.to_milli_i64(),
                rect.height.to_milli_i64()
            ),
        });
    }

    pub(crate) fn record_html_scrollable_top(&mut self, top: Pt) {
        if self.html_scrollable_top_clip_depth > 0 {
            return;
        }
        self.current.commands.push(Command::Meta {
            key: META_HTML_SCROLLABLE_TOP_KEY.to_string(),
            value: top.to_milli_i64().to_string(),
        });
    }

    pub(crate) fn record_html_scrollable_bottom(&mut self, bottom: Pt) {
        if self.html_scrollable_bottom_clip_depth > 0 {
            return;
        }
        self.current.commands.push(Command::Meta {
            key: META_HTML_SCROLLABLE_BOTTOM_KEY.to_string(),
            value: bottom.to_milli_i64().to_string(),
        });
    }

    pub(crate) fn push_html_scrollable_right_clip(&mut self) {
        self.html_scrollable_right_clip_depth =
            self.html_scrollable_right_clip_depth.saturating_add(1);
    }

    pub(crate) fn pop_html_scrollable_right_clip(&mut self) {
        debug_assert!(self.html_scrollable_right_clip_depth > 0);
        self.html_scrollable_right_clip_depth =
            self.html_scrollable_right_clip_depth.saturating_sub(1);
    }

    pub(crate) fn push_html_scrollable_bottom_clip(&mut self) {
        self.html_scrollable_bottom_clip_depth =
            self.html_scrollable_bottom_clip_depth.saturating_add(1);
    }

    pub(crate) fn pop_html_scrollable_bottom_clip(&mut self) {
        debug_assert!(self.html_scrollable_bottom_clip_depth > 0);
        self.html_scrollable_bottom_clip_depth =
            self.html_scrollable_bottom_clip_depth.saturating_sub(1);
    }

    pub(crate) fn push_html_scrollable_top_clip(&mut self) {
        self.html_scrollable_top_clip_depth = self.html_scrollable_top_clip_depth.saturating_add(1);
    }

    pub(crate) fn pop_html_scrollable_top_clip(&mut self) {
        debug_assert!(self.html_scrollable_top_clip_depth > 0);
        self.html_scrollable_top_clip_depth = self.html_scrollable_top_clip_depth.saturating_sub(1);
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

    pub(crate) fn set_stroke_color_to_fill(&mut self) {
        self.set_stroke_color(self.current_state.fill_color);
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
        if self.current_state.projective_transform.is_some() {
            let top_left = self.project_point(x, y);
            let top_right = self.project_point(x + width, y);
            let bottom_right = self.project_point(x + width, y + height);
            let bottom_left = self.project_point(x, y + height);
            self.current.commands.push(Command::MoveTo {
                x: top_left.0,
                y: top_left.1,
            });
            for (x, y) in [top_right, bottom_right, bottom_left] {
                self.current.commands.push(Command::LineTo { x, y });
            }
            self.current.commands.push(Command::ClosePath);
            self.current
                .commands
                .push(Command::ClipPath { evenodd: false });
            return;
        }
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
        let (x, y) = self.project_point(x, y);
        self.current.commands.push(Command::MoveTo { x, y });
    }

    pub fn line_to(&mut self, x: Pt, y: Pt) {
        let (x, y) = self.project_point(x, y);
        self.current.commands.push(Command::LineTo { x, y });
    }

    pub fn curve_to(&mut self, x1: Pt, y1: Pt, x2: Pt, y2: Pt, x: Pt, y: Pt) {
        let (x1, y1) = self.project_point(x1, y1);
        let (x2, y2) = self.project_point(x2, y2);
        let (x, y) = self.project_point(x, y);
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

    pub(crate) fn draw_synthetic_bold_glyph_run(
        &mut self,
        x: Pt,
        baseline_y_from_top: Pt,
        glyph_ids: Vec<u16>,
        advances: Vec<(Pt, Pt)>,
        offsets: Vec<(Pt, Pt)>,
        stroke_width: Pt,
    ) {
        self.current
            .commands
            .push(Command::DrawSyntheticBoldGlyphRun {
                x,
                y: baseline_y_from_top,
                glyph_ids,
                advances,
                offsets,
                stroke_width: stroke_width.max(Pt::ZERO),
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
        if self.current_state.projective_transform.is_some() {
            let top_left = self.project_point(x, y);
            let top_right = self.project_point(x + width, y);
            let bottom_right = self.project_point(x + width, y + height);
            let bottom_left = self.project_point(x, y + height);
            self.current.commands.push(Command::MoveTo {
                x: top_left.0,
                y: top_left.1,
            });
            for (x, y) in [top_right, bottom_right, bottom_left] {
                self.current.commands.push(Command::LineTo { x, y });
            }
            self.current.commands.push(Command::ClosePath);
            self.current.commands.push(Command::Fill);
            return;
        }
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
        self.draw_image_with_interpolation_and_source_clip(
            x,
            y,
            width,
            height,
            resource_id,
            interpolate,
            None,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub fn draw_image_with_interpolation_and_source_clip(
        &mut self,
        x: Pt,
        y: Pt,
        width: Pt,
        height: Pt,
        resource_id: impl Into<String>,
        interpolate: bool,
        source_clip: Option<ImageSourceClip>,
    ) {
        self.current.commands.push(Command::DrawImage {
            x,
            y,
            width,
            height,
            resource_id: resource_id.into(),
            interpolate,
            source_clip,
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
            css_shadow: false,
        });
    }

    pub fn draw_css_shadow_filtered_form(
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
            css_shadow: true,
        });
    }

    pub fn draw_masked_form(
        &mut self,
        x: Pt,
        y: Pt,
        width: Pt,
        height: Pt,
        resource_id: impl Into<String>,
        layers: Vec<CompiledMaskLayer>,
    ) {
        self.current.commands.push(Command::DrawMaskedForm {
            x,
            y,
            width,
            height,
            resource_id: resource_id.into(),
            layers,
        });
    }

    pub fn show_page(&mut self) {
        while !self.compositor_scopes.is_empty() {
            self.end_compositor_scope();
        }
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
            projective_transform: None,
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

    pub fn begin_tag_actual_text(
        &mut self,
        role: impl Into<String>,
        actual_text: impl Into<String>,
    ) -> u32 {
        let mcid = self.current_mcid;
        self.current_mcid = self.current_mcid.saturating_add(1);
        self.current.commands.push(Command::BeginTagActualText {
            role: role.into(),
            mcid,
            actual_text: actual_text.into(),
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
            page_size: self.initial_page_size,
            pages: self.pages,
        }
    }

    pub fn finish_without_show(self) -> Document {
        Document {
            page_size: self.initial_page_size,
            pages: self.pages,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inline_overflow_clip_excludes_descendants_from_html_scroll_width() {
        let mut canvas = Canvas::new(Size {
            width: Pt::from_f32(100.0),
            height: Pt::from_f32(100.0),
        });
        canvas.record_html_scrollable_right(Pt::from_f32(80.0));
        canvas.push_html_scrollable_right_clip();
        canvas.record_html_scrollable_right(Pt::from_f32(180.0));
        canvas.push_html_scrollable_right_clip();
        canvas.record_html_scrollable_right(Pt::from_f32(280.0));
        canvas.pop_html_scrollable_right_clip();
        canvas.pop_html_scrollable_right_clip();
        canvas.record_html_scrollable_right(Pt::from_f32(90.0));

        let document = canvas.finish();
        let rights: Vec<_> = document.pages[0]
            .commands
            .iter()
            .filter_map(|command| match command {
                Command::Meta { key, value } if key == META_HTML_SCROLLABLE_RIGHT_KEY => {
                    Some(value.as_str())
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            rights,
            vec![
                Pt::from_f32(80.0).to_milli_i64().to_string(),
                Pt::from_f32(90.0).to_milli_i64().to_string(),
            ]
        );
    }

    #[test]
    fn block_overflow_clip_excludes_descendants_from_html_scroll_extents() {
        let mut canvas = Canvas::new(Size {
            width: Pt::from_f32(100.0),
            height: Pt::from_f32(100.0),
        });
        canvas.record_html_scrollable_top(Pt::from_f32(-2.0));
        canvas.record_html_scrollable_bottom(Pt::from_f32(102.0));
        canvas.push_html_scrollable_top_clip();
        canvas.push_html_scrollable_bottom_clip();
        canvas.record_html_scrollable_top(Pt::from_f32(-20.0));
        canvas.record_html_scrollable_bottom(Pt::from_f32(120.0));
        canvas.pop_html_scrollable_bottom_clip();
        canvas.pop_html_scrollable_top_clip();

        let document = canvas.finish();
        let extents: Vec<_> = document.pages[0]
            .commands
            .iter()
            .filter_map(|command| match command {
                Command::Meta { key, value }
                    if key == META_HTML_SCROLLABLE_TOP_KEY
                        || key == META_HTML_SCROLLABLE_BOTTOM_KEY =>
                {
                    Some((key.clone(), value.clone()))
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            extents,
            vec![
                (
                    META_HTML_SCROLLABLE_TOP_KEY.to_string(),
                    Pt::from_f32(-2.0).to_milli_i64().to_string(),
                ),
                (
                    META_HTML_SCROLLABLE_BOTTOM_KEY.to_string(),
                    Pt::from_f32(102.0).to_milli_i64().to_string(),
                ),
            ]
        );
    }

    #[test]
    fn projective_rect_is_compiled_to_an_ordinary_vector_path() {
        let mut canvas = Canvas::new(Size {
            width: Pt::from_f32(100.0),
            height: Pt::from_f32(100.0),
        });
        let transform = ProjectiveTransform::perspective(Pt::from_f32(200.0)).multiply(
            ProjectiveTransform::translation(Pt::ZERO, Pt::ZERO, Pt::from_f32(50.0)),
        );
        canvas.apply_projective_transform(transform);
        canvas.draw_rect(
            Pt::from_f32(10.0),
            Pt::from_f32(20.0),
            Pt::from_f32(30.0),
            Pt::from_f32(40.0),
        );

        let document = canvas.finish();
        let commands = &document.pages[0].commands;
        assert!(
            !commands
                .iter()
                .any(|command| matches!(command, Command::DrawRect { .. }))
        );
        assert!(matches!(
            commands.as_slice(),
            [
                Command::MoveTo { .. },
                Command::LineTo { .. },
                Command::LineTo { .. },
                Command::LineTo { .. },
                Command::ClosePath,
                Command::Fill,
            ]
        ));
        let Command::MoveTo { x, y } = commands[0] else {
            unreachable!()
        };
        assert!((x.to_f32() - 13.333_333).abs() < 0.001);
        assert!((y.to_f32() - 26.666_666).abs() < 0.001);
    }

    #[test]
    fn compositor_scope_flushes_deferred_layer_after_vector_commands() {
        let mut canvas = Canvas::new(Size {
            width: Pt::from_f32(100.0),
            height: Pt::from_f32(100.0),
        });
        canvas.begin_compositor_scope();
        let layer_start = canvas.current_command_count();
        canvas.draw_filtered_form(
            Pt::ZERO,
            Pt::ZERO,
            Pt::from_f32(10.0),
            Pt::from_f32(10.0),
            "filter",
            PaintFilterSpec::identity(),
        );
        canvas.defer_compositor_commands_since(layer_start);
        canvas.draw_rect(
            Pt::from_f32(20.0),
            Pt::from_f32(20.0),
            Pt::from_f32(10.0),
            Pt::from_f32(10.0),
        );
        canvas.end_compositor_scope();
        let document = canvas.finish();
        assert!(matches!(
            document.pages[0].commands.as_slice(),
            [Command::DrawRect { .. }, Command::DrawFilteredForm { .. }]
        ));
    }

    #[test]
    fn nested_compositor_layers_rejoin_the_parent_in_z_index_order() {
        let mut canvas = Canvas::new(Size {
            width: Pt::from_f32(100.0),
            height: Pt::from_f32(100.0),
        });
        canvas.begin_compositor_scope();
        canvas.begin_compositor_scope();
        let high_start = canvas.current_command_count();
        canvas.draw_form(
            Pt::ZERO,
            Pt::ZERO,
            Pt::from_f32(10.0),
            Pt::from_f32(10.0),
            "high",
        );
        canvas.defer_compositor_commands_since_at_z(high_start, 100);
        canvas.end_compositor_scope_to_parent();

        let cap_start = canvas.current_command_count();
        canvas.draw_form(
            Pt::ZERO,
            Pt::ZERO,
            Pt::from_f32(10.0),
            Pt::from_f32(10.0),
            "cap",
        );
        canvas.defer_compositor_commands_since_at_z(cap_start, 1);
        canvas.end_compositor_scope();

        let document = canvas.finish();
        let ids = document.pages[0]
            .commands
            .iter()
            .filter_map(|command| match command {
                Command::DrawForm { resource_id, .. } => Some(resource_id.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["cap", "high"]);
    }

    #[test]
    fn source_clip_resolves_to_enclosing_pixels_and_preserves_image_lattice() {
        let clip = ImageSourceClip {
            left: Pt::from_f32(9.8),
            top: Pt::ZERO,
            right: Pt::from_f32(43.8),
            bottom: Pt::from_f32(24.0),
            snap_target_origin_to_css_pixel: false,
        };
        let crop = clip
            .resolve(Pt::from_f32(48.0), Pt::from_f32(24.0), 8, 4)
            .expect("one completely hidden source column");
        assert_eq!(
            crop,
            ResolvedImageSourceCrop {
                x: 1,
                y: 0,
                width: 7,
                height: 4,
                source_width: 8,
                source_height: 4,
            }
        );
        assert_eq!(
            crop.target_rect(
                Pt::from_f32(400.2),
                Pt::from_f32(20.0),
                Pt::from_f32(48.0),
                Pt::from_f32(24.0),
            ),
            (
                Pt::from_f32(406.2),
                Pt::from_f32(20.0),
                Pt::from_f32(42.0),
                Pt::from_f32(24.0),
            )
        );
    }

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
