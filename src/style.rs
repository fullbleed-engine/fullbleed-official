use crate::debug::{DebugLogger, json_escape};
use crate::flowable::CalcLength;
use crate::flowable::{
    BackgroundPaint, BorderCollapseMode, BorderRadiusSpec, BorderSpacingSpec, BoxShadowSpec,
    BreakAfter, BreakBefore, BreakInside, EdgeSizes, LengthSpec, Pagination, TextStyle,
};
use crate::types::{BoxSizingMode, Color, Margins, Pt, ShadingStop, Size};
use fixed::types::I32F32;
use lightningcss::media_query::{
    MediaCondition, MediaFeature, MediaFeatureComparison, MediaFeatureId, MediaFeatureName,
    MediaFeatureValue, MediaList, MediaQuery, MediaType, Operator, Qualifier,
};
use lightningcss::properties::align as css_align;
use lightningcss::properties::border::{BorderSideWidth, LineStyle};
use lightningcss::properties::custom::{CustomPropertyName, Token, TokenOrValue};
use lightningcss::properties::display::{Display, DisplayInside, DisplayKeyword, DisplayOutside};
use lightningcss::properties::flex as css_flex;
use lightningcss::properties::font::{
    AbsoluteFontSize, FontFamily, FontSize, FontStyle as CssFontStyle, GenericFontFamily,
    LineHeight, RelativeFontSize, VerticalAlign as CssVerticalAlign, VerticalAlignKeyword,
};
use lightningcss::properties::list::ListStyleType;
use lightningcss::properties::overflow as css_overflow;
use lightningcss::properties::position::Position as CssPosition;
use lightningcss::properties::size as css_size;
use lightningcss::properties::text::{
    OverflowWrap, TextAlign, TextDecorationLine, WhiteSpace, WordBreak,
};
use lightningcss::properties::{Property, PropertyId};
use lightningcss::rules::{CssRule, CssRuleList};
use lightningcss::stylesheet::{ParserOptions, PrinterOptions, StyleAttribute, StyleSheet};
use lightningcss::traits::ToCss;
use lightningcss::values::calc::{Calc, MathFunction};
use lightningcss::values::color::{CssColor, SRGB};
use lightningcss::values::length::{LengthPercentage, LengthValue};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Specificity(u16, u16, u16);

#[derive(Debug, Clone)]
struct SimpleSelector {
    tag: Option<String>,
    id: Option<String>,
    classes: Vec<String>,
    attrs: Vec<AttrSelector>,
    pseudos: Vec<PseudoClass>,
}

impl SimpleSelector {
    fn matches_with_pseudo(&self, element: &ElementInfo, pseudo: PseudoTarget) -> bool {
        if let Some(tag) = &self.tag {
            if tag != "*" && tag != &element.tag {
                return false;
            }
        }
        if let Some(id) = &self.id {
            if element.id.as_deref() != Some(id.as_str()) {
                return false;
            }
        }
        for class in &self.classes {
            if !element.classes.iter().any(|c| c == class) {
                return false;
            }
        }
        for attr in &self.attrs {
            if !attr.matches(element) {
                return false;
            }
        }
        for pseudo_class in &self.pseudos {
            if !pseudo_class.matches_with_pseudo(element, pseudo) {
                return false;
            }
        }
        true
    }

    fn specificity(&self) -> Specificity {
        let mut id_count = self.id.as_ref().map(|_| 1).unwrap_or(0);
        let mut class_count = (self.classes.len() + self.attrs.len()) as u16;
        let mut tag_count = self
            .tag
            .as_ref()
            .filter(|tag| tag.as_str() != "*")
            .map(|_| 1)
            .unwrap_or(0);

        for pseudo in &self.pseudos {
            match pseudo {
                PseudoClass::Not(inner) => {
                    let inner_spec = inner.specificity();
                    id_count += inner_spec.0;
                    class_count += inner_spec.1;
                    tag_count += inner_spec.2;
                }
                _ => {
                    class_count += 1;
                }
            }
        }

        Specificity(id_count, class_count, tag_count)
    }
}

#[derive(Debug, Clone)]
enum PseudoClass {
    Root,
    FirstChild,
    LastChild,
    NthChildEven,
    NthChildOdd,
    NthChild(usize),
    NthChildFormula { a: i32, b: i32 },
    Hover,
    Before,
    After,
    Unsupported,
    Not(SimpleSelector),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PseudoTarget {
    None,
    Before,
    After,
}

impl PseudoClass {
    fn is_positional(&self) -> bool {
        matches!(
            self,
            PseudoClass::FirstChild
                | PseudoClass::LastChild
                | PseudoClass::NthChildEven
                | PseudoClass::NthChildOdd
                | PseudoClass::NthChild(_)
                | PseudoClass::NthChildFormula { .. }
        )
    }

    fn matches_with_pseudo(&self, element: &ElementInfo, pseudo: PseudoTarget) -> bool {
        match self {
            PseudoClass::Root => element.is_root,
            PseudoClass::FirstChild => element.child_index == 1,
            PseudoClass::LastChild => element.child_index == element.child_count,
            PseudoClass::NthChildEven => element.child_index % 2 == 0,
            PseudoClass::NthChildOdd => element.child_index % 2 == 1,
            PseudoClass::NthChild(n) => element.child_index == *n,
            PseudoClass::NthChildFormula { a, b } => {
                let idx = element.child_index as i32;
                if *a == 0 {
                    return idx == *b;
                }
                if *a > 0 {
                    if idx < *b {
                        return false;
                    }
                    (idx - *b) % *a == 0
                } else {
                    if idx > *b {
                        return false;
                    }
                    (b - idx) % (-*a) == 0
                }
            }
            PseudoClass::Hover => false,
            PseudoClass::Before => matches!(pseudo, PseudoTarget::Before),
            PseudoClass::After => matches!(pseudo, PseudoTarget::After),
            PseudoClass::Unsupported => false,
            PseudoClass::Not(selector) => !selector.matches_with_pseudo(element, pseudo),
        }
    }
}

#[derive(Debug, Clone)]
struct AttrSelector {
    name: String,
    op: AttrOp,
    value: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AttrOp {
    Exists,
    Equals,
    Includes,
    DashMatch,
    Prefix,
    Suffix,
    Substring,
}

impl AttrSelector {
    fn matches(&self, element: &ElementInfo) -> bool {
        let Some(value) = element.attrs.get(&self.name) else {
            return false;
        };
        match self.op {
            AttrOp::Exists => true,
            AttrOp::Equals => self.value.as_deref().map(|v| value == v).unwrap_or(false),
            AttrOp::Includes => self
                .value
                .as_deref()
                .map(|v| value.split_whitespace().any(|part| part == v))
                .unwrap_or(false),
            AttrOp::DashMatch => self
                .value
                .as_deref()
                .map(|v| value == v || value.starts_with(&format!("{v}-")))
                .unwrap_or(false),
            AttrOp::Prefix => self
                .value
                .as_deref()
                .map(|v| value.starts_with(v))
                .unwrap_or(false),
            AttrOp::Suffix => self
                .value
                .as_deref()
                .map(|v| value.ends_with(v))
                .unwrap_or(false),
            AttrOp::Substring => self
                .value
                .as_deref()
                .map(|v| value.contains(v))
                .unwrap_or(false),
        }
    }
}

#[derive(Debug, Clone)]
enum Combinator {
    Descendant,
    Child,
    AdjacentSibling,
    GeneralSibling,
}

#[derive(Debug, Clone)]
struct SelectorPattern {
    parts: Vec<SimpleSelector>,
    combinators: Vec<Combinator>,
}

impl SelectorPattern {
    fn has_positional_pseudos(&self) -> bool {
        self.parts.iter().any(|part| {
            part.pseudos.iter().any(|pseudo| match pseudo {
                PseudoClass::Not(inner) => inner.pseudos.iter().any(|p| p.is_positional()),
                _ => pseudo.is_positional(),
            })
        })
    }

    fn has_sibling_combinators(&self) -> bool {
        self.combinators.iter().any(|comb| {
            matches!(
                comb,
                Combinator::AdjacentSibling | Combinator::GeneralSibling
            )
        })
    }

    fn pseudo_target(&self) -> Option<PseudoTarget> {
        for part in &self.parts {
            for pseudo in &part.pseudos {
                match pseudo {
                    PseudoClass::Before => return Some(PseudoTarget::Before),
                    PseudoClass::After => return Some(PseudoTarget::After),
                    _ => {}
                }
            }
        }
        None
    }

    fn matches(&self, element: &ElementInfo, ancestors: &[ElementInfo]) -> bool {
        self.matches_with_pseudo(element, ancestors, PseudoTarget::None)
    }

    fn matches_with_pseudo(
        &self,
        element: &ElementInfo,
        ancestors: &[ElementInfo],
        pseudo: PseudoTarget,
    ) -> bool {
        if self.parts.is_empty() {
            return false;
        }
        let last = self.parts.last().expect("non-empty");
        if !last.matches_with_pseudo(element, pseudo) {
            return false;
        }

        if self.parts.len() == 1 {
            return true;
        }

        let mut current: &ElementInfo = element;
        let mut current_index = ancestors.len();
        for (idx, part) in self.parts[..self.parts.len() - 1].iter().rev().enumerate() {
            let comb_index = self.combinators.len().saturating_sub(1 + idx);
            let combinator = self
                .combinators
                .get(comb_index)
                .unwrap_or(&Combinator::Descendant);
            match combinator {
                Combinator::Child => {
                    if current_index == 0 {
                        return false;
                    }
                    current_index -= 1;
                    current = &ancestors[current_index];
                    if !part.matches_with_pseudo(current, pseudo) {
                        return false;
                    }
                }
                Combinator::Descendant => {
                    let mut found = false;
                    while current_index > 0 {
                        current_index -= 1;
                        let ancestor = &ancestors[current_index];
                        if part.matches_with_pseudo(ancestor, pseudo) {
                            current = ancestor;
                            found = true;
                            break;
                        }
                    }
                    if !found {
                        return false;
                    }
                }
                Combinator::AdjacentSibling => {
                    let Some(prev) = current.prev_siblings.last() else {
                        return false;
                    };
                    current = prev;
                    if !part.matches_with_pseudo(current, pseudo) {
                        return false;
                    }
                }
                Combinator::GeneralSibling => {
                    let mut found = false;
                    for prev in current.prev_siblings.iter().rev() {
                        if part.matches_with_pseudo(prev, pseudo) {
                            current = prev;
                            found = true;
                            break;
                        }
                    }
                    if !found {
                        return false;
                    }
                }
            }
        }
        true
    }

    fn specificity(&self) -> Specificity {
        let mut spec = Specificity(0, 0, 0);
        for part in &self.parts {
            let part_spec = part.specificity();
            spec.0 += part_spec.0;
            spec.1 += part_spec.1;
            spec.2 += part_spec.2;
        }
        spec
    }
}

#[derive(Debug, Clone)]
struct RuleEntry {
    selector: SelectorPattern,
    specificity: Specificity,
    order: usize,
    delta: StyleDelta,
    selector_text: String,
}

#[derive(Debug, Clone, Copy)]
struct FontCalcLength {
    abs: Pt,
    em: I32F32,
    rem: I32F32,
    vw: I32F32,
    vh: I32F32,
    vmin: I32F32,
    vmax: I32F32,
}

impl FontCalcLength {
    fn zero() -> Self {
        Self {
            abs: Pt::ZERO,
            em: I32F32::from_bits(0),
            rem: I32F32::from_bits(0),
            vw: I32F32::from_bits(0),
            vh: I32F32::from_bits(0),
            vmin: I32F32::from_bits(0),
            vmax: I32F32::from_bits(0),
        }
    }

    fn add(self, other: Self) -> Self {
        Self {
            abs: self.abs + other.abs,
            em: self.em + other.em,
            rem: self.rem + other.rem,
            vw: self.vw + other.vw,
            vh: self.vh + other.vh,
            vmin: self.vmin + other.vmin,
            vmax: self.vmax + other.vmax,
        }
    }

    fn scale(self, factor: I32F32) -> Self {
        Self {
            abs: self.abs.mul_fixed(factor),
            em: self.em * factor,
            rem: self.rem * factor,
            vw: self.vw * factor,
            vh: self.vh * factor,
            vmin: self.vmin * factor,
            vmax: self.vmax * factor,
        }
    }

    fn resolve(self, parent_font_size: Pt, root_font_size: Pt, viewport: Size) -> Pt {
        let vw = viewport.width.mul_fixed(self.vw);
        let vh = viewport.height.mul_fixed(self.vh);
        let vmin = viewport.width.min(viewport.height).mul_fixed(self.vmin);
        let vmax = viewport.width.max(viewport.height).mul_fixed(self.vmax);
        self.abs
            + parent_font_size.mul_fixed(self.em)
            + root_font_size.mul_fixed(self.rem)
            + vw
            + vh
            + vmin
            + vmax
    }
}

#[derive(Debug, Clone)]
enum FontSizeSpec {
    AbsolutePt(Pt),
    RelativeScale(I32F32),
    Calc(FontCalcLength),
    Inherit,
    Initial,
}

#[derive(Debug, Clone)]
enum LineHeightSpec {
    Normal,
    Number(f32),
    AbsolutePt(Pt),
    Inherit,
    Initial,
}

impl LineHeightSpec {
    fn to_line_height(&self, font_size: Pt) -> Pt {
        let value = match self {
            LineHeightSpec::Normal => font_size.mul_ratio(6, 5),
            LineHeightSpec::Number(scale) => font_size * *scale,
            LineHeightSpec::AbsolutePt(value) => *value,
            LineHeightSpec::Inherit | LineHeightSpec::Initial => font_size.mul_ratio(6, 5),
        };
        value
    }
}

#[derive(Debug, Clone)]
enum ColorSpec {
    Value(Color),
    Inherit,
    Initial,
    CurrentColor,
}

#[derive(Debug, Clone)]
enum BackgroundSpec {
    Value(Color),
    Inherit,
    Initial,
    CurrentColor,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum FontSpec {
    Value(Vec<Arc<str>>),
    Inherit,
    Initial,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WhiteSpaceMode {
    Normal,
    Pre,
    NoWrap,
    PreWrap,
    BreakSpaces,
    PreLine,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextAlignMode {
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerticalAlignMode {
    Top,
    Middle,
    Bottom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontStyleMode {
    Normal,
    Italic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextTransformMode {
    None,
    Uppercase,
    Lowercase,
    Capitalize,
}

#[derive(Debug, Clone)]
enum ContentSpec {
    None,
    Text(String),
    Inherit,
    Initial,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TextDecorationMode {
    pub underline: bool,
    pub overline: bool,
    pub line_through: bool,
}

impl TextDecorationMode {
    pub fn is_none(&self) -> bool {
        !self.underline && !self.overline && !self.line_through
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WordBreakMode {
    Normal,
    BreakWord,
    BreakAll,
    KeepAll,
    Anywhere,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayMode {
    Block,
    Inline,
    InlineBlock,
    Table,
    InlineTable,
    TableRowGroup,
    TableHeaderGroup,
    TableFooterGroup,
    TableRow,
    TableCell,
    TableCaption,
    Flex,
    InlineFlex,
    Grid,
    InlineGrid,
    None,
    Contents,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PositionMode {
    Static,
    Relative,
    Absolute,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlexDirectionMode {
    Row,
    Column,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlexWrapMode {
    NoWrap,
    Wrap,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JustifyContentMode {
    FlexStart,
    FlexEnd,
    Center,
    SpaceBetween,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlignItemsMode {
    FlexStart,
    FlexEnd,
    Center,
    Stretch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WhiteSpaceSpec {
    Value(WhiteSpaceMode),
    Inherit,
    Initial,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TextDecorationSpec {
    Value(TextDecorationMode),
    Inherit,
    Initial,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DisplaySpec {
    Value(DisplayMode),
    Inherit,
    Initial,
}

#[derive(Debug, Clone, Default)]
struct PaginationDelta {
    break_before: Option<BreakBefore>,
    break_after: Option<BreakAfter>,
    break_inside: Option<BreakInside>,
    orphans: Option<usize>,
    widows: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverflowMode {
    Visible,
    Hidden,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptionSideMode {
    Top,
    Bottom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextOverflowMode {
    Clip,
    Ellipsis,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TextOverflowSpec {
    Value(TextOverflowMode),
    Inherit,
    Initial,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListStyleTypeMode {
    Auto,
    None,
}

#[derive(Debug, Clone, Default)]
struct StyleDelta {
    font_size: Option<FontSizeSpec>,
    line_height: Option<LineHeightSpec>,
    color: Option<ColorSpec>,
    background_color: Option<BackgroundSpec>,
    color_var: Option<String>,
    background_color_var: Option<String>,
    border_color_var: Option<String>,
    width: Option<LengthSpec>,
    height: Option<LengthSpec>,
    width_var: Option<String>,
    height_var: Option<String>,
    max_width: Option<LengthSpec>,
    min_width: Option<LengthSpec>,
    min_height: Option<LengthSpec>,
    max_height: Option<LengthSpec>,
    max_width_var: Option<String>,
    text_align: Option<TextAlignMode>,
    vertical_align: Option<VerticalAlignMode>,
    font_weight: Option<u16>,
    font_style: Option<FontStyleMode>,
    text_transform: Option<TextTransformMode>,
    text_decoration: Option<TextDecorationSpec>,
    text_overflow: Option<TextOverflowSpec>,
    content: Option<ContentSpec>,
    word_break: Option<WordBreakMode>,
    list_style_type: Option<ListStyleTypeMode>,
    letter_spacing: Option<LengthSpec>,
    border_width: EdgeDelta,
    border_color: Option<ColorSpec>,
    border_style: BorderStyleDelta,
    border_collapse: Option<BorderCollapseMode>,
    caption_side: Option<CaptionSideMode>,
    border_spacing: Option<BorderSpacingSpec>,
    border_radius: Option<BorderRadiusSpec>,
    font_name: Option<FontSpec>,
    font_name_var: Option<String>,
    box_shadow: Option<BoxShadowSpec>,
    background_paint: Option<BackgroundPaint>,
    pagination: PaginationDelta,
    margin: EdgeDelta,
    padding: EdgeDelta,
    white_space: Option<WhiteSpaceSpec>,
    display: Option<DisplaySpec>,
    position: Option<PositionMode>,
    z_index: Option<i32>,
    inset_left: Option<LengthSpec>,
    inset_top: Option<LengthSpec>,
    inset_right: Option<LengthSpec>,
    inset_bottom: Option<LengthSpec>,
    box_sizing: Option<BoxSizingMode>,
    flex_direction: Option<FlexDirectionMode>,
    flex_wrap: Option<FlexWrapMode>,
    flex_basis: Option<LengthSpec>,
    order: Option<i32>,
    justify_content: Option<JustifyContentMode>,
    align_items: Option<AlignItemsMode>,
    grid_columns: Option<usize>,
    gap: Option<LengthSpec>,
    flex_grow: Option<f32>,
    flex_shrink: Option<f32>,
    overflow: Option<OverflowMode>,
    custom_lengths: HashMap<String, LengthSpec>,
    custom_colors: HashMap<String, Color>,
    custom_color_alpha: HashMap<String, f32>,
    custom_color_refs: HashMap<String, String>,
    custom_font_stacks: HashMap<String, Vec<Arc<str>>>,
}

#[derive(Debug, Clone)]
pub struct ComputedStyle {
    pub font_size: Pt,
    line_height: LineHeightSpec,
    pub color: Color,
    pub background_color: Option<Color>,
    pub background_paint: Option<BackgroundPaint>,
    pending_color_var: Option<String>,
    pending_background_color_var: Option<String>,
    pending_border_color_var: Option<String>,
    pending_font_name_var: Option<String>,
    pub pagination: Pagination,
    pub margin: EdgeSizes,
    pub padding: EdgeSizes,
    pub width: LengthSpec,
    pub height: LengthSpec,
    pending_width_var: Option<String>,
    pending_height_var: Option<String>,
    pub max_width: LengthSpec,
    pub min_width: LengthSpec,
    pub min_height: LengthSpec,
    pub max_height: LengthSpec,
    pending_max_width_var: Option<String>,
    pub text_align: TextAlignMode,
    pub vertical_align: VerticalAlignMode,
    pub font_weight: u16,
    pub font_style: FontStyleMode,
    pub text_transform: TextTransformMode,
    pub text_decoration: TextDecorationMode,
    pub text_overflow: TextOverflowMode,
    pub content: Option<String>,
    pub word_break: WordBreakMode,
    pub list_style_type: ListStyleTypeMode,
    pub letter_spacing: Pt,
    pub border_width: EdgeSizes,
    pub border_color: Option<Color>,
    border_style: BorderStyleState,
    pub border_collapse: BorderCollapseMode,
    pub caption_side: CaptionSideMode,
    pub border_spacing: BorderSpacingSpec,
    pub border_radius: BorderRadiusSpec,
    pub box_shadow: Option<BoxShadowSpec>,
    pub root_font_size: Pt,
    pub white_space: WhiteSpaceMode,
    pub display: DisplayMode,
    pub font_stack: Vec<Arc<str>>,
    pub font_name: Arc<str>,
    pub position: PositionMode,
    pub z_index: i32,
    pub inset_left: LengthSpec,
    pub inset_top: LengthSpec,
    pub inset_right: LengthSpec,
    pub inset_bottom: LengthSpec,
    pub box_sizing: BoxSizingMode,
    pub flex_direction: FlexDirectionMode,
    pub flex_wrap: FlexWrapMode,
    pub flex_basis: LengthSpec,
    pub order: i32,
    pub justify_content: JustifyContentMode,
    pub align_items: AlignItemsMode,
    pub grid_columns: Option<usize>,
    pub gap: LengthSpec,
    pub flex_grow: f32,
    pub flex_shrink: f32,
    pub overflow: OverflowMode,
    pub custom_lengths: HashMap<String, LengthSpec>,
    pub custom_colors: HashMap<String, Color>,
    pub custom_color_alpha: HashMap<String, f32>,
    pub custom_color_refs: HashMap<String, String>,
    pub custom_font_stacks: HashMap<String, Vec<Arc<str>>>,
}

impl ComputedStyle {
    pub fn to_text_style(&self) -> TextStyle {
        let (line_height, is_auto) = match self.line_height {
            LineHeightSpec::Normal => (self.font_size.mul_ratio(6, 5), true),
            LineHeightSpec::Number(scale) => (self.font_size * scale, false),
            LineHeightSpec::AbsolutePt(value) => (value, false),
            LineHeightSpec::Inherit | LineHeightSpec::Initial => {
                (self.font_size.mul_ratio(6, 5), true)
            }
        };
        let mut fallbacks = self.font_stack.clone();
        let primary = if fallbacks.is_empty() {
            Arc::<str>::from("Helvetica")
        } else {
            fallbacks.remove(0)
        };
        TextStyle {
            font_size: self.font_size,
            line_height,
            line_height_is_auto: is_auto,
            color: self.color,
            font_name: primary,
            font_fallbacks: fallbacks,
            font_weight: self.font_weight,
            font_style: self.font_style,
            text_decoration: self.text_decoration,
            text_overflow: self.text_overflow,
            word_break: self.word_break,
            letter_spacing: self.letter_spacing,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BorderLineStyle {
    None,
    Visible,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BorderStyleState {
    top: BorderLineStyle,
    right: BorderLineStyle,
    bottom: BorderLineStyle,
    left: BorderLineStyle,
}

impl BorderStyleState {
    fn none() -> Self {
        Self {
            top: BorderLineStyle::None,
            right: BorderLineStyle::None,
            bottom: BorderLineStyle::None,
            left: BorderLineStyle::None,
        }
    }
}

#[derive(Debug, Clone, Default)]
struct BorderStyleDelta {
    top: Option<BorderLineStyle>,
    right: Option<BorderLineStyle>,
    bottom: Option<BorderLineStyle>,
    left: Option<BorderLineStyle>,
}

#[derive(Debug, Clone, Default)]
struct EdgeDelta {
    top: Option<LengthSpec>,
    right: Option<LengthSpec>,
    bottom: Option<LengthSpec>,
    left: Option<LengthSpec>,
    top_var: Option<LengthVarExpr>,
    right_var: Option<LengthVarExpr>,
    bottom_var: Option<LengthVarExpr>,
    left_var: Option<LengthVarExpr>,
}

#[derive(Debug, Clone)]
struct LengthVarExpr {
    name: String,
    scale: f32,
}

#[derive(Debug, Clone)]
pub struct ElementInfo {
    pub tag: String,
    pub id: Option<String>,
    pub classes: Vec<String>,
    pub attrs: HashMap<String, String>,
    pub is_root: bool,
    pub child_index: usize,
    pub child_count: usize,
    pub prev_siblings: Vec<ElementInfo>,
}

pub struct StyleResolver {
    normal_rules: Vec<RuleEntry>,
    important_rules: Vec<RuleEntry>,
    normal_index: RuleIndex,
    important_index: RuleIndex,
    root_font_size: Pt,
    viewport: Size,
    debug: Option<Arc<DebugLogger>>,
    root_normal: Vec<StyleDelta>,
    root_important: Vec<StyleDelta>,
    has_positional_selectors: bool,
    has_sibling_selectors: bool,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct CssPageSetup {
    pub size: Option<Size>,
    pub margin_top: Option<Pt>,
    pub margin_right: Option<Pt>,
    pub margin_bottom: Option<Pt>,
    pub margin_left: Option<Pt>,
}

impl CssPageSetup {
    pub fn has_margin_override(&self) -> bool {
        self.margin_top.is_some()
            || self.margin_right.is_some()
            || self.margin_bottom.is_some()
            || self.margin_left.is_some()
    }

    pub fn resolve_margins(self, base: Margins) -> Option<Margins> {
        if !self.has_margin_override() {
            return None;
        }
        Some(Margins {
            top: self.margin_top.unwrap_or(base.top),
            right: self.margin_right.unwrap_or(base.right),
            bottom: self.margin_bottom.unwrap_or(base.bottom),
            left: self.margin_left.unwrap_or(base.left),
        })
    }
}

#[derive(Default)]
struct RuleIndex {
    by_tag: HashMap<String, Vec<usize>>,
    by_id: HashMap<String, Vec<usize>>,
    by_class: HashMap<String, Vec<usize>>,
    universal: Vec<usize>,
}

impl RuleIndex {
    fn new(rules: &[RuleEntry]) -> Self {
        let mut index = Self::default();

        for (i, rule) in rules.iter().enumerate() {
            let Some(last) = rule.selector.parts.last() else {
                continue;
            };
            let mut indexed = false;

            if let Some(id) = &last.id {
                index.by_id.entry(id.clone()).or_default().push(i);
                indexed = true;
            }
            for class in &last.classes {
                index.by_class.entry(class.clone()).or_default().push(i);
                indexed = true;
            }
            if let Some(tag) = &last.tag {
                if tag != "*" {
                    index.by_tag.entry(tag.clone()).or_default().push(i);
                    indexed = true;
                }
            }

            if !indexed {
                index.universal.push(i);
            }
        }

        index
    }

    fn candidate_indices(&self, element: &ElementInfo) -> Vec<usize> {
        let mut out: Vec<usize> = Vec::new();
        if let Some(id) = &element.id {
            if let Some(v) = self.by_id.get(id) {
                out.extend(v);
            }
        }
        for class in &element.classes {
            if let Some(v) = self.by_class.get(class) {
                out.extend(v);
            }
        }
        if let Some(v) = self.by_tag.get(&element.tag) {
            out.extend(v);
        }
        out.extend(&self.universal);
        out.sort_unstable();
        out.dedup();
        out
    }
}

impl StyleResolver {
    #[cfg(test)]
    pub fn new(css: &str) -> Self {
        Self::new_with_debug(css, None)
    }

    #[allow(dead_code)]
    pub fn new_with_debug(css: &str, debug: Option<Arc<DebugLogger>>) -> Self {
        Self::new_with_debug_and_viewport(css, debug, None)
    }

    pub fn new_with_debug_and_viewport(
        css: &str,
        debug: Option<Arc<DebugLogger>>,
        viewport: Option<Size>,
    ) -> Self {
        let viewport = viewport.unwrap_or(Size {
            width: Pt::ZERO,
            height: Pt::ZERO,
        });
        let mut normal_rules = Vec::new();
        let mut important_rules = Vec::new();
        let mut order = 0usize;
        let mut root_normal = Vec::new();
        let mut root_important = Vec::new();
        let mut has_positional_selectors = false;
        let mut has_sibling_selectors = false;

        fn append_rule_list(
            rules: CssRuleList,
            normal_rules: &mut Vec<RuleEntry>,
            important_rules: &mut Vec<RuleEntry>,
            order: &mut usize,
            root_normal: &mut Vec<StyleDelta>,
            root_important: &mut Vec<StyleDelta>,
            debug: Option<&DebugLogger>,
            has_positional_selectors: &mut bool,
            has_sibling_selectors: &mut bool,
            viewport: Size,
            prefer_print: bool,
        ) {
            for rule in rules.0 {
                match rule {
                    CssRule::Style(style) => {
                        let (normal_delta, important_delta) =
                            style_from_declarations(&style.declarations);
                        let selectors = style
                            .selectors
                            .to_css_string(PrinterOptions::default())
                            .unwrap_or_default();
                        if let Some(logger) = debug {
                            log_declaration_no_effects(&style.declarations, &selectors, logger);
                            for selector in selectors.split(',') {
                                let selector_trimmed = selector.trim().to_string();
                                let parsed = parse_selector_pattern(&selector_trimmed).is_some();
                                let json = format!(
                                    "{{\"type\":\"css.rule\",\"selector\":{},\"parsed\":{}}}",
                                    json_string(&selector_trimmed),
                                    if parsed { "true" } else { "false" }
                                );
                                logger.log_json(&json);
                                if !parsed {
                                    logger.increment("css.selector_unparsed", 1);
                                }
                            }
                            for (name, token_debug) in
                                collect_custom_properties(&style.declarations)
                            {
                                let json = format!(
                                    "{{\"type\":\"css.custom\",\"name\":{},\"value\":{},\"tokens\":{}}}",
                                    json_string(&name),
                                    json_string(&token_debug),
                                    json_string(&token_debug)
                                );
                                logger.log_json(&json);
                            }
                        }
                        for selector in selectors.split(',') {
                            let selector_trimmed = selector.trim();
                            let is_root_selector =
                                selector_trimmed.to_ascii_lowercase().contains(":root");
                            if let Some(pattern) = parse_selector_pattern(selector) {
                                if pattern.has_positional_pseudos() {
                                    *has_positional_selectors = true;
                                }
                                if pattern.has_sibling_combinators() {
                                    *has_sibling_selectors = true;
                                }
                                if !normal_delta.is_empty() {
                                    normal_rules.push(RuleEntry {
                                        specificity: pattern.specificity(),
                                        selector: pattern.clone(),
                                        order: *order,
                                        delta: normal_delta.clone(),
                                        selector_text: selector.trim().to_string(),
                                    });
                                }
                                if !important_delta.is_empty() {
                                    important_rules.push(RuleEntry {
                                        specificity: pattern.specificity(),
                                        selector: pattern.clone(),
                                        order: *order,
                                        delta: important_delta.clone(),
                                        selector_text: selector.trim().to_string(),
                                    });
                                }
                            } else if is_root_selector {
                                if !normal_delta.is_empty() {
                                    root_normal.push(normal_delta.clone());
                                }
                                if !important_delta.is_empty() {
                                    root_important.push(important_delta.clone());
                                }
                            }
                        }
                        *order += 1;
                    }
                    CssRule::Media(media) => {
                        let matched =
                            media_list_matches(&media.query, viewport, prefer_print, debug);
                        if let Some(logger) = debug {
                            logger.increment("css.media.rules", 1);
                            if matched {
                                logger.increment("css.media.rules_matched", 1);
                            } else {
                                logger.increment("css.media.rules_skipped", 1);
                            }
                        }
                        if matched {
                            append_rule_list(
                                media.rules,
                                normal_rules,
                                important_rules,
                                order,
                                root_normal,
                                root_important,
                                debug,
                                has_positional_selectors,
                                has_sibling_selectors,
                                viewport,
                                prefer_print,
                            );
                        }
                    }
                    _ => {}
                }
            }
        }

        let ua_sheet = StyleSheet::parse(default_ua_css(), ParserOptions::default()).ok();
        let user_sheet = if css.trim().is_empty() {
            None
        } else {
            StyleSheet::parse(css, ParserOptions::default()).ok()
        };
        let prefer_print = ua_sheet
            .as_ref()
            .map(|sheet| stylesheet_has_print_media(&sheet.rules))
            .unwrap_or(false)
            || user_sheet
                .as_ref()
                .map(|sheet| stylesheet_has_print_media(&sheet.rules))
                .unwrap_or(false);

        if let Some(sheet) = ua_sheet {
            append_rule_list(
                sheet.rules,
                &mut normal_rules,
                &mut important_rules,
                &mut order,
                &mut root_normal,
                &mut root_important,
                debug.as_deref(),
                &mut has_positional_selectors,
                &mut has_sibling_selectors,
                viewport,
                prefer_print,
            );
        }

        if let Some(sheet) = user_sheet {
            append_rule_list(
                sheet.rules,
                &mut normal_rules,
                &mut important_rules,
                &mut order,
                &mut root_normal,
                &mut root_important,
                debug.as_deref(),
                &mut has_positional_selectors,
                &mut has_sibling_selectors,
                viewport,
                prefer_print,
            );
        }

        let normal_index = RuleIndex::new(&normal_rules);
        let important_index = RuleIndex::new(&important_rules);

        if let Some(logger) = debug.as_deref() {
            let json = format!(
                "{{\"type\":\"css.flags\",\"has_positional_selectors\":{},\"has_sibling_selectors\":{}}}",
                if has_positional_selectors {
                    "true"
                } else {
                    "false"
                },
                if has_sibling_selectors {
                    "true"
                } else {
                    "false"
                }
            );
            logger.log_json(&json);
        }

        if let Some(logger) = debug.as_deref() {
            let json = format!(
                "{{\"type\":\"css.media.prefer_print\",\"enabled\":{}}}",
                if prefer_print { "true" } else { "false" }
            );
            logger.log_json(&json);
        }

        Self {
            normal_rules,
            important_rules,
            normal_index,
            important_index,
            root_font_size: TextStyle::default().font_size,
            viewport,
            debug,
            root_normal,
            root_important,
            has_positional_selectors,
            has_sibling_selectors,
        }
    }

    pub fn has_positional_selectors(&self) -> bool {
        self.has_positional_selectors
    }

    pub fn has_sibling_selectors(&self) -> bool {
        self.has_sibling_selectors
    }

    pub fn debug_logger(&self) -> Option<Arc<DebugLogger>> {
        self.debug.clone()
    }

    pub fn default_style(&self) -> ComputedStyle {
        ComputedStyle {
            font_size: self.root_font_size,
            line_height: LineHeightSpec::Normal,
            color: Color::BLACK,
            background_color: None,
            background_paint: None,
            pending_color_var: None,
            pending_background_color_var: None,
            pending_border_color_var: None,
            pending_font_name_var: None,
            pagination: Pagination::default(),
            margin: EdgeSizes::zero(),
            padding: EdgeSizes::zero(),
            width: LengthSpec::Auto,
            height: LengthSpec::Auto,
            pending_width_var: None,
            pending_height_var: None,
            max_width: LengthSpec::Auto,
            min_width: LengthSpec::Auto,
            min_height: LengthSpec::Auto,
            max_height: LengthSpec::Auto,
            pending_max_width_var: None,
            text_align: TextAlignMode::Left,
            vertical_align: VerticalAlignMode::Top,
            font_weight: 400,
            font_style: FontStyleMode::Normal,
            text_transform: TextTransformMode::None,
            text_decoration: TextDecorationMode::default(),
            text_overflow: TextOverflowMode::Clip,
            content: None,
            word_break: WordBreakMode::Normal,
            list_style_type: ListStyleTypeMode::Auto,
            letter_spacing: Pt::ZERO,
            border_width: EdgeSizes::zero(),
            border_color: None,
            border_style: BorderStyleState::none(),
            border_collapse: BorderCollapseMode::Separate,
            caption_side: CaptionSideMode::Top,
            border_spacing: BorderSpacingSpec::zero(),
            border_radius: BorderRadiusSpec::zero(),
            box_shadow: None,
            root_font_size: self.root_font_size,
            white_space: WhiteSpaceMode::Normal,
            display: DisplayMode::Inline,
            font_stack: vec![Arc::<str>::from("Helvetica")],
            font_name: Arc::<str>::from("Helvetica"),
            position: PositionMode::Static,
            z_index: 0,
            inset_left: LengthSpec::Auto,
            inset_top: LengthSpec::Auto,
            inset_right: LengthSpec::Auto,
            inset_bottom: LengthSpec::Auto,
            box_sizing: BoxSizingMode::ContentBox,
            flex_direction: FlexDirectionMode::Row,
            flex_wrap: FlexWrapMode::NoWrap,
            flex_basis: LengthSpec::Auto,
            order: 0,
            justify_content: JustifyContentMode::FlexStart,
            align_items: AlignItemsMode::Stretch,
            grid_columns: None,
            gap: LengthSpec::Absolute(Pt::ZERO),
            flex_grow: 0.0,
            flex_shrink: 1.0,
            overflow: OverflowMode::Visible,
            custom_lengths: HashMap::new(),
            custom_colors: HashMap::new(),
            custom_color_alpha: HashMap::new(),
            custom_color_refs: HashMap::new(),
            custom_font_stacks: HashMap::new(),
        }
    }

    pub fn compute_style(
        &self,
        element: &ElementInfo,
        parent: &ComputedStyle,
        inline_style: Option<&str>,
        ancestors: &[ElementInfo],
    ) -> ComputedStyle {
        let debug = self.debug.as_ref();
        let debug_node = debug.map(|_| format_element_path(element, ancestors));
        let mut computed = ComputedStyle {
            font_size: parent.font_size,
            line_height: parent.line_height.clone(),
            color: parent.color,
            background_color: None,
            background_paint: None,
            pending_color_var: None,
            pending_background_color_var: None,
            pending_border_color_var: None,
            pending_font_name_var: None,
            pagination: Pagination::default(),
            margin: EdgeSizes::zero(),
            padding: EdgeSizes::zero(),
            width: LengthSpec::Auto,
            height: LengthSpec::Auto,
            pending_width_var: None,
            pending_height_var: None,
            max_width: LengthSpec::Auto,
            min_width: LengthSpec::Auto,
            min_height: LengthSpec::Auto,
            max_height: LengthSpec::Auto,
            pending_max_width_var: None,
            text_align: parent.text_align,
            vertical_align: parent.vertical_align,
            font_weight: parent.font_weight,
            font_style: parent.font_style,
            text_transform: parent.text_transform,
            text_decoration: parent.text_decoration,
            text_overflow: TextOverflowMode::Clip,
            content: None,
            word_break: parent.word_break,
            list_style_type: parent.list_style_type,
            letter_spacing: parent.letter_spacing,
            border_width: EdgeSizes::zero(),
            border_color: None,
            border_style: BorderStyleState::none(),
            border_collapse: BorderCollapseMode::Separate,
            caption_side: parent.caption_side,
            border_spacing: BorderSpacingSpec::zero(),
            border_radius: BorderRadiusSpec::zero(),
            box_shadow: None,
            root_font_size: parent.root_font_size,
            white_space: parent.white_space,
            display: DisplayMode::Inline,
            font_stack: parent.font_stack.clone(),
            font_name: parent.font_name.clone(),
            position: PositionMode::Static,
            z_index: 0,
            inset_left: LengthSpec::Auto,
            inset_top: LengthSpec::Auto,
            inset_right: LengthSpec::Auto,
            inset_bottom: LengthSpec::Auto,
            box_sizing: BoxSizingMode::ContentBox,
            flex_direction: parent.flex_direction,
            flex_wrap: parent.flex_wrap,
            flex_basis: parent.flex_basis,
            order: parent.order,
            justify_content: parent.justify_content,
            align_items: parent.align_items,
            grid_columns: None,
            gap: parent.gap,
            flex_grow: parent.flex_grow,
            flex_shrink: parent.flex_shrink,
            overflow: OverflowMode::Visible,
            custom_lengths: parent.custom_lengths.clone(),
            custom_colors: parent.custom_colors.clone(),
            custom_color_alpha: parent.custom_color_alpha.clone(),
            custom_color_refs: parent.custom_color_refs.clone(),
            custom_font_stacks: parent.custom_font_stacks.clone(),
        };
        let parent_font_size = parent.font_size;
        let parent_line_height = parent.line_height.clone();
        let root_font_size = parent.root_font_size;

        if element.is_root {
            for delta in &self.root_normal {
                apply_delta(
                    &mut computed,
                    delta,
                    parent,
                    parent_font_size,
                    parent_line_height.clone(),
                    root_font_size,
                    self.viewport,
                );
            }
        }

        let mut matches: Vec<&RuleEntry> = self
            .normal_index
            .candidate_indices(element)
            .into_iter()
            .filter_map(|idx| self.normal_rules.get(idx))
            .filter(|rule| rule.selector.matches(element, ancestors))
            .collect();
        matches.sort_by(|a, b| {
            a.specificity
                .cmp(&b.specificity)
                .then_with(|| a.order.cmp(&b.order))
        });

        if let (Some(logger), Some(node)) = (debug, debug_node.as_ref()) {
            let selectors: Vec<String> = matches
                .iter()
                .map(|rule| rule.selector_text.clone())
                .collect();
            if !selectors.is_empty() {
                let list = json_array(&selectors);
                let json = format!(
                    "{{\"type\":\"css.match\",\"node\":{},\"selectors\":{}}}",
                    json_string(node),
                    list
                );
                logger.log_json(&json);
            }
        }

        for rule in matches {
            apply_delta(
                &mut computed,
                &rule.delta,
                parent,
                parent_font_size,
                parent_line_height.clone(),
                root_font_size,
                self.viewport,
            );
        }

        let mut inline_normal = StyleDelta::default();
        let mut inline_important = StyleDelta::default();
        if let Some(inline) = inline_style {
            if let Ok(style) = StyleAttribute::parse(inline, ParserOptions::default()) {
                if let Some(logger) = debug {
                    let selector = debug_node
                        .as_deref()
                        .map(|node| format!("@inline {node}"))
                        .unwrap_or_else(|| "@inline".to_string());
                    log_declaration_no_effects(&style.declarations, &selector, logger);
                }
                let (normal_delta, important_delta) = style_from_declarations(&style.declarations);
                inline_normal = normal_delta;
                inline_important = important_delta;
            }
        }

        apply_delta(
            &mut computed,
            &inline_normal,
            parent,
            parent_font_size,
            parent_line_height.clone(),
            root_font_size,
            self.viewport,
        );

        if element.is_root {
            for delta in &self.root_important {
                apply_delta(
                    &mut computed,
                    delta,
                    parent,
                    parent_font_size,
                    parent_line_height.clone(),
                    root_font_size,
                    self.viewport,
                );
            }
        }

        let mut important_matches: Vec<&RuleEntry> = self
            .important_index
            .candidate_indices(element)
            .into_iter()
            .filter_map(|idx| self.important_rules.get(idx))
            .filter(|rule| rule.selector.matches(element, ancestors))
            .collect();
        important_matches.sort_by(|a, b| {
            a.specificity
                .cmp(&b.specificity)
                .then_with(|| a.order.cmp(&b.order))
        });

        for rule in important_matches {
            apply_delta(
                &mut computed,
                &rule.delta,
                parent,
                parent_font_size,
                parent_line_height.clone(),
                root_font_size,
                self.viewport,
            );
        }

        apply_delta(
            &mut computed,
            &inline_important,
            parent,
            parent_font_size,
            parent_line_height,
            root_font_size,
            self.viewport,
        );

        let mut unresolved = Vec::new();
        if let Some(name) = computed.pending_width_var.clone() {
            if !computed.custom_lengths.contains_key(&name) {
                unresolved.push(name);
            }
        }
        if let Some(name) = computed.pending_max_width_var.clone() {
            if !computed.custom_lengths.contains_key(&name) {
                unresolved.push(name);
            }
        }
        if let Some(name) = computed.pending_height_var.clone() {
            if !computed.custom_lengths.contains_key(&name) {
                unresolved.push(name);
            }
        }
        if let Some(name) = computed.pending_color_var.clone() {
            if resolve_custom_color(&computed, &name).is_none() {
                unresolved.push(name);
            }
        }
        if let Some(name) = computed.pending_background_color_var.clone() {
            if resolve_custom_color(&computed, &name).is_none() {
                unresolved.push(name);
            }
        }
        if let Some(name) = computed.pending_border_color_var.clone() {
            if resolve_custom_color(&computed, &name).is_none() {
                unresolved.push(name);
            }
        }
        if let Some(name) = computed.pending_font_name_var.clone() {
            if !computed.custom_font_stacks.contains_key(&name) {
                unresolved.push(name);
            }
        }

        if element.is_root {
            computed.root_font_size = computed.font_size;
        }
        resolve_pending_vars(&mut computed);
        apply_border_style_mask(&mut computed);
        if let (Some(logger), Some(node)) = (debug, debug_node.as_ref()) {
            let style_json = debug_style_json(&computed);
            let unresolved_json = json_array(&unresolved);
            let json = format!(
                "{{\"type\":\"css.computed\",\"node\":{},\"style\":{},\"vars_unresolved\":{}}}",
                json_string(node),
                style_json,
                unresolved_json
            );
            logger.log_json(&json);
        }
        computed
    }

    pub fn compute_pseudo_style(
        &self,
        element: &ElementInfo,
        parent: &ComputedStyle,
        ancestors: &[ElementInfo],
        pseudo: PseudoTarget,
    ) -> Option<ComputedStyle> {
        if matches!(pseudo, PseudoTarget::None) {
            return None;
        }

        let mut computed = ComputedStyle {
            font_size: parent.font_size,
            line_height: parent.line_height.clone(),
            color: parent.color,
            background_color: None,
            background_paint: None,
            pending_color_var: None,
            pending_background_color_var: None,
            pending_border_color_var: None,
            pending_font_name_var: None,
            pagination: Pagination::default(),
            margin: EdgeSizes::zero(),
            padding: EdgeSizes::zero(),
            width: LengthSpec::Auto,
            height: LengthSpec::Auto,
            pending_width_var: None,
            pending_height_var: None,
            max_width: LengthSpec::Auto,
            min_width: LengthSpec::Auto,
            min_height: LengthSpec::Auto,
            max_height: LengthSpec::Auto,
            pending_max_width_var: None,
            text_align: parent.text_align,
            vertical_align: parent.vertical_align,
            font_weight: parent.font_weight,
            font_style: parent.font_style,
            text_transform: parent.text_transform,
            text_decoration: parent.text_decoration,
            text_overflow: TextOverflowMode::Clip,
            content: None,
            word_break: parent.word_break,
            list_style_type: parent.list_style_type,
            letter_spacing: parent.letter_spacing,
            border_width: EdgeSizes::zero(),
            border_color: None,
            border_style: BorderStyleState::none(),
            border_collapse: BorderCollapseMode::Separate,
            caption_side: parent.caption_side,
            border_spacing: BorderSpacingSpec::zero(),
            border_radius: BorderRadiusSpec::zero(),
            box_shadow: None,
            root_font_size: parent.root_font_size,
            white_space: parent.white_space,
            display: DisplayMode::Inline,
            font_stack: parent.font_stack.clone(),
            font_name: parent.font_name.clone(),
            position: PositionMode::Static,
            z_index: 0,
            inset_left: LengthSpec::Auto,
            inset_top: LengthSpec::Auto,
            inset_right: LengthSpec::Auto,
            inset_bottom: LengthSpec::Auto,
            box_sizing: BoxSizingMode::ContentBox,
            flex_direction: parent.flex_direction,
            flex_wrap: parent.flex_wrap,
            flex_basis: parent.flex_basis,
            order: parent.order,
            justify_content: parent.justify_content,
            align_items: parent.align_items,
            grid_columns: None,
            gap: parent.gap,
            flex_grow: parent.flex_grow,
            flex_shrink: parent.flex_shrink,
            overflow: OverflowMode::Visible,
            custom_lengths: parent.custom_lengths.clone(),
            custom_colors: parent.custom_colors.clone(),
            custom_color_alpha: parent.custom_color_alpha.clone(),
            custom_color_refs: parent.custom_color_refs.clone(),
            custom_font_stacks: parent.custom_font_stacks.clone(),
        };

        let parent_font_size = parent.font_size;
        let parent_line_height = parent.line_height.clone();
        let root_font_size = parent.root_font_size;

        let mut matches: Vec<&RuleEntry> = self
            .normal_index
            .candidate_indices(element)
            .into_iter()
            .filter_map(|idx| self.normal_rules.get(idx))
            .filter(|rule| rule.selector.pseudo_target() == Some(pseudo))
            .filter(|rule| {
                rule.selector
                    .matches_with_pseudo(element, ancestors, pseudo)
            })
            .collect();
        if matches.is_empty() {
            return None;
        }
        matches.sort_by(|a, b| {
            a.specificity
                .cmp(&b.specificity)
                .then_with(|| a.order.cmp(&b.order))
        });

        for rule in matches {
            apply_delta(
                &mut computed,
                &rule.delta,
                parent,
                parent_font_size,
                parent_line_height.clone(),
                root_font_size,
                self.viewport,
            );
        }
        apply_border_style_mask(&mut computed);

        match computed.content.as_deref() {
            Some(text) if !text.is_empty() => Some(computed),
            _ => None,
        }
    }
}

fn parse_selector_pattern(selector: &str) -> Option<SelectorPattern> {
    let selector = selector.trim();
    if selector.is_empty() {
        return None;
    }
    let mut parts: Vec<SimpleSelector> = Vec::new();
    let mut combinators: Vec<Combinator> = Vec::new();
    let mut buf = String::new();
    let mut pending: Option<Combinator> = None;

    let flush_buf = |buf: &mut String,
                     parts: &mut Vec<SimpleSelector>,
                     combinators: &mut Vec<Combinator>,
                     pending: &mut Option<Combinator>|
     -> Option<()> {
        let trimmed = buf.trim();
        if trimmed.is_empty() {
            buf.clear();
            return Some(());
        }
        let simple = parse_simple_selector(trimmed)?;
        if !parts.is_empty() {
            combinators.push(pending.take().unwrap_or(Combinator::Descendant));
        }
        parts.push(simple);
        buf.clear();
        Some(())
    };

    for ch in selector.chars() {
        if ch == '>' {
            if flush_buf(&mut buf, &mut parts, &mut combinators, &mut pending).is_none() {
                return None;
            }
            pending = Some(Combinator::Child);
            continue;
        }
        if ch == '+' {
            if flush_buf(&mut buf, &mut parts, &mut combinators, &mut pending).is_none() {
                return None;
            }
            pending = Some(Combinator::AdjacentSibling);
            continue;
        }
        if ch == '~' {
            if flush_buf(&mut buf, &mut parts, &mut combinators, &mut pending).is_none() {
                return None;
            }
            pending = Some(Combinator::GeneralSibling);
            continue;
        }
        if ch.is_whitespace() {
            if !buf.trim().is_empty() {
                if flush_buf(&mut buf, &mut parts, &mut combinators, &mut pending).is_none() {
                    return None;
                }
                pending = Some(Combinator::Descendant);
            }
            continue;
        }
        buf.push(ch);
    }
    if flush_buf(&mut buf, &mut parts, &mut combinators, &mut pending).is_none() {
        return None;
    }

    if parts.is_empty() {
        return None;
    }

    Some(SelectorPattern { parts, combinators })
}

fn media_list_matches(
    list: &MediaList,
    viewport: Size,
    prefer_print: bool,
    debug: Option<&DebugLogger>,
) -> bool {
    if viewport.width == Pt::ZERO && viewport.height == Pt::ZERO {
        return true;
    }
    if list.media_queries.is_empty() {
        return true;
    }
    let mut matched = false;
    for query in &list.media_queries {
        match media_query_matches(query, viewport, prefer_print, debug) {
            Some(true) => {
                matched = true;
                if let Some(logger) = debug {
                    logger.increment("css.media.matched", 1);
                }
                break;
            }
            Some(false) => {
                if let Some(logger) = debug {
                    logger.increment("css.media.unmatched", 1);
                }
            }
            None => {
                if let Some(logger) = debug {
                    logger.increment("css.media.unsupported", 1);
                }
            }
        }
    }
    matched
}

fn media_query_matches(
    query: &MediaQuery,
    viewport: Size,
    prefer_print: bool,
    debug: Option<&DebugLogger>,
) -> Option<bool> {
    let media_type_matches = if prefer_print {
        matches!(query.media_type, MediaType::All | MediaType::Print)
    } else {
        matches!(
            query.media_type,
            MediaType::All | MediaType::Print | MediaType::Screen
        )
    };
    if !media_type_matches {
        return Some(false);
    }
    let condition_matches = match &query.condition {
        Some(condition) => media_condition_matches(condition, viewport, debug)?,
        None => true,
    };
    let mut result = condition_matches;
    if let Some(Qualifier::Not) = query.qualifier {
        result = !result;
    }
    Some(result)
}

fn media_condition_matches(
    condition: &MediaCondition,
    viewport: Size,
    debug: Option<&DebugLogger>,
) -> Option<bool> {
    match condition {
        MediaCondition::Feature(feature) => media_feature_matches(feature, viewport, debug),
        MediaCondition::Not(inner) => media_condition_matches(inner, viewport, debug).map(|v| !v),
        MediaCondition::Operation {
            operator,
            conditions,
        } => match operator {
            Operator::And => {
                let mut any = false;
                for cond in conditions {
                    let value = media_condition_matches(cond, viewport, debug)?;
                    any = true;
                    if !value {
                        return Some(false);
                    }
                }
                Some(any)
            }
            Operator::Or => {
                let mut any = false;
                for cond in conditions {
                    let value = media_condition_matches(cond, viewport, debug)?;
                    any = true;
                    if value {
                        return Some(true);
                    }
                }
                let _ = any;
                Some(false)
            }
        },
        MediaCondition::Unknown(_) => None,
    }
}

fn media_feature_matches(
    feature: &MediaFeature,
    viewport: Size,
    _debug: Option<&DebugLogger>,
) -> Option<bool> {
    match feature {
        MediaFeature::Plain { name, value } => {
            media_feature_compare(name, MediaFeatureComparison::Equal, value, viewport)
        }
        MediaFeature::Range {
            name,
            operator,
            value,
        } => media_feature_compare(name, *operator, value, viewport),
        MediaFeature::Interval {
            name,
            start,
            start_operator,
            end,
            end_operator,
        } => {
            let left = media_feature_compare(name, *start_operator, start, viewport)?;
            let right = media_feature_compare(name, *end_operator, end, viewport)?;
            Some(left && right)
        }
        MediaFeature::Boolean { .. } => None,
    }
}

fn media_feature_compare(
    name: &MediaFeatureName<MediaFeatureId>,
    operator: MediaFeatureComparison,
    value: &MediaFeatureValue,
    viewport: Size,
) -> Option<bool> {
    let target = match name {
        MediaFeatureName::Standard(id) => match id {
            MediaFeatureId::Width | MediaFeatureId::DeviceWidth => viewport.width,
            MediaFeatureId::Height | MediaFeatureId::DeviceHeight => viewport.height,
            _ => return None,
        },
        _ => return None,
    };
    let rhs = match value {
        MediaFeatureValue::Length(length) => length.to_px().map(px_to_pt),
        _ => None,
    }?;
    Some(match operator {
        MediaFeatureComparison::GreaterThan => target > rhs,
        MediaFeatureComparison::GreaterThanEqual => target >= rhs,
        MediaFeatureComparison::LessThan => target < rhs,
        MediaFeatureComparison::LessThanEqual => target <= rhs,
        MediaFeatureComparison::Equal => target == rhs,
    })
}

fn stylesheet_has_print_media(rules: &CssRuleList) -> bool {
    for rule in &rules.0 {
        match rule {
            CssRule::Media(media) => {
                if media
                    .query
                    .media_queries
                    .iter()
                    .any(|q| matches!(q.media_type, MediaType::Print))
                {
                    return true;
                }
                if stylesheet_has_print_media(&media.rules) {
                    return true;
                }
            }
            _ => {}
        }
    }
    false
}

pub(crate) fn extract_css_page_setup(
    css: &str,
    debug: Option<&DebugLogger>,
    viewport: Option<Size>,
) -> CssPageSetup {
    if css.trim().is_empty() {
        return CssPageSetup::default();
    }
    let Ok(sheet) = StyleSheet::parse(css, ParserOptions::default()) else {
        return CssPageSetup::default();
    };
    let viewport = viewport.unwrap_or(Size {
        width: Pt::ZERO,
        height: Pt::ZERO,
    });
    let prefer_print = stylesheet_has_print_media(&sheet.rules);
    let mut setup = CssPageSetup::default();
    extract_css_page_setup_from_rules(&sheet.rules, &mut setup, viewport, prefer_print, debug);
    setup
}

fn extract_css_page_setup_from_rules(
    rules: &CssRuleList,
    setup: &mut CssPageSetup,
    viewport: Size,
    prefer_print: bool,
    debug: Option<&DebugLogger>,
) {
    for rule in &rules.0 {
        match rule {
            CssRule::Page(page_rule) => {
                if !page_rule_targets_default(page_rule) {
                    continue;
                }
                apply_page_rule_declarations(page_rule, setup);
            }
            CssRule::Media(media) => {
                if media_list_matches(&media.query, viewport, prefer_print, debug) {
                    extract_css_page_setup_from_rules(
                        &media.rules,
                        setup,
                        viewport,
                        prefer_print,
                        debug,
                    );
                }
            }
            _ => {}
        }
    }
}

fn page_rule_targets_default(rule: &lightningcss::rules::page::PageRule) -> bool {
    if rule.selectors.is_empty() {
        return true;
    }
    rule.selectors
        .iter()
        .any(|selector| selector.name.is_none() && selector.pseudo_classes.is_empty())
}

fn apply_page_rule_declarations(
    rule: &lightningcss::rules::page::PageRule,
    setup: &mut CssPageSetup,
) {
    for property in &rule.declarations.declarations {
        apply_page_property(property, setup);
    }
    for property in &rule.declarations.important_declarations {
        apply_page_property(property, setup);
    }
}

fn apply_page_property(property: &Property, setup: &mut CssPageSetup) {
    match property {
        Property::Margin(value) => {
            setup.margin_top = lpa_to_absolute_pt(&value.top);
            setup.margin_right = lpa_to_absolute_pt(&value.right);
            setup.margin_bottom = lpa_to_absolute_pt(&value.bottom);
            setup.margin_left = lpa_to_absolute_pt(&value.left);
        }
        Property::MarginTop(value) => {
            setup.margin_top = lpa_to_absolute_pt(value);
        }
        Property::MarginRight(value) => {
            setup.margin_right = lpa_to_absolute_pt(value);
        }
        Property::MarginBottom(value) => {
            setup.margin_bottom = lpa_to_absolute_pt(value);
        }
        Property::MarginLeft(value) => {
            setup.margin_left = lpa_to_absolute_pt(value);
        }
        Property::Custom(custom) => {
            if !matches!(custom.name, CustomPropertyName::Unknown(..)) {
                return;
            }
            let name = custom.name.as_ref().to_ascii_lowercase();
            let raw = tokens_debug_string(&custom.value.0);
            match name.as_str() {
                "size" => {
                    if let Some(size) = parse_page_size_from_str(&raw) {
                        setup.size = Some(size);
                    }
                }
                "margin" => apply_margin_shorthand_str(setup, &raw),
                "margin-top" => setup.margin_top = parse_absolute_pt_from_str(&raw),
                "margin-right" => setup.margin_right = parse_absolute_pt_from_str(&raw),
                "margin-bottom" => setup.margin_bottom = parse_absolute_pt_from_str(&raw),
                "margin-left" => setup.margin_left = parse_absolute_pt_from_str(&raw),
                _ => {}
            }
        }
        Property::Unparsed(unparsed) => match &unparsed.property_id {
            PropertyId::Custom(name) if name.as_ref().eq_ignore_ascii_case("size") => {
                let raw = tokens_debug_string(&unparsed.value.0);
                if let Some(size) = parse_page_size_from_str(&raw) {
                    setup.size = Some(size);
                }
            }
            PropertyId::Margin => {
                let raw = tokens_debug_string(&unparsed.value.0);
                apply_margin_shorthand_str(setup, &raw);
            }
            PropertyId::MarginTop => {
                let raw = tokens_debug_string(&unparsed.value.0);
                setup.margin_top = parse_absolute_pt_from_str(&raw);
            }
            PropertyId::MarginRight => {
                let raw = tokens_debug_string(&unparsed.value.0);
                setup.margin_right = parse_absolute_pt_from_str(&raw);
            }
            PropertyId::MarginBottom => {
                let raw = tokens_debug_string(&unparsed.value.0);
                setup.margin_bottom = parse_absolute_pt_from_str(&raw);
            }
            PropertyId::MarginLeft => {
                let raw = tokens_debug_string(&unparsed.value.0);
                setup.margin_left = parse_absolute_pt_from_str(&raw);
            }
            _ => {}
        },
        _ => {}
    }
}

fn lpa_to_absolute_pt(value: &lightningcss::values::length::LengthPercentageOrAuto) -> Option<Pt> {
    match length_spec_from_lpa(value) {
        Some(LengthSpec::Absolute(value)) => Some(value),
        _ => None,
    }
}

fn parse_absolute_pt_from_str(raw: &str) -> Option<Pt> {
    match length_spec_from_string(raw) {
        Some(LengthSpec::Absolute(value)) => Some(value),
        _ => None,
    }
}

fn apply_margin_shorthand_str(setup: &mut CssPageSetup, raw: &str) {
    let values: Vec<Pt> = parse_length_list(raw)
        .into_iter()
        .filter_map(|spec| match spec {
            LengthSpec::Absolute(value) => Some(value),
            _ => None,
        })
        .collect();
    if values.is_empty() {
        return;
    }
    let (top, right, bottom, left) = match values.len() {
        1 => (values[0], values[0], values[0], values[0]),
        2 => (values[0], values[1], values[0], values[1]),
        3 => (values[0], values[1], values[2], values[1]),
        _ => (values[0], values[1], values[2], values[3]),
    };
    setup.margin_top = Some(top);
    setup.margin_right = Some(right);
    setup.margin_bottom = Some(bottom);
    setup.margin_left = Some(left);
}

fn parse_page_size_from_str(raw: &str) -> Option<Size> {
    let normalized = raw.replace(',', " ").replace('\n', " ").replace('\r', " ");
    let mut orientation: Option<&str> = None;
    let mut parts: Vec<&str> = Vec::new();
    for token in normalized.split_whitespace() {
        let lower = token.to_ascii_lowercase();
        if lower == "landscape" || lower == "portrait" {
            orientation = Some(if lower == "landscape" {
                "landscape"
            } else {
                "portrait"
            });
            continue;
        }
        parts.push(token);
    }
    if parts.is_empty() {
        return None;
    }
    if parts.len() == 1 {
        let ident = parts[0].to_ascii_lowercase();
        if ident == "auto" {
            return None;
        }
        let mut size = page_named_size(&ident)?;
        if let Some(value) = orientation {
            size = orient_page_size(size, value);
        }
        return Some(size);
    }
    if parts.len() == 2 {
        let mut size = Size {
            width: parse_absolute_pt_from_str(parts[0])?,
            height: parse_absolute_pt_from_str(parts[1])?,
        };
        if let Some(value) = orientation {
            size = orient_page_size(size, value);
        }
        return Some(size);
    }
    None
}

fn page_named_size(name: &str) -> Option<Size> {
    match name {
        "a5" => Some(Size::from_mm(148.0, 210.0)),
        "a4" => Some(Size::from_mm(210.0, 297.0)),
        "a3" => Some(Size::from_mm(297.0, 420.0)),
        "letter" => Some(Size::from_inches(8.5, 11.0)),
        "legal" => Some(Size::from_inches(8.5, 14.0)),
        "ledger" | "tabloid" => Some(Size::from_inches(11.0, 17.0)),
        _ => None,
    }
}

fn orient_page_size(size: Size, orientation: &str) -> Size {
    match orientation {
        "landscape" => {
            if size.width < size.height {
                Size {
                    width: size.height,
                    height: size.width,
                }
            } else {
                size
            }
        }
        "portrait" => {
            if size.width > size.height {
                Size {
                    width: size.height,
                    height: size.width,
                }
            } else {
                size
            }
        }
        _ => size,
    }
}

fn log_declaration_no_effects(
    declarations: &lightningcss::declaration::DeclarationBlock,
    selectors: &str,
    logger: &DebugLogger,
) {
    let selector = selectors.trim();
    for property in declarations
        .declarations
        .iter()
        .chain(declarations.important_declarations.iter())
    {
        if let Some((requested, applied, reason)) = declaration_layout_mode_normalization(property)
        {
            let json = format!(
                "{{\"type\":\"jit.known_loss\",\"code\":\"LAYOUT_MODE_NORMALIZED\",\"property\":\"display\",\"requested\":{},\"applied\":{},\"reason\":{},\"selector\":{}}}",
                json_string(&requested),
                json_string(applied),
                json_string(reason),
                json_string(selector)
            );
            logger.log_json(&json);
            logger.increment("jit.known_loss.layout_mode_normalized", 1);
        }

        let name = declaration_no_effect_property_name(property).or_else(|| {
            declaration_parsed_no_effect_property_name(property).map(|v| v.to_string())
        });
        let Some(name) = name else {
            continue;
        };
        let json = format!(
            "{{\"type\":\"jit.known_loss\",\"code\":\"DECLARATION_PARSED_NO_EFFECT\",\"property\":{},\"selector\":{}}}",
            json_string(&name),
            json_string(selector)
        );
        logger.log_json(&json);
        logger.increment("jit.known_loss.declaration_parsed_no_effect", 1);
    }
}

fn declaration_no_effect_property_name(property: &Property) -> Option<String> {
    match property {
        Property::Custom(custom) => match &custom.name {
            CustomPropertyName::Unknown(name) => {
                let name = name.as_ref().to_ascii_lowercase();
                if name.starts_with("--") || is_engine_supported_unknown_property(&name) {
                    None
                } else {
                    Some(name)
                }
            }
            _ => None,
        },
        _ => None,
    }
}

fn declaration_layout_mode_normalization(
    property: &Property,
) -> Option<(String, &'static str, &'static str)> {
    let Property::Display(display) = property else {
        return None;
    };
    let reason = match display {
        Display::Pair(pair) => match pair.inside {
            DisplayInside::Table => return None,
            DisplayInside::Ruby => "display_ruby_not_supported",
            DisplayInside::Box(_) => "display_legacy_box_not_supported",
            _ => return None,
        },
        Display::Keyword(keyword) => match keyword {
            DisplayKeyword::TableRowGroup
            | DisplayKeyword::TableHeaderGroup
            | DisplayKeyword::TableFooterGroup
            | DisplayKeyword::TableRow
            | DisplayKeyword::TableCell
            | DisplayKeyword::TableCaption => return None,
            DisplayKeyword::TableColumnGroup | DisplayKeyword::TableColumn => {
                "display_table_column_not_supported"
            }
            DisplayKeyword::RubyBase
            | DisplayKeyword::RubyText
            | DisplayKeyword::RubyBaseContainer
            | DisplayKeyword::RubyTextContainer => "display_ruby_internal_not_supported",
            _ => return None,
        },
    };
    let requested = display
        .to_css_string(PrinterOptions::default())
        .unwrap_or_else(|_| "display".to_string());
    let applied = display_mode_name(display_mode_from_display(display));
    Some((requested, applied, reason))
}

fn declaration_parsed_no_effect_property_name(property: &Property) -> Option<&'static str> {
    match property {
        Property::AlignContent(_, _) => Some("align-content"),
        Property::AlignSelf(_, _) => Some("align-self"),
        Property::JustifyItems(_) => Some("justify-items"),
        Property::JustifySelf(_) => Some("justify-self"),
        Property::PlaceContent(_) => Some("place-content"),
        Property::PlaceItems(_) => Some("place-items"),
        Property::PlaceSelf(_) => Some("place-self"),
        Property::RowGap(_) => Some("row-gap"),
        Property::ColumnGap(_) => Some("column-gap"),
        Property::FlexFlow(_, _) => Some("flex-flow"),
        Property::GridTemplateRows(_) => Some("grid-template-rows"),
        Property::GridAutoColumns(_) => Some("grid-auto-columns"),
        Property::GridAutoRows(_) => Some("grid-auto-rows"),
        Property::GridAutoFlow(_) => Some("grid-auto-flow"),
        Property::GridTemplateAreas(_) => Some("grid-template-areas"),
        Property::GridTemplate(_) => Some("grid-template"),
        Property::Grid(_) => Some("grid"),
        Property::GridRowStart(_) => Some("grid-row-start"),
        Property::GridRowEnd(_) => Some("grid-row-end"),
        Property::GridColumnStart(_) => Some("grid-column-start"),
        Property::GridColumnEnd(_) => Some("grid-column-end"),
        Property::GridRow(_) => Some("grid-row"),
        Property::GridColumn(_) => Some("grid-column"),
        Property::GridArea(_) => Some("grid-area"),
        Property::Unparsed(unparsed) => match &unparsed.property_id {
            PropertyId::AlignContent(_) => Some("align-content"),
            PropertyId::AlignSelf(_) => Some("align-self"),
            PropertyId::JustifyItems => Some("justify-items"),
            PropertyId::JustifySelf => Some("justify-self"),
            PropertyId::PlaceContent => Some("place-content"),
            PropertyId::PlaceItems => Some("place-items"),
            PropertyId::PlaceSelf => Some("place-self"),
            PropertyId::RowGap => Some("row-gap"),
            PropertyId::ColumnGap => Some("column-gap"),
            PropertyId::FlexFlow(_) => Some("flex-flow"),
            PropertyId::GridTemplateRows => Some("grid-template-rows"),
            PropertyId::GridAutoColumns => Some("grid-auto-columns"),
            PropertyId::GridAutoRows => Some("grid-auto-rows"),
            PropertyId::GridAutoFlow => Some("grid-auto-flow"),
            PropertyId::GridTemplateAreas => Some("grid-template-areas"),
            PropertyId::GridTemplate => Some("grid-template"),
            PropertyId::Grid => Some("grid"),
            PropertyId::GridRowStart => Some("grid-row-start"),
            PropertyId::GridRowEnd => Some("grid-row-end"),
            PropertyId::GridColumnStart => Some("grid-column-start"),
            PropertyId::GridColumnEnd => Some("grid-column-end"),
            PropertyId::GridRow => Some("grid-row"),
            PropertyId::GridColumn => Some("grid-column"),
            PropertyId::GridArea => Some("grid-area"),
            _ => None,
        },
        _ => None,
    }
}

fn is_engine_supported_unknown_property(name: &str) -> bool {
    matches!(
        name,
        "border-collapse"
            | "caption-side"
            | "border-spacing"
            | "border-radius"
            | "border"
            | "border-top"
            | "border-right"
            | "border-bottom"
            | "border-left"
            | "border-block-start"
            | "border-block-end"
            | "border-inline-start"
            | "border-inline-end"
            | "border-color"
            | "border-top-color"
            | "border-right-color"
            | "border-bottom-color"
            | "border-left-color"
            | "border-width"
            | "border-top-width"
            | "border-right-width"
            | "border-bottom-width"
            | "border-left-width"
            | "border-style"
            | "border-top-style"
            | "border-right-style"
            | "border-bottom-style"
            | "border-left-style"
            | "break-before"
            | "break-after"
            | "break-inside"
            | "orphans"
            | "widows"
            | "display"
            | "position"
            | "z-index"
            | "left"
            | "right"
            | "top"
            | "bottom"
            | "flex-direction"
            | "flex-wrap"
            | "order"
            | "justify-content"
            | "align-items"
            | "box-sizing"
            | "gap"
            | "box-shadow"
            | "content"
            | "size"
            | "margin"
            | "margin-top"
            | "margin-right"
            | "margin-bottom"
            | "margin-left"
    )
}

fn parse_simple_selector(selector: &str) -> Option<SimpleSelector> {
    let selector = selector.trim();
    if selector.is_empty() {
        return None;
    }

    let mut base = String::new();
    let mut pseudos_raw: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut depth = 0usize;
    let mut in_pseudo = false;
    for ch in selector.chars() {
        if ch == ':' && depth == 0 {
            if !in_pseudo {
                base = current.clone();
                current.clear();
                in_pseudo = true;
            } else if current.trim().is_empty() {
                // Handle ::before/::after
                continue;
            } else {
                pseudos_raw.push(current.clone());
                current.clear();
            }
            continue;
        }
        if ch == '(' {
            depth += 1;
        } else if ch == ')' {
            depth = depth.saturating_sub(1);
        }
        current.push(ch);
    }
    if in_pseudo {
        if !current.trim().is_empty() {
            pseudos_raw.push(current);
        }
    } else {
        base = current;
    }

    let (base, attrs_raw) = extract_attr_selectors(&base);

    let mut tag = None;
    let mut id = None;
    let mut classes = Vec::new();
    let mut buffer = String::new();
    let mut mode = SelectorPart::Tag;

    for ch in base.chars() {
        match ch {
            '#' => {
                flush_selector_part(&mut mode, &mut buffer, &mut tag, &mut id, &mut classes);
                mode = SelectorPart::Id;
            }
            '.' => {
                flush_selector_part(&mut mode, &mut buffer, &mut tag, &mut id, &mut classes);
                mode = SelectorPart::Class;
            }
            _ => buffer.push(ch),
        }
    }
    flush_selector_part(&mut mode, &mut buffer, &mut tag, &mut id, &mut classes);

    let mut pseudos = Vec::new();
    for pseudo_raw in pseudos_raw {
        if let Some(pseudo) = parse_pseudo_class(&pseudo_raw) {
            pseudos.push(pseudo);
        }
    }

    let mut attrs = Vec::new();
    for raw in attrs_raw {
        if let Some(attr) = parse_attr_selector(&raw) {
            attrs.push(attr);
        }
    }

    if tag.is_none() && id.is_none() && classes.is_empty() && attrs.is_empty() && pseudos.is_empty()
    {
        return None;
    }

    Some(SimpleSelector {
        tag: tag.map(|t| t.to_ascii_lowercase()),
        id: id.map(|i| i.to_ascii_lowercase()),
        classes: classes
            .into_iter()
            .map(|c| c.to_ascii_lowercase())
            .collect(),
        attrs,
        pseudos,
    })
}

fn extract_attr_selectors(input: &str) -> (String, Vec<String>) {
    let mut base = String::new();
    let mut attrs = Vec::new();
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '[' {
            base.push(ch);
            continue;
        }
        let mut buf = String::new();
        let mut in_quote: Option<char> = None;
        while let Some(c) = chars.next() {
            if let Some(q) = in_quote {
                if c == q {
                    in_quote = None;
                }
                buf.push(c);
                continue;
            }
            if c == '"' || c == '\'' {
                in_quote = Some(c);
                buf.push(c);
                continue;
            }
            if c == ']' {
                break;
            }
            buf.push(c);
        }
        if !buf.trim().is_empty() {
            attrs.push(buf.trim().to_string());
        }
    }
    (base, attrs)
}

fn parse_attr_selector(raw: &str) -> Option<AttrSelector> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }

    let mut op: Option<AttrOp> = None;
    let mut split_at: Option<usize> = None;
    let ops = ["~=", "|=", "^=", "$=", "*=", "="];
    let mut in_quote: Option<char> = None;
    let chars: Vec<char> = raw.chars().collect();
    let mut i = 0usize;
    while i < chars.len() {
        let ch = chars[i];
        if let Some(q) = in_quote {
            if ch == q {
                in_quote = None;
            }
            i += 1;
            continue;
        }
        if ch == '"' || ch == '\'' {
            in_quote = Some(ch);
            i += 1;
            continue;
        }
        for token in ops {
            if i + token.len() <= chars.len() {
                let slice: String = chars[i..i + token.len()].iter().collect();
                if slice == token {
                    split_at = Some(i);
                    op = Some(match token {
                        "~=" => AttrOp::Includes,
                        "|=" => AttrOp::DashMatch,
                        "^=" => AttrOp::Prefix,
                        "$=" => AttrOp::Suffix,
                        "*=" => AttrOp::Substring,
                        "=" => AttrOp::Equals,
                        _ => AttrOp::Exists,
                    });
                    i = chars.len();
                    break;
                }
            }
        }
        i += 1;
    }

    let (name, value) = if let (Some(pos), Some(op_kind)) = (split_at, op) {
        let name = raw[..pos].trim();
        let value_raw = raw[(pos + if op_kind == AttrOp::Equals { 1 } else { 2 })..].trim();
        (name, Some(value_raw))
    } else {
        (raw, None)
    };

    let name = name.trim().to_ascii_lowercase();
    if name.is_empty() {
        return None;
    }

    let value = value.and_then(|v| {
        let v = v.trim();
        if v.is_empty() {
            return None;
        }
        let v = v.split_whitespace().next().unwrap_or(v);
        let unquoted = v.trim_matches('"').trim_matches('\'').to_string();
        Some(unquoted)
    });

    let op = op.unwrap_or(AttrOp::Exists);
    Some(AttrSelector { name, op, value })
}

fn parse_pseudo_class(raw: &str) -> Option<PseudoClass> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    if let Some(args) = raw.strip_prefix("not(") {
        let args = args.trim_end_matches(')').trim();
        let inner = parse_simple_selector(args)?;
        return Some(PseudoClass::Not(inner));
    }
    if raw.eq_ignore_ascii_case("root") {
        return Some(PseudoClass::Root);
    }
    if raw.eq_ignore_ascii_case("first-child") {
        return Some(PseudoClass::FirstChild);
    }
    if raw.eq_ignore_ascii_case("last-child") {
        return Some(PseudoClass::LastChild);
    }
    if raw.eq_ignore_ascii_case("hover") {
        return Some(PseudoClass::Hover);
    }
    if raw.eq_ignore_ascii_case("before") {
        return Some(PseudoClass::Before);
    }
    if raw.eq_ignore_ascii_case("after") {
        return Some(PseudoClass::After);
    }
    if let Some(args) = raw.strip_prefix("nth-child(") {
        let args = args.trim_end_matches(')').trim();
        if args.eq_ignore_ascii_case("even") {
            return Some(PseudoClass::NthChildEven);
        }
        if args.eq_ignore_ascii_case("odd") {
            return Some(PseudoClass::NthChildOdd);
        }
        if let Ok(n) = args.parse::<usize>() {
            if n >= 1 {
                return Some(PseudoClass::NthChild(n));
            }
        }
        if let Some((a, b)) = parse_nth_formula(args) {
            return Some(PseudoClass::NthChildFormula { a, b });
        }
        return None;
    }
    if let Some(args) = raw.strip_prefix("nth-of-type(") {
        let args = args.trim_end_matches(')').trim();
        if args.eq_ignore_ascii_case("even") {
            return Some(PseudoClass::NthChildEven);
        }
        if args.eq_ignore_ascii_case("odd") {
            return Some(PseudoClass::NthChildOdd);
        }
        if let Ok(n) = args.parse::<usize>() {
            if n >= 1 {
                return Some(PseudoClass::NthChild(n));
            }
        }
        if let Some((a, b)) = parse_nth_formula(args) {
            return Some(PseudoClass::NthChildFormula { a, b });
        }
        return None;
    }
    Some(PseudoClass::Unsupported)
}

fn parse_nth_formula(raw: &str) -> Option<(i32, i32)> {
    let mut s = raw.replace(' ', "");
    if s.is_empty() {
        return None;
    }
    let lower = s.to_ascii_lowercase();
    s = lower;
    if !s.contains('n') {
        let b = s.parse::<i32>().ok()?;
        return Some((0, b));
    }
    let n_pos = s.find('n')?;
    let (a_str, b_str) = s.split_at(n_pos);
    let b_str = b_str.trim_start_matches('n');
    let a = if a_str.is_empty() {
        1
    } else if a_str == "+" {
        1
    } else if a_str == "-" {
        -1
    } else {
        a_str.parse::<i32>().ok()?
    };
    let b = if b_str.is_empty() {
        0
    } else {
        b_str.parse::<i32>().ok()?
    };
    Some((a, b))
}

enum SelectorPart {
    Tag,
    Id,
    Class,
}

fn flush_selector_part(
    mode: &mut SelectorPart,
    buffer: &mut String,
    tag: &mut Option<String>,
    id: &mut Option<String>,
    classes: &mut Vec<String>,
) {
    if buffer.is_empty() {
        return;
    }
    let value = std::mem::take(buffer);
    match mode {
        SelectorPart::Tag => {
            *tag = Some(value);
        }
        SelectorPart::Id => {
            *id = Some(value);
        }
        SelectorPart::Class => {
            classes.push(value);
        }
    }
}

fn style_from_declarations(
    declarations: &lightningcss::declaration::DeclarationBlock,
) -> (StyleDelta, StyleDelta) {
    let mut normal = StyleDelta::default();
    let mut important = StyleDelta::default();
    apply_properties(&declarations.declarations, &mut normal);
    apply_properties(&declarations.important_declarations, &mut important);
    (normal, important)
}

fn apply_properties(props: &[Property], delta: &mut StyleDelta) {
    for prop in props {
        match prop {
            Property::FontSize(size) => {
                delta.font_size = font_size_spec(size);
            }
            Property::FontWeight(weight) => {
                if let Ok(raw) = weight.to_css_string(PrinterOptions::default()) {
                    delta.font_weight = parse_font_weight_str(&raw);
                }
            }
            Property::FontStyle(style) => {
                delta.font_style = font_style_spec(style);
            }
            Property::LineHeight(line_height) => {
                delta.line_height = line_height_spec(line_height);
            }
            Property::Color(color) => {
                if let Ok(raw) = color.to_css_string(PrinterOptions::default()) {
                    if raw.to_ascii_lowercase().contains("var(") {
                        if let Some((color, alpha)) = parse_color_string(&raw) {
                            delta.color = Some(ColorSpec::Value(blend_over_white(color, alpha)));
                        } else {
                            delta.color_var = Some(raw.trim().to_ascii_lowercase());
                        }
                        continue;
                    }
                    if let Some((color, alpha)) = parse_color_string(&raw) {
                        delta.color = Some(ColorSpec::Value(blend_over_white(color, alpha)));
                        continue;
                    } else if let Some(var) = var_name_from_string(&raw) {
                        delta.color_var = Some(var);
                        continue;
                    }
                }
                if let Some(color) = css_color_to_color(color) {
                    delta.color = Some(ColorSpec::Value(color));
                } else if matches!(color, CssColor::CurrentColor) {
                    delta.color = Some(ColorSpec::Inherit);
                }
            }
            Property::BackgroundColor(color) => {
                if let Ok(raw) = color.to_css_string(PrinterOptions::default()) {
                    if raw.to_ascii_lowercase().contains("var(") {
                        if let Some((color, alpha)) = parse_color_string(&raw) {
                            delta.background_color =
                                Some(BackgroundSpec::Value(blend_over_white(color, alpha)));
                        } else {
                            delta.background_color_var = Some(raw.trim().to_ascii_lowercase());
                        }
                        continue;
                    }
                    if let Some((color, alpha)) = parse_color_string(&raw) {
                        delta.background_color =
                            Some(BackgroundSpec::Value(blend_over_white(color, alpha)));
                        continue;
                    } else if let Some(var) = var_name_from_string(&raw) {
                        delta.background_color_var = Some(var);
                        continue;
                    }
                }
                if let Some(color) = css_color_to_color(color) {
                    delta.background_color = Some(BackgroundSpec::Value(color));
                } else if matches!(color, CssColor::CurrentColor) {
                    delta.background_color = Some(BackgroundSpec::CurrentColor);
                }
            }
            Property::Background(background) => {
                if let Ok(raw) = background.to_css_string(PrinterOptions::default()) {
                    apply_background_from_string(&raw, delta);
                }
            }
            Property::TextTransform(value) => {
                if let Ok(raw) = value.to_css_string(PrinterOptions::default()) {
                    delta.text_transform = parse_text_transform_str(&raw);
                }
            }
            Property::TextDecorationLine(value, _) => {
                delta.text_decoration =
                    Some(TextDecorationSpec::Value(text_decoration_from_line(value)));
            }
            Property::TextDecoration(value, _) => {
                if let Ok(raw) = value.to_css_string(PrinterOptions::default()) {
                    if let Some(mode) = parse_text_decoration_str(&raw) {
                        delta.text_decoration = Some(TextDecorationSpec::Value(mode));
                    }
                }
            }
            Property::TextOverflow(value, _) => {
                let mode = match value {
                    css_overflow::TextOverflow::Clip => TextOverflowMode::Clip,
                    css_overflow::TextOverflow::Ellipsis => TextOverflowMode::Ellipsis,
                };
                delta.text_overflow = Some(TextOverflowSpec::Value(mode));
            }
            Property::WordBreak(value) => {
                delta.word_break = Some(word_break_mode_from_css(value));
            }
            Property::OverflowWrap(value) => {
                delta.word_break = Some(word_break_mode_from_overflow_wrap(value));
            }
            Property::WordWrap(value) => {
                delta.word_break = Some(word_break_mode_from_overflow_wrap(value));
            }
            Property::LetterSpacing(value) => {
                if let Ok(raw) = value.to_css_string(PrinterOptions::default()) {
                    delta.letter_spacing = parse_letter_spacing_str(&raw);
                }
            }
            Property::BorderRadius(value, _) => {
                if let Ok(raw) = value.to_css_string(PrinterOptions::default()) {
                    if let Some(radius) = parse_border_radius_str(&raw) {
                        delta.border_radius = Some(radius);
                    }
                }
            }
            Property::BorderSpacing(value) => {
                if let Ok(raw) = value.to_css_string(PrinterOptions::default()) {
                    if let Some(spacing) = parse_border_spacing_str(&raw) {
                        delta.border_spacing = Some(spacing);
                    }
                }
            }
            Property::BoxShadow(value, _) => {
                if let Ok(raw) = value.to_css_string(PrinterOptions::default()) {
                    if let Some(shadow) = parse_box_shadow_str(&raw) {
                        delta.box_shadow = Some(shadow);
                    }
                }
            }
            Property::FontFamily(families) => {
                if let Some(spec) = font_spec_from_family(families) {
                    delta.font_name = Some(spec);
                }
            }
            Property::ListStyleType(value) => {
                delta.list_style_type = Some(list_style_type_mode_from_css(value));
            }
            Property::ListStyle(value) => {
                delta.list_style_type = Some(list_style_type_mode_from_css(&value.list_style_type));
            }
            Property::WhiteSpace(white_space) => {
                let value = match white_space {
                    WhiteSpace::Normal => WhiteSpaceMode::Normal,
                    WhiteSpace::Pre => WhiteSpaceMode::Pre,
                    WhiteSpace::NoWrap => WhiteSpaceMode::NoWrap,
                    WhiteSpace::PreWrap => WhiteSpaceMode::PreWrap,
                    WhiteSpace::BreakSpaces => WhiteSpaceMode::BreakSpaces,
                    WhiteSpace::PreLine => WhiteSpaceMode::PreLine,
                };
                delta.white_space = Some(WhiteSpaceSpec::Value(value));
            }
            Property::Display(display) => {
                delta.display = Some(DisplaySpec::Value(display_mode_from_display(display)));
            }
            Property::FlexDirection(direction, _) => {
                delta.flex_direction = Some(match direction {
                    css_flex::FlexDirection::Column => FlexDirectionMode::Column,
                    css_flex::FlexDirection::ColumnReverse => FlexDirectionMode::Column,
                    _ => FlexDirectionMode::Row,
                });
            }
            Property::FlexWrap(value, _) => {
                delta.flex_wrap = Some(match value {
                    css_flex::FlexWrap::Wrap | css_flex::FlexWrap::WrapReverse => {
                        FlexWrapMode::Wrap
                    }
                    _ => FlexWrapMode::NoWrap,
                });
            }
            Property::Order(value, _) => {
                delta.order = Some(*value);
            }
            Property::FlexOrder(value, _) => {
                delta.order = Some(*value);
            }
            Property::JustifyContent(value, _) => {
                delta.justify_content = Some(match value {
                    css_align::JustifyContent::ContentDistribution(
                        css_align::ContentDistribution::SpaceBetween,
                    ) => JustifyContentMode::SpaceBetween,
                    css_align::JustifyContent::ContentPosition {
                        value: css_align::ContentPosition::Center,
                        ..
                    } => JustifyContentMode::Center,
                    css_align::JustifyContent::ContentPosition {
                        value: css_align::ContentPosition::End | css_align::ContentPosition::FlexEnd,
                        ..
                    } => JustifyContentMode::FlexEnd,
                    _ => JustifyContentMode::FlexStart,
                });
            }
            Property::AlignItems(value, _) => {
                delta.align_items = Some(match value {
                    css_align::AlignItems::Stretch => AlignItemsMode::Stretch,
                    css_align::AlignItems::SelfPosition {
                        value: css_align::SelfPosition::Center,
                        ..
                    } => AlignItemsMode::Center,
                    css_align::AlignItems::SelfPosition {
                        value: css_align::SelfPosition::End | css_align::SelfPosition::FlexEnd,
                        ..
                    } => AlignItemsMode::FlexEnd,
                    _ => AlignItemsMode::FlexStart,
                });
            }
            Property::GridTemplateColumns(value) => {
                if let Ok(raw) = value.to_css_string(PrinterOptions::default()) {
                    delta.grid_columns = parse_grid_track_count(&raw);
                }
            }
            Property::Gap(value) => {
                // Use row gap (single-value `gap:` sets both row/column the same).
                let row = &value.row;
                if let css_align::GapValue::LengthPercentage(lp) = row {
                    if let Some(spec) = length_spec_from_lp(lp) {
                        delta.gap = Some(spec);
                    }
                }
            }
            Property::Flex(value, _) => {
                // flex: <grow> <shrink> <basis>
                delta.flex_grow = Some(value.grow);
                delta.flex_shrink = Some(value.shrink);
                if let Some(spec) = length_spec_from_lpa(&value.basis) {
                    delta.flex_basis = Some(spec);
                }
            }
            Property::FlexGrow(value, _) => {
                delta.flex_grow = Some(*value);
            }
            Property::FlexShrink(value, _) => {
                delta.flex_shrink = Some(*value);
            }
            Property::FlexBasis(value, _) => {
                delta.flex_basis = length_spec_from_lpa(value);
            }
            Property::Overflow(value) => {
                delta.overflow = Some(overflow_mode_from_overflow(value));
            }
            Property::OverflowX(value) => {
                delta.overflow = Some(overflow_mode_from_keyword(value));
            }
            Property::OverflowY(value) => {
                delta.overflow = Some(overflow_mode_from_keyword(value));
            }
            Property::TextAlign(align) => {
                delta.text_align = Some(text_align_mode_from_css(align));
            }
            Property::VerticalAlign(align) => {
                delta.vertical_align = Some(vertical_align_mode_from_css(align));
            }
            Property::Position(position) => {
                delta.position = Some(position_mode_from_css(position));
            }
            Property::ZIndex(value) => {
                delta.z_index = Some(match value {
                    lightningcss::properties::position::ZIndex::Auto => 0,
                    lightningcss::properties::position::ZIndex::Integer(v) => *v,
                });
            }
            Property::BoxSizing(value, _) => {
                delta.box_sizing = Some(match value {
                    css_size::BoxSizing::BorderBox => BoxSizingMode::BorderBox,
                    _ => BoxSizingMode::ContentBox,
                });
            }
            Property::Width(value) => {
                delta.width = length_spec_from_size(value);
            }
            Property::MinWidth(value) => {
                delta.min_width = length_spec_from_size(value);
            }
            Property::MaxWidth(value) => {
                delta.max_width = length_spec_from_max_size(value);
            }
            Property::Height(value) => {
                delta.height = length_spec_from_size(value);
            }
            Property::MinHeight(value) => {
                delta.min_height = length_spec_from_size(value);
            }
            Property::MaxHeight(value) => {
                delta.max_height = length_spec_from_max_size(value);
            }
            Property::Left(value) => {
                delta.inset_left = length_spec_from_lpa(value);
            }
            Property::Top(value) => {
                delta.inset_top = length_spec_from_lpa(value);
            }
            Property::Right(value) => {
                delta.inset_right = length_spec_from_lpa(value);
            }
            Property::Bottom(value) => {
                delta.inset_bottom = length_spec_from_lpa(value);
            }
            Property::MarginTop(value) => {
                delta.margin.top = length_spec_from_lpa(value);
            }
            Property::MarginRight(value) => {
                delta.margin.right = length_spec_from_lpa(value);
            }
            Property::MarginBottom(value) => {
                delta.margin.bottom = length_spec_from_lpa(value);
            }
            Property::MarginLeft(value) => {
                delta.margin.left = length_spec_from_lpa(value);
            }
            Property::Margin(value) => {
                delta.margin.top = length_spec_from_lpa(&value.top);
                delta.margin.right = length_spec_from_lpa(&value.right);
                delta.margin.bottom = length_spec_from_lpa(&value.bottom);
                delta.margin.left = length_spec_from_lpa(&value.left);
            }
            Property::PaddingTop(value) => {
                delta.padding.top = length_spec_from_lpa(value);
            }
            Property::PaddingRight(value) => {
                delta.padding.right = length_spec_from_lpa(value);
            }
            Property::PaddingBottom(value) => {
                delta.padding.bottom = length_spec_from_lpa(value);
            }
            Property::PaddingLeft(value) => {
                delta.padding.left = length_spec_from_lpa(value);
            }
            Property::Padding(value) => {
                delta.padding.top = length_spec_from_lpa(&value.top);
                delta.padding.right = length_spec_from_lpa(&value.right);
                delta.padding.bottom = length_spec_from_lpa(&value.bottom);
                delta.padding.left = length_spec_from_lpa(&value.left);
            }
            Property::BorderWidth(value) => {
                delta.border_width.top = border_width_spec(&value.top);
                delta.border_width.right = border_width_spec(&value.right);
                delta.border_width.bottom = border_width_spec(&value.bottom);
                delta.border_width.left = border_width_spec(&value.left);
            }
            Property::BorderTopWidth(value) => {
                delta.border_width.top = border_width_spec(value);
            }
            Property::BorderRightWidth(value) => {
                delta.border_width.right = border_width_spec(value);
            }
            Property::BorderBottomWidth(value) => {
                delta.border_width.bottom = border_width_spec(value);
            }
            Property::BorderLeftWidth(value) => {
                delta.border_width.left = border_width_spec(value);
            }
            Property::BorderBlockStartWidth(value) => {
                delta.border_width.top = border_width_spec(value);
            }
            Property::BorderBlockEndWidth(value) => {
                delta.border_width.bottom = border_width_spec(value);
            }
            Property::BorderInlineStartWidth(value) => {
                delta.border_width.left = border_width_spec(value);
            }
            Property::BorderInlineEndWidth(value) => {
                delta.border_width.right = border_width_spec(value);
            }
            Property::BorderColor(value) => {
                if let Ok(raw) = value.top.to_css_string(PrinterOptions::default()) {
                    if raw.to_ascii_lowercase().contains("var(") {
                        if let Some((color, alpha)) = parse_color_string(&raw) {
                            delta.border_color =
                                Some(ColorSpec::Value(blend_over_white(color, alpha)));
                        } else {
                            delta.border_color_var = Some(raw.trim().to_ascii_lowercase());
                        }
                        continue;
                    }
                    if let Some((color, alpha)) = parse_color_string(&raw) {
                        delta.border_color = Some(ColorSpec::Value(blend_over_white(color, alpha)));
                        continue;
                    } else if let Some(var) = var_name_from_string(&raw) {
                        delta.border_color_var = Some(var);
                        continue;
                    }
                }
                if let Some(color) = css_color_to_color(&value.top) {
                    delta.border_color = Some(ColorSpec::Value(color));
                }
            }
            Property::BorderTopColor(value) => {
                if let Ok(raw) = value.to_css_string(PrinterOptions::default()) {
                    if raw.to_ascii_lowercase().contains("var(") {
                        if let Some((color, alpha)) = parse_color_string(&raw) {
                            delta.border_color =
                                Some(ColorSpec::Value(blend_over_white(color, alpha)));
                        } else {
                            delta.border_color_var = Some(raw.trim().to_ascii_lowercase());
                        }
                        continue;
                    }
                    if let Some((color, alpha)) = parse_color_string(&raw) {
                        delta.border_color = Some(ColorSpec::Value(blend_over_white(color, alpha)));
                        continue;
                    } else if let Some(var) = var_name_from_string(&raw) {
                        delta.border_color_var = Some(var);
                        continue;
                    }
                }
                if let Some(color) = css_color_to_color(value) {
                    delta.border_color = Some(ColorSpec::Value(color));
                }
            }
            Property::BorderRightColor(value) => {
                if let Ok(raw) = value.to_css_string(PrinterOptions::default()) {
                    if raw.to_ascii_lowercase().contains("var(") {
                        if let Some((color, alpha)) = parse_color_string(&raw) {
                            delta.border_color =
                                Some(ColorSpec::Value(blend_over_white(color, alpha)));
                        } else {
                            delta.border_color_var = Some(raw.trim().to_ascii_lowercase());
                        }
                        continue;
                    }
                    if let Some((color, alpha)) = parse_color_string(&raw) {
                        delta.border_color = Some(ColorSpec::Value(blend_over_white(color, alpha)));
                        continue;
                    } else if let Some(var) = var_name_from_string(&raw) {
                        delta.border_color_var = Some(var);
                        continue;
                    }
                }
                if let Some(color) = css_color_to_color(value) {
                    delta.border_color = Some(ColorSpec::Value(color));
                }
            }
            Property::BorderBottomColor(value) => {
                if let Ok(raw) = value.to_css_string(PrinterOptions::default()) {
                    if raw.to_ascii_lowercase().contains("var(") {
                        if let Some((color, alpha)) = parse_color_string(&raw) {
                            delta.border_color =
                                Some(ColorSpec::Value(blend_over_white(color, alpha)));
                        } else {
                            delta.border_color_var = Some(raw.trim().to_ascii_lowercase());
                        }
                        continue;
                    }
                    if let Some((color, alpha)) = parse_color_string(&raw) {
                        delta.border_color = Some(ColorSpec::Value(blend_over_white(color, alpha)));
                        continue;
                    } else if let Some(var) = var_name_from_string(&raw) {
                        delta.border_color_var = Some(var);
                        continue;
                    }
                }
                if let Some(color) = css_color_to_color(value) {
                    delta.border_color = Some(ColorSpec::Value(color));
                }
            }
            Property::BorderLeftColor(value) => {
                if let Ok(raw) = value.to_css_string(PrinterOptions::default()) {
                    if raw.to_ascii_lowercase().contains("var(") {
                        if let Some((color, alpha)) = parse_color_string(&raw) {
                            delta.border_color =
                                Some(ColorSpec::Value(blend_over_white(color, alpha)));
                        } else {
                            delta.border_color_var = Some(raw.trim().to_ascii_lowercase());
                        }
                        continue;
                    }
                    if let Some((color, alpha)) = parse_color_string(&raw) {
                        delta.border_color = Some(ColorSpec::Value(blend_over_white(color, alpha)));
                        continue;
                    } else if let Some(var) = var_name_from_string(&raw) {
                        delta.border_color_var = Some(var);
                        continue;
                    }
                }
                if let Some(color) = css_color_to_color(value) {
                    delta.border_color = Some(ColorSpec::Value(color));
                }
            }
            Property::BorderBlockStartColor(value) => {
                if let Ok(raw) = value.to_css_string(PrinterOptions::default()) {
                    if raw.to_ascii_lowercase().contains("var(") {
                        if let Some((color, alpha)) = parse_color_string(&raw) {
                            delta.border_color =
                                Some(ColorSpec::Value(blend_over_white(color, alpha)));
                        } else {
                            delta.border_color_var = Some(raw.trim().to_ascii_lowercase());
                        }
                        continue;
                    }
                    if let Some((color, alpha)) = parse_color_string(&raw) {
                        delta.border_color = Some(ColorSpec::Value(blend_over_white(color, alpha)));
                        continue;
                    } else if let Some(var) = var_name_from_string(&raw) {
                        delta.border_color_var = Some(var);
                        continue;
                    }
                }
                if let Some(color) = css_color_to_color(value) {
                    delta.border_color = Some(ColorSpec::Value(color));
                }
            }
            Property::BorderBlockEndColor(value) => {
                if let Ok(raw) = value.to_css_string(PrinterOptions::default()) {
                    if raw.to_ascii_lowercase().contains("var(") {
                        if let Some((color, alpha)) = parse_color_string(&raw) {
                            delta.border_color =
                                Some(ColorSpec::Value(blend_over_white(color, alpha)));
                        } else {
                            delta.border_color_var = Some(raw.trim().to_ascii_lowercase());
                        }
                        continue;
                    }
                    if let Some((color, alpha)) = parse_color_string(&raw) {
                        delta.border_color = Some(ColorSpec::Value(blend_over_white(color, alpha)));
                        continue;
                    } else if let Some(var) = var_name_from_string(&raw) {
                        delta.border_color_var = Some(var);
                        continue;
                    }
                }
                if let Some(color) = css_color_to_color(value) {
                    delta.border_color = Some(ColorSpec::Value(color));
                }
            }
            Property::BorderInlineStartColor(value) => {
                if let Ok(raw) = value.to_css_string(PrinterOptions::default()) {
                    if raw.to_ascii_lowercase().contains("var(") {
                        if let Some((color, alpha)) = parse_color_string(&raw) {
                            delta.border_color =
                                Some(ColorSpec::Value(blend_over_white(color, alpha)));
                        } else {
                            delta.border_color_var = Some(raw.trim().to_ascii_lowercase());
                        }
                        continue;
                    }
                    if let Some((color, alpha)) = parse_color_string(&raw) {
                        delta.border_color = Some(ColorSpec::Value(blend_over_white(color, alpha)));
                        continue;
                    } else if let Some(var) = var_name_from_string(&raw) {
                        delta.border_color_var = Some(var);
                        continue;
                    }
                }
                if let Some(color) = css_color_to_color(value) {
                    delta.border_color = Some(ColorSpec::Value(color));
                }
            }
            Property::BorderInlineEndColor(value) => {
                if let Ok(raw) = value.to_css_string(PrinterOptions::default()) {
                    if raw.to_ascii_lowercase().contains("var(") {
                        if let Some((color, alpha)) = parse_color_string(&raw) {
                            delta.border_color =
                                Some(ColorSpec::Value(blend_over_white(color, alpha)));
                        } else {
                            delta.border_color_var = Some(raw.trim().to_ascii_lowercase());
                        }
                        continue;
                    }
                    if let Some((color, alpha)) = parse_color_string(&raw) {
                        delta.border_color = Some(ColorSpec::Value(blend_over_white(color, alpha)));
                        continue;
                    } else if let Some(var) = var_name_from_string(&raw) {
                        delta.border_color_var = Some(var);
                        continue;
                    }
                }
                if let Some(color) = css_color_to_color(value) {
                    delta.border_color = Some(ColorSpec::Value(color));
                }
            }
            Property::BorderStyle(value) => {
                delta.border_style.top = Some(border_line_style_from_line_style(&value.top));
                delta.border_style.right = Some(border_line_style_from_line_style(&value.right));
                delta.border_style.bottom = Some(border_line_style_from_line_style(&value.bottom));
                delta.border_style.left = Some(border_line_style_from_line_style(&value.left));
            }
            Property::BorderTopStyle(value) => {
                delta.border_style.top = Some(border_line_style_from_line_style(value));
            }
            Property::BorderRightStyle(value) => {
                delta.border_style.right = Some(border_line_style_from_line_style(value));
            }
            Property::BorderBottomStyle(value) => {
                delta.border_style.bottom = Some(border_line_style_from_line_style(value));
            }
            Property::BorderLeftStyle(value) => {
                delta.border_style.left = Some(border_line_style_from_line_style(value));
            }
            Property::BorderBlockStartStyle(value) => {
                delta.border_style.top = Some(border_line_style_from_line_style(value));
            }
            Property::BorderBlockEndStyle(value) => {
                delta.border_style.bottom = Some(border_line_style_from_line_style(value));
            }
            Property::BorderInlineStartStyle(value) => {
                delta.border_style.left = Some(border_line_style_from_line_style(value));
            }
            Property::BorderInlineEndStyle(value) => {
                delta.border_style.right = Some(border_line_style_from_line_style(value));
            }
            Property::Border(value) => {
                let w = border_width_spec(&value.width);
                delta.border_width.top = w;
                delta.border_width.right = w;
                delta.border_width.bottom = w;
                delta.border_width.left = w;
                if let Some(color) = css_color_to_color(&value.color) {
                    delta.border_color = Some(ColorSpec::Value(color));
                }
                let style = border_line_style_from_line_style(&value.style);
                delta.border_style.top = Some(style);
                delta.border_style.right = Some(style);
                delta.border_style.bottom = Some(style);
                delta.border_style.left = Some(style);
            }
            Property::BorderTop(value) => {
                delta.border_width.top = border_width_spec(&value.width);
                if let Some(color) = css_color_to_color(&value.color) {
                    delta.border_color = Some(ColorSpec::Value(color));
                }
                delta.border_style.top = Some(border_line_style_from_line_style(&value.style));
            }
            Property::BorderRight(value) => {
                delta.border_width.right = border_width_spec(&value.width);
                if let Some(color) = css_color_to_color(&value.color) {
                    delta.border_color = Some(ColorSpec::Value(color));
                }
                delta.border_style.right = Some(border_line_style_from_line_style(&value.style));
            }
            Property::BorderBottom(value) => {
                delta.border_width.bottom = border_width_spec(&value.width);
                if let Some(color) = css_color_to_color(&value.color) {
                    delta.border_color = Some(ColorSpec::Value(color));
                }
                delta.border_style.bottom = Some(border_line_style_from_line_style(&value.style));
            }
            Property::BorderLeft(value) => {
                delta.border_width.left = border_width_spec(&value.width);
                if let Some(color) = css_color_to_color(&value.color) {
                    delta.border_color = Some(ColorSpec::Value(color));
                }
                delta.border_style.left = Some(border_line_style_from_line_style(&value.style));
            }
            Property::BorderBlockStart(value) => {
                delta.border_width.top = border_width_spec(&value.width);
                if let Some(color) = css_color_to_color(&value.color) {
                    delta.border_color = Some(ColorSpec::Value(color));
                }
                delta.border_style.top = Some(border_line_style_from_line_style(&value.style));
            }
            Property::BorderBlockEnd(value) => {
                delta.border_width.bottom = border_width_spec(&value.width);
                if let Some(color) = css_color_to_color(&value.color) {
                    delta.border_color = Some(ColorSpec::Value(color));
                }
                delta.border_style.bottom = Some(border_line_style_from_line_style(&value.style));
            }
            Property::BorderInlineStart(value) => {
                delta.border_width.left = border_width_spec(&value.width);
                if let Some(color) = css_color_to_color(&value.color) {
                    delta.border_color = Some(ColorSpec::Value(color));
                }
                delta.border_style.left = Some(border_line_style_from_line_style(&value.style));
            }
            Property::BorderInlineEnd(value) => {
                delta.border_width.right = border_width_spec(&value.width);
                if let Some(color) = css_color_to_color(&value.color) {
                    delta.border_color = Some(ColorSpec::Value(color));
                }
                delta.border_style.right = Some(border_line_style_from_line_style(&value.style));
            }
            Property::Custom(custom) => {
                let property_name = custom.name.as_ref().to_ascii_lowercase();
                apply_custom_property(&property_name, &custom.value.0, delta);
            }
            Property::Unparsed(unparsed) => match &unparsed.property_id {
                PropertyId::FontSize => {
                    apply_inherit_initial_font_size(&unparsed.value.0, delta);
                }
                PropertyId::LineHeight => {
                    apply_inherit_initial_line_height(&unparsed.value.0, delta);
                }
                PropertyId::Color => {
                    if let Some(color) = color_from_tokens(&unparsed.value.0) {
                        delta.color = Some(ColorSpec::Value(color));
                    } else {
                        let raw = tokens_debug_string(&unparsed.value.0);
                        if let Some((color, alpha)) = parse_color_string(&raw) {
                            delta.color = Some(ColorSpec::Value(blend_over_white(color, alpha)));
                        } else if raw.to_ascii_lowercase().contains("var(") {
                            delta.color_var = Some(raw.to_ascii_lowercase());
                        } else if let Some(var) = var_name_from_tokens(&unparsed.value.0) {
                            delta.color_var = Some(var);
                        } else {
                            apply_inherit_initial_color(&unparsed.value.0, delta);
                        }
                    }
                }
                PropertyId::BackgroundColor => {
                    if let Some(color) = color_from_tokens(&unparsed.value.0) {
                        delta.background_color = Some(BackgroundSpec::Value(color));
                    } else {
                        let raw = tokens_debug_string(&unparsed.value.0);
                        if let Some((color, alpha)) = parse_color_string(&raw) {
                            delta.background_color =
                                Some(BackgroundSpec::Value(blend_over_white(color, alpha)));
                        } else if raw.to_ascii_lowercase().contains("var(") {
                            delta.background_color_var = Some(raw.to_ascii_lowercase());
                        } else if let Some(var) = var_name_from_tokens(&unparsed.value.0) {
                            delta.background_color_var = Some(var);
                        } else {
                            apply_inherit_initial_background_color(&unparsed.value.0, delta);
                        }
                    }
                }
                PropertyId::Background => {
                    if let Some(color) = color_from_tokens(&unparsed.value.0) {
                        delta.background_color = Some(BackgroundSpec::Value(color));
                    } else {
                        let raw = tokens_debug_string(&unparsed.value.0);
                        if let Some((color, alpha)) = parse_color_string(&raw) {
                            delta.background_color =
                                Some(BackgroundSpec::Value(blend_over_white(color, alpha)));
                        } else if raw.to_ascii_lowercase().contains("var(") {
                            delta.background_color_var = Some(raw.to_ascii_lowercase());
                        } else if let Some(var) = var_name_from_tokens(&unparsed.value.0) {
                            delta.background_color_var = Some(var);
                        }
                    }
                }
                PropertyId::FontFamily => {
                    if let Some(var) = var_name_from_tokens(&unparsed.value.0) {
                        delta.font_name_var = Some(var);
                    } else {
                        apply_inherit_initial_font_name(&unparsed.value.0, delta);
                    }
                }
                PropertyId::WhiteSpace => {
                    apply_inherit_initial_white_space(&unparsed.value.0, delta);
                }
                PropertyId::Display => {
                    apply_inherit_initial_display(&unparsed.value.0, delta);
                }
                PropertyId::Width => {
                    if let Some(var) = var_name_from_tokens(&unparsed.value.0) {
                        delta.width_var = Some(var);
                    }
                }
                PropertyId::MaxWidth => {
                    if let Some(var) = var_name_from_tokens(&unparsed.value.0) {
                        delta.max_width_var = Some(var);
                    }
                }
                PropertyId::Height => {
                    if let Some(var) = var_name_from_tokens(&unparsed.value.0) {
                        delta.height_var = Some(var);
                    }
                }
                PropertyId::FontWeight => {
                    apply_inherit_initial_font_weight(&unparsed.value.0, delta);
                }
                PropertyId::FontStyle => {
                    apply_inherit_initial_font_style(&unparsed.value.0, delta);
                }
                PropertyId::TextTransform => {
                    apply_inherit_initial_text_transform(&unparsed.value.0, delta);
                }
                PropertyId::TextDecorationLine(_) | PropertyId::TextDecoration(_) => {
                    apply_inherit_initial_text_decoration(&unparsed.value.0, delta);
                }
                PropertyId::TextOverflow(_) => {
                    apply_inherit_initial_text_overflow(&unparsed.value.0, delta);
                }
                PropertyId::LetterSpacing => {
                    apply_inherit_initial_letter_spacing(&unparsed.value.0, delta);
                }
                PropertyId::Border
                | PropertyId::BorderTop
                | PropertyId::BorderRight
                | PropertyId::BorderBottom
                | PropertyId::BorderLeft
                | PropertyId::BorderBlockStart
                | PropertyId::BorderBlockEnd
                | PropertyId::BorderInlineStart
                | PropertyId::BorderInlineEnd => {
                    apply_inherit_initial_border_color(&unparsed.value.0, delta);
                    if let Some(spec) = length_spec_from_custom_tokens(&unparsed.value.0) {
                        match &unparsed.property_id {
                            PropertyId::Border => {
                                delta.border_width.top = Some(spec);
                                delta.border_width.right = Some(spec);
                                delta.border_width.bottom = Some(spec);
                                delta.border_width.left = Some(spec);
                            }
                            PropertyId::BorderTop | PropertyId::BorderBlockStart => {
                                delta.border_width.top = Some(spec)
                            }
                            PropertyId::BorderRight | PropertyId::BorderInlineEnd => {
                                delta.border_width.right = Some(spec)
                            }
                            PropertyId::BorderBottom | PropertyId::BorderBlockEnd => {
                                delta.border_width.bottom = Some(spec)
                            }
                            PropertyId::BorderLeft | PropertyId::BorderInlineStart => {
                                delta.border_width.left = Some(spec)
                            }
                            _ => {}
                        }
                    } else if let Some(expr) = length_var_expr_from_tokens(&unparsed.value.0) {
                        match &unparsed.property_id {
                            PropertyId::Border => {
                                delta.border_width.top_var = Some(expr.clone());
                                delta.border_width.right_var = Some(expr.clone());
                                delta.border_width.bottom_var = Some(expr.clone());
                                delta.border_width.left_var = Some(expr);
                            }
                            PropertyId::BorderTop | PropertyId::BorderBlockStart => {
                                delta.border_width.top_var = Some(expr)
                            }
                            PropertyId::BorderRight | PropertyId::BorderInlineEnd => {
                                delta.border_width.right_var = Some(expr)
                            }
                            PropertyId::BorderBottom | PropertyId::BorderBlockEnd => {
                                delta.border_width.bottom_var = Some(expr)
                            }
                            PropertyId::BorderLeft | PropertyId::BorderInlineStart => {
                                delta.border_width.left_var = Some(expr)
                            }
                            _ => {}
                        }
                    }
                    if let Some(color) = color_from_tokens(&unparsed.value.0) {
                        delta.border_color = Some(ColorSpec::Value(color));
                    } else {
                        let raw = tokens_debug_string(&unparsed.value.0);
                        if let Some((color, alpha)) = parse_color_string(&raw) {
                            delta.border_color =
                                Some(ColorSpec::Value(blend_over_white(color, alpha)));
                        } else if let Some(var) = last_var_name_from_tokens(&unparsed.value.0)
                            .or_else(|| var_name_from_tokens(&unparsed.value.0))
                        {
                            delta.border_color_var = Some(var);
                        }
                    }
                    if let Some(style) = border_line_style_from_tokens(&unparsed.value.0) {
                        match &unparsed.property_id {
                            PropertyId::Border => {
                                delta.border_style.top = Some(style);
                                delta.border_style.right = Some(style);
                                delta.border_style.bottom = Some(style);
                                delta.border_style.left = Some(style);
                            }
                            PropertyId::BorderTop | PropertyId::BorderBlockStart => {
                                delta.border_style.top = Some(style);
                            }
                            PropertyId::BorderRight | PropertyId::BorderInlineEnd => {
                                delta.border_style.right = Some(style);
                            }
                            PropertyId::BorderBottom | PropertyId::BorderBlockEnd => {
                                delta.border_style.bottom = Some(style);
                            }
                            PropertyId::BorderLeft | PropertyId::BorderInlineStart => {
                                delta.border_style.left = Some(style);
                            }
                            _ => {}
                        }
                    }
                }
                PropertyId::BorderColor
                | PropertyId::BorderTopColor
                | PropertyId::BorderRightColor
                | PropertyId::BorderBottomColor
                | PropertyId::BorderLeftColor
                | PropertyId::BorderBlockStartColor
                | PropertyId::BorderBlockEndColor
                | PropertyId::BorderInlineStartColor
                | PropertyId::BorderInlineEndColor => {
                    apply_inherit_initial_border_color(&unparsed.value.0, delta);
                    if let Some(color) = color_from_tokens(&unparsed.value.0) {
                        delta.border_color = Some(ColorSpec::Value(color));
                    } else {
                        let raw = tokens_debug_string(&unparsed.value.0);
                        if let Some((color, alpha)) = parse_color_string(&raw) {
                            delta.border_color =
                                Some(ColorSpec::Value(blend_over_white(color, alpha)));
                        } else if raw.to_ascii_lowercase().contains("var(") {
                            delta.border_color_var = Some(raw.to_ascii_lowercase());
                        } else if let Some(var) = last_var_name_from_tokens(&unparsed.value.0)
                            .or_else(|| var_name_from_tokens(&unparsed.value.0))
                        {
                            delta.border_color_var = Some(var);
                        }
                    }
                }
                PropertyId::BorderWidth
                | PropertyId::BorderTopWidth
                | PropertyId::BorderRightWidth
                | PropertyId::BorderBottomWidth
                | PropertyId::BorderLeftWidth
                | PropertyId::BorderBlockStartWidth
                | PropertyId::BorderBlockEndWidth
                | PropertyId::BorderInlineStartWidth
                | PropertyId::BorderInlineEndWidth => {
                    if let Some(spec) = length_spec_from_custom_tokens(&unparsed.value.0) {
                        match &unparsed.property_id {
                            PropertyId::BorderWidth => {
                                delta.border_width.top = Some(spec);
                                delta.border_width.right = Some(spec);
                                delta.border_width.bottom = Some(spec);
                                delta.border_width.left = Some(spec);
                            }
                            PropertyId::BorderTopWidth | PropertyId::BorderBlockStartWidth => {
                                delta.border_width.top = Some(spec)
                            }
                            PropertyId::BorderRightWidth | PropertyId::BorderInlineEndWidth => {
                                delta.border_width.right = Some(spec)
                            }
                            PropertyId::BorderBottomWidth | PropertyId::BorderBlockEndWidth => {
                                delta.border_width.bottom = Some(spec)
                            }
                            PropertyId::BorderLeftWidth | PropertyId::BorderInlineStartWidth => {
                                delta.border_width.left = Some(spec)
                            }
                            _ => {}
                        }
                    } else if let Some(expr) = length_var_expr_from_tokens(&unparsed.value.0) {
                        match &unparsed.property_id {
                            PropertyId::BorderWidth => {
                                delta.border_width.top_var = Some(expr.clone());
                                delta.border_width.right_var = Some(expr.clone());
                                delta.border_width.bottom_var = Some(expr.clone());
                                delta.border_width.left_var = Some(expr);
                            }
                            PropertyId::BorderTopWidth | PropertyId::BorderBlockStartWidth => {
                                delta.border_width.top_var = Some(expr)
                            }
                            PropertyId::BorderRightWidth | PropertyId::BorderInlineEndWidth => {
                                delta.border_width.right_var = Some(expr)
                            }
                            PropertyId::BorderBottomWidth | PropertyId::BorderBlockEndWidth => {
                                delta.border_width.bottom_var = Some(expr)
                            }
                            PropertyId::BorderLeftWidth | PropertyId::BorderInlineStartWidth => {
                                delta.border_width.left_var = Some(expr)
                            }
                            _ => {}
                        }
                    }
                }
                PropertyId::BorderStyle
                | PropertyId::BorderTopStyle
                | PropertyId::BorderRightStyle
                | PropertyId::BorderBottomStyle
                | PropertyId::BorderLeftStyle
                | PropertyId::BorderBlockStartStyle
                | PropertyId::BorderBlockEndStyle
                | PropertyId::BorderInlineStartStyle
                | PropertyId::BorderInlineEndStyle => {
                    if let Some(style) = border_line_style_from_tokens(&unparsed.value.0) {
                        match &unparsed.property_id {
                            PropertyId::BorderStyle => {
                                delta.border_style.top = Some(style);
                                delta.border_style.right = Some(style);
                                delta.border_style.bottom = Some(style);
                                delta.border_style.left = Some(style);
                            }
                            PropertyId::BorderTopStyle | PropertyId::BorderBlockStartStyle => {
                                delta.border_style.top = Some(style);
                            }
                            PropertyId::BorderRightStyle | PropertyId::BorderInlineEndStyle => {
                                delta.border_style.right = Some(style);
                            }
                            PropertyId::BorderBottomStyle | PropertyId::BorderBlockEndStyle => {
                                delta.border_style.bottom = Some(style);
                            }
                            PropertyId::BorderLeftStyle | PropertyId::BorderInlineStartStyle => {
                                delta.border_style.left = Some(style);
                            }
                            _ => {}
                        }
                    }
                }
                PropertyId::FlexDirection(_) => {
                    if let Some(value) = first_ident(&unparsed.value.0) {
                        delta.flex_direction = Some(match value.as_str() {
                            "column" => FlexDirectionMode::Column,
                            _ => FlexDirectionMode::Row,
                        });
                    }
                }
                PropertyId::FlexWrap(_) => {
                    if let Some(value) = first_ident(&unparsed.value.0) {
                        delta.flex_wrap = Some(match value.as_str() {
                            "wrap" | "wrap-reverse" => FlexWrapMode::Wrap,
                            _ => FlexWrapMode::NoWrap,
                        });
                    }
                }
                PropertyId::Order(_) | PropertyId::FlexOrder(_) => {
                    if let Some(value) = first_number(&unparsed.value.0) {
                        delta.order = Some(value.round() as i32);
                    }
                }
                PropertyId::JustifyContent(_) => {
                    if let Some(value) = first_ident(&unparsed.value.0) {
                        delta.justify_content = Some(match value.as_str() {
                            "flex-end" | "end" => JustifyContentMode::FlexEnd,
                            "center" => JustifyContentMode::Center,
                            "space-between" => JustifyContentMode::SpaceBetween,
                            _ => JustifyContentMode::FlexStart,
                        });
                    }
                }
                PropertyId::AlignItems(_) => {
                    if let Some(value) = first_ident(&unparsed.value.0) {
                        delta.align_items = Some(match value.as_str() {
                            "flex-end" | "end" => AlignItemsMode::FlexEnd,
                            "center" => AlignItemsMode::Center,
                            "stretch" => AlignItemsMode::Stretch,
                            _ => AlignItemsMode::FlexStart,
                        });
                    }
                }
                PropertyId::GridTemplateColumns => {
                    let raw = tokens_debug_string(&unparsed.value.0);
                    delta.grid_columns = parse_grid_track_count(&raw);
                }
                PropertyId::BoxSizing(_) => {
                    if let Some(value) = first_ident(&unparsed.value.0) {
                        delta.box_sizing = Some(match value.as_str() {
                            "border-box" => BoxSizingMode::BorderBox,
                            _ => BoxSizingMode::ContentBox,
                        });
                    }
                }
                PropertyId::Gap => {
                    if let Some(spec) = length_spec_from_custom_tokens(&unparsed.value.0) {
                        delta.gap = Some(spec);
                    }
                }
                PropertyId::BorderSpacing => {
                    let raw = tokens_debug_string(&unparsed.value.0);
                    if let Some(spacing) = parse_border_spacing_str(&raw) {
                        delta.border_spacing = Some(spacing);
                    }
                }
                PropertyId::BorderRadius(_) => {
                    let raw = tokens_debug_string(&unparsed.value.0);
                    if let Some(radius) = parse_border_radius_str(&raw) {
                        delta.border_radius = Some(radius);
                    }
                }
                PropertyId::BoxShadow(_) => {
                    let raw = tokens_debug_string(&unparsed.value.0);
                    if let Some(shadow) = parse_box_shadow_str(&raw) {
                        delta.box_shadow = Some(shadow);
                    }
                }
                PropertyId::Custom(name) => {
                    let name = name.as_ref().to_ascii_lowercase();
                    apply_custom_property(&name, &unparsed.value.0, delta);
                }
                PropertyId::Flex(_) => {
                    if let Some(value) = first_number(&unparsed.value.0) {
                        delta.flex_grow = Some(value);
                    }
                    if let Some(spec) = length_spec_from_custom_tokens(&unparsed.value.0) {
                        delta.flex_basis = Some(spec);
                    } else if let Some(value) = first_ident(&unparsed.value.0) {
                        if value.eq_ignore_ascii_case("auto") {
                            delta.flex_basis = Some(LengthSpec::Auto);
                        }
                    }
                }
                PropertyId::FlexBasis(_) => {
                    if let Some(spec) = length_spec_from_custom_tokens(&unparsed.value.0) {
                        delta.flex_basis = Some(spec);
                    } else if let Some(value) = first_ident(&unparsed.value.0) {
                        if value.eq_ignore_ascii_case("auto") {
                            delta.flex_basis = Some(LengthSpec::Auto);
                        }
                    }
                }
                PropertyId::Margin
                | PropertyId::MarginTop
                | PropertyId::MarginRight
                | PropertyId::MarginBottom
                | PropertyId::MarginLeft => {
                    apply_inherit_initial_edge(
                        &unparsed.value.0,
                        &unparsed.property_id,
                        delta,
                        true,
                    );
                }
                PropertyId::Padding
                | PropertyId::PaddingTop
                | PropertyId::PaddingRight
                | PropertyId::PaddingBottom
                | PropertyId::PaddingLeft => {
                    apply_inherit_initial_edge(
                        &unparsed.value.0,
                        &unparsed.property_id,
                        delta,
                        false,
                    );
                }
                _ => {}
            },
            _ => {}
        }
    }
}

fn apply_custom_property(property_name: &str, tokens: &[TokenOrValue], delta: &mut StyleDelta) {
    match property_name {
        "border-collapse" => {
            if let Some(value) = first_ident(tokens) {
                delta.border_collapse = Some(match value.as_str() {
                    "collapse" => BorderCollapseMode::Collapse,
                    _ => BorderCollapseMode::Separate,
                });
            }
        }
        "caption-side" => {
            if let Some(value) = first_ident(tokens) {
                delta.caption_side = Some(match value.as_str() {
                    "bottom" => CaptionSideMode::Bottom,
                    _ => CaptionSideMode::Top,
                });
            }
        }
        "border-spacing" => {
            let raw = tokens_debug_string(tokens);
            if let Some(spacing) = parse_border_spacing_str(&raw) {
                delta.border_spacing = Some(spacing);
            }
        }
        "border-radius" => {
            let raw = tokens_debug_string(tokens);
            if let Some(radius) = parse_border_radius_str(&raw) {
                delta.border_radius = Some(radius);
            }
        }
        "border"
        | "border-top"
        | "border-right"
        | "border-bottom"
        | "border-left"
        | "border-block-start"
        | "border-block-end"
        | "border-inline-start"
        | "border-inline-end" => {
            if let Some(spec) = length_spec_from_custom_tokens(tokens) {
                match property_name {
                    "border" => {
                        delta.border_width.top = Some(spec);
                        delta.border_width.right = Some(spec);
                        delta.border_width.bottom = Some(spec);
                        delta.border_width.left = Some(spec);
                    }
                    "border-top" | "border-block-start" => delta.border_width.top = Some(spec),
                    "border-right" | "border-inline-end" => delta.border_width.right = Some(spec),
                    "border-bottom" | "border-block-end" => delta.border_width.bottom = Some(spec),
                    "border-left" | "border-inline-start" => delta.border_width.left = Some(spec),
                    _ => {}
                }
            } else if let Some(expr) = length_var_expr_from_tokens(tokens) {
                match property_name {
                    "border" => {
                        delta.border_width.top_var = Some(expr.clone());
                        delta.border_width.right_var = Some(expr.clone());
                        delta.border_width.bottom_var = Some(expr.clone());
                        delta.border_width.left_var = Some(expr);
                    }
                    "border-top" | "border-block-start" => delta.border_width.top_var = Some(expr),
                    "border-right" | "border-inline-end" => {
                        delta.border_width.right_var = Some(expr)
                    }
                    "border-bottom" | "border-block-end" => {
                        delta.border_width.bottom_var = Some(expr)
                    }
                    "border-left" | "border-inline-start" => {
                        delta.border_width.left_var = Some(expr)
                    }
                    _ => {}
                }
            }
            if let Some(color) = color_from_tokens(tokens) {
                delta.border_color = Some(ColorSpec::Value(color));
            } else {
                let raw = tokens_debug_string(tokens);
                if let Some((color, alpha)) = parse_color_string(&raw) {
                    delta.border_color = Some(ColorSpec::Value(blend_over_white(color, alpha)));
                } else if let Some(var) =
                    last_var_name_from_tokens(tokens).or_else(|| var_name_from_tokens(tokens))
                {
                    delta.border_color_var = Some(var);
                }
            }
            if let Some(style) = border_line_style_from_tokens(tokens) {
                match property_name {
                    "border" => {
                        delta.border_style.top = Some(style);
                        delta.border_style.right = Some(style);
                        delta.border_style.bottom = Some(style);
                        delta.border_style.left = Some(style);
                    }
                    "border-top" | "border-block-start" => delta.border_style.top = Some(style),
                    "border-right" | "border-inline-end" => delta.border_style.right = Some(style),
                    "border-bottom" | "border-block-end" => delta.border_style.bottom = Some(style),
                    "border-left" | "border-inline-start" => delta.border_style.left = Some(style),
                    _ => {}
                }
            }
        }
        "border-color"
        | "border-top-color"
        | "border-right-color"
        | "border-bottom-color"
        | "border-left-color"
        | "border-block-start-color"
        | "border-block-end-color"
        | "border-inline-start-color"
        | "border-inline-end-color" => {
            if let Some(color) = color_from_tokens(tokens) {
                delta.border_color = Some(ColorSpec::Value(color));
            } else {
                let raw = tokens_debug_string(tokens);
                if let Some((color, alpha)) = parse_color_string(&raw) {
                    delta.border_color = Some(ColorSpec::Value(blend_over_white(color, alpha)));
                } else if raw.to_ascii_lowercase().contains("var(") {
                    delta.border_color_var = Some(raw.to_ascii_lowercase());
                } else if let Some(var) =
                    last_var_name_from_tokens(tokens).or_else(|| var_name_from_tokens(tokens))
                {
                    delta.border_color_var = Some(var);
                }
            }
        }
        "border-width"
        | "border-top-width"
        | "border-right-width"
        | "border-bottom-width"
        | "border-left-width"
        | "border-block-start-width"
        | "border-block-end-width"
        | "border-inline-start-width"
        | "border-inline-end-width" => {
            if let Some(spec) = length_spec_from_custom_tokens(tokens) {
                match property_name {
                    "border-width" => {
                        delta.border_width.top = Some(spec);
                        delta.border_width.right = Some(spec);
                        delta.border_width.bottom = Some(spec);
                        delta.border_width.left = Some(spec);
                    }
                    "border-top-width" | "border-block-start-width" => {
                        delta.border_width.top = Some(spec)
                    }
                    "border-right-width" | "border-inline-end-width" => {
                        delta.border_width.right = Some(spec)
                    }
                    "border-bottom-width" | "border-block-end-width" => {
                        delta.border_width.bottom = Some(spec)
                    }
                    "border-left-width" | "border-inline-start-width" => {
                        delta.border_width.left = Some(spec)
                    }
                    _ => {}
                }
            } else if let Some(expr) = length_var_expr_from_tokens(tokens) {
                match property_name {
                    "border-width" => {
                        delta.border_width.top_var = Some(expr.clone());
                        delta.border_width.right_var = Some(expr.clone());
                        delta.border_width.bottom_var = Some(expr.clone());
                        delta.border_width.left_var = Some(expr);
                    }
                    "border-top-width" | "border-block-start-width" => {
                        delta.border_width.top_var = Some(expr)
                    }
                    "border-right-width" | "border-inline-end-width" => {
                        delta.border_width.right_var = Some(expr)
                    }
                    "border-bottom-width" | "border-block-end-width" => {
                        delta.border_width.bottom_var = Some(expr)
                    }
                    "border-left-width" | "border-inline-start-width" => {
                        delta.border_width.left_var = Some(expr)
                    }
                    _ => {}
                }
            }
        }
        "border-style"
        | "border-top-style"
        | "border-right-style"
        | "border-bottom-style"
        | "border-left-style"
        | "border-block-start-style"
        | "border-block-end-style"
        | "border-inline-start-style"
        | "border-inline-end-style" => {
            if let Some(style) = border_line_style_from_tokens(tokens) {
                match property_name {
                    "border-style" => {
                        delta.border_style.top = Some(style);
                        delta.border_style.right = Some(style);
                        delta.border_style.bottom = Some(style);
                        delta.border_style.left = Some(style);
                    }
                    "border-top-style" | "border-block-start-style" => {
                        delta.border_style.top = Some(style)
                    }
                    "border-right-style" | "border-inline-end-style" => {
                        delta.border_style.right = Some(style)
                    }
                    "border-bottom-style" | "border-block-end-style" => {
                        delta.border_style.bottom = Some(style)
                    }
                    "border-left-style" | "border-inline-start-style" => {
                        delta.border_style.left = Some(style)
                    }
                    _ => {}
                }
            }
        }
        "box-shadow" => {
            let raw = tokens_debug_string(tokens);
            if let Some(shadow) = parse_box_shadow_str(&raw) {
                delta.box_shadow = Some(shadow);
            }
        }
        "break-before" | "page-break-before" => {
            if let Some(value) = first_ident(tokens) {
                delta.pagination.break_before = Some(match value.as_str() {
                    "page" | "always" => BreakBefore::Page,
                    _ => BreakBefore::Auto,
                });
            }
        }
        "break-after" | "page-break-after" => {
            if let Some(value) = first_ident(tokens) {
                delta.pagination.break_after = Some(match value.as_str() {
                    "page" | "always" => BreakAfter::Page,
                    _ => BreakAfter::Auto,
                });
            }
        }
        "break-inside" | "page-break-inside" => {
            if let Some(value) = first_ident(tokens) {
                delta.pagination.break_inside = Some(match value.as_str() {
                    "avoid" | "avoid-page" => BreakInside::Avoid,
                    _ => BreakInside::Auto,
                });
            }
        }
        "orphans" => {
            if let Some(value) = first_integer(tokens) {
                delta.pagination.orphans = Some(value.max(1) as usize);
            }
        }
        "widows" => {
            if let Some(value) = first_integer(tokens) {
                delta.pagination.widows = Some(value.max(1) as usize);
            }
        }
        "content" => {
            if let Some(spec) = content_from_tokens(tokens) {
                delta.content = Some(spec);
            }
        }
        _ => {
            if property_name.starts_with("--") {
                let name = property_name.to_string();
                if let Some(color) = color_from_tokens(tokens) {
                    delta.custom_colors.insert(name.clone(), color);
                    delta.custom_color_alpha.insert(name.clone(), 1.0);
                    delta.custom_color_refs.remove(&name);
                } else if let Some(var) = var_name_from_tokens(tokens) {
                    delta.custom_color_refs.insert(name.clone(), var);
                    delta.custom_colors.remove(&name);
                    delta.custom_color_alpha.remove(&name);
                } else {
                    let raw = tokens_debug_string(tokens);
                    let raw_lower = raw.trim().to_ascii_lowercase();
                    if let Some((color, alpha)) = parse_color_string(&raw) {
                        let blended = if alpha < 1.0 {
                            Color::rgb(
                                color.r * alpha + (1.0 - alpha),
                                color.g * alpha + (1.0 - alpha),
                                color.b * alpha + (1.0 - alpha),
                            )
                        } else {
                            color
                        };
                        delta.custom_colors.insert(name.clone(), blended);
                        delta
                            .custom_color_alpha
                            .insert(name.clone(), alpha.clamp(0.0, 1.0));
                        delta.custom_color_refs.remove(&name);
                    } else if let Some(color) = parse_rgb_triplet_string(&raw) {
                        delta.custom_colors.insert(name.clone(), color);
                        delta.custom_color_alpha.insert(name.clone(), 1.0);
                        delta.custom_color_refs.remove(&name);
                    } else if raw_lower.contains("var(")
                        && (raw_lower.starts_with("var(")
                            || raw_lower.starts_with("rgb(")
                            || raw_lower.starts_with("rgba(")
                            || raw_lower.starts_with("hsl(")
                            || raw_lower.starts_with("hsla("))
                    {
                        delta.custom_color_refs.insert(name.clone(), raw_lower);
                        delta.custom_colors.remove(&name);
                        delta.custom_color_alpha.remove(&name);
                    } else if let Some(spec) = length_spec_from_custom_tokens(tokens) {
                        delta.custom_lengths.insert(name.clone(), spec);
                    } else if let Some(stack) = font_stack_from_tokens(tokens) {
                        delta.custom_font_stacks.insert(name, stack);
                    }
                }
            }
        }
    }
}

fn overflow_mode_from_keyword(value: &css_overflow::OverflowKeyword) -> OverflowMode {
    match value {
        css_overflow::OverflowKeyword::Hidden | css_overflow::OverflowKeyword::Clip => {
            OverflowMode::Hidden
        }
        _ => OverflowMode::Visible,
    }
}

fn overflow_mode_from_overflow(value: &css_overflow::Overflow) -> OverflowMode {
    // `overflow:` is a shorthand that sets both axes. Treat any hidden/clip as hidden.
    if matches!(overflow_mode_from_keyword(&value.x), OverflowMode::Hidden)
        || matches!(overflow_mode_from_keyword(&value.y), OverflowMode::Hidden)
    {
        OverflowMode::Hidden
    } else {
        OverflowMode::Visible
    }
}

fn first_ident(tokens: &[TokenOrValue]) -> Option<String> {
    for token in tokens {
        match token {
            TokenOrValue::Token(Token::Ident(ident)) => {
                return Some(ident.as_ref().to_ascii_lowercase());
            }
            TokenOrValue::Token(Token::WhiteSpace(_)) => continue,
            _ => {}
        }
    }
    None
}

fn first_integer(tokens: &[TokenOrValue]) -> Option<i32> {
    for token in tokens {
        match token {
            TokenOrValue::Token(Token::Number {
                value, int_value, ..
            }) => {
                if let Some(int_value) = int_value {
                    return Some(*int_value);
                }
                return Some(*value as i32);
            }
            TokenOrValue::Token(Token::WhiteSpace(_)) => continue,
            _ => {}
        }
    }
    None
}

fn first_number(tokens: &[TokenOrValue]) -> Option<f32> {
    for token in tokens {
        match token {
            TokenOrValue::Token(Token::Number { value, .. }) => return Some(*value),
            TokenOrValue::Token(Token::WhiteSpace(_)) => continue,
            _ => {}
        }
    }
    None
}

fn content_from_tokens(tokens: &[TokenOrValue]) -> Option<ContentSpec> {
    let mut out = String::new();
    let mut saw_string = false;
    for token in tokens {
        match token {
            TokenOrValue::Token(Token::Ident(ident)) => {
                let value = ident.as_ref();
                if value.eq_ignore_ascii_case("none") || value.eq_ignore_ascii_case("normal") {
                    return Some(ContentSpec::None);
                }
                if value.eq_ignore_ascii_case("inherit") {
                    return Some(ContentSpec::Inherit);
                }
                if value.eq_ignore_ascii_case("initial") {
                    return Some(ContentSpec::Initial);
                }
            }
            TokenOrValue::Token(Token::String(value)) => {
                out.push_str(value.as_ref());
                saw_string = true;
            }
            TokenOrValue::Token(Token::WhiteSpace(_)) => continue,
            _ => {}
        }
    }
    if saw_string {
        Some(ContentSpec::Text(out))
    } else {
        None
    }
}

fn var_name_from_tokens(tokens: &[TokenOrValue]) -> Option<String> {
    let mut saw_var = false;
    for token in tokens {
        match token {
            TokenOrValue::Var(var) => {
                // lightningcss often parses `var(--x)` into a dedicated Var token.
                let name = var.name.ident.as_ref();
                if name.starts_with("--") {
                    return Some(name.to_ascii_lowercase());
                }
            }
            TokenOrValue::Function(func) => {
                if func.name.as_ref().eq_ignore_ascii_case("var") {
                    // Parse var(--name[, fallback]) and extract the first custom prop name.
                    for arg in &func.arguments.0 {
                        if let TokenOrValue::Token(Token::Ident(ident)) = arg {
                            let v = ident.as_ref();
                            if v.starts_with("--") {
                                return Some(v.to_ascii_lowercase());
                            }
                        }
                    }
                    saw_var = true;
                }
            }
            TokenOrValue::Token(Token::Ident(ident)) => {
                let v = ident.as_ref();
                if !saw_var && v.eq_ignore_ascii_case("var") {
                    saw_var = true;
                    continue;
                }
                if saw_var && v.starts_with("--") {
                    return Some(v.to_ascii_lowercase());
                }
            }
            // Most lightningcss versions tokenize `var(...)` as a function token.
            TokenOrValue::Token(Token::Function(name)) => {
                if name.as_ref().eq_ignore_ascii_case("var") {
                    saw_var = true;
                }
            }
            TokenOrValue::Token(Token::WhiteSpace(_)) => continue,
            _ => {}
        }
    }
    None
}

fn last_var_name_from_tokens(tokens: &[TokenOrValue]) -> Option<String> {
    let mut last: Option<String> = None;
    for token in tokens {
        match token {
            TokenOrValue::Var(var) => {
                let name = var.name.ident.as_ref();
                if name.starts_with("--") {
                    last = Some(name.to_ascii_lowercase());
                }
            }
            TokenOrValue::Function(func) => {
                if func.name.as_ref().eq_ignore_ascii_case("var") {
                    for arg in &func.arguments.0 {
                        if let TokenOrValue::Token(Token::Ident(ident)) = arg {
                            let v = ident.as_ref();
                            if v.starts_with("--") {
                                last = Some(v.to_ascii_lowercase());
                                break;
                            }
                        }
                    }
                } else {
                    let nested = last_var_name_from_tokens(&func.arguments.0);
                    if nested.is_some() {
                        last = nested;
                    }
                }
            }
            TokenOrValue::Token(Token::Ident(ident)) => {
                let v = ident.as_ref();
                if v.starts_with("--") {
                    last = Some(v.to_ascii_lowercase());
                }
            }
            _ => {}
        }
    }
    last
}

fn length_spec_from_custom_tokens(tokens: &[TokenOrValue]) -> Option<LengthSpec> {
    // Minimal parser for `40%`, `20px`, `12pt`, `10mm`, etc. Used for custom properties and gap.
    let mut i = 0usize;
    while i < tokens.len() {
        match &tokens[i] {
            TokenOrValue::Token(Token::WhiteSpace(_)) => {
                i += 1;
                continue;
            }
            TokenOrValue::Length(length) => {
                return match length {
                    LengthValue::Em(val) => Some(LengthSpec::Em(*val)),
                    LengthValue::Rem(val) => Some(LengthSpec::Rem(*val)),
                    _ => length_value_to_pt(length).map(LengthSpec::Absolute),
                };
            }
            TokenOrValue::Token(Token::Percentage { unit_value, .. }) => {
                return Some(LengthSpec::Percent(*unit_value));
            }
            TokenOrValue::Token(Token::Dimension { value, unit, .. }) => {
                let unit = unit.as_ref().to_ascii_lowercase();
                let v = *value;
                let pt = match unit.as_str() {
                    "px" => px_to_pt(v),
                    "pt" => Pt::from_f32(v),
                    "em" => return Some(LengthSpec::Em(v)),
                    "rem" => return Some(LengthSpec::Rem(v)),
                    "in" => Pt::from_f32(v * 72.0),
                    "cm" => Pt::from_f32(v * (72.0 / 2.54)),
                    "mm" => Pt::from_f32(v * (72.0 / 25.4)),
                    _ => return None,
                };
                return Some(LengthSpec::Absolute(pt));
            }
            TokenOrValue::Token(Token::Number { value, .. }) => {
                let v = *value;
                // Look ahead for `%` delimiter.
                let mut j = i + 1;
                while j < tokens.len() {
                    match &tokens[j] {
                        TokenOrValue::Token(Token::WhiteSpace(_)) => {
                            j += 1;
                            continue;
                        }
                        TokenOrValue::Token(Token::Delim(c)) if *c == '%' => {
                            return Some(LengthSpec::Percent(v / 100.0));
                        }
                        _ => break,
                    }
                }
                // Bare numbers are uncommon for lengths; treat as px for convenience.
                return Some(LengthSpec::Absolute(px_to_pt(v)));
            }
            _ => {}
        }
        i += 1;
    }
    None
}

#[derive(Debug, Clone)]
enum VarExprToken {
    Var(String),
    Number(f32),
    Op(char),
}

fn length_var_expr_from_tokens(tokens: &[TokenOrValue]) -> Option<LengthVarExpr> {
    let mut flat: Vec<VarExprToken> = Vec::new();
    flatten_var_expr_tokens(tokens, &mut flat);
    if flat.is_empty() {
        return None;
    }

    let mut var_index: Option<usize> = None;
    let mut var_name: Option<String> = None;
    for (idx, token) in flat.iter().enumerate() {
        if let VarExprToken::Var(name) = token {
            var_index = Some(idx);
            var_name = Some(name.clone());
            break;
        }
    }
    let var_index = var_index?;
    let name = var_name?;
    let mut scale = 1.0f32;

    if var_index >= 2 {
        if let (VarExprToken::Number(value), VarExprToken::Op(op)) =
            (&flat[var_index - 2], &flat[var_index - 1])
        {
            if *op == '*' {
                scale *= *value;
            }
        }
    }
    if var_index + 2 < flat.len() {
        if let (VarExprToken::Op(op), VarExprToken::Number(value)) =
            (&flat[var_index + 1], &flat[var_index + 2])
        {
            match *op {
                '*' => scale *= *value,
                '/' => {
                    if *value != 0.0 {
                        scale *= 1.0 / *value;
                    }
                }
                _ => {}
            }
        }
    }
    if var_index > 0 {
        if let VarExprToken::Op('-') = flat[var_index - 1] {
            if var_index == 1 {
                scale *= -1.0;
            } else if matches!(flat[var_index - 2], VarExprToken::Op(_)) {
                scale *= -1.0;
            }
        }
    }

    Some(LengthVarExpr { name, scale })
}

fn flatten_var_expr_tokens(tokens: &[TokenOrValue], out: &mut Vec<VarExprToken>) {
    let mut i = 0usize;
    while i < tokens.len() {
        match &tokens[i] {
            TokenOrValue::Token(Token::WhiteSpace(_)) => {
                i += 1;
                continue;
            }
            TokenOrValue::Token(Token::Comma) => {
                i += 1;
                continue;
            }
            TokenOrValue::Var(var) => {
                out.push(VarExprToken::Var(
                    var.name.ident.as_ref().to_ascii_lowercase(),
                ));
            }
            TokenOrValue::Function(func) => {
                let name = func.name.as_ref().to_ascii_lowercase();
                if name == "calc" {
                    flatten_var_expr_tokens(&func.arguments.0, out);
                } else if name == "var" {
                    if let Some(var_name) = var_name_from_tokens(&func.arguments.0) {
                        out.push(VarExprToken::Var(var_name));
                    }
                }
            }
            TokenOrValue::Token(Token::Function(name)) => {
                if name.as_ref().eq_ignore_ascii_case("calc") {
                    // lightningcss should surface calc arguments in Function; ignore otherwise.
                }
            }
            TokenOrValue::Token(Token::Ident(ident)) => {
                let value = ident.as_ref();
                if value.starts_with("--") {
                    out.push(VarExprToken::Var(value.to_ascii_lowercase()));
                }
            }
            TokenOrValue::Token(Token::Delim('-')) => {
                let mut j = i + 1;
                let mut handled = false;
                while j < tokens.len() {
                    match &tokens[j] {
                        TokenOrValue::Token(Token::WhiteSpace(_)) => {
                            j += 1;
                            continue;
                        }
                        TokenOrValue::Token(Token::Number { value, .. }) => {
                            out.push(VarExprToken::Number(-*value));
                            i = j;
                            handled = true;
                            break;
                        }
                        _ => {
                            out.push(VarExprToken::Op('-'));
                            handled = true;
                            break;
                        }
                    }
                }
                if !handled {
                    out.push(VarExprToken::Op('-'));
                }
            }
            TokenOrValue::Token(Token::Number { value, .. }) => {
                out.push(VarExprToken::Number(*value));
            }
            TokenOrValue::Token(Token::Delim(op)) => {
                if *op == '*' || *op == '/' {
                    out.push(VarExprToken::Op(*op));
                }
            }
            _ => {}
        }
        i += 1;
    }
}

fn scale_length_spec(spec: LengthSpec, scale: f32) -> LengthSpec {
    match spec {
        LengthSpec::Absolute(value) => LengthSpec::Absolute(value * scale),
        LengthSpec::Percent(pct) => LengthSpec::Percent(pct * scale),
        LengthSpec::Em(value) => LengthSpec::Em(value * scale),
        LengthSpec::Rem(value) => LengthSpec::Rem(value * scale),
        LengthSpec::Calc(calc) => LengthSpec::Calc(CalcLength {
            abs: calc.abs * scale,
            percent: calc.percent * scale,
            em: calc.em * scale,
            rem: calc.rem * scale,
        }),
        _ => spec,
    }
}

fn parse_rgb_triplet_string(raw: &str) -> Option<Color> {
    let parts: Vec<&str> = raw
        .split(',')
        .map(|p| p.trim())
        .filter(|p| !p.is_empty())
        .collect();
    if parts.len() != 3 {
        return None;
    }
    let r = parts[0].parse::<f32>().ok()?;
    let g = parts[1].parse::<f32>().ok()?;
    let b = parts[2].parse::<f32>().ok()?;
    Some(Color::rgb(r / 255.0, g / 255.0, b / 255.0))
}

fn font_stack_from_tokens(tokens: &[TokenOrValue]) -> Option<Vec<Arc<str>>> {
    let mut stack = Vec::new();
    let mut current = String::new();
    let push_current = |stack: &mut Vec<Arc<str>>, current: &mut String| {
        let cleaned = current.trim().trim_matches('"').trim_matches('\'');
        if !cleaned.is_empty() {
            let lower = cleaned.to_ascii_lowercase();
            let mapped = match lower.as_str() {
                "monospace" | "ui-monospace" => Some("Courier"),
                "serif" | "ui-serif" => Some("Times-Roman"),
                "sans-serif" | "ui-sans-serif" => Some("Helvetica"),
                _ => None,
            };
            let name = mapped.unwrap_or(cleaned);
            stack.push(Arc::<str>::from(name.to_string()));
        }
        current.clear();
    };
    for token in tokens {
        match token {
            TokenOrValue::Token(Token::Ident(ident)) => {
                if !current.is_empty() {
                    current.push(' ');
                }
                current.push_str(ident.as_ref());
            }
            TokenOrValue::Token(Token::String(s)) => {
                if !current.is_empty() {
                    current.push(' ');
                }
                current.push_str(s.as_ref());
            }
            TokenOrValue::Token(Token::WhiteSpace(_)) => {
                if !current.ends_with(' ') && !current.is_empty() {
                    current.push(' ');
                }
            }
            TokenOrValue::Token(Token::Comma) => {
                push_current(&mut stack, &mut current);
            }
            TokenOrValue::Token(Token::Delim(',')) => {
                push_current(&mut stack, &mut current);
            }
            _ => {}
        }
    }
    if !current.trim().is_empty() {
        push_current(&mut stack, &mut current);
    }
    if stack.is_empty() {
        None
    } else {
        let has_base14 = stack.iter().any(|name| {
            let lower = name.to_ascii_lowercase();
            lower == "helvetica" || lower == "times-roman" || lower == "courier"
        });
        if !has_base14 {
            stack.push(Arc::<str>::from("Helvetica"));
        }
        Some(stack)
    }
}

fn color_from_tokens(tokens: &[TokenOrValue]) -> Option<Color> {
    let mut i = 0usize;
    while i < tokens.len() {
        match &tokens[i] {
            TokenOrValue::Color(color) => {
                if let Some(c) = css_color_to_color(color) {
                    return Some(c);
                }
            }
            TokenOrValue::Function(func) => {
                let name = func.name.as_ref().to_ascii_lowercase();
                if name == "rgb" || name == "rgba" {
                    let args = tokens_debug_string(&func.arguments.0);
                    let raw = format!("{}({})", name, args);
                    if let Some((color, alpha)) = parse_color_string(&raw) {
                        return Some(if alpha < 1.0 {
                            Color::rgb(
                                color.r * alpha + (1.0 - alpha),
                                color.g * alpha + (1.0 - alpha),
                                color.b * alpha + (1.0 - alpha),
                            )
                        } else {
                            color
                        });
                    }
                }
            }
            TokenOrValue::Token(Token::Hash(hash)) => return parse_hex_color(hash.as_ref()),
            TokenOrValue::Token(Token::IDHash(hash)) => return parse_hex_color(hash.as_ref()),
            TokenOrValue::Token(Token::Delim('#')) => {
                let mut j = i + 1;
                while j < tokens.len() {
                    match &tokens[j] {
                        TokenOrValue::Token(Token::WhiteSpace(_)) => {
                            j += 1;
                            continue;
                        }
                        TokenOrValue::Token(Token::Ident(ident)) => {
                            if let Some(color) = parse_hex_color(ident.as_ref()) {
                                return Some(color);
                            }
                            break;
                        }
                        TokenOrValue::Token(Token::Number {
                            value, int_value, ..
                        }) => {
                            let raw = if let Some(int_value) = int_value {
                                int_value.to_string()
                            } else {
                                let v = *value;
                                if v.fract() == 0.0 {
                                    format!("{:.0}", v)
                                } else {
                                    format!("{v}")
                                }
                            };
                            if let Some(color) = parse_hex_color(&raw) {
                                return Some(color);
                            }
                            break;
                        }
                        _ => break,
                    }
                }
            }
            TokenOrValue::Token(Token::Ident(ident)) => {
                let name = ident.as_ref().to_ascii_lowercase();
                match name.as_str() {
                    "white" => return Some(Color::rgb(1.0, 1.0, 1.0)),
                    "black" => return Some(Color::BLACK),
                    "transparent" => return None,
                    "currentcolor" => return None,
                    _ => {}
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn collect_custom_properties(
    declarations: &lightningcss::declaration::DeclarationBlock,
) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for prop in declarations
        .declarations
        .iter()
        .chain(declarations.important_declarations.iter())
    {
        if let Property::Custom(custom) = prop {
            let name = custom.name.as_ref().to_string();
            let token_debug = tokens_debug_string(&custom.value.0);
            out.push((name, token_debug));
        }
    }
    out
}

fn tokens_debug_string(tokens: &[TokenOrValue]) -> String {
    let mut out = Vec::new();
    for token in tokens {
        let value = match token {
            TokenOrValue::Color(color) => css_color_to_color(color)
                .map(color_to_hex)
                .unwrap_or_else(|| "color".to_string()),
            TokenOrValue::Var(var) => {
                let mut rendered = format!("var({}", var.name.ident.as_ref());
                if let Some(fallback) = &var.fallback {
                    let fallback_css = tokens_debug_string(&fallback.0);
                    if !fallback_css.is_empty() {
                        rendered.push_str(", ");
                        rendered.push_str(&fallback_css);
                    }
                }
                rendered.push(')');
                rendered
            }
            TokenOrValue::Function(func) => {
                let args = tokens_debug_string(&func.arguments.0);
                format!("{}({})", func.name.as_ref(), args)
            }
            TokenOrValue::Length(length) => match length {
                LengthValue::Em(val) => format!("{val}em"),
                LengthValue::Rem(val) => format!("{val}rem"),
                _ => length_value_to_pt(length)
                    .map(|pt| format!("{:.3}pt", pt.to_f32()))
                    .unwrap_or_else(|| "length".to_string()),
            },
            TokenOrValue::Token(Token::Ident(ident)) => ident.as_ref().to_string(),
            TokenOrValue::Token(Token::String(s)) => s.as_ref().to_string(),
            TokenOrValue::Token(Token::Hash(hash)) => format!("#{}", hash.as_ref()),
            TokenOrValue::Token(Token::IDHash(hash)) => format!("#{}", hash.as_ref()),
            TokenOrValue::Token(Token::Delim(ch)) => ch.to_string(),
            TokenOrValue::Token(Token::Number { value, .. }) => format!("{value}"),
            TokenOrValue::Token(Token::Dimension { value, unit, .. }) => {
                format!("{}{}", value, unit.as_ref())
            }
            TokenOrValue::Token(Token::Percentage { unit_value, .. }) => {
                format!("{}%", unit_value * 100.0)
            }
            TokenOrValue::Token(Token::Comma) => ",".to_string(),
            TokenOrValue::Token(Token::WhiteSpace(_)) => " ".to_string(),
            TokenOrValue::Token(_) => "token".to_string(),
            _ => "value".to_string(),
        };
        out.push(value);
    }
    out.join("")
}

fn parse_hex_color(value: &str) -> Option<Color> {
    let s = value.trim();
    let s = s.strip_prefix('#').unwrap_or(s);
    let hex = match s.len() {
        3 => {
            let mut out = String::with_capacity(6);
            for ch in s.chars() {
                out.push(ch);
                out.push(ch);
            }
            out
        }
        6 => s.to_string(),
        _ => return None,
    };

    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some(Color::rgb(
        r as f32 / 255.0,
        g as f32 / 255.0,
        b as f32 / 255.0,
    ))
}

fn json_string(value: &str) -> String {
    format!("\"{}\"", json_escape(value))
}

fn json_array(values: &[String]) -> String {
    let mut out = String::from("[");
    for (idx, value) in values.iter().enumerate() {
        if idx > 0 {
            out.push(',');
        }
        out.push_str(&json_string(value));
    }
    out.push(']');
    out
}

fn json_opt_string(value: Option<String>) -> String {
    match value {
        Some(v) => json_string(&v),
        None => "null".to_string(),
    }
}

fn color_to_hex(color: Color) -> String {
    let r = (color.r.clamp(0.0, 1.0) * 255.0).round() as u8;
    let g = (color.g.clamp(0.0, 1.0) * 255.0).round() as u8;
    let b = (color.b.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", r, g, b)
}

fn blend_over_white(color: Color, alpha: f32) -> Color {
    let alpha = alpha.clamp(0.0, 1.0);
    if alpha >= 1.0 {
        return color;
    }
    Color::rgb(
        color.r * alpha + (1.0 - alpha),
        color.g * alpha + (1.0 - alpha),
        color.b * alpha + (1.0 - alpha),
    )
}

fn length_spec_debug(value: LengthSpec) -> String {
    match value {
        LengthSpec::Auto => "auto".to_string(),
        LengthSpec::Absolute(pt) => format!("{:.3}pt", pt.to_f32()),
        LengthSpec::Percent(v) => format!("{:.3}%", v * 100.0),
        LengthSpec::Em(v) => format!("{:.3}em", v),
        LengthSpec::Rem(v) => format!("{:.3}rem", v),
        LengthSpec::Calc(calc) => format!(
            "calc({:.3}pt + {:.3}% + {:.3}em + {:.3}rem)",
            calc.abs.to_f32(),
            calc.percent * 100.0,
            calc.em,
            calc.rem
        ),
        LengthSpec::Inherit => "inherit".to_string(),
        LengthSpec::Initial => "initial".to_string(),
    }
}

fn edges_debug(edges: EdgeSizes) -> String {
    format!(
        "{} {} {} {}",
        length_spec_debug(edges.top),
        length_spec_debug(edges.right),
        length_spec_debug(edges.bottom),
        length_spec_debug(edges.left)
    )
}

fn border_spacing_debug(spacing: BorderSpacingSpec) -> String {
    format!(
        "{} {}",
        length_spec_debug(spacing.horizontal),
        length_spec_debug(spacing.vertical)
    )
}

fn border_radius_debug(radius: BorderRadiusSpec) -> String {
    format!(
        "{} {} {} {}",
        length_spec_debug(radius.top_left),
        length_spec_debug(radius.top_right),
        length_spec_debug(radius.bottom_right),
        length_spec_debug(radius.bottom_left)
    )
}

fn text_decoration_debug(decoration: TextDecorationMode) -> String {
    let mut parts = Vec::new();
    if decoration.underline {
        parts.push("underline");
    }
    if decoration.overline {
        parts.push("overline");
    }
    if decoration.line_through {
        parts.push("line-through");
    }
    if parts.is_empty() {
        "none".to_string()
    } else {
        parts.join(" ")
    }
}

fn debug_style_json(style: &ComputedStyle) -> String {
    let mut fields = Vec::new();
    fields.push(format!(
        "\"display\":{}",
        json_string(&format!("{:?}", style.display))
    ));
    fields.push(format!(
        "\"position\":{}",
        json_string(&format!("{:?}", style.position))
    ));
    fields.push(format!("\"font_size\":{:.3}", style.font_size.to_f32()));
    fields.push(format!(
        "\"line_height\":{:.3}",
        style.line_height.to_line_height(style.font_size).to_f32()
    ));
    fields.push(format!("\"font_weight\":{}", style.font_weight));
    fields.push(format!(
        "\"font_style\":{}",
        json_string(&format!("{:?}", style.font_style))
    ));
    fields.push(format!(
        "\"text_decoration\":{}",
        json_string(&text_decoration_debug(style.text_decoration))
    ));
    fields.push(format!(
        "\"text_overflow\":{}",
        json_string(&format!("{:?}", style.text_overflow))
    ));
    fields.push(format!(
        "\"text_align\":{}",
        json_string(&format!("{:?}", style.text_align))
    ));
    fields.push(format!(
        "\"vertical_align\":{}",
        json_string(&format!("{:?}", style.vertical_align))
    ));
    fields.push(format!(
        "\"white_space\":{}",
        json_string(&format!("{:?}", style.white_space))
    ));
    fields.push(format!(
        "\"list_style_type\":{}",
        json_string(&format!("{:?}", style.list_style_type))
    ));
    fields.push(format!(
        "\"color\":{}",
        json_string(&color_to_hex(style.color))
    ));
    fields.push(format!(
        "\"background\":{}",
        json_opt_string(style.background_color.map(color_to_hex))
    ));
    if let Some(paint) = &style.background_paint {
        fields.push(format!(
            "\"background_paint\":{}",
            json_string(&format!("{:?}", paint))
        ));
    }
    fields.push(format!(
        "\"border_width\":{}",
        json_string(&edges_debug(style.border_width))
    ));
    fields.push(format!(
        "\"border_color\":{}",
        json_opt_string(style.border_color.map(color_to_hex))
    ));
    fields.push(format!(
        "\"border_collapse\":{}",
        json_string(&format!("{:?}", style.border_collapse))
    ));
    fields.push(format!(
        "\"caption_side\":{}",
        json_string(&format!("{:?}", style.caption_side))
    ));
    fields.push(format!(
        "\"border_spacing\":{}",
        json_string(&border_spacing_debug(style.border_spacing))
    ));
    fields.push(format!(
        "\"border_radius\":{}",
        json_string(&border_radius_debug(style.border_radius))
    ));
    fields.push(format!(
        "\"margin\":{}",
        json_string(&edges_debug(style.margin))
    ));
    fields.push(format!(
        "\"padding\":{}",
        json_string(&edges_debug(style.padding))
    ));
    fields.push(format!(
        "\"width\":{}",
        json_string(&length_spec_debug(style.width))
    ));
    fields.push(format!(
        "\"max_width\":{}",
        json_string(&length_spec_debug(style.max_width))
    ));
    fields.push(format!(
        "\"min_width\":{}",
        json_string(&length_spec_debug(style.min_width))
    ));
    fields.push(format!(
        "\"height\":{}",
        json_string(&length_spec_debug(style.height))
    ));
    fields.push(format!(
        "\"min_height\":{}",
        json_string(&length_spec_debug(style.min_height))
    ));
    fields.push(format!(
        "\"max_height\":{}",
        json_string(&length_spec_debug(style.max_height))
    ));
    fields.push(format!(
        "\"box_sizing\":{}",
        json_string(&format!("{:?}", style.box_sizing))
    ));
    fields.push(format!(
        "\"gap\":{}",
        json_string(&length_spec_debug(style.gap))
    ));
    fields.push(format!(
        "\"grid_columns\":{}",
        style
            .grid_columns
            .map(|v| v.to_string())
            .unwrap_or_else(|| "null".to_string())
    ));
    fields.push(format!(
        "\"flex_wrap\":{}",
        json_string(&format!("{:?}", style.flex_wrap))
    ));
    fields.push(format!(
        "\"flex_basis\":{}",
        json_string(&length_spec_debug(style.flex_basis))
    ));
    fields.push(format!("\"order\":{}", style.order));
    fields.push(format!("\"flex_grow\":{:.3}", style.flex_grow));
    fields.push(format!("\"flex_shrink\":{:.3}", style.flex_shrink));
    fields.push(format!(
        "\"overflow\":{}",
        json_string(&format!("{:?}", style.overflow))
    ));
    format!("{{{}}}", fields.join(","))
}

fn format_element_path(element: &ElementInfo, ancestors: &[ElementInfo]) -> String {
    let mut parts = Vec::new();
    for ancestor in ancestors {
        parts.push(format_element_segment(ancestor));
    }
    parts.push(format_element_segment(element));
    parts.join(" > ")
}

fn format_element_segment(info: &ElementInfo) -> String {
    let mut out = info.tag.clone();
    if let Some(id) = &info.id {
        out.push('#');
        out.push_str(id);
    }
    for class in &info.classes {
        out.push('.');
        out.push_str(class);
    }
    if info.child_count > 0 {
        out.push_str(&format!(":nth-child({})", info.child_index));
    }
    out
}

fn apply_delta(
    computed: &mut ComputedStyle,
    delta: &StyleDelta,
    parent: &ComputedStyle,
    parent_font_size: Pt,
    parent_line_height: LineHeightSpec,
    root_font_size: Pt,
    viewport: Size,
) {
    if let Some(font_size) = &delta.font_size {
        computed.font_size = match font_size {
            FontSizeSpec::AbsolutePt(value) => *value,
            FontSizeSpec::RelativeScale(scale) => parent_font_size.mul_fixed(*scale),
            FontSizeSpec::Calc(calc) => calc.resolve(parent_font_size, root_font_size, viewport),
            FontSizeSpec::Inherit => parent_font_size,
            FontSizeSpec::Initial => root_font_size,
        };
    }
    if let Some(line_height) = &delta.line_height {
        computed.line_height = match line_height {
            LineHeightSpec::Inherit => parent_line_height,
            LineHeightSpec::Initial => LineHeightSpec::Normal,
            _ => line_height.clone(),
        };
    }
    if let Some(color) = &delta.color {
        computed.color = match color {
            ColorSpec::Value(value) => *value,
            ColorSpec::Inherit => parent.color,
            ColorSpec::Initial => Color::BLACK,
            ColorSpec::CurrentColor => computed.color,
        };
    }
    if let Some(var) = &delta.color_var {
        computed.pending_color_var = Some(var.clone());
    }
    if let Some(color) = &delta.background_color {
        computed.background_color = match color {
            BackgroundSpec::Value(value) => Some(*value),
            BackgroundSpec::Inherit => parent.background_color,
            BackgroundSpec::CurrentColor => Some(parent.color),
            BackgroundSpec::Initial => None,
        };
    }
    if let Some(paint) = &delta.background_paint {
        computed.background_paint = Some(paint.clone());
    }
    if let Some(var) = &delta.background_color_var {
        computed.pending_background_color_var = Some(var.clone());
    }
    if let Some(font_name) = &delta.font_name {
        match font_name {
            FontSpec::Value(stack) => {
                if !stack.is_empty() {
                    computed.font_stack = stack.clone();
                    computed.font_name = stack[0].clone();
                }
            }
            FontSpec::Inherit => {
                computed.font_stack = parent.font_stack.clone();
                computed.font_name = parent.font_name.clone();
            }
            FontSpec::Initial => {
                computed.font_stack = vec![Arc::<str>::from("Helvetica")];
                computed.font_name = Arc::<str>::from("Helvetica");
            }
        }
    }
    if let Some(var) = &delta.font_name_var {
        computed.pending_font_name_var = Some(var.clone());
    }
    if let Some(weight) = delta.font_weight {
        computed.font_weight = weight;
    }
    if let Some(style) = delta.font_style {
        computed.font_style = style;
    }
    if let Some(transform) = delta.text_transform {
        computed.text_transform = transform;
    }
    if let Some(decoration) = &delta.text_decoration {
        computed.text_decoration = match decoration {
            TextDecorationSpec::Value(value) => *value,
            TextDecorationSpec::Inherit => parent.text_decoration,
            TextDecorationSpec::Initial => TextDecorationMode::default(),
        };
    }
    if let Some(overflow) = delta.text_overflow {
        computed.text_overflow = match overflow {
            TextOverflowSpec::Value(value) => value,
            TextOverflowSpec::Inherit => parent.text_overflow,
            TextOverflowSpec::Initial => TextOverflowMode::Clip,
        };
    }
    if let Some(content) = &delta.content {
        computed.content = match content {
            ContentSpec::None => None,
            ContentSpec::Text(value) => Some(value.clone()),
            ContentSpec::Inherit => parent.content.clone(),
            ContentSpec::Initial => None,
        };
    }
    if let Some(word_break) = delta.word_break {
        computed.word_break = word_break;
    }
    if let Some(list_style_type) = delta.list_style_type {
        computed.list_style_type = list_style_type;
    }
    if let Some(spacing) = &delta.letter_spacing {
        let resolved = normalize_length_spec(*spacing, LengthSpec::Absolute(parent.letter_spacing));
        computed.letter_spacing = match resolved {
            LengthSpec::Absolute(value) => value,
            LengthSpec::Percent(pct) => parent.font_size * pct,
            LengthSpec::Em(scale) => parent.font_size * scale,
            LengthSpec::Rem(scale) => root_font_size * scale,
            LengthSpec::Calc(calc) => {
                calc.resolve(parent.font_size, parent.font_size, root_font_size)
            }
            LengthSpec::Auto | LengthSpec::Inherit | LengthSpec::Initial => Pt::ZERO,
        };
    }
    if let Some(value) = delta.pagination.break_before {
        computed.pagination.break_before = value;
    }
    if let Some(value) = delta.pagination.break_after {
        computed.pagination.break_after = value;
    }
    if let Some(value) = delta.pagination.break_inside {
        computed.pagination.break_inside = value;
    }
    if let Some(value) = delta.pagination.orphans {
        computed.pagination.orphans = value;
    }
    if let Some(value) = delta.pagination.widows {
        computed.pagination.widows = value;
    }

    apply_edge_delta(&mut computed.margin, &delta.margin, &parent.margin);
    apply_edge_delta(&mut computed.padding, &delta.padding, &parent.padding);

    if let Some(width) = delta.width {
        computed.width = normalize_size_spec(width, parent.width);
    }
    if let Some(min_width) = delta.min_width {
        computed.min_width = normalize_size_spec(min_width, parent.min_width);
    }
    if let Some(max_width) = delta.max_width {
        computed.max_width = normalize_size_spec(max_width, parent.max_width);
    }
    if let Some(height) = delta.height {
        computed.height = normalize_size_spec(height, parent.height);
    }
    if let Some(min_height) = delta.min_height {
        computed.min_height = normalize_size_spec(min_height, parent.min_height);
    }
    if let Some(max_height) = delta.max_height {
        computed.max_height = normalize_size_spec(max_height, parent.max_height);
    }
    if let Some(var) = &delta.width_var {
        computed.pending_width_var = Some(var.clone());
    }
    if let Some(var) = &delta.max_width_var {
        computed.pending_max_width_var = Some(var.clone());
    }
    if let Some(var) = &delta.height_var {
        computed.pending_height_var = Some(var.clone());
    }

    if let Some(align) = delta.text_align {
        computed.text_align = align;
    }
    if let Some(align) = delta.vertical_align {
        computed.vertical_align = align;
    }

    if let Some(spec) = delta.border_width.top {
        computed.border_width.top = normalize_length_spec(spec, parent.border_width.top);
    }
    if let Some(spec) = delta.border_width.right {
        computed.border_width.right = normalize_length_spec(spec, parent.border_width.right);
    }
    if let Some(spec) = delta.border_width.bottom {
        computed.border_width.bottom = normalize_length_spec(spec, parent.border_width.bottom);
    }
    if let Some(spec) = delta.border_width.left {
        computed.border_width.left = normalize_length_spec(spec, parent.border_width.left);
    }
    if let Some(color) = &delta.border_color {
        computed.border_color = Some(match color {
            ColorSpec::Value(value) => *value,
            ColorSpec::Inherit => parent.border_color.unwrap_or(parent.color),
            ColorSpec::Initial => Color::BLACK,
            ColorSpec::CurrentColor => computed.color,
        });
    }
    if let Some(var) = &delta.border_color_var {
        computed.pending_border_color_var = Some(var.clone());
    }
    if let Some(style) = delta.border_style.top {
        computed.border_style.top = style;
    }
    if let Some(style) = delta.border_style.right {
        computed.border_style.right = style;
    }
    if let Some(style) = delta.border_style.bottom {
        computed.border_style.bottom = style;
    }
    if let Some(style) = delta.border_style.left {
        computed.border_style.left = style;
    }

    if let Some(mode) = delta.border_collapse {
        computed.border_collapse = mode;
    }
    if let Some(side) = delta.caption_side {
        computed.caption_side = side;
    }
    if let Some(spacing) = &delta.border_spacing {
        computed.border_spacing = *spacing;
    }
    if let Some(radius) = &delta.border_radius {
        computed.border_radius = *radius;
    }
    if let Some(shadow) = &delta.box_shadow {
        computed.box_shadow = Some(shadow.clone());
    }

    if let Some(mode) = delta.white_space {
        computed.white_space = match mode {
            WhiteSpaceSpec::Value(value) => value,
            WhiteSpaceSpec::Inherit => parent.white_space,
            WhiteSpaceSpec::Initial => WhiteSpaceMode::Normal,
        };
    }
    if let Some(display) = delta.display {
        computed.display = match display {
            DisplaySpec::Value(value) => value,
            DisplaySpec::Inherit => parent.display,
            DisplaySpec::Initial => DisplayMode::Inline,
        };
    }
    if let Some(position) = delta.position {
        computed.position = position;
    }
    if let Some(z_index) = delta.z_index {
        computed.z_index = z_index;
    }
    if let Some(box_sizing) = delta.box_sizing {
        computed.box_sizing = box_sizing;
    }
    if let Some(inset_left) = delta.inset_left {
        computed.inset_left = normalize_length_spec(inset_left, parent.inset_left);
    }
    if let Some(inset_top) = delta.inset_top {
        computed.inset_top = normalize_length_spec(inset_top, parent.inset_top);
    }
    if let Some(inset_right) = delta.inset_right {
        computed.inset_right = normalize_length_spec(inset_right, parent.inset_right);
    }
    if let Some(inset_bottom) = delta.inset_bottom {
        computed.inset_bottom = normalize_length_spec(inset_bottom, parent.inset_bottom);
    }

    if let Some(dir) = delta.flex_direction {
        computed.flex_direction = dir;
    }
    if let Some(wrap) = delta.flex_wrap {
        computed.flex_wrap = wrap;
    }
    if let Some(basis) = delta.flex_basis {
        computed.flex_basis = basis;
    }
    if let Some(order) = delta.order {
        computed.order = order;
    }
    if let Some(justify) = delta.justify_content {
        computed.justify_content = justify;
    }
    if let Some(align) = delta.align_items {
        computed.align_items = align;
    }
    if let Some(columns) = delta.grid_columns {
        computed.grid_columns = if columns == 0 { None } else { Some(columns) };
    }
    if let Some(gap) = delta.gap {
        computed.gap = normalize_length_spec(gap, parent.gap);
    }
    if let Some(grow) = delta.flex_grow {
        computed.flex_grow = grow.max(0.0);
    }
    if let Some(shrink) = delta.flex_shrink {
        computed.flex_shrink = shrink.max(0.0);
    }
    if let Some(overflow) = delta.overflow {
        computed.overflow = overflow;
    }

    if !delta.custom_lengths.is_empty() {
        for (k, v) in &delta.custom_lengths {
            computed.custom_lengths.insert(k.clone(), *v);
        }
    }
    if !delta.custom_colors.is_empty() {
        for (k, v) in &delta.custom_colors {
            computed.custom_colors.insert(k.clone(), *v);
        }
    }
    if !delta.custom_color_alpha.is_empty() {
        for (k, v) in &delta.custom_color_alpha {
            computed.custom_color_alpha.insert(k.clone(), *v);
        }
    }
    if !delta.custom_color_refs.is_empty() {
        for (k, v) in &delta.custom_color_refs {
            computed.custom_color_refs.insert(k.clone(), v.clone());
            computed.custom_colors.remove(k);
            computed.custom_color_alpha.remove(k);
        }
    }
    if !delta.custom_font_stacks.is_empty() {
        for (k, v) in &delta.custom_font_stacks {
            computed.custom_font_stacks.insert(k.clone(), v.clone());
        }
    }

    apply_edge_var_delta(
        &mut computed.margin,
        &delta.margin,
        &parent.margin,
        &computed.custom_lengths,
    );
    apply_edge_var_delta(
        &mut computed.padding,
        &delta.padding,
        &parent.padding,
        &computed.custom_lengths,
    );
    apply_edge_var_delta(
        &mut computed.border_width,
        &delta.border_width,
        &parent.border_width,
        &computed.custom_lengths,
    );

    if matches!(computed.pagination.break_inside, BreakInside::AvoidPage) {
        computed.pagination.break_inside = BreakInside::Avoid;
    }

    if computed.pagination.orphans == 0 {
        computed.pagination.orphans = 1;
    }
    if computed.pagination.widows == 0 {
        computed.pagination.widows = 1;
    }

    if computed.font_size <= Pt::ZERO {
        computed.font_size = root_font_size;
    }
}

fn apply_border_style_mask(style: &mut ComputedStyle) {
    if matches!(style.border_style.top, BorderLineStyle::None) {
        style.border_width.top = LengthSpec::Absolute(Pt::ZERO);
    }
    if matches!(style.border_style.right, BorderLineStyle::None) {
        style.border_width.right = LengthSpec::Absolute(Pt::ZERO);
    }
    if matches!(style.border_style.bottom, BorderLineStyle::None) {
        style.border_width.bottom = LengthSpec::Absolute(Pt::ZERO);
    }
    if matches!(style.border_style.left, BorderLineStyle::None) {
        style.border_width.left = LengthSpec::Absolute(Pt::ZERO);
    }
}

fn resolve_custom_color(style: &ComputedStyle, expr: &str) -> Option<Color> {
    resolve_custom_color_expr_with_alpha(style, expr).map(|(color, _)| color)
}

fn resolve_custom_color_with_alpha(style: &ComputedStyle, name: &str) -> Option<(Color, f32)> {
    let mut current = name.trim().to_ascii_lowercase();
    if current.is_empty() {
        return None;
    }
    if let Some(color) = style.custom_colors.get(&current).copied() {
        let alpha = style
            .custom_color_alpha
            .get(&current)
            .copied()
            .unwrap_or(1.0);
        return Some((color, alpha.clamp(0.0, 1.0)));
    }
    let mut hops = 0usize;
    while let Some(next) = style.custom_color_refs.get(&current) {
        if let Some(color) = style.custom_colors.get(next).copied() {
            let alpha = style.custom_color_alpha.get(next).copied().unwrap_or(1.0);
            return Some((color, alpha.clamp(0.0, 1.0)));
        }
        if !next.starts_with("--") {
            return resolve_custom_color_expr_with_alpha_inner(style, next, hops + 1);
        }
        current = next.clone();
        hops += 1;
        if hops > 8 {
            break;
        }
    }
    None
}

fn resolve_custom_color_expr_with_alpha(style: &ComputedStyle, expr: &str) -> Option<(Color, f32)> {
    resolve_custom_color_expr_with_alpha_inner(style, expr, 0)
}

fn resolve_custom_color_expr_with_alpha_inner(
    style: &ComputedStyle,
    expr: &str,
    depth: usize,
) -> Option<(Color, f32)> {
    if depth > 12 {
        return None;
    }
    let expr = expr.trim();
    if expr.is_empty() {
        return None;
    }
    if expr.starts_with("--") {
        return resolve_custom_color_with_alpha(style, expr);
    }
    if let Some((name, fallback)) = parse_var_function(expr) {
        if let Some(color) = resolve_custom_color_with_alpha(style, &name) {
            return Some(color);
        }
        if let Some(fallback_expr) = fallback {
            return resolve_custom_color_expr_with_alpha_inner(style, &fallback_expr, depth + 1);
        }
        return None;
    }
    if let Some((color, alpha)) = resolve_rgb_color_function_with_vars(style, expr, depth + 1) {
        return Some((color, alpha.clamp(0.0, 1.0)));
    }
    if let Some((color, alpha)) = parse_color_string(expr) {
        return Some((color, alpha.clamp(0.0, 1.0)));
    }
    if let Some(color) = parse_rgb_triplet_string(expr) {
        return Some((color, 1.0));
    }
    None
}

fn resolve_rgb_color_function_with_vars(
    style: &ComputedStyle,
    expr: &str,
    depth: usize,
) -> Option<(Color, f32)> {
    if depth > 12 {
        return None;
    }
    let expr = expr.trim();
    let lower = expr.to_ascii_lowercase();
    if !(lower.starts_with("rgb(") || lower.starts_with("rgba(")) || !expr.ends_with(')') {
        return None;
    }
    let open = expr.find('(')?;
    let inner = &expr[open + 1..expr.len() - 1];
    let args = split_args(inner);
    if args.is_empty() {
        return None;
    }

    // Bootstrap utility pattern: rgba(var(--bs-success-rgb), var(--bs-text-opacity))
    if args.len() == 2 && args[0].trim().to_ascii_lowercase().starts_with("var(") {
        let (color, color_alpha) =
            resolve_custom_color_expr_with_alpha_inner(style, args[0].trim(), depth + 1)?;
        let alpha = resolve_custom_number_expr(style, args[1].trim(), depth + 1).unwrap_or(1.0);
        return Some((color, (alpha * color_alpha).clamp(0.0, 1.0)));
    }

    // Accept direct rgb(var(--triplet)) form by resolving the var to a color.
    if args.len() == 1 && args[0].trim().to_ascii_lowercase().starts_with("var(") {
        return resolve_custom_color_expr_with_alpha_inner(style, args[0].trim(), depth + 1);
    }

    if args.len() < 3 {
        return None;
    }
    let r = resolve_rgb_component(style, args[0].trim(), depth + 1)?;
    let g = resolve_rgb_component(style, args[1].trim(), depth + 1)?;
    let b = resolve_rgb_component(style, args[2].trim(), depth + 1)?;
    let alpha = if args.len() >= 4 {
        resolve_custom_number_expr(style, args[3].trim(), depth + 1).unwrap_or(1.0)
    } else {
        1.0
    };
    Some((
        Color::rgb(
            (r / 255.0).clamp(0.0, 1.0),
            (g / 255.0).clamp(0.0, 1.0),
            (b / 255.0).clamp(0.0, 1.0),
        ),
        alpha.clamp(0.0, 1.0),
    ))
}

fn resolve_rgb_component(style: &ComputedStyle, expr: &str, depth: usize) -> Option<f32> {
    if depth > 12 {
        return None;
    }
    let expr = expr.trim();
    if expr.ends_with('%') {
        let pct = expr.trim_end_matches('%').trim().parse::<f32>().ok()?;
        return Some((pct.clamp(0.0, 100.0) / 100.0) * 255.0);
    }
    if let Ok(value) = expr.parse::<f32>() {
        return Some(value.clamp(0.0, 255.0));
    }
    if let Some(value) = resolve_custom_number_expr(style, expr, depth + 1) {
        // Preserve historical behavior where 0..1 can still be used as normalized values.
        return Some(if value <= 1.0 { value * 255.0 } else { value });
    }
    None
}

fn resolve_custom_number_expr(style: &ComputedStyle, expr: &str, depth: usize) -> Option<f32> {
    if depth > 12 {
        return None;
    }
    let expr = expr.trim();
    if expr.is_empty() {
        return None;
    }
    if let Some((name, fallback)) = parse_var_function(expr) {
        if let Some(value) = resolve_custom_number(style, &name) {
            return Some(value);
        }
        if let Some(fallback_expr) = fallback {
            return resolve_custom_number_expr(style, &fallback_expr, depth + 1);
        }
        return None;
    }
    if expr.starts_with("--") {
        return resolve_custom_number(style, expr);
    }
    if expr.ends_with('%') {
        let pct = expr.trim_end_matches('%').trim().parse::<f32>().ok()?;
        return Some((pct / 100.0).clamp(0.0, 1.0));
    }
    expr.parse::<f32>().ok()
}

fn resolve_custom_number(style: &ComputedStyle, name: &str) -> Option<f32> {
    let mut current = name.trim().to_ascii_lowercase();
    if current.is_empty() {
        return None;
    }
    let mut hops = 0usize;
    loop {
        if let Some(value) = style
            .custom_lengths
            .get(&current)
            .and_then(length_spec_to_scalar)
        {
            return Some(value);
        }
        let Some(next) = style.custom_color_refs.get(&current) else {
            break;
        };
        current = next.clone();
        hops += 1;
        if hops > 8 {
            break;
        }
    }
    None
}

fn length_spec_to_scalar(spec: &LengthSpec) -> Option<f32> {
    match spec {
        LengthSpec::Absolute(value) => Some(value.to_f32() / 0.75),
        LengthSpec::Percent(value) => Some(*value),
        LengthSpec::Em(value) => Some(*value),
        LengthSpec::Rem(value) => Some(*value),
        _ => None,
    }
}

fn parse_var_function(raw: &str) -> Option<(String, Option<String>)> {
    let raw = raw.trim();
    if !raw.to_ascii_lowercase().starts_with("var(") || !raw.ends_with(')') {
        return None;
    }
    let inside = &raw[4..raw.len() - 1];
    let args = split_args(inside);
    if args.is_empty() {
        return None;
    }
    let name = args[0].trim().to_ascii_lowercase();
    if !name.starts_with("--") {
        return None;
    }
    let fallback = if args.len() > 1 {
        let joined = args[1..].join(",");
        let trimmed = joined.trim().to_string();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    } else {
        None
    };
    Some((name, fallback))
}

fn resolve_pending_vars(style: &mut ComputedStyle) {
    if let Some(name) = style.pending_width_var.take() {
        if let Some(spec) = style.custom_lengths.get(&name).copied() {
            style.width = spec;
        }
    }
    if let Some(name) = style.pending_max_width_var.take() {
        if let Some(spec) = style.custom_lengths.get(&name).copied() {
            style.max_width = spec;
        }
    }
    if let Some(name) = style.pending_height_var.take() {
        if let Some(spec) = style.custom_lengths.get(&name).copied() {
            style.height = spec;
        }
    }
    if let Some(name) = style.pending_color_var.take() {
        if let Some((color, alpha)) = resolve_custom_color_expr_with_alpha(style, &name) {
            style.color = blend_over_white(color, alpha);
        }
    }
    if let Some(name) = style.pending_background_color_var.take() {
        if let Some((color, alpha)) = resolve_custom_color_expr_with_alpha(style, &name) {
            style.background_color = Some(blend_over_white(color, alpha));
        }
    }
    if let Some(name) = style.pending_border_color_var.take() {
        if let Some((color, alpha)) = resolve_custom_color_expr_with_alpha(style, &name) {
            style.border_color = Some(blend_over_white(color, alpha));
        }
    }
    if let Some(name) = style.pending_font_name_var.take() {
        if let Some(stack) = style.custom_font_stacks.get(&name) {
            if !stack.is_empty() {
                style.font_stack = stack.clone();
                style.font_name = stack[0].clone();
            }
        }
    }
    let shadow_expr = style
        .box_shadow
        .as_ref()
        .and_then(|shadow| shadow.color_var.clone());
    let shadow_resolved = shadow_expr
        .as_ref()
        .and_then(|expr| resolve_custom_color_expr_with_alpha(style, expr));
    if let Some(shadow) = style.box_shadow.as_mut() {
        if shadow_expr.is_some() {
            if let Some((color, alpha)) = shadow_resolved {
                shadow.color = color;
                shadow.opacity = (shadow.opacity * alpha).clamp(0.0, 1.0);
                shadow.color_var = None;
            } else {
                // Unresolved color var should not silently paint opaque black.
                shadow.opacity = 0.0;
            }
        }
    }
}

fn font_size_spec(value: &FontSize) -> Option<FontSizeSpec> {
    match value {
        FontSize::Length(length) => font_size_from_length(length),
        FontSize::Absolute(size) => Some(FontSizeSpec::AbsolutePt(absolute_font_size(*size))),
        FontSize::Relative(size) => Some(FontSizeSpec::RelativeScale(match size {
            RelativeFontSize::Smaller => I32F32::from_num(0.8),
            RelativeFontSize::Larger => I32F32::from_num(1.2),
        })),
    }
}

fn font_size_from_length(value: &LengthPercentage) -> Option<FontSizeSpec> {
    match value {
        LengthPercentage::Percentage(pct) => {
            Some(FontSizeSpec::RelativeScale(I32F32::from_num(pct.0)))
        }
        LengthPercentage::Dimension(length) => match length {
            LengthValue::Em(val) => Some(FontSizeSpec::RelativeScale(I32F32::from_num(*val))),
            LengthValue::Rem(val) => font_calc_from_length_value(length)
                .map(FontSizeSpec::Calc)
                .or_else(|| Some(FontSizeSpec::AbsolutePt(px_to_pt(val * 16.0)))),
            LengthValue::Vw(_)
            | LengthValue::Vh(_)
            | LengthValue::Vmin(_)
            | LengthValue::Vmax(_) => font_calc_from_length_value(length).map(FontSizeSpec::Calc),
            _ => length_value_to_pt(length).map(FontSizeSpec::AbsolutePt),
        },
        LengthPercentage::Calc(calc) => font_calc_from_calc(calc).map(FontSizeSpec::Calc),
    }
}

fn font_calc_from_length_value(length: &LengthValue) -> Option<FontCalcLength> {
    let mut calc = FontCalcLength::zero();
    match length {
        LengthValue::Em(val) => {
            calc.em = I32F32::from_num(*val);
            Some(calc)
        }
        LengthValue::Rem(val) => {
            calc.rem = I32F32::from_num(*val);
            Some(calc)
        }
        LengthValue::Vw(val) => {
            calc.vw = I32F32::from_num(*val / 100.0);
            Some(calc)
        }
        LengthValue::Vh(val) => {
            calc.vh = I32F32::from_num(*val / 100.0);
            Some(calc)
        }
        LengthValue::Vmin(val) => {
            calc.vmin = I32F32::from_num(*val / 100.0);
            Some(calc)
        }
        LengthValue::Vmax(val) => {
            calc.vmax = I32F32::from_num(*val / 100.0);
            Some(calc)
        }
        _ => length_value_to_pt(length).map(|pt| FontCalcLength { abs: pt, ..calc }),
    }
}

fn font_calc_from_length_percentage(value: &LengthPercentage) -> Option<FontCalcLength> {
    match value {
        LengthPercentage::Percentage(pct) => {
            let mut calc = FontCalcLength::zero();
            calc.em = I32F32::from_num(pct.0);
            Some(calc)
        }
        LengthPercentage::Dimension(length) => font_calc_from_length_value(length),
        LengthPercentage::Calc(calc) => font_calc_from_calc(calc),
    }
}

fn font_calc_from_calc(calc: &Calc<LengthPercentage>) -> Option<FontCalcLength> {
    match calc {
        Calc::Value(value) => font_calc_from_length_percentage(value),
        Calc::Sum(a, b) => Some(font_calc_from_calc(a)?.add(font_calc_from_calc(b)?)),
        Calc::Product(value, inner) => {
            let factor = I32F32::from_num(*value);
            font_calc_from_calc(inner).map(|calc| calc.scale(factor))
        }
        Calc::Function(func) => match func.as_ref() {
            MathFunction::Calc(inner) => font_calc_from_calc(inner),
            _ => None,
        },
        Calc::Number(_) => None,
    }
}

fn line_height_spec(value: &LineHeight) -> Option<LineHeightSpec> {
    match value {
        LineHeight::Normal => Some(LineHeightSpec::Normal),
        LineHeight::Number(value) => Some(LineHeightSpec::Number(*value)),
        LineHeight::Length(length) => match length {
            LengthPercentage::Percentage(pct) => Some(LineHeightSpec::Number(pct.0)),
            LengthPercentage::Dimension(length) => match length {
                LengthValue::Em(val) => Some(LineHeightSpec::Number(*val)),
                LengthValue::Rem(val) => Some(LineHeightSpec::AbsolutePt(px_to_pt(val * 16.0))),
                _ => length_value_to_pt(length).map(LineHeightSpec::AbsolutePt),
            },
            _ => None,
        },
    }
}

fn parse_font_weight_str(raw: &str) -> Option<u16> {
    let raw = raw.trim().to_ascii_lowercase();
    if raw.is_empty() {
        return None;
    }
    match raw.as_str() {
        "normal" => Some(400),
        "bold" => Some(700),
        "bolder" => Some(700),
        "lighter" => Some(300),
        _ => raw.parse::<u16>().ok(),
    }
}

fn parse_text_transform_str(raw: &str) -> Option<TextTransformMode> {
    let raw = raw.trim().to_ascii_lowercase();
    if raw.is_empty() {
        return None;
    }
    match raw.as_str() {
        "uppercase" => Some(TextTransformMode::Uppercase),
        "lowercase" => Some(TextTransformMode::Lowercase),
        "capitalize" => Some(TextTransformMode::Capitalize),
        "none" => Some(TextTransformMode::None),
        _ => None,
    }
}

fn text_decoration_from_line(value: &TextDecorationLine) -> TextDecorationMode {
    TextDecorationMode {
        underline: value.contains(TextDecorationLine::Underline),
        overline: value.contains(TextDecorationLine::Overline),
        line_through: value.contains(TextDecorationLine::LineThrough),
    }
}

fn parse_text_decoration_str(raw: &str) -> Option<TextDecorationMode> {
    let raw = raw.trim().to_ascii_lowercase();
    if raw.is_empty() {
        return None;
    }
    let mut mode = TextDecorationMode::default();
    let mut saw = false;
    for token in raw.split_whitespace() {
        match token {
            "none" => return Some(TextDecorationMode::default()),
            "underline" => {
                mode.underline = true;
                saw = true;
            }
            "overline" => {
                mode.overline = true;
                saw = true;
            }
            "line-through" => {
                mode.line_through = true;
                saw = true;
            }
            _ => {}
        }
    }
    if saw { Some(mode) } else { None }
}

fn parse_letter_spacing_str(raw: &str) -> Option<LengthSpec> {
    let raw = raw.trim().to_ascii_lowercase();
    if raw.is_empty() {
        return None;
    }
    if raw == "normal" {
        return Some(LengthSpec::Absolute(Pt::ZERO));
    }
    length_spec_from_string(&raw)
}

fn length_spec_from_string(raw: &str) -> Option<LengthSpec> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    if let Some(value) = raw.strip_suffix('%') {
        if let Ok(v) = value.trim().parse::<f32>() {
            return Some(LengthSpec::Percent(v / 100.0));
        }
    }
    let units = ["px", "pt", "in", "cm", "mm", "em", "rem"];
    for unit in &units {
        if let Some(value) = raw.strip_suffix(unit) {
            if let Ok(v) = value.trim().parse::<f32>() {
                return Some(match *unit {
                    "px" => LengthSpec::Absolute(px_to_pt(v)),
                    "pt" => LengthSpec::Absolute(Pt::from_f32(v)),
                    "in" => LengthSpec::Absolute(Pt::from_f32(v * 72.0)),
                    "cm" => LengthSpec::Absolute(Pt::from_f32(v * (72.0 / 2.54))),
                    "mm" => LengthSpec::Absolute(Pt::from_f32(v * (72.0 / 25.4))),
                    "em" => LengthSpec::Em(v),
                    "rem" => LengthSpec::Rem(v),
                    _ => return None,
                });
            }
        }
    }
    if let Ok(v) = raw.parse::<f32>() {
        return Some(LengthSpec::Absolute(px_to_pt(v)));
    }
    None
}

fn parse_length_list(raw: &str) -> Vec<LengthSpec> {
    raw.split_whitespace()
        .filter_map(length_spec_from_string)
        .collect()
}

fn parse_grid_track_count(raw: &str) -> Option<usize> {
    let mut normalized = raw.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return None;
    }

    if let Some(columns_only) = parse_repeat_track_count(&normalized) {
        return Some(columns_only.max(1));
    }

    if let Some((before_slash, _)) = normalized.split_once('/') {
        normalized = before_slash.trim().to_string();
    }

    if normalized.is_empty() || normalized == "none" {
        return None;
    }

    let tracks = split_top_level_whitespace_tokens(&normalized);
    let mut count = 0usize;
    for track in tracks {
        let track = track.trim();
        if track.is_empty() {
            continue;
        }
        if track.starts_with('[') && track.ends_with(']') {
            continue;
        }
        count += 1;
    }
    if count > 0 { Some(count) } else { None }
}

fn parse_repeat_track_count(raw: &str) -> Option<usize> {
    let text = raw.trim();
    if !(text.starts_with("repeat(") && text.ends_with(')')) {
        return None;
    }
    let inner = &text[7..text.len().saturating_sub(1)];
    let (count_raw, tracks_raw) = inner.split_once(',')?;
    if tracks_raw.trim().is_empty() {
        return None;
    }
    let count = count_raw.trim().parse::<usize>().ok()?;
    (count > 0).then_some(count)
}

fn split_top_level_whitespace_tokens(raw: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut buf = String::new();
    let mut paren_depth = 0usize;
    let mut bracket_depth = 0usize;

    for ch in raw.chars() {
        match ch {
            '(' => {
                paren_depth += 1;
                buf.push(ch);
            }
            ')' => {
                paren_depth = paren_depth.saturating_sub(1);
                buf.push(ch);
            }
            '[' => {
                bracket_depth += 1;
                buf.push(ch);
            }
            ']' => {
                bracket_depth = bracket_depth.saturating_sub(1);
                buf.push(ch);
            }
            _ if ch.is_whitespace() && paren_depth == 0 && bracket_depth == 0 => {
                if !buf.trim().is_empty() {
                    out.push(buf.trim().to_string());
                }
                buf.clear();
            }
            _ => buf.push(ch),
        }
    }
    if !buf.trim().is_empty() {
        out.push(buf.trim().to_string());
    }
    out
}

fn parse_border_spacing_str(raw: &str) -> Option<BorderSpacingSpec> {
    let values = parse_length_list(raw);
    if values.is_empty() {
        return None;
    }
    let horizontal = values[0];
    let vertical = if values.len() > 1 {
        values[1]
    } else {
        values[0]
    };
    Some(BorderSpacingSpec {
        horizontal,
        vertical,
    })
}

fn parse_border_radius_str(raw: &str) -> Option<BorderRadiusSpec> {
    let raw = raw.split('/').next().unwrap_or(raw).trim();
    let values = parse_length_list(raw);
    if values.is_empty() {
        return None;
    }
    let (tl, tr, br, bl) = match values.len() {
        1 => (values[0], values[0], values[0], values[0]),
        2 => (values[0], values[1], values[0], values[1]),
        3 => (values[0], values[1], values[2], values[1]),
        _ => (values[0], values[1], values[2], values[3]),
    };
    Some(BorderRadiusSpec {
        top_left: tl,
        top_right: tr,
        bottom_right: br,
        bottom_left: bl,
    })
}

fn apply_background_from_string(raw: &str, delta: &mut StyleDelta) {
    if let Some(paint) = parse_linear_gradient_str(raw) {
        delta.background_paint = Some(paint);
        return;
    }
    if let Some((color, alpha)) = parse_color_string(raw) {
        delta.background_color = Some(BackgroundSpec::Value(blend_over_white(color, alpha)));
        return;
    }
    if raw.to_ascii_lowercase().contains("var(") {
        delta.background_color_var = Some(raw.trim().to_ascii_lowercase());
        return;
    }
    if let Some(var) = var_name_from_string(raw) {
        delta.background_color_var = Some(var);
    }
}

fn var_name_from_string(raw: &str) -> Option<String> {
    let lower = raw.to_ascii_lowercase();
    let start = lower.find("var(")?;
    let after = &raw[start + 4..];
    let end = after.find(')')?;
    let inside = &after[..end];
    for part in inside.split(',') {
        let name = part.trim();
        if name.starts_with("--") {
            return Some(name.to_ascii_lowercase());
        }
    }
    None
}

fn parse_box_shadow_str(raw: &str) -> Option<BoxShadowSpec> {
    let shadow = split_args(raw).get(0)?.clone();
    let tokens = split_ws_preserve_parens(&shadow);
    let mut lengths: Vec<LengthSpec> = Vec::new();
    let mut color: Option<(Color, f32)> = None;
    let mut color_var: Option<String> = None;
    let mut inset = false;
    for token in tokens {
        let t = token.trim();
        if t.is_empty() {
            continue;
        }
        if t.eq_ignore_ascii_case("inset") {
            inset = true;
            continue;
        }
        if let Some(len) = length_spec_from_string(t) {
            lengths.push(len);
            continue;
        }
        if let Some(c) = parse_color_string(t) {
            color = Some(c);
            continue;
        }
        if color_var.is_none() {
            if t.to_ascii_lowercase().contains("var(") {
                color_var = Some(t.to_ascii_lowercase());
                continue;
            }
            if let Some(var) = var_name_from_string(t) {
                color_var = Some(var);
                continue;
            }
        }
    }
    if lengths.len() < 2 {
        return None;
    }
    let offset_x = lengths[0];
    let offset_y = lengths[1];
    let blur = lengths
        .get(2)
        .copied()
        .unwrap_or(LengthSpec::Absolute(Pt::ZERO));
    let spread = lengths
        .get(3)
        .copied()
        .unwrap_or(LengthSpec::Absolute(Pt::ZERO));
    let (color, opacity) = if let Some(color) = color {
        color
    } else if color_var.is_some() {
        (Color::BLACK, 1.0)
    } else {
        (Color::BLACK, 0.25)
    };
    Some(BoxShadowSpec {
        offset_x,
        offset_y,
        blur,
        spread,
        color,
        opacity,
        inset,
        color_var,
    })
}

fn parse_linear_gradient_str(raw: &str) -> Option<BackgroundPaint> {
    let raw = raw.trim();
    let lower = raw.to_ascii_lowercase();
    let start = lower.find("linear-gradient(")?;
    let inside = &raw[start + "linear-gradient(".len()..];
    let inside = inside.strip_suffix(')')?;
    let parts = split_args(inside);
    if parts.len() < 2 {
        return None;
    }

    let mut angle_deg = 180.0;
    let mut stop_start = 0usize;
    if let Some(angle) = parse_gradient_angle(&parts[0]) {
        angle_deg = angle;
        stop_start = 1;
    }

    let mut stops: Vec<(Color, Option<f32>)> = Vec::new();
    for part in parts.iter().skip(stop_start) {
        if let Some(stop) = parse_gradient_stop(part) {
            stops.push(stop);
        }
    }
    if stops.len() < 2 {
        return None;
    }

    let mut shading_stops = Vec::with_capacity(stops.len());
    let has_offsets = stops.iter().any(|(_, off)| off.is_some());
    if has_offsets {
        for (idx, (color, offset)) in stops.iter().enumerate() {
            let off = offset.unwrap_or_else(|| {
                if stops.len() == 1 {
                    0.0
                } else {
                    idx as f32 / ((stops.len() - 1) as f32)
                }
            });
            shading_stops.push(ShadingStop {
                offset: off.clamp(0.0, 1.0),
                color: *color,
            });
        }
    } else {
        for (idx, (color, _)) in stops.iter().enumerate() {
            let off = if stops.len() == 1 {
                0.0
            } else {
                idx as f32 / ((stops.len() - 1) as f32)
            };
            shading_stops.push(ShadingStop {
                offset: off,
                color: *color,
            });
        }
    }

    Some(BackgroundPaint::LinearGradient {
        angle_deg,
        stops: shading_stops,
    })
}

fn parse_gradient_angle(part: &str) -> Option<f32> {
    let part = part.trim().to_ascii_lowercase();
    if part.ends_with("deg") {
        let value = part.trim_end_matches("deg").trim();
        return value.parse::<f32>().ok();
    }
    if let Some(rest) = part.strip_prefix("to ") {
        let mut dx = 0;
        let mut dy = 0;
        for dir in rest.split_whitespace() {
            match dir {
                "left" => dx -= 1,
                "right" => dx += 1,
                "top" => dy -= 1,
                "bottom" => dy += 1,
                _ => {}
            }
        }
        if dx == 0 && dy == 0 {
            return None;
        }
        let angle = match (dx, dy) {
            (0, -1) => 0.0,
            (1, -1) => 45.0,
            (1, 0) => 90.0,
            (1, 1) => 135.0,
            (0, 1) => 180.0,
            (-1, 1) => 225.0,
            (-1, 0) => 270.0,
            (-1, -1) => 315.0,
            _ => 180.0,
        };
        return Some(angle);
    }
    None
}

fn parse_gradient_stop(part: &str) -> Option<(Color, Option<f32>)> {
    let part = part.trim();
    if part.is_empty() {
        return None;
    }
    let mut split_at = None;
    let mut depth = 0usize;
    for (idx, ch) in part.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            ' ' if depth == 0 => {
                split_at = Some(idx);
                break;
            }
            _ => {}
        }
    }
    let (color_part, offset_part) = if let Some(idx) = split_at {
        (&part[..idx], part[idx + 1..].trim())
    } else {
        (part, "")
    };
    let (color, _) = parse_color_string(color_part)?;
    let offset = if offset_part.ends_with('%') {
        let v = offset_part
            .trim_end_matches('%')
            .trim()
            .parse::<f32>()
            .ok()?;
        Some((v / 100.0).clamp(0.0, 1.0))
    } else {
        None
    };
    Some((color, offset))
}

fn parse_color_string(raw: &str) -> Option<(Color, f32)> {
    let s = raw.trim().trim_end_matches(',');
    if s.is_empty() {
        return None;
    }
    if let Some(color) = parse_hex_color(s) {
        return Some((color, 1.0));
    }
    let lower = s.to_ascii_lowercase();
    match lower.as_str() {
        "black" => return Some((Color::BLACK, 1.0)),
        "white" => return Some((Color::rgb(1.0, 1.0, 1.0), 1.0)),
        "transparent" => return Some((Color::rgb(1.0, 1.0, 1.0), 0.0)),
        _ => {}
    }
    if lower.starts_with("rgb(") || lower.starts_with("rgba(") {
        let inner = lower
            .trim_start_matches("rgba(")
            .trim_start_matches("rgb(")
            .trim_end_matches(')');
        let parts: Vec<&str> = inner.split(',').map(|p| p.trim()).collect();
        if parts.len() < 3 {
            return None;
        }
        let r = parts[0].parse::<f32>().ok()? / 255.0;
        let g = parts[1].parse::<f32>().ok()? / 255.0;
        let b = parts[2].parse::<f32>().ok()? / 255.0;
        let a = if parts.len() >= 4 {
            parts[3].parse::<f32>().ok()?.clamp(0.0, 1.0)
        } else {
            1.0
        };
        return Some((Color::rgb(r, g, b), a));
    }
    None
}

fn split_args(raw: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut depth = 0usize;
    for ch in raw.chars() {
        match ch {
            '(' => {
                depth += 1;
                current.push(ch);
            }
            ')' => {
                depth = depth.saturating_sub(1);
                current.push(ch);
            }
            ',' if depth == 0 => {
                if !current.trim().is_empty() {
                    out.push(current.trim().to_string());
                }
                current.clear();
            }
            _ => current.push(ch),
        }
    }
    if !current.trim().is_empty() {
        out.push(current.trim().to_string());
    }
    out
}

fn split_ws_preserve_parens(raw: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut depth = 0usize;
    for ch in raw.chars() {
        match ch {
            '(' => {
                depth += 1;
                current.push(ch);
            }
            ')' => {
                depth = depth.saturating_sub(1);
                current.push(ch);
            }
            c if c.is_whitespace() && depth == 0 => {
                if !current.trim().is_empty() {
                    out.push(current.trim().to_string());
                }
                current.clear();
            }
            _ => current.push(ch),
        }
    }
    if !current.trim().is_empty() {
        out.push(current.trim().to_string());
    }
    out
}

fn font_style_spec(value: &CssFontStyle) -> Option<FontStyleMode> {
    let style = match value {
        CssFontStyle::Italic | CssFontStyle::Oblique(_) => FontStyleMode::Italic,
        _ => FontStyleMode::Normal,
    };
    Some(style)
}

fn length_value_to_pt(value: &LengthValue) -> Option<Pt> {
    value.to_px().map(px_to_pt)
}

fn px_to_pt(px: f32) -> Pt {
    Pt::from_f32(px * 0.75)
}

fn absolute_font_size(size: AbsoluteFontSize) -> Pt {
    let px = match size {
        AbsoluteFontSize::XXSmall => 9.0,
        AbsoluteFontSize::XSmall => 10.0,
        AbsoluteFontSize::Small => 13.0,
        AbsoluteFontSize::Medium => 16.0,
        AbsoluteFontSize::Large => 18.0,
        AbsoluteFontSize::XLarge => 24.0,
        AbsoluteFontSize::XXLarge => 32.0,
        AbsoluteFontSize::XXXLarge => 40.0,
    };
    px_to_pt(px)
}

fn css_color_to_color(color: &CssColor) -> Option<Color> {
    if let CssColor::RGBA(rgba) = color {
        let alpha = rgba.alpha as f32 / 255.0;
        // Preblend over white until we support alpha fills directly.
        let r = (rgba.red as f32 / 255.0) * alpha + (1.0 - alpha);
        let g = (rgba.green as f32 / 255.0) * alpha + (1.0 - alpha);
        let b = (rgba.blue as f32 / 255.0) * alpha + (1.0 - alpha);
        return Some(Color::rgb(r, g, b));
    }
    if let Ok(srgb) = SRGB::try_from(color) {
        return Some(Color::rgb(srgb.r, srgb.g, srgb.b));
    }
    None
}

fn font_spec_from_family(families: &[FontFamily]) -> Option<FontSpec> {
    if families.is_empty() {
        return None;
    }

    let mut stack: Vec<Arc<str>> = Vec::new();
    for family in families {
        match family {
            FontFamily::FamilyName(name) => {
                let css_name = name
                    .to_css_string(PrinterOptions::default())
                    .unwrap_or_default();
                let cleaned = css_name
                    .trim()
                    .trim_matches('"')
                    .trim_matches('\'')
                    .to_string();
                if !cleaned.is_empty() {
                    stack.push(Arc::<str>::from(cleaned));
                }
            }
            FontFamily::Generic(generic) => {
                let mapped = match generic {
                    GenericFontFamily::Serif => Some("Times-Roman"),
                    GenericFontFamily::SansSerif => Some("Helvetica"),
                    GenericFontFamily::Monospace => Some("Courier"),
                    GenericFontFamily::Cursive => Some("Helvetica"),
                    GenericFontFamily::Fantasy => Some("Helvetica"),
                    GenericFontFamily::SystemUI => Some("Helvetica"),
                    GenericFontFamily::Emoji => Some("Helvetica"),
                    GenericFontFamily::Math => Some("Helvetica"),
                    GenericFontFamily::FangSong => Some("Helvetica"),
                    GenericFontFamily::UISerif => Some("Times-Roman"),
                    GenericFontFamily::UISansSerif => Some("Helvetica"),
                    GenericFontFamily::UIMonospace => Some("Courier"),
                    GenericFontFamily::UIRounded => Some("Helvetica"),
                    GenericFontFamily::Inherit => return Some(FontSpec::Inherit),
                    GenericFontFamily::Initial
                    | GenericFontFamily::Unset
                    | GenericFontFamily::Default
                    | GenericFontFamily::Revert
                    | GenericFontFamily::RevertLayer => return Some(FontSpec::Initial),
                };
                if let Some(name) = mapped {
                    stack.push(Arc::<str>::from(name));
                }
            }
        }
    }

    if stack.is_empty() {
        None
    } else {
        // Deduplicate while preserving order.
        let mut seen: HashMap<Arc<str>, ()> = HashMap::new();
        let mut deduped = Vec::new();
        for name in stack {
            if !seen.contains_key(&name) {
                seen.insert(name.clone(), ());
                deduped.push(name);
            }
        }
        Some(FontSpec::Value(deduped))
    }
}

fn text_align_mode_from_css(value: &TextAlign) -> TextAlignMode {
    match value {
        TextAlign::Center => TextAlignMode::Center,
        TextAlign::Right | TextAlign::End => TextAlignMode::Right,
        _ => TextAlignMode::Left,
    }
}

fn word_break_mode_from_css(value: &WordBreak) -> WordBreakMode {
    match value {
        WordBreak::KeepAll => WordBreakMode::KeepAll,
        WordBreak::BreakAll => WordBreakMode::BreakAll,
        WordBreak::BreakWord => WordBreakMode::BreakWord,
        WordBreak::Normal => WordBreakMode::Normal,
    }
}

fn word_break_mode_from_overflow_wrap(value: &OverflowWrap) -> WordBreakMode {
    match value {
        OverflowWrap::BreakWord => WordBreakMode::BreakWord,
        OverflowWrap::Anywhere => WordBreakMode::Anywhere,
        OverflowWrap::Normal => WordBreakMode::Normal,
    }
}

fn list_style_type_mode_from_css(value: &ListStyleType) -> ListStyleTypeMode {
    match value {
        ListStyleType::None => ListStyleTypeMode::None,
        _ => ListStyleTypeMode::Auto,
    }
}

fn vertical_align_mode_from_css(value: &CssVerticalAlign) -> VerticalAlignMode {
    match value {
        CssVerticalAlign::Keyword(VerticalAlignKeyword::Middle) => VerticalAlignMode::Middle,
        CssVerticalAlign::Keyword(VerticalAlignKeyword::Bottom)
        | CssVerticalAlign::Keyword(VerticalAlignKeyword::TextBottom) => VerticalAlignMode::Bottom,
        _ => VerticalAlignMode::Top,
    }
}

fn border_line_style_from_line_style(value: &LineStyle) -> BorderLineStyle {
    match value {
        LineStyle::None | LineStyle::Hidden => BorderLineStyle::None,
        _ => BorderLineStyle::Visible,
    }
}

fn border_line_style_from_ident(value: &str) -> Option<BorderLineStyle> {
    match value {
        "none" | "hidden" => Some(BorderLineStyle::None),
        "solid" | "dashed" | "dotted" | "double" | "groove" | "ridge" | "inset" | "outset" => {
            Some(BorderLineStyle::Visible)
        }
        _ => None,
    }
}

fn border_line_style_from_tokens(tokens: &[TokenOrValue]) -> Option<BorderLineStyle> {
    for token in tokens {
        match token {
            TokenOrValue::Token(Token::Ident(ident)) => {
                let value = ident.as_ref().to_ascii_lowercase();
                if let Some(style) = border_line_style_from_ident(&value) {
                    return Some(style);
                }
            }
            TokenOrValue::Token(Token::WhiteSpace(_)) => continue,
            _ => {}
        }
    }
    None
}

fn border_width_spec(value: &BorderSideWidth) -> Option<LengthSpec> {
    match value {
        BorderSideWidth::Thin => Some(LengthSpec::Absolute(Pt::from_f32(0.75))),
        BorderSideWidth::Medium => Some(LengthSpec::Absolute(Pt::from_f32(1.5))),
        BorderSideWidth::Thick => Some(LengthSpec::Absolute(Pt::from_f32(3.0))),
        BorderSideWidth::Length(length) => length.to_px().map(px_to_pt).map(LengthSpec::Absolute),
    }
}

fn display_mode_from_display(display: &Display) -> DisplayMode {
    match display {
        Display::Keyword(keyword) => match keyword {
            DisplayKeyword::None => DisplayMode::None,
            DisplayKeyword::Contents => DisplayMode::Contents,
            DisplayKeyword::TableRowGroup => DisplayMode::TableRowGroup,
            DisplayKeyword::TableHeaderGroup => DisplayMode::TableHeaderGroup,
            DisplayKeyword::TableFooterGroup => DisplayMode::TableFooterGroup,
            DisplayKeyword::TableRow => DisplayMode::TableRow,
            DisplayKeyword::TableCell => DisplayMode::TableCell,
            DisplayKeyword::TableCaption => DisplayMode::TableCaption,
            _ => DisplayMode::Block,
        },
        Display::Pair(pair) => {
            if matches!(pair.inside, DisplayInside::Table) {
                return if pair.outside == DisplayOutside::Inline {
                    DisplayMode::InlineTable
                } else {
                    DisplayMode::Table
                };
            }
            if matches!(pair.inside, DisplayInside::Flex(_)) {
                return if pair.outside == DisplayOutside::Inline {
                    DisplayMode::InlineFlex
                } else {
                    DisplayMode::Flex
                };
            }
            if matches!(pair.inside, DisplayInside::Grid) {
                return if pair.outside == DisplayOutside::Inline {
                    DisplayMode::InlineGrid
                } else {
                    DisplayMode::Grid
                };
            }
            if pair.outside == DisplayOutside::Inline {
                if matches!(pair.inside, DisplayInside::FlowRoot) {
                    DisplayMode::InlineBlock
                } else {
                    DisplayMode::Inline
                }
            } else {
                DisplayMode::Block
            }
        }
    }
}

fn display_mode_name(mode: DisplayMode) -> &'static str {
    match mode {
        DisplayMode::Inline => "inline",
        DisplayMode::Block => "block",
        DisplayMode::Table => "table",
        DisplayMode::InlineTable => "inline-table",
        DisplayMode::TableRowGroup => "table-row-group",
        DisplayMode::TableHeaderGroup => "table-header-group",
        DisplayMode::TableFooterGroup => "table-footer-group",
        DisplayMode::TableRow => "table-row",
        DisplayMode::TableCell => "table-cell",
        DisplayMode::TableCaption => "table-caption",
        DisplayMode::None => "none",
        DisplayMode::Contents => "contents",
        DisplayMode::Flex => "flex",
        DisplayMode::InlineBlock => "inline-block",
        DisplayMode::InlineFlex => "inline-flex",
        DisplayMode::Grid => "grid",
        DisplayMode::InlineGrid => "inline-grid",
    }
}

fn position_mode_from_css(position: &CssPosition) -> PositionMode {
    match position {
        CssPosition::Absolute | CssPosition::Fixed => PositionMode::Absolute,
        CssPosition::Relative | CssPosition::Sticky(_) => PositionMode::Relative,
        _ => PositionMode::Static,
    }
}

fn length_spec_from_lpa(
    value: &lightningcss::values::length::LengthPercentageOrAuto,
) -> Option<LengthSpec> {
    use lightningcss::values::length::LengthPercentageOrAuto;
    match value {
        LengthPercentageOrAuto::Auto => Some(LengthSpec::Auto),
        LengthPercentageOrAuto::LengthPercentage(length) => length_spec_from_lp(length),
    }
}

fn length_spec_from_size(value: &lightningcss::properties::size::Size) -> Option<LengthSpec> {
    use lightningcss::properties::size::Size;
    match value {
        Size::Auto => Some(LengthSpec::Auto),
        Size::LengthPercentage(length) => length_spec_from_lp(length),
        _ => None,
    }
}

fn length_spec_from_max_size(
    value: &lightningcss::properties::size::MaxSize,
) -> Option<LengthSpec> {
    use lightningcss::properties::size::MaxSize;
    match value {
        MaxSize::None => Some(LengthSpec::Auto),
        MaxSize::LengthPercentage(length) => length_spec_from_lp(length),
        _ => None,
    }
}

fn length_spec_from_lp(value: &LengthPercentage) -> Option<LengthSpec> {
    match value {
        LengthPercentage::Percentage(pct) => Some(LengthSpec::Percent(pct.0)),
        LengthPercentage::Dimension(length) => match length {
            LengthValue::Em(val) => Some(LengthSpec::Em(*val)),
            LengthValue::Rem(val) => Some(LengthSpec::Rem(*val)),
            _ => length_value_to_pt(length).map(LengthSpec::Absolute),
        },
        LengthPercentage::Calc(calc) => calc_length_from_calc(calc).map(LengthSpec::Calc),
    }
}

fn calc_length_from_calc(
    calc: &lightningcss::values::calc::Calc<
        lightningcss::values::percentage::DimensionPercentage<LengthValue>,
    >,
) -> Option<CalcLength> {
    use lightningcss::values::calc::{Calc, MathFunction};
    match calc {
        Calc::Value(value) => calc_length_from_dimperc(value),
        Calc::Number(_) => None,
        Calc::Sum(a, b) => {
            let left = calc_length_from_calc(a)?;
            let right = calc_length_from_calc(b)?;
            Some(CalcLength {
                abs: left.abs + right.abs,
                percent: left.percent + right.percent,
                em: left.em + right.em,
                rem: left.rem + right.rem,
            })
        }
        Calc::Product(scale, inner) => {
            let base = calc_length_from_calc(inner)?;
            Some(CalcLength {
                abs: base.abs * *scale,
                percent: base.percent * *scale,
                em: base.em * *scale,
                rem: base.rem * *scale,
            })
        }
        Calc::Function(func) => match func.as_ref() {
            MathFunction::Calc(inner) => calc_length_from_calc(inner),
            _ => None,
        },
    }
}

fn calc_length_from_dimperc(
    value: &lightningcss::values::percentage::DimensionPercentage<LengthValue>,
) -> Option<CalcLength> {
    match value {
        lightningcss::values::percentage::DimensionPercentage::Dimension(length) => {
            calc_length_from_length_value(length)
        }
        lightningcss::values::percentage::DimensionPercentage::Percentage(pct) => {
            Some(CalcLength {
                abs: Pt::ZERO,
                percent: pct.0,
                em: 0.0,
                rem: 0.0,
            })
        }
        lightningcss::values::percentage::DimensionPercentage::Calc(calc) => {
            calc_length_from_calc(calc)
        }
    }
}

fn calc_length_from_length_value(value: &LengthValue) -> Option<CalcLength> {
    match value {
        LengthValue::Em(val) => Some(CalcLength {
            abs: Pt::ZERO,
            percent: 0.0,
            em: *val,
            rem: 0.0,
        }),
        LengthValue::Rem(val) => Some(CalcLength {
            abs: Pt::ZERO,
            percent: 0.0,
            em: 0.0,
            rem: *val,
        }),
        _ => length_value_to_pt(value).map(|abs| CalcLength {
            abs,
            percent: 0.0,
            em: 0.0,
            rem: 0.0,
        }),
    }
}

fn apply_edge_delta(target: &mut EdgeSizes, delta: &EdgeDelta, parent: &EdgeSizes) {
    if let Some(spec) = delta.top {
        target.top = normalize_length_spec(spec, parent.top);
    }
    if let Some(spec) = delta.right {
        target.right = normalize_length_spec(spec, parent.right);
    }
    if let Some(spec) = delta.bottom {
        target.bottom = normalize_length_spec(spec, parent.bottom);
    }
    if let Some(spec) = delta.left {
        target.left = normalize_length_spec(spec, parent.left);
    }
}

fn apply_edge_var_delta(
    target: &mut EdgeSizes,
    delta: &EdgeDelta,
    parent: &EdgeSizes,
    custom_lengths: &HashMap<String, LengthSpec>,
) {
    if let Some(expr) = &delta.top_var {
        if let Some(spec) = custom_lengths.get(&expr.name).copied() {
            target.top = normalize_length_spec(scale_length_spec(spec, expr.scale), parent.top);
        }
    }
    if let Some(expr) = &delta.right_var {
        if let Some(spec) = custom_lengths.get(&expr.name).copied() {
            target.right = normalize_length_spec(scale_length_spec(spec, expr.scale), parent.right);
        }
    }
    if let Some(expr) = &delta.bottom_var {
        if let Some(spec) = custom_lengths.get(&expr.name).copied() {
            target.bottom =
                normalize_length_spec(scale_length_spec(spec, expr.scale), parent.bottom);
        }
    }
    if let Some(expr) = &delta.left_var {
        if let Some(spec) = custom_lengths.get(&expr.name).copied() {
            target.left = normalize_length_spec(scale_length_spec(spec, expr.scale), parent.left);
        }
    }
}

fn normalize_length_spec(spec: LengthSpec, inherited: LengthSpec) -> LengthSpec {
    let spec = match spec {
        LengthSpec::Inherit => inherited,
        LengthSpec::Initial => LengthSpec::Absolute(Pt::ZERO),
        _ => spec,
    };
    quantize_length_spec(spec)
}

fn normalize_size_spec(spec: LengthSpec, inherited: LengthSpec) -> LengthSpec {
    let spec = match spec {
        LengthSpec::Inherit => inherited,
        LengthSpec::Initial => LengthSpec::Auto,
        _ => spec,
    };
    quantize_length_spec(spec)
}

fn quantize_length_spec(spec: LengthSpec) -> LengthSpec {
    match spec {
        LengthSpec::Absolute(value) => LengthSpec::Absolute(value),
        _ => spec,
    }
}

fn apply_inherit_initial_edge(
    tokens: &[TokenOrValue],
    property_id: &PropertyId,
    delta: &mut StyleDelta,
    is_margin: bool,
) {
    if let Some(ident) = first_ident(tokens) {
        let value = match ident.as_str() {
            "inherit" => LengthSpec::Inherit,
            "initial" => LengthSpec::Initial,
            _ => return,
        };
        let target = if is_margin {
            &mut delta.margin
        } else {
            &mut delta.padding
        };
        match property_id {
            PropertyId::Margin | PropertyId::Padding => {
                target.top = Some(value);
                target.right = Some(value);
                target.bottom = Some(value);
                target.left = Some(value);
            }
            PropertyId::MarginTop | PropertyId::PaddingTop => target.top = Some(value),
            PropertyId::MarginRight | PropertyId::PaddingRight => target.right = Some(value),
            PropertyId::MarginBottom | PropertyId::PaddingBottom => target.bottom = Some(value),
            PropertyId::MarginLeft | PropertyId::PaddingLeft => target.left = Some(value),
            _ => {}
        }
    }
    let target = if is_margin {
        &mut delta.margin
    } else {
        &mut delta.padding
    };
    if let Some(expr) = length_var_expr_from_tokens(tokens) {
        match property_id {
            PropertyId::Margin | PropertyId::Padding => {
                target.top_var = Some(expr.clone());
                target.right_var = Some(expr.clone());
                target.bottom_var = Some(expr.clone());
                target.left_var = Some(expr);
            }
            PropertyId::MarginTop | PropertyId::PaddingTop => target.top_var = Some(expr),
            PropertyId::MarginRight | PropertyId::PaddingRight => target.right_var = Some(expr),
            PropertyId::MarginBottom | PropertyId::PaddingBottom => target.bottom_var = Some(expr),
            PropertyId::MarginLeft | PropertyId::PaddingLeft => target.left_var = Some(expr),
            _ => {}
        }
        return;
    }
    if let Some(spec) = length_spec_from_custom_tokens(tokens) {
        match property_id {
            PropertyId::Margin | PropertyId::Padding => {
                target.top = Some(spec);
                target.right = Some(spec);
                target.bottom = Some(spec);
                target.left = Some(spec);
            }
            PropertyId::MarginTop | PropertyId::PaddingTop => target.top = Some(spec),
            PropertyId::MarginRight | PropertyId::PaddingRight => target.right = Some(spec),
            PropertyId::MarginBottom | PropertyId::PaddingBottom => target.bottom = Some(spec),
            PropertyId::MarginLeft | PropertyId::PaddingLeft => target.left = Some(spec),
            _ => {}
        }
    }
}

fn apply_inherit_initial_font_size(tokens: &[TokenOrValue], delta: &mut StyleDelta) {
    if let Some(ident) = first_ident(tokens) {
        match ident.as_str() {
            "inherit" => delta.font_size = Some(FontSizeSpec::Inherit),
            "initial" => delta.font_size = Some(FontSizeSpec::Initial),
            _ => {}
        }
    }
}

fn apply_inherit_initial_line_height(tokens: &[TokenOrValue], delta: &mut StyleDelta) {
    if let Some(ident) = first_ident(tokens) {
        match ident.as_str() {
            "inherit" => delta.line_height = Some(LineHeightSpec::Inherit),
            "initial" => delta.line_height = Some(LineHeightSpec::Initial),
            _ => {}
        }
    }
}

fn apply_inherit_initial_color(tokens: &[TokenOrValue], delta: &mut StyleDelta) {
    if let Some(ident) = first_ident(tokens) {
        match ident.as_str() {
            "inherit" => delta.color = Some(ColorSpec::Inherit),
            "initial" => delta.color = Some(ColorSpec::Initial),
            "currentcolor" => delta.color = Some(ColorSpec::CurrentColor),
            _ => {}
        }
    }
}

fn apply_inherit_initial_background_color(tokens: &[TokenOrValue], delta: &mut StyleDelta) {
    if let Some(ident) = first_ident(tokens) {
        match ident.as_str() {
            "inherit" => delta.background_color = Some(BackgroundSpec::Inherit),
            "initial" => delta.background_color = Some(BackgroundSpec::Initial),
            "currentcolor" => delta.background_color = Some(BackgroundSpec::CurrentColor),
            _ => {}
        }
    }
}

fn apply_inherit_initial_border_color(tokens: &[TokenOrValue], delta: &mut StyleDelta) {
    if let Some(ident) = first_ident(tokens) {
        match ident.as_str() {
            "inherit" => delta.border_color = Some(ColorSpec::Inherit),
            "initial" => delta.border_color = Some(ColorSpec::Initial),
            "currentcolor" => delta.border_color = Some(ColorSpec::CurrentColor),
            _ => {}
        }
    }
}

fn apply_inherit_initial_font_name(tokens: &[TokenOrValue], delta: &mut StyleDelta) {
    if let Some(ident) = first_ident(tokens) {
        match ident.as_str() {
            "inherit" => delta.font_name = Some(FontSpec::Inherit),
            "initial" => delta.font_name = Some(FontSpec::Initial),
            _ => {}
        }
    }
}

fn apply_inherit_initial_font_weight(tokens: &[TokenOrValue], delta: &mut StyleDelta) {
    if let Some(ident) = first_ident(tokens) {
        match ident.as_str() {
            "inherit" => {}
            "initial" => delta.font_weight = Some(400),
            "normal" => delta.font_weight = Some(400),
            "bold" => delta.font_weight = Some(700),
            _ => {}
        }
    }
}

fn apply_inherit_initial_font_style(tokens: &[TokenOrValue], delta: &mut StyleDelta) {
    if let Some(ident) = first_ident(tokens) {
        match ident.as_str() {
            "inherit" => {}
            "initial" | "normal" => delta.font_style = Some(FontStyleMode::Normal),
            "italic" | "oblique" => delta.font_style = Some(FontStyleMode::Italic),
            _ => {}
        }
    }
}

fn apply_inherit_initial_text_transform(tokens: &[TokenOrValue], delta: &mut StyleDelta) {
    if let Some(ident) = first_ident(tokens) {
        match ident.as_str() {
            "inherit" => {}
            "initial" | "none" => delta.text_transform = Some(TextTransformMode::None),
            "uppercase" => delta.text_transform = Some(TextTransformMode::Uppercase),
            "lowercase" => delta.text_transform = Some(TextTransformMode::Lowercase),
            "capitalize" => delta.text_transform = Some(TextTransformMode::Capitalize),
            _ => {}
        }
    }
}

fn apply_inherit_initial_text_decoration(tokens: &[TokenOrValue], delta: &mut StyleDelta) {
    if let Some(ident) = first_ident(tokens) {
        match ident.as_str() {
            "inherit" => delta.text_decoration = Some(TextDecorationSpec::Inherit),
            "initial" => delta.text_decoration = Some(TextDecorationSpec::Initial),
            "none" => {
                delta.text_decoration =
                    Some(TextDecorationSpec::Value(TextDecorationMode::default()))
            }
            _ => {
                if let Some(mode) = parse_text_decoration_str(ident.as_str()) {
                    delta.text_decoration = Some(TextDecorationSpec::Value(mode));
                }
            }
        }
    }
}

fn apply_inherit_initial_text_overflow(tokens: &[TokenOrValue], delta: &mut StyleDelta) {
    if let Some(ident) = first_ident(tokens) {
        match ident.as_str() {
            "inherit" => delta.text_overflow = Some(TextOverflowSpec::Inherit),
            "initial" => delta.text_overflow = Some(TextOverflowSpec::Initial),
            "clip" => delta.text_overflow = Some(TextOverflowSpec::Value(TextOverflowMode::Clip)),
            "ellipsis" => {
                delta.text_overflow = Some(TextOverflowSpec::Value(TextOverflowMode::Ellipsis))
            }
            _ => {}
        }
    }
}

fn apply_inherit_initial_letter_spacing(tokens: &[TokenOrValue], delta: &mut StyleDelta) {
    if let Some(ident) = first_ident(tokens) {
        match ident.as_str() {
            "inherit" => delta.letter_spacing = Some(LengthSpec::Inherit),
            "initial" | "normal" => delta.letter_spacing = Some(LengthSpec::Absolute(Pt::ZERO)),
            _ => {}
        }
    }
}
fn apply_inherit_initial_white_space(tokens: &[TokenOrValue], delta: &mut StyleDelta) {
    if let Some(ident) = first_ident(tokens) {
        match ident.as_str() {
            "inherit" => delta.white_space = Some(WhiteSpaceSpec::Inherit),
            "initial" => delta.white_space = Some(WhiteSpaceSpec::Initial),
            _ => {}
        }
    }
}

fn apply_inherit_initial_display(tokens: &[TokenOrValue], delta: &mut StyleDelta) {
    if let Some(ident) = first_ident(tokens) {
        match ident.as_str() {
            "inherit" => delta.display = Some(DisplaySpec::Inherit),
            "initial" => delta.display = Some(DisplaySpec::Initial),
            _ => {}
        }
    }
}

impl StyleDelta {
    fn is_empty(&self) -> bool {
        self.font_size.is_none()
            && self.line_height.is_none()
            && self.color.is_none()
            && self.background_color.is_none()
            && self.color_var.is_none()
            && self.background_color_var.is_none()
            && self.border_color_var.is_none()
            && self.width.is_none()
            && self.height.is_none()
            && self.width_var.is_none()
            && self.height_var.is_none()
            && self.max_width.is_none()
            && self.min_width.is_none()
            && self.min_height.is_none()
            && self.max_height.is_none()
            && self.max_width_var.is_none()
            && self.text_align.is_none()
            && self.vertical_align.is_none()
            && self.font_weight.is_none()
            && self.font_style.is_none()
            && self.text_transform.is_none()
            && self.text_decoration.is_none()
            && self.text_overflow.is_none()
            && self.content.is_none()
            && self.word_break.is_none()
            && self.list_style_type.is_none()
            && self.letter_spacing.is_none()
            && self.border_width.top.is_none()
            && self.border_width.right.is_none()
            && self.border_width.bottom.is_none()
            && self.border_width.left.is_none()
            && self.border_width.top_var.is_none()
            && self.border_width.right_var.is_none()
            && self.border_width.bottom_var.is_none()
            && self.border_width.left_var.is_none()
            && self.border_color.is_none()
            && self.border_style.top.is_none()
            && self.border_style.right.is_none()
            && self.border_style.bottom.is_none()
            && self.border_style.left.is_none()
            && self.border_collapse.is_none()
            && self.caption_side.is_none()
            && self.border_spacing.is_none()
            && self.border_radius.is_none()
            && self.font_name.is_none()
            && self.font_name_var.is_none()
            && self.box_shadow.is_none()
            && self.background_paint.is_none()
            && self.pagination.break_before.is_none()
            && self.pagination.break_after.is_none()
            && self.pagination.break_inside.is_none()
            && self.pagination.orphans.is_none()
            && self.pagination.widows.is_none()
            && self.margin.top.is_none()
            && self.margin.right.is_none()
            && self.margin.bottom.is_none()
            && self.margin.left.is_none()
            && self.margin.top_var.is_none()
            && self.margin.right_var.is_none()
            && self.margin.bottom_var.is_none()
            && self.margin.left_var.is_none()
            && self.padding.top.is_none()
            && self.padding.right.is_none()
            && self.padding.bottom.is_none()
            && self.padding.left.is_none()
            && self.padding.top_var.is_none()
            && self.padding.right_var.is_none()
            && self.padding.bottom_var.is_none()
            && self.padding.left_var.is_none()
            && self.white_space.is_none()
            && self.display.is_none()
            && self.position.is_none()
            && self.z_index.is_none()
            && self.box_sizing.is_none()
            && self.inset_left.is_none()
            && self.inset_top.is_none()
            && self.inset_right.is_none()
            && self.inset_bottom.is_none()
            && self.flex_direction.is_none()
            && self.flex_wrap.is_none()
            && self.flex_basis.is_none()
            && self.order.is_none()
            && self.justify_content.is_none()
            && self.align_items.is_none()
            && self.grid_columns.is_none()
            && self.gap.is_none()
            && self.flex_grow.is_none()
            && self.flex_shrink.is_none()
            && self.overflow.is_none()
            && self.custom_lengths.is_empty()
            && self.custom_colors.is_empty()
            && self.custom_color_alpha.is_empty()
            && self.custom_color_refs.is_empty()
            && self.custom_font_stacks.is_empty()
    }
}

fn default_ua_css() -> &'static str {
    r#"
    body { font-size: 16px; line-height: 1.2; color: #000; }
    h1 { font-size: 2em; }
    h2 { font-size: 1.5em; }
    h3 { font-size: 1.17em; }
    h4 { font-size: 1em; }
    h5 { font-size: 0.83em; }
    h6 { font-size: 0.67em; }
    pre { white-space: pre; }
    code, kbd, samp, tt { white-space: pre; }
    span, a, em, strong, i, b, u, small, label { display: inline; }
    /* Treat <svg> like a replaced inline element so it participates in inline layout. */
    svg { display: inline-block; }
    /* Treat <img> like a replaced inline element so it is rendered as an atomic box. */
    img { display: inline-block; }
    div, p, section, article, header, footer, aside, nav, main, blockquote,
    h1, h2, h3, h4, h5, h6,
    ul, ol, dl, dt, dd, li, table, thead, tbody, tfoot, tr, td, th, pre, hr { display: block; }
    table { break-inside: avoid; }
    thead { break-inside: avoid; }
    "#
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn element(tag: &str, id: Option<&str>, classes: &[&str]) -> ElementInfo {
        ElementInfo {
            tag: tag.to_string(),
            id: id.map(|v| v.to_string()),
            classes: classes.iter().map(|c| c.to_string()).collect(),
            attrs: HashMap::new(),
            is_root: false,
            child_index: 1,
            child_count: 1,
            prev_siblings: Vec::new(),
        }
    }

    #[test]
    fn descendant_selector_overrides_simple() {
        let css = "p { color: blue; } div p { color: red; }";
        let resolver = StyleResolver::new(css);
        let root = resolver.default_style();
        let div_info = element("div", None, &[]);
        let div_style = resolver.compute_style(&div_info, &root, None, &[]);
        let p_info = element("p", None, &[]);
        let p_style = resolver.compute_style(&p_info, &div_style, None, &[div_info]);
        assert_eq!(p_style.color, Color::rgb(1.0, 0.0, 0.0));
    }

    #[test]
    fn important_overrides_specificity() {
        let css = "p.note { color: blue; } p { color: red !important; }";
        let resolver = StyleResolver::new(css);
        let root = resolver.default_style();
        let p_info = element("p", None, &["note"]);
        let p_style = resolver.compute_style(&p_info, &root, None, &[]);
        assert_eq!(p_style.color, Color::rgb(1.0, 0.0, 0.0));
    }

    #[test]
    fn inherit_and_initial_color() {
        let css = "div { color: red; } p { color: inherit; } span { color: initial; }";
        let resolver = StyleResolver::new(css);
        let root = resolver.default_style();
        let div_info = element("div", None, &[]);
        let div_style = resolver.compute_style(&div_info, &root, None, &[]);
        let p_info = element("p", None, &[]);
        let p_style = resolver.compute_style(&p_info, &div_style, None, &[div_info.clone()]);
        assert_eq!(p_style.color, Color::rgb(1.0, 0.0, 0.0));

        let span_info = element("span", None, &[]);
        let span_style = resolver.compute_style(&span_info, &p_style, None, &[div_info, p_info]);
        assert_eq!(span_style.color, Color::BLACK);
    }

    #[test]
    fn inline_style_wins_over_rules() {
        let css = "p { color: blue; }";
        let resolver = StyleResolver::new(css);
        let root = resolver.default_style();
        let p_info = element("p", None, &[]);
        let p_style = resolver.compute_style(&p_info, &root, Some("color: rgb(0,255,0);"), &[]);
        assert_eq!(p_style.color, Color::rgb(0.0, 1.0, 0.0));
    }

    #[test]
    fn ua_css_sets_img_display_inline_block() {
        let resolver = StyleResolver::new("");
        let root = resolver.default_style();
        let img_info = element("img", None, &[]);
        let img_style = resolver.compute_style(&img_info, &root, None, &[]);
        assert_eq!(img_style.display, DisplayMode::InlineBlock);
    }

    #[test]
    fn extract_css_page_setup_parses_size_and_margin() {
        let css = "@page { size: 8.5in 11in; margin: 0.5in 1in; }";
        let setup = extract_css_page_setup(css, None, None);
        let size = setup.size.expect("expected @page size");
        assert!((size.width.to_f32() - 612.0).abs() < 0.01);
        assert!((size.height.to_f32() - 792.0).abs() < 0.01);
        assert_eq!(setup.margin_top, Some(Pt::from_f32(36.0)));
        assert_eq!(setup.margin_right, Some(Pt::from_f32(72.0)));
        assert_eq!(setup.margin_bottom, Some(Pt::from_f32(36.0)));
        assert_eq!(setup.margin_left, Some(Pt::from_f32(72.0)));
    }

    #[test]
    fn extract_css_page_setup_handles_named_size_orientation() {
        let css = "@page { size: letter landscape; }";
        let setup = extract_css_page_setup(css, None, None);
        let size = setup.size.expect("expected @page size");
        assert!((size.width.to_f32() - 792.0).abs() < 0.01);
        assert!((size.height.to_f32() - 612.0).abs() < 0.01);
    }

    #[test]
    fn debug_logs_declaration_parsed_no_effect_for_unknown_property() {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let path = std::env::temp_dir().join(format!(
            "fullbleed_style_known_loss_{}_{}.jsonl",
            std::process::id(),
            nanos
        ));
        let logger = Arc::new(DebugLogger::new(&path).expect("debug logger"));
        let resolver = StyleResolver::new_with_debug(
            "div { vendor-unknown-prop: 12px; color: red; }",
            Some(logger.clone()),
        );
        drop(resolver);
        drop(logger);
        let log = std::fs::read_to_string(&path).expect("read debug log");
        assert!(log.contains("\"DECLARATION_PARSED_NO_EFFECT\""));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn display_table_is_not_normalized() {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let path = std::env::temp_dir().join(format!(
            "fullbleed_style_layout_known_loss_{}_{}.jsonl",
            std::process::id(),
            nanos
        ));
        let logger = Arc::new(DebugLogger::new(&path).expect("debug logger"));
        let resolver =
            StyleResolver::new_with_debug(".menu { display: table; }", Some(logger.clone()));
        drop(resolver);
        drop(logger);
        let log = std::fs::read_to_string(&path).expect("read debug log");
        assert!(
            !log.contains("\"LAYOUT_MODE_NORMALIZED\""),
            "display:table should be preserved, log={log}"
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn grid_template_columns_sets_column_count() {
        let resolver =
            StyleResolver::new(".menu { display: grid; grid-template-columns: 1fr 1fr 1fr; }");
        let root = resolver.default_style();
        let info = element("div", None, &["menu"]);
        let style = resolver.compute_style(&info, &root, None, &[]);
        assert_eq!(style.display, DisplayMode::Grid);
        assert_eq!(style.grid_columns, Some(3));
    }

    #[test]
    fn display_table_keywords_map_to_table_modes() {
        let resolver = StyleResolver::new(
            ".t{display:table}.it{display:inline-table}.tr{display:table-row}.tc{display:table-cell}.th{display:table-header-group}",
        );
        let root = resolver.default_style();
        let t = resolver.compute_style(&element("div", None, &["t"]), &root, None, &[]);
        let it = resolver.compute_style(&element("div", None, &["it"]), &root, None, &[]);
        let tr = resolver.compute_style(&element("div", None, &["tr"]), &root, None, &[]);
        let tc = resolver.compute_style(&element("div", None, &["tc"]), &root, None, &[]);
        let th = resolver.compute_style(&element("div", None, &["th"]), &root, None, &[]);
        assert_eq!(t.display, DisplayMode::Table);
        assert_eq!(it.display, DisplayMode::InlineTable);
        assert_eq!(tr.display, DisplayMode::TableRow);
        assert_eq!(tc.display, DisplayMode::TableCell);
        assert_eq!(th.display, DisplayMode::TableHeaderGroup);
    }

    #[test]
    fn relative_font_size_uses_parent() {
        let css = "div { font-size: 20px; } p { font-size: 50%; }";
        let resolver = StyleResolver::new(css);
        let root = resolver.default_style();
        let div_info = element("div", None, &[]);
        let div_style = resolver.compute_style(&div_info, &root, None, &[]);
        let p_info = element("p", None, &[]);
        let p_style = resolver.compute_style(&p_info, &div_style, None, &[div_info]);
        assert!((p_style.font_size.to_f32() - 7.5).abs() < 0.01);
    }

    #[test]
    fn font_size_calc_resolves_viewport_units() {
        let css = ":root { font-size: 20px; } .display { font-size: calc(1rem + 1vw); }";
        let viewport = Size {
            width: Pt::from_f32(600.0),
            height: Pt::from_f32(800.0),
        };
        let resolver = StyleResolver::new_with_debug_and_viewport(css, None, Some(viewport));
        let root = resolver.default_style();
        let mut root_info = element("html", None, &[]);
        root_info.is_root = true;
        let root_style = resolver.compute_style(&root_info, &root, None, &[]);
        let child_info = element("div", None, &["display"]);
        let child_style = resolver.compute_style(&child_info, &root_style, None, &[root_info]);
        assert!((child_style.font_size.to_f32() - 21.0).abs() < 0.01);
    }

    #[test]
    fn margin_and_padding_shorthand_apply() {
        let css = "div { margin: 10px 20px; padding: 5px; }";
        let resolver = StyleResolver::new(css);
        let root = resolver.default_style();
        let div_info = element("div", None, &[]);
        let div_style = resolver.compute_style(&div_info, &root, None, &[]);
        assert_eq!(
            div_style.margin.top,
            LengthSpec::Absolute(Pt::from_f32(7.5))
        );
        assert_eq!(
            div_style.margin.right,
            LengthSpec::Absolute(Pt::from_f32(15.0))
        );
        assert_eq!(
            div_style.margin.bottom,
            LengthSpec::Absolute(Pt::from_f32(7.5))
        );
        assert_eq!(
            div_style.margin.left,
            LengthSpec::Absolute(Pt::from_f32(15.0))
        );
        assert_eq!(
            div_style.padding.top,
            LengthSpec::Absolute(Pt::from_f32(3.75))
        );
        assert_eq!(
            div_style.padding.right,
            LengthSpec::Absolute(Pt::from_f32(3.75))
        );
        assert_eq!(
            div_style.padding.bottom,
            LengthSpec::Absolute(Pt::from_f32(3.75))
        );
        assert_eq!(
            div_style.padding.left,
            LengthSpec::Absolute(Pt::from_f32(3.75))
        );
    }

    #[test]
    fn custom_property_var_height_resolves() {
        let css = ".bar { height: var(--h); }";
        let resolver = StyleResolver::new(css);
        let root = resolver.default_style();
        let bar_info = element("div", None, &["bar"]);
        let bar_style = resolver.compute_style(&bar_info, &root, Some("--h: 40%;"), &[]);
        assert_eq!(
            bar_style.custom_lengths.get("--h").copied(),
            Some(LengthSpec::Percent(0.4))
        );
        assert_eq!(bar_style.height, LengthSpec::Percent(0.4));
    }

    #[test]
    fn text_decoration_parses_and_resets() {
        let css = "div { text-decoration: underline; } span { text-decoration: none; }";
        let resolver = StyleResolver::new(css);
        let root = resolver.default_style();
        let div_info = element("div", None, &[]);
        let div_style = resolver.compute_style(&div_info, &root, None, &[]);
        assert!(div_style.text_decoration.underline);
        assert!(!div_style.text_decoration.line_through);

        let span_info = element("span", None, &[]);
        let span_style = resolver.compute_style(&span_info, &div_style, None, &[div_info]);
        assert!(span_style.text_decoration.is_none());
    }

    #[test]
    fn border_collapse_custom_property_applies() {
        let css = "table { border-collapse: collapse; }";
        let resolver = StyleResolver::new(css);
        let root = resolver.default_style();
        let table_info = element("table", None, &[]);
        let table_style = resolver.compute_style(&table_info, &root, None, &[]);
        assert_eq!(table_style.border_collapse, BorderCollapseMode::Collapse);
    }

    #[test]
    fn caption_side_custom_property_applies() {
        let css = "table { caption-side: bottom; } .caption-top { caption-side: top; }";
        let resolver = StyleResolver::new(css);
        let root = resolver.default_style();

        let table_info = element("table", None, &[]);
        let table_style = resolver.compute_style(&table_info, &root, None, &[]);
        assert_eq!(table_style.caption_side, CaptionSideMode::Bottom);

        let table_top_info = element("table", None, &["caption-top"]);
        let table_top_style = resolver.compute_style(&table_top_info, &root, None, &[]);
        assert_eq!(table_top_style.caption_side, CaptionSideMode::Top);
    }

    #[test]
    fn border_spacing_two_values_parse() {
        let css = "table { border-spacing: 2px 4px; }";
        let resolver = StyleResolver::new(css);
        let root = resolver.default_style();
        let table_info = element("table", None, &[]);
        let table_style = resolver.compute_style(&table_info, &root, None, &[]);
        assert_eq!(
            table_style.border_spacing.horizontal,
            LengthSpec::Absolute(Pt::from_f32(1.5))
        );
        assert_eq!(
            table_style.border_spacing.vertical,
            LengthSpec::Absolute(Pt::from_f32(3.0))
        );
    }

    #[test]
    fn box_shadow_var_parses_for_unparsed_paths() {
        let css =
            ".x { --shade: rgba(0, 0, 0, 0.05); box-shadow: inset 0 0 0 9999px var(--shade); }";
        let resolver = StyleResolver::new(css);
        let root = resolver.default_style();
        let info = element("div", None, &["x"]);
        let style = resolver.compute_style(&info, &root, None, &[]);
        let shadow = style.box_shadow.expect("expected box-shadow to parse");
        assert!(shadow.inset);
        match shadow.spread {
            LengthSpec::Absolute(value) => assert!(value > Pt::from_f32(100.0)),
            other => panic!("expected absolute spread, got {other:?}"),
        }
        assert!((shadow.color.r - 0.95).abs() < 0.01);
        assert!((shadow.color.g - 0.95).abs() < 0.01);
        assert!((shadow.color.b - 0.95).abs() < 0.01);
        assert!((shadow.opacity - 1.0).abs() < 0.001);
    }

    #[test]
    fn color_rgba_var_expression_resolves_in_parsed_path() {
        let css = ":root { --bs-success-rgb: 25, 135, 84; } .x { --bs-text-opacity: 1; color: rgba(var(--bs-success-rgb), var(--bs-text-opacity)); }";
        let resolver = StyleResolver::new(css);
        let root = resolver.default_style();
        let mut root_info = element("html", None, &[]);
        root_info.is_root = true;
        let root_style = resolver.compute_style(&root_info, &root, None, &[]);
        assert!(
            root_style.custom_colors.contains_key("--bs-success-rgb"),
            "root custom colors missing --bs-success-rgb: {:?}",
            root_style.custom_colors.keys().collect::<Vec<_>>()
        );
        let info = element("p", None, &["x"]);
        let style = resolver.compute_style(&info, &root_style, None, &[root_info]);
        assert!(
            style.custom_colors.contains_key("--bs-success-rgb"),
            "child custom colors missing --bs-success-rgb: {:?}",
            style.custom_colors.keys().collect::<Vec<_>>()
        );
        assert!(
            (style.color.r - (25.0 / 255.0)).abs() < 0.01,
            "unexpected r={:?}",
            style.color
        );
        assert!(
            (style.color.g - (135.0 / 255.0)).abs() < 0.01,
            "unexpected g={:?}",
            style.color
        );
        assert!(
            (style.color.b - (84.0 / 255.0)).abs() < 0.01,
            "unexpected b={:?}",
            style.color
        );
    }

    #[test]
    fn box_shadow_var_fallback_chain_resolves() {
        let css = ".x { --shade: rgba(0, 0, 0, 0.05); box-shadow: inset 0 0 0 9999px var(--missing, var(--shade)); }";
        let resolver = StyleResolver::new(css);
        let root = resolver.default_style();
        let info = element("div", None, &["x"]);
        let style = resolver.compute_style(&info, &root, None, &[]);
        let shadow = style.box_shadow.expect("expected box-shadow to parse");
        assert!(shadow.inset);
        assert!(
            (shadow.color.r - 0.95).abs() < 0.01,
            "unexpected shadow={shadow:?}"
        );
        assert!(
            (shadow.color.g - 0.95).abs() < 0.01,
            "unexpected shadow={shadow:?}"
        );
        assert!(
            (shadow.color.b - 0.95).abs() < 0.01,
            "unexpected shadow={shadow:?}"
        );
        assert!(shadow.opacity > 0.9);
    }

    #[test]
    fn unresolved_box_shadow_var_is_transparent() {
        let css = ".x { box-shadow: inset 0 0 0 9999px var(--missing); }";
        let resolver = StyleResolver::new(css);
        let root = resolver.default_style();
        let info = element("div", None, &["x"]);
        let style = resolver.compute_style(&info, &root, None, &[]);
        let shadow = style.box_shadow.expect("expected box-shadow to parse");
        assert!(shadow.inset);
        assert!(shadow.opacity <= 0.001);
    }

    #[test]
    fn custom_property_rgba_var_expression_resolves_for_background() {
        let css = ":root { --bs-emphasis-color-rgb: 0, 0, 0; --bs-table-striped-bg: rgba(var(--bs-emphasis-color-rgb), 0.05); } .x { background-color: var(--bs-table-striped-bg); }";
        let resolver = StyleResolver::new(css);
        let root = resolver.default_style();
        let mut root_info = element("html", None, &[]);
        root_info.is_root = true;
        let root_style = resolver.compute_style(&root_info, &root, None, &[]);
        let info = element("div", None, &["x"]);
        let style = resolver.compute_style(&info, &root_style, None, &[root_info]);
        let bg = style
            .background_color
            .expect("expected background color from custom rgba expression");
        assert!((bg.r - 0.95).abs() < 0.01, "unexpected background={bg:?}");
        assert!((bg.g - 0.95).abs() < 0.01, "unexpected background={bg:?}");
        assert!((bg.b - 0.95).abs() < 0.01, "unexpected background={bg:?}");
    }

    #[test]
    fn border_color_rgba_var_expression_resolves_for_unparsed_property() {
        let css = ":root { --bs-success-rgb: 25, 135, 84; --bs-border-opacity: 1; } .x { border-width: 1px; border-color: rgba(var(--bs-success-rgb), var(--bs-border-opacity)); }";
        let resolver = StyleResolver::new(css);
        let root = resolver.default_style();
        let mut root_info = element("html", None, &[]);
        root_info.is_root = true;
        let root_style = resolver.compute_style(&root_info, &root, None, &[]);
        let info = element("div", None, &["x"]);
        let style = resolver.compute_style(&info, &root_style, None, &[root_info]);
        let border = style
            .border_color
            .expect("expected resolved border-color expression");
        assert!((border.r - (25.0 / 255.0)).abs() < 0.01);
        assert!((border.g - (135.0 / 255.0)).abs() < 0.01);
        assert!((border.b - (84.0 / 255.0)).abs() < 0.01);
    }

    #[test]
    fn border_inline_start_var_width_and_color_resolve() {
        let css = ":root { --bs-border-width: 1px; --bs-border-color: #dee2e6; } .x { border-inline-start: var(--bs-border-width) solid var(--bs-border-color); }";
        let resolver = StyleResolver::new(css);
        let root = resolver.default_style();
        let mut root_info = element("html", None, &[]);
        root_info.is_root = true;
        let root_style = resolver.compute_style(&root_info, &root, None, &[]);
        let info = element("div", None, &["x"]);
        let style = resolver.compute_style(&info, &root_style, None, &[root_info]);
        assert_eq!(
            style.border_width.left,
            LengthSpec::Absolute(Pt::from_f32(0.75))
        );
        assert_eq!(style.border_width.top, LengthSpec::Absolute(Pt::ZERO));
        assert_eq!(style.border_width.right, LengthSpec::Absolute(Pt::ZERO));
        assert_eq!(style.border_width.bottom, LengthSpec::Absolute(Pt::ZERO));
        let border = style
            .border_color
            .expect("expected border color from border-inline-start");
        assert!((border.r - (0xde as f32 / 255.0)).abs() < 0.01);
        assert!((border.g - (0xe2 as f32 / 255.0)).abs() < 0.01);
        assert!((border.b - (0xe6 as f32 / 255.0)).abs() < 0.01);
    }

    #[test]
    fn bootstrap_border_start_with_border_width_keeps_only_start_edge() {
        let css =
            ".border-start { border-left: 1px solid #198754; } .border-3 { border-width: 3px; }";
        let resolver = StyleResolver::new(css);
        let root = resolver.default_style();
        let info = element("div", None, &["border-start", "border-3"]);
        let style = resolver.compute_style(&info, &root, None, &[]);
        assert_eq!(style.border_width.top, LengthSpec::Absolute(Pt::ZERO));
        assert_eq!(style.border_width.right, LengthSpec::Absolute(Pt::ZERO));
        assert_eq!(style.border_width.bottom, LengthSpec::Absolute(Pt::ZERO));
        assert_eq!(
            style.border_width.left,
            LengthSpec::Absolute(Pt::from_f32(2.25))
        );
    }

    #[test]
    fn inline_style_matches_stylesheet_for_table_custom_properties() {
        let sheet_resolver = StyleResolver::new(
            ".from-sheet { border-collapse: collapse; caption-side: bottom; border-spacing: 2px 4px; }",
        );
        let sheet_root = sheet_resolver.default_style();
        let sheet_table = element("table", None, &["from-sheet"]);
        let from_sheet = sheet_resolver.compute_style(&sheet_table, &sheet_root, None, &[]);

        let inline_resolver = StyleResolver::new("");
        let inline_root = inline_resolver.default_style();
        let inline_table = element("table", None, &[]);
        let from_inline = inline_resolver.compute_style(
            &inline_table,
            &inline_root,
            Some("border-collapse: collapse; caption-side: bottom; border-spacing: 2px 4px;"),
            &[],
        );

        assert_eq!(from_inline.border_collapse, from_sheet.border_collapse);
        assert_eq!(from_inline.caption_side, from_sheet.caption_side);
        assert_eq!(
            from_inline.border_spacing.horizontal,
            from_sheet.border_spacing.horizontal
        );
        assert_eq!(
            from_inline.border_spacing.vertical,
            from_sheet.border_spacing.vertical
        );
    }

    #[test]
    fn inline_style_matches_stylesheet_for_pagination_properties() {
        let sheet_resolver = StyleResolver::new(
            ".from-sheet { break-before: page; break-after: page; break-inside: avoid-page; orphans: 4; widows: 5; }",
        );
        let sheet_root = sheet_resolver.default_style();
        let sheet_block = element("div", None, &["from-sheet"]);
        let from_sheet = sheet_resolver.compute_style(&sheet_block, &sheet_root, None, &[]);

        let inline_resolver = StyleResolver::new("");
        let inline_root = inline_resolver.default_style();
        let inline_block = element("div", None, &[]);
        let from_inline = inline_resolver.compute_style(
            &inline_block,
            &inline_root,
            Some(
                "break-before: page; break-after: page; break-inside: avoid-page; orphans: 4; widows: 5;",
            ),
            &[],
        );

        assert_eq!(from_inline.pagination, from_sheet.pagination);
    }
}
