//! Dependency-free OpenType shaping for the layout contract used by FullBleed.

use crate::sfnt::{Face, GlyphId};
use crate::text_shape::{ShapeOptions, ShapedGlyph, ShapedText};

type Tag = [u8; 4];

const GSUB_EARLY_FEATURES: [Tag; 1] = [*b"rvrn"];
const LTR_FEATURES: [Tag; 2] = [*b"ltra", *b"ltrm"];
const RTL_FEATURES: [Tag; 1] = [*b"rtla"];
const GSUB_FEATURES: [Tag; 7] = [
    *b"ccmp", *b"locl", *b"rlig", *b"rclt", *b"calt", *b"liga", *b"clig",
];
const HANGUL_GSUB_FEATURES: [Tag; 6] = [*b"ccmp", *b"locl", *b"rlig", *b"rclt", *b"liga", *b"clig"];
const GPOS_FEATURES: [Tag; 7] = [
    *b"abvm", *b"blwm", *b"curs", *b"dist", *b"kern", *b"mark", *b"mkmk",
];
const INDIC_GPOS_FEATURES: [Tag; 7] = [
    *b"kern", *b"dist", *b"abvm", *b"blwm", *b"mark", *b"mkmk", *b"curs",
];
const INDIC_INITIAL_FEATURES: [Tag; 2] = [*b"locl", *b"ccmp"];
const INDIC_BASIC_FEATURES: [Tag; 10] = [
    *b"nukt", *b"akhn", *b"rphf", *b"rkrf", *b"pref", *b"blwf", *b"abvf", *b"half", *b"pstf",
    *b"vatu",
];
const INDIC_FINAL_FEATURES: [Tag; 7] = [
    *b"cjct", *b"init", *b"pres", *b"abvs", *b"blws", *b"psts", *b"haln",
];
const ARABIC_INITIAL_FEATURES: [Tag; 3] = [*b"stch", *b"ccmp", *b"locl"];
const ARABIC_FORM_FEATURES: [Tag; 7] = [
    *b"isol", *b"fina", *b"fin2", *b"fin3", *b"medi", *b"med2", *b"init",
];
const ARABIC_REQUIRED_FEATURES: [Tag; 1] = [*b"rlig"];
const ARABIC_CONTEXT_FEATURES: [Tag; 2] = [*b"rclt", *b"calt"];
const ARABIC_FINAL_FEATURES: [Tag; 3] = [*b"mset", *b"liga", *b"clig"];
const FRACTION_NUMERATOR: u8 = 0x01;
const FRACTION_DENOMINATOR: u8 = 0x02;
const FRACTION_ALL: u8 = 0x04;

fn configured_features(features: &[Tag], options: ShapeOptions) -> Vec<Tag> {
    features
        .iter()
        .copied()
        .filter(|feature| {
            (options.kerning || *feature != *b"kern")
                && (options.common_ligatures || (*feature != *b"liga" && *feature != *b"clig"))
        })
        .collect()
}

fn apply_configured_gsub_features(
    table: &[u8],
    gdef: Option<&[u8]>,
    script: Tag,
    features: &[Tag],
    options: ShapeOptions,
    glyphs: &mut Vec<BufferGlyph>,
) -> Option<()> {
    let configured = configured_features(features, options);
    apply_gsub_features(table, gdef, script, &configured, glyphs)
}

#[derive(Clone, Debug)]
struct BufferGlyph {
    id: u16,
    cluster: u32,
    codepoint: u32,
    x_advance: i32,
    y_advance: i32,
    x_offset: i32,
    y_offset: i32,
    arabic_action: u8,
    indic_reph: bool,
    cursive_parent: Option<usize>,
    mark_parent: Option<usize>,
    mark_x_delta: i32,
    mark_y_delta: i32,
    ligature_id: u32,
    ligature_component: u16,
    ligature_components: u16,
    rtlm: bool,
    fraction_mask: u8,
}

#[derive(Clone, Copy, Debug)]
struct LookupFilter {
    flags: u16,
    mark_filtering_set: Option<u16>,
}

pub(crate) fn shape(font_data: &[u8], text: &str, options: ShapeOptions) -> Option<ShapedText> {
    let face = Face::parse(font_data, 0).ok()?;
    let right_to_left =
        crate::text_shape::detect_direction(text) == crate::text_shape::TextDirection::RightToLeft;
    let mut glyphs = initial_glyphs(&face, text);
    if glyphs.is_empty() {
        return Some(ShapedText {
            units_per_em: face.units_per_em(),
            glyphs: Vec::new(),
        });
    }
    canonical_order(&mut glyphs);
    let script = script_tag(text);
    if right_to_left {
        for glyph in &mut glyphs {
            if let Some(mirrored) = crate::unicode_data::bidi_mirror(glyph.codepoint) {
                if let Some(glyph_id) = face.glyph_index(mirrored) {
                    glyph.codepoint = mirrored;
                    glyph.id = glyph_id.0;
                    continue;
                }
            }
            glyph.rtlm = true;
        }
    }
    assign_fraction_masks(&mut glyphs, right_to_left);
    let gdef = face.table(*b"GDEF");
    if let Some(gsub) = face.table(*b"GSUB") {
        apply_gsub_features(gsub, gdef, script, &GSUB_EARLY_FEATURES, &mut glyphs)?;
        apply_gsub_features(
            gsub,
            gdef,
            script,
            if right_to_left {
                &RTL_FEATURES[..]
            } else {
                &LTR_FEATURES[..]
            },
            &mut glyphs,
        )?;
        if right_to_left {
            apply_gsub_feature_for_rtlm(gsub, gdef, script, &mut glyphs)?;
        }
        if script == *b"dev2" {
            apply_gsub_features(gsub, gdef, script, &INDIC_INITIAL_FEATURES, &mut glyphs)?;
            insert_devanagari_dotted_circles(&face, &mut glyphs);
            initial_reorder_devanagari(&mut glyphs);
            for feature in INDIC_BASIC_FEATURES {
                apply_gsub_features(gsub, gdef, script, &[feature], &mut glyphs)?;
            }
            final_reorder_devanagari(&mut glyphs);
            apply_gsub_features(gsub, gdef, script, &INDIC_FINAL_FEATURES, &mut glyphs)?;
        } else if script == *b"arab" {
            assign_arabic_actions(&mut glyphs);
            apply_gsub_features(gsub, gdef, script, &ARABIC_INITIAL_FEATURES, &mut glyphs)?;
            for (action, feature) in ARABIC_FORM_FEATURES.into_iter().enumerate() {
                apply_gsub_feature_for_action(
                    gsub,
                    gdef,
                    script,
                    feature,
                    u8::try_from(action).ok()?,
                    &mut glyphs,
                )?;
            }
            apply_gsub_features(gsub, gdef, script, &ARABIC_REQUIRED_FEATURES, &mut glyphs)?;
            apply_gsub_features(gsub, gdef, script, &ARABIC_CONTEXT_FEATURES, &mut glyphs)?;
            apply_configured_gsub_features(
                gsub,
                gdef,
                script,
                &ARABIC_FINAL_FEATURES,
                options,
                &mut glyphs,
            )?;
        } else if script == *b"hang" {
            apply_configured_gsub_features(
                gsub,
                gdef,
                script,
                &HANGUL_GSUB_FEATURES,
                options,
                &mut glyphs,
            )?;
        } else {
            apply_configured_gsub_features(
                gsub,
                gdef,
                script,
                &GSUB_FEATURES,
                options,
                &mut glyphs,
            )?;
        }
        apply_fraction_features(gsub, gdef, script, &mut glyphs)?;
    }
    if script == *b"arab" {
        reorder_arabic_shadda(&mut glyphs);
    }
    let space_glyph = face.glyph_index(0x20).map(|glyph| glyph.0).unwrap_or(0);
    for glyph in &mut glyphs {
        if is_default_ignorable(glyph.codepoint) {
            glyph.id = space_glyph;
        } else if face.glyph_index(glyph.codepoint).is_none()
            && unicode_space_fallback(glyph.codepoint) != 0
        {
            glyph.id = space_glyph;
        }
        glyph.x_advance = i32::from(
            face.glyph_hor_advance(GlyphId(glyph.id))
                .unwrap_or_default(),
        );
        if is_default_ignorable(glyph.codepoint)
            || (script != *b"dev2" && gdef_glyph_class(gdef, glyph.id) == 3)
        {
            glyph.x_advance = 0;
        }
    }
    if let Some(gpos) = face.table(*b"GPOS") {
        let features = if script == *b"dev2" {
            &INDIC_GPOS_FEATURES[..]
        } else {
            &GPOS_FEATURES[..]
        };
        let configured = configured_features(features, options);
        apply_gpos(gpos, gdef, script, &configured, &mut glyphs, right_to_left)?;
    } else if options.kerning {
        apply_legacy_kerning(&face, &mut glyphs);
    }
    resolve_cursive_offsets(&mut glyphs);
    resolve_mark_offsets(&mut glyphs, right_to_left);
    apply_space_fallbacks(&face, &mut glyphs);
    if right_to_left {
        glyphs.reverse();
    }
    Some(ShapedText {
        units_per_em: face.units_per_em(),
        glyphs: glyphs
            .into_iter()
            .map(|glyph| ShapedGlyph {
                glyph_id: glyph.id,
                cluster: glyph.cluster,
                x_advance: glyph.x_advance,
                y_advance: glyph.y_advance,
                x_offset: glyph.x_offset,
                y_offset: glyph.y_offset,
            })
            .collect(),
    })
}

fn initial_glyphs(face: &Face<'_>, text: &str) -> Vec<BufferGlyph> {
    let mut glyphs = Vec::new();
    for (cluster, character) in text.char_indices() {
        let codepoint = character as u32;
        let cluster = if is_thai_or_lao_cluster_mark(codepoint) {
            glyphs
                .last()
                .filter(|glyph: &&BufferGlyph| matches!(glyph.codepoint, 0x0e00..=0x0e7f | 0x25cc))
                .map(|glyph| glyph.cluster)
                .unwrap_or(cluster as u32)
        } else {
            cluster as u32
        };
        append_normalized(face, codepoint, cluster, &mut glyphs, 0);
    }
    canonical_order(&mut glyphs);
    canonical_compose(face, &mut glyphs);
    apply_variation_selectors(face, &mut glyphs);
    glyphs
}

fn apply_variation_selectors(face: &Face<'_>, glyphs: &mut Vec<BufferGlyph>) {
    let mut index = 1usize;
    while index < glyphs.len() {
        if !is_variation_selector(glyphs[index].codepoint) {
            index += 1;
            continue;
        }
        let selector = glyphs[index].codepoint;
        let base = index - 1;
        let Some(glyph_id) = face.glyph_variation_index(glyphs[base].codepoint, selector) else {
            index += 1;
            continue;
        };
        glyphs[base].id = glyph_id.0;
        glyphs[base].cluster = glyphs[base].cluster.min(glyphs[index].cluster);
        glyphs.remove(index);
    }
}

fn is_variation_selector(codepoint: u32) -> bool {
    matches!(codepoint, 0xfe00..=0xfe0f | 0xe0100..=0xe01ef)
}

fn assign_fraction_masks(glyphs: &mut [BufferGlyph], right_to_left: bool) {
    for slash in 0..glyphs.len() {
        if glyphs[slash].codepoint != 0x2044 {
            continue;
        }
        let mut start = slash;
        while start > 0 && crate::unicode_data::is_decimal_number(glyphs[start - 1].codepoint) {
            start -= 1;
        }
        let mut end = slash + 1;
        while end < glyphs.len() && crate::unicode_data::is_decimal_number(glyphs[end].codepoint) {
            end += 1;
        }
        let before = if right_to_left {
            FRACTION_DENOMINATOR
        } else {
            FRACTION_NUMERATOR
        };
        let after = if right_to_left {
            FRACTION_NUMERATOR
        } else {
            FRACTION_DENOMINATOR
        };
        for glyph in &mut glyphs[start..slash] {
            glyph.fraction_mask |= FRACTION_ALL | before;
        }
        glyphs[slash].fraction_mask |= FRACTION_ALL;
        for glyph in &mut glyphs[slash + 1..end] {
            glyph.fraction_mask |= FRACTION_ALL | after;
        }
    }
}

fn is_thai_or_lao_cluster_mark(codepoint: u32) -> bool {
    matches!(
        codepoint,
        0x0e31 | 0x0e33..=0x0e3a | 0x0e47..=0x0e4e | 0x0eb1 | 0x0eb3..=0x0ebc | 0x0ec8..=0x0ecd
    )
}

fn append_normalized(
    face: &Face<'_>,
    codepoint: u32,
    cluster: u32,
    glyphs: &mut Vec<BufferGlyph>,
    depth: usize,
) {
    if depth <= 8 && face.glyph_index(codepoint).is_none() {
        if let Some(decomposition) = hangul_decomposition(codepoint) {
            let mut candidate = Vec::new();
            for part in decomposition.into_iter().flatten() {
                append_normalized(face, part, cluster, &mut candidate, depth + 1);
            }
            if candidate.iter().all(|glyph| glyph.id != 0) {
                glyphs.extend(candidate);
                return;
            }
        }
        if !matches!(codepoint, 0x0931 | 0x09dc | 0x09dd | 0x0b94) {
            if let Some(decomposition) = crate::unicode_data::canonical_decomposition(codepoint) {
                let mut candidate = Vec::new();
                for &part in decomposition {
                    append_normalized(face, part, cluster, &mut candidate, depth + 1);
                }
                if candidate.iter().all(|glyph| glyph.id != 0) {
                    glyphs.extend(candidate);
                    return;
                }
            }
        }
        let script_decomposition: &[u32] = match codepoint {
            0x0e33 => &[0x0e4d, 0x0e32],
            0x0eb3 => &[0x0ecd, 0x0eb2],
            _ => &[],
        };
        if !script_decomposition.is_empty() {
            for &part in script_decomposition {
                append_normalized(face, part, cluster, glyphs, depth + 1);
            }
            return;
        }
    }
    glyphs.push(BufferGlyph {
        id: face
            .glyph_index(codepoint)
            .map(|glyph| glyph.0)
            .unwrap_or(0),
        cluster,
        codepoint,
        x_advance: 0,
        y_advance: 0,
        x_offset: 0,
        y_offset: 0,
        arabic_action: 7,
        indic_reph: false,
        cursive_parent: None,
        mark_parent: None,
        mark_x_delta: 0,
        mark_y_delta: 0,
        ligature_id: 0,
        ligature_component: 0,
        ligature_components: 1,
        rtlm: false,
        fraction_mask: 0,
    });
}

fn canonical_order(glyphs: &mut [BufferGlyph]) {
    let mut start = 0usize;
    while start < glyphs.len() {
        let has_starter = combining_class(glyphs[start].codepoint) == 0;
        let marks_start = start + usize::from(has_starter);
        let mut end = marks_start;
        while end < glyphs.len() && combining_class(glyphs[end].codepoint) != 0 {
            end += 1;
        }
        let cluster = glyphs[start..end]
            .iter()
            .map(|glyph| glyph.cluster)
            .min()
            .unwrap_or(glyphs[start].cluster);
        for glyph in &mut glyphs[marks_start..end] {
            glyph.cluster = cluster;
        }
        glyphs[marks_start..end].sort_by_key(|glyph| combining_class(glyph.codepoint));
        start = end;
    }
}

fn combining_class(codepoint: u32) -> u8 {
    crate::unicode_data::combining_class(codepoint)
}

fn canonical_compose(face: &Face<'_>, glyphs: &mut Vec<BufferGlyph>) {
    let mut starter: Option<usize> = None;
    let mut previous_class = 0u8;
    let mut index = 0usize;
    while index < glyphs.len() {
        let class = combining_class(glyphs[index].codepoint);
        if let Some(starter_index) = starter {
            if previous_class == 0 || previous_class < class {
                if let Some(composed) =
                    canonical_composition(glyphs[starter_index].codepoint, glyphs[index].codepoint)
                {
                    if let Some(glyph_id) = face.glyph_index(composed) {
                        glyphs[starter_index].codepoint = composed;
                        glyphs[starter_index].id = glyph_id.0;
                        glyphs[starter_index].cluster =
                            glyphs[starter_index].cluster.min(glyphs[index].cluster);
                        glyphs.remove(index);
                        continue;
                    }
                }
            }
        }
        if class == 0 {
            starter = Some(index);
        }
        previous_class = class;
        index += 1;
    }
}

fn canonical_composition(first: u32, second: u32) -> Option<u32> {
    const S_BASE: u32 = 0xac00;
    const L_BASE: u32 = 0x1100;
    const V_BASE: u32 = 0x1161;
    const T_BASE: u32 = 0x11a7;
    const L_COUNT: u32 = 19;
    const V_COUNT: u32 = 21;
    const T_COUNT: u32 = 28;
    const N_COUNT: u32 = V_COUNT * T_COUNT;
    const S_COUNT: u32 = L_COUNT * N_COUNT;
    if (L_BASE..L_BASE + L_COUNT).contains(&first) && (V_BASE..V_BASE + V_COUNT).contains(&second) {
        return Some(S_BASE + (first - L_BASE) * N_COUNT + (second - V_BASE) * T_COUNT);
    }
    if (S_BASE..S_BASE + S_COUNT).contains(&first)
        && (first - S_BASE) % T_COUNT == 0
        && (T_BASE + 1..T_BASE + T_COUNT).contains(&second)
    {
        return Some(first + second - T_BASE);
    }
    crate::unicode_data::canonical_composition(first, second)
}

fn hangul_decomposition(codepoint: u32) -> Option<[Option<u32>; 3]> {
    const S_BASE: u32 = 0xac00;
    const L_BASE: u32 = 0x1100;
    const V_BASE: u32 = 0x1161;
    const T_BASE: u32 = 0x11a7;
    const V_COUNT: u32 = 21;
    const T_COUNT: u32 = 28;
    const N_COUNT: u32 = V_COUNT * T_COUNT;
    const S_COUNT: u32 = 19 * N_COUNT;
    let index = codepoint.checked_sub(S_BASE)?;
    if index >= S_COUNT {
        return None;
    }
    let trailing = index % T_COUNT;
    Some([
        Some(L_BASE + index / N_COUNT),
        Some(V_BASE + (index % N_COUNT) / T_COUNT),
        (trailing != 0).then_some(T_BASE + trailing),
    ])
}

fn is_default_ignorable(codepoint: u32) -> bool {
    matches!(
        codepoint,
        0x00ad
            | 0x034f
            | 0x061c
            | 0x17b4..=0x17b5
            | 0x180b..=0x180e
            | 0x200b..=0x200f
            | 0x202a..=0x202e
            | 0x2060..=0x206f
            | 0xfe00..=0xfe0f
            | 0xfeff
            | 0xfff0..=0xfff8
            | 0x1d173..=0x1d17a
            | 0xe0000..=0xe0fff
    )
}

fn unicode_space_fallback(codepoint: u32) -> u8 {
    match codepoint {
        0x0020 | 0x00a0 => 18,
        0x2000 | 0x2002 => 2,
        0x2001 | 0x2003 | 0x3000 => 1,
        0x2004 => 3,
        0x2005 => 4,
        0x2006 => 6,
        0x2007 => 19,
        0x2008 => 20,
        0x2009 => 5,
        0x200a => 16,
        0x202f => 21,
        0x205f => 17,
        _ => 0,
    }
}

fn apply_space_fallbacks(face: &Face<'_>, glyphs: &mut [BufferGlyph]) {
    for glyph in glyphs {
        if face.glyph_index(glyph.codepoint).is_some() {
            continue;
        }
        let fallback = unicode_space_fallback(glyph.codepoint);
        glyph.x_advance = match fallback {
            1..=6 | 16 => {
                let divisor = i32::from(fallback);
                (i32::from(face.units_per_em()) + divisor / 2) / divisor
            }
            17 => i32::from(face.units_per_em()) * 4 / 18,
            19 => ('0'..='9')
                .find_map(|character| {
                    face.glyph_index(character as u32)
                        .and_then(|id| face.glyph_hor_advance(id))
                })
                .map(i32::from)
                .unwrap_or(glyph.x_advance),
            20 => ['.', ',']
                .into_iter()
                .find_map(|character| {
                    face.glyph_index(character as u32)
                        .and_then(|id| face.glyph_hor_advance(id))
                })
                .map(i32::from)
                .unwrap_or(glyph.x_advance),
            21 => glyph.x_advance / 2,
            _ => glyph.x_advance,
        };
    }
}

fn script_tag(text: &str) -> Tag {
    for character in text.chars() {
        match character as u32 {
            0x0370..=0x03ff | 0x1f00..=0x1fff => return *b"grek",
            0x0400..=0x052f | 0x2de0..=0x2dff | 0xa640..=0xa69f => return *b"cyrl",
            0x0590..=0x05ff | 0xfb1d..=0xfb4f => return *b"hebr",
            0x0600..=0x08ff | 0xfb50..=0xfdff | 0xfe70..=0xfeff => return *b"arab",
            0x0e00..=0x0e7f => return *b"thai",
            0x0900..=0x097f | 0xa8e0..=0xa8ff => return *b"dev2",
            0x1100..=0x11ff | 0x3130..=0x318f | 0xa960..=0xa97f | 0xac00..=0xd7ff => {
                return *b"hang";
            }
            0x3040..=0x30ff | 0x31f0..=0x31ff | 0x1b000..=0x1b16f => return *b"kana",
            0x3100..=0x312f | 0x31a0..=0x31bf => return *b"bopo",
            0x2e80..=0x2fff
            | 0x31c0..=0x31ef
            | 0x3400..=0x4dbf
            | 0x4e00..=0x9fff
            | 0xf900..=0xfaff
            | 0x20000..=0x323af => return *b"hani",
            0x0041..=0x024f | 0x1e00..=0x1eff => return *b"latn",
            _ => {}
        }
    }
    *b"DFLT"
}

fn assign_arabic_actions(glyphs: &mut [BufferGlyph]) {
    const NONE: u8 = 7;
    const STATE_TABLE: [[(u8, u8, usize); 6]; 7] = [
        [
            (NONE, NONE, 0),
            (NONE, 0, 2),
            (NONE, 0, 1),
            (NONE, 0, 2),
            (NONE, 0, 1),
            (NONE, 0, 6),
        ],
        [
            (NONE, NONE, 0),
            (NONE, 0, 2),
            (NONE, 0, 1),
            (NONE, 0, 2),
            (NONE, 2, 5),
            (NONE, 0, 6),
        ],
        [
            (NONE, NONE, 0),
            (NONE, 0, 2),
            (6, 1, 1),
            (6, 1, 3),
            (6, 1, 4),
            (6, 1, 6),
        ],
        [
            (NONE, NONE, 0),
            (NONE, 0, 2),
            (4, 1, 1),
            (4, 1, 3),
            (4, 1, 4),
            (4, 1, 6),
        ],
        [
            (NONE, NONE, 0),
            (NONE, 0, 2),
            (5, 0, 1),
            (5, 0, 2),
            (5, 2, 5),
            (5, 0, 6),
        ],
        [
            (NONE, NONE, 0),
            (NONE, 0, 2),
            (0, 0, 1),
            (0, 0, 2),
            (0, 2, 5),
            (0, 0, 6),
        ],
        [
            (NONE, NONE, 0),
            (NONE, 0, 2),
            (NONE, 0, 1),
            (NONE, 0, 2),
            (NONE, 3, 5),
            (NONE, 0, 6),
        ],
    ];
    for glyph in glyphs.iter_mut() {
        glyph.arabic_action = NONE;
    }
    let mut previous: Option<usize> = None;
    let mut state = 0usize;
    for index in 0..glyphs.len() {
        let joining_type = usize::from(crate::unicode_data::joining_type(glyphs[index].codepoint));
        if joining_type == 7 {
            continue;
        }
        let joining_type = joining_type.min(5);
        let entry = STATE_TABLE[state][joining_type];
        if entry.0 != NONE {
            if let Some(previous) = previous {
                glyphs[previous].arabic_action = entry.0;
            }
        }
        glyphs[index].arabic_action = entry.1;
        previous = Some(index);
        state = entry.2;
    }
}

fn reorder_arabic_shadda(glyphs: &mut Vec<BufferGlyph>) {
    let mut index = 1usize;
    while index < glyphs.len() {
        if glyphs[index].codepoint != 0x0651 {
            index += 1;
            continue;
        }
        let cluster = glyphs[index].cluster;
        let mut target = index;
        while target > 0
            && glyphs[target - 1].cluster == cluster
            && matches!(glyphs[target - 1].codepoint, 0x064b..=0x0650)
        {
            target -= 1;
        }
        if target != index {
            let shadda = glyphs.remove(index);
            glyphs.insert(target, shadda);
        }
        index += 1;
    }
}

fn insert_devanagari_dotted_circles(face: &Face<'_>, glyphs: &mut Vec<BufferGlyph>) {
    let Some(dotted_circle) = face.glyph_index(0x25cc).map(|glyph| glyph.0) else {
        return;
    };
    let mut start = 0usize;
    while start < glyphs.len() {
        if !is_devanagari_item(glyphs[start].codepoint) {
            start += 1;
            continue;
        }
        let mut end = start + 1;
        while end < glyphs.len() && is_devanagari_item(glyphs[end].codepoint) {
            end += 1;
        }
        if !glyphs[start..end]
            .iter()
            .any(|glyph| is_devanagari_dependent(glyph.codepoint))
            || glyphs[start..end]
                .iter()
                .any(|glyph| is_devanagari_base(glyph.codepoint))
        {
            start = end;
            continue;
        }
        let template = glyphs[start].clone();
        glyphs.insert(
            start,
            BufferGlyph {
                id: dotted_circle,
                cluster: template.cluster,
                codepoint: 0x25cc,
                x_advance: 0,
                y_advance: 0,
                x_offset: 0,
                y_offset: 0,
                arabic_action: 7,
                indic_reph: false,
                cursive_parent: None,
                mark_parent: None,
                mark_x_delta: 0,
                mark_y_delta: 0,
                ligature_id: 0,
                ligature_component: 0,
                ligature_components: 1,
                rtlm: false,
                fraction_mask: 0,
            },
        );
        start = end + 1;
    }
}

fn initial_reorder_devanagari(glyphs: &mut [BufferGlyph]) {
    let mut start = 0usize;
    while start < glyphs.len() {
        if !is_devanagari_item(glyphs[start].codepoint) {
            start += 1;
            continue;
        }
        let mut end = start + 1;
        while end < glyphs.len() && is_devanagari_item(glyphs[end].codepoint) {
            end += 1;
        }
        if start + 2 < end
            && glyphs[start].codepoint == 0x0930
            && glyphs[start + 1].codepoint == 0x094d
            && is_devanagari_base(glyphs[start + 2].codepoint)
        {
            glyphs[start].indic_reph = true;
        }
        let mut local_base_cluster = None;
        for glyph in &mut glyphs[start..end] {
            if is_devanagari_base(glyph.codepoint) {
                local_base_cluster = Some(glyph.cluster);
            } else if is_devanagari_dependent(glyph.codepoint)
                || matches!(glyph.codepoint, 0x200c | 0x200d | 0xa8e0..=0xa8ff)
            {
                if let Some(cluster) = local_base_cluster {
                    glyph.cluster = cluster;
                }
            }
        }
        let base = (start..end)
            .rev()
            .find(|index| is_devanagari_base(glyphs[*index].codepoint))
            .unwrap_or(start);
        let base_cluster = glyphs[base].cluster;
        glyphs[start..end].sort_by_key(|glyph| {
            let original = glyph.cluster;
            let position = if matches!(glyph.codepoint, 0x093f | 0x094e) {
                2u8
            } else if is_devanagari_base(glyph.codepoint) {
                if original < base_cluster { 3 } else { 4 }
            } else if original < base_cluster {
                3
            } else if matches!(glyph.codepoint, 0x0900..=0x0903 | 0x0951..=0x0957) {
                14
            } else {
                5
            };
            (position, original)
        });
        start = end;
    }
}

fn final_reorder_devanagari(glyphs: &mut [BufferGlyph]) {
    let mut start = 0usize;
    while start < glyphs.len() {
        if !is_devanagari_item(glyphs[start].codepoint) {
            start += 1;
            continue;
        }
        let mut end = start + 1;
        while end < glyphs.len() && is_devanagari_item(glyphs[end].codepoint) {
            end += 1;
        }
        if glyphs[start..end].iter().any(|glyph| glyph.indic_reph) {
            let cluster = glyphs[start..end]
                .iter()
                .map(|glyph| glyph.cluster)
                .min()
                .unwrap_or(glyphs[start].cluster);
            for glyph in &mut glyphs[start..end] {
                glyph.cluster = cluster;
            }
        }
        glyphs[start..end].sort_by_key(|glyph| {
            if matches!(glyph.codepoint, 0x093f | 0x094e) {
                0u8
            } else if glyph.indic_reph {
                2u8
            } else {
                1u8
            }
        });
        let reordered = &mut glyphs[start..end];
        for index in 1..reordered.len() {
            let cluster = reordered[index].cluster;
            if reordered[index - 1].cluster <= cluster {
                continue;
            }
            let mut merge_start = index;
            while merge_start > 0 && reordered[merge_start - 1].cluster > cluster {
                merge_start -= 1;
            }
            for glyph in &mut reordered[merge_start..=index] {
                glyph.cluster = cluster;
            }
        }
        start = end;
    }
}

fn is_devanagari_item(codepoint: u32) -> bool {
    matches!(codepoint, 0x0900..=0x097f | 0x25cc | 0xa8e0..=0xa8ff | 0x200c | 0x200d)
}

fn is_devanagari_base(codepoint: u32) -> bool {
    matches!(
        codepoint,
        0x0904..=0x0939 | 0x0958..=0x0961 | 0x0972..=0x097f | 0x25cc
    )
}

fn is_devanagari_dependent(codepoint: u32) -> bool {
    matches!(
        codepoint,
        0x0900..=0x0903 | 0x093a..=0x093c | 0x093e..=0x094f | 0x0955..=0x0957 | 0x0962..=0x0963
    )
}

fn apply_legacy_kerning(face: &Face<'_>, glyphs: &mut [BufferGlyph]) {
    for index in 0..glyphs.len().saturating_sub(1) {
        glyphs[index].x_advance += i32::from(
            face.legacy_kerning(GlyphId(glyphs[index].id), GlyphId(glyphs[index + 1].id)),
        );
    }
}

fn apply_gsub_features(
    table: &[u8],
    gdef: Option<&[u8]>,
    script: Tag,
    features: &[Tag],
    glyphs: &mut Vec<BufferGlyph>,
) -> Option<()> {
    let lookups = feature_lookups(table, script, features)?;
    for lookup in lookups {
        apply_gsub_lookup(table, gdef, lookup, glyphs, 0)?;
    }
    Some(())
}

fn apply_gsub_feature_for_action(
    table: &[u8],
    gdef: Option<&[u8]>,
    script: Tag,
    feature: Tag,
    action: u8,
    glyphs: &mut Vec<BufferGlyph>,
) -> Option<()> {
    let lookups = feature_lookups(table, script, &[feature])?;
    for lookup in lookups {
        let mut index = 0usize;
        while index < glyphs.len() {
            if glyphs[index].arabic_action == action {
                apply_gsub_lookup_at(table, gdef, lookup, glyphs, index, 0)?;
            }
            index += 1;
        }
    }
    Some(())
}

fn apply_gsub_feature_for_rtlm(
    table: &[u8],
    gdef: Option<&[u8]>,
    script: Tag,
    glyphs: &mut Vec<BufferGlyph>,
) -> Option<()> {
    let lookups = feature_lookups(table, script, &[*b"rtlm"])?;
    for lookup in lookups {
        let mut index = 0usize;
        while index < glyphs.len() {
            if glyphs[index].rtlm {
                apply_gsub_lookup_at(table, gdef, lookup, glyphs, index, 0)?;
            }
            index += 1;
        }
    }
    Some(())
}

fn apply_fraction_features(
    table: &[u8],
    gdef: Option<&[u8]>,
    script: Tag,
    glyphs: &mut Vec<BufferGlyph>,
) -> Option<()> {
    let mut masked_lookups: Vec<(usize, u8)> = Vec::new();
    for (feature, mask) in [
        (*b"frac", FRACTION_ALL),
        (*b"numr", FRACTION_NUMERATOR),
        (*b"dnom", FRACTION_DENOMINATOR),
    ] {
        for lookup in feature_lookups(table, script, &[feature])? {
            if let Some((_, existing_mask)) = masked_lookups
                .iter_mut()
                .find(|(existing, _)| *existing == lookup)
            {
                *existing_mask |= mask;
            } else {
                masked_lookups.push((lookup, mask));
            }
        }
    }
    if masked_lookups.is_empty() {
        return Some(());
    }

    let lookup_list = usize::from(read_u16(table, 8)?);
    let lookup_count = usize::from(read_u16(table, lookup_list)?);
    checked_slice(table, lookup_list + 2, lookup_count.checked_mul(2)?)?;
    let mut ordered = Vec::with_capacity(masked_lookups.len());
    for index in 0..lookup_count {
        let lookup =
            lookup_list.checked_add(usize::from(read_u16(table, lookup_list + 2 + index * 2)?))?;
        if let Some(position) = masked_lookups
            .iter()
            .position(|(candidate, _)| *candidate == lookup)
        {
            ordered.push(masked_lookups.remove(position));
        }
    }
    for (lookup, mask) in ordered {
        let mut index = 0usize;
        while index < glyphs.len() {
            if glyphs[index].fraction_mask & mask != 0 {
                apply_gsub_lookup_at(table, gdef, lookup, glyphs, index, 0)?;
            }
            index += 1;
        }
    }
    Some(())
}

fn apply_gsub_lookup(
    table: &[u8],
    gdef: Option<&[u8]>,
    lookup_offset: usize,
    glyphs: &mut Vec<BufferGlyph>,
    depth: usize,
) -> Option<()> {
    if depth > 8 {
        return None;
    }
    if read_u16(table, lookup_offset)? == 8 {
        for index in (0..glyphs.len()).rev() {
            apply_gsub_lookup_at(table, gdef, lookup_offset, glyphs, index, depth)?;
        }
        return Some(());
    }
    let mut index = 0usize;
    while index < glyphs.len() {
        apply_gsub_lookup_at(table, gdef, lookup_offset, glyphs, index, depth)?;
        index += 1;
    }
    Some(())
}

fn apply_gsub_lookup_at(
    table: &[u8],
    gdef: Option<&[u8]>,
    lookup_offset: usize,
    glyphs: &mut Vec<BufferGlyph>,
    index: usize,
    depth: usize,
) -> Option<bool> {
    if depth > 8 || index >= glyphs.len() {
        return Some(false);
    }
    let lookup_type = read_u16(table, lookup_offset)?;
    let filter = lookup_filter(table, lookup_offset)?;
    if ignored_glyph(gdef, glyphs[index].id, glyphs[index].codepoint, filter) {
        return Some(false);
    }
    let subtable_count = usize::from(read_u16(table, lookup_offset + 4)?);
    checked_slice(table, lookup_offset + 6, subtable_count.checked_mul(2)?)?;
    for subtable_index in 0..subtable_count {
        let relative = usize::from(read_u16(table, lookup_offset + 6 + subtable_index * 2)?);
        let subtable = lookup_offset.checked_add(relative)?;
        if apply_gsub_subtable(
            table,
            gdef,
            lookup_type,
            filter,
            subtable,
            glyphs,
            index,
            depth,
        )? {
            return Some(true);
        }
    }
    Some(false)
}

fn apply_gsub_subtable(
    table: &[u8],
    gdef: Option<&[u8]>,
    lookup_type: u16,
    filter: LookupFilter,
    subtable: usize,
    glyphs: &mut Vec<BufferGlyph>,
    index: usize,
    depth: usize,
) -> Option<bool> {
    if lookup_type == 7 {
        if read_u16(table, subtable)? != 1 {
            return Some(false);
        }
        let extended_type = read_u16(table, subtable + 2)?;
        let offset = usize::try_from(read_u32(table, subtable + 4)?).ok()?;
        return apply_gsub_subtable(
            table,
            gdef,
            extended_type,
            filter,
            subtable.checked_add(offset)?,
            glyphs,
            index,
            depth + 1,
        );
    }
    let glyph = glyphs.get(index)?.id;
    match lookup_type {
        1 => {
            let format = read_u16(table, subtable)?;
            let coverage = subtable.checked_add(usize::from(read_u16(table, subtable + 2)?))?;
            let Some(coverage_index) = coverage_index(table, coverage, glyph) else {
                return Some(false);
            };
            let replacement = if format == 1 {
                glyph.wrapping_add(read_i16(table, subtable + 4)? as u16)
            } else if format == 2 {
                let count = usize::from(read_u16(table, subtable + 4)?);
                if coverage_index >= count {
                    return None;
                }
                read_u16(table, subtable + 6 + coverage_index * 2)?
            } else {
                return Some(false);
            };
            glyphs[index].id = replacement;
            Some(true)
        }
        2 => {
            if read_u16(table, subtable)? != 1 {
                return Some(false);
            }
            let coverage = subtable.checked_add(usize::from(read_u16(table, subtable + 2)?))?;
            let Some(coverage_index) = coverage_index(table, coverage, glyph) else {
                return Some(false);
            };
            let count = usize::from(read_u16(table, subtable + 4)?);
            if coverage_index >= count {
                return None;
            }
            let sequence = subtable.checked_add(usize::from(read_u16(
                table,
                subtable + 6 + coverage_index * 2,
            )?))?;
            let glyph_count = usize::from(read_u16(table, sequence)?);
            checked_slice(table, sequence + 2, glyph_count.checked_mul(2)?)?;
            if glyph_count == 0 {
                glyphs.remove(index);
            } else {
                let template = glyphs[index].clone();
                glyphs[index].id = read_u16(table, sequence + 2)?;
                for replacement in 1..glyph_count {
                    let mut inserted = template.clone();
                    inserted.id = read_u16(table, sequence + 2 + replacement * 2)?;
                    glyphs.insert(index + replacement, inserted);
                }
            }
            Some(true)
        }
        3 => {
            if read_u16(table, subtable)? != 1 {
                return Some(false);
            }
            let coverage = subtable.checked_add(usize::from(read_u16(table, subtable + 2)?))?;
            let Some(coverage_index) = coverage_index(table, coverage, glyph) else {
                return Some(false);
            };
            let count = usize::from(read_u16(table, subtable + 4)?);
            if coverage_index >= count {
                return None;
            }
            let alternate_set = subtable.checked_add(usize::from(read_u16(
                table,
                subtable + 6 + coverage_index * 2,
            )?))?;
            if read_u16(table, alternate_set)? == 0 {
                return Some(false);
            }
            glyphs[index].id = read_u16(table, alternate_set + 2)?;
            Some(true)
        }
        4 => apply_ligature_substitution(table, gdef, filter, subtable, glyphs, index),
        5 => apply_context_substitution(table, gdef, filter, subtable, glyphs, index, depth),
        6 => apply_chain_context_substitution(table, gdef, filter, subtable, glyphs, index, depth),
        8 => apply_reverse_chain_substitution(table, gdef, filter, subtable, glyphs, index),
        _ => Some(false),
    }
}

fn apply_ligature_substitution(
    table: &[u8],
    gdef: Option<&[u8]>,
    filter: LookupFilter,
    subtable: usize,
    glyphs: &mut Vec<BufferGlyph>,
    index: usize,
) -> Option<bool> {
    if read_u16(table, subtable)? != 1 {
        return Some(false);
    }
    let coverage = subtable.checked_add(usize::from(read_u16(table, subtable + 2)?))?;
    let Some(coverage_index) = coverage_index(table, coverage, glyphs.get(index)?.id) else {
        return Some(false);
    };
    let set_count = usize::from(read_u16(table, subtable + 4)?);
    if coverage_index >= set_count {
        return None;
    }
    let set = subtable.checked_add(usize::from(read_u16(
        table,
        subtable + 6 + coverage_index * 2,
    )?))?;
    let ligature_count = usize::from(read_u16(table, set)?);
    for ligature_index in 0..ligature_count {
        let ligature =
            set.checked_add(usize::from(read_u16(table, set + 2 + ligature_index * 2)?))?;
        let replacement = read_u16(table, ligature)?;
        let component_count = usize::from(read_u16(table, ligature + 2)?);
        if component_count < 2 {
            continue;
        }
        let Some(positions) = eligible_positions(glyphs, index, component_count, gdef, filter)
        else {
            continue;
        };
        let mut matches = true;
        for component in 1..component_count {
            if glyphs[positions[component]].id != read_u16(table, ligature + 2 + component * 2)? {
                matches = false;
                break;
            }
        }
        if matches {
            let last = *positions.last()?;
            let cluster = glyphs[index..=last]
                .iter()
                .map(|glyph| glyph.cluster)
                .min()?;
            for glyph in &mut glyphs[index..=last] {
                glyph.cluster = cluster;
            }
            let ligature_id = if glyphs[index].ligature_id != 0 {
                glyphs[index].ligature_id
            } else {
                cluster.saturating_add(1)
            };
            let mut completed_components = 0u16;
            for component in 0..component_count {
                completed_components = completed_components
                    .saturating_add(glyphs[positions[component]].ligature_components.max(1));
                if component + 1 < component_count {
                    for skipped in positions[component] + 1..positions[component + 1] {
                        if is_mark_glyph(gdef, &glyphs[skipped]) {
                            glyphs[skipped].ligature_id = ligature_id;
                            glyphs[skipped].ligature_component = completed_components;
                        }
                    }
                }
            }
            glyphs[index].id = replacement;
            glyphs[index].ligature_id = ligature_id;
            glyphs[index].ligature_component = 0;
            glyphs[index].ligature_components = completed_components;
            for &position in positions[1..].iter().rev() {
                glyphs.remove(position);
            }
            return Some(true);
        }
    }
    Some(false)
}

fn apply_context_substitution(
    table: &[u8],
    gdef: Option<&[u8]>,
    filter: LookupFilter,
    subtable: usize,
    glyphs: &mut Vec<BufferGlyph>,
    index: usize,
    depth: usize,
) -> Option<bool> {
    let glyph = glyphs.get(index)?.id;
    match read_u16(table, subtable)? {
        1 => {
            let coverage = subtable.checked_add(usize::from(read_u16(table, subtable + 2)?))?;
            let Some(coverage_index) = coverage_index(table, coverage, glyph) else {
                return Some(false);
            };
            let set_count = usize::from(read_u16(table, subtable + 4)?);
            if coverage_index >= set_count {
                return None;
            }
            let set_offset = read_u16(table, subtable + 6 + coverage_index * 2)?;
            if set_offset == 0 {
                return Some(false);
            }
            let set = subtable.checked_add(usize::from(set_offset))?;
            let rule_count = usize::from(read_u16(table, set)?);
            checked_slice(table, set + 2, rule_count.checked_mul(2)?)?;
            for rule_index in 0..rule_count {
                let rule =
                    set.checked_add(usize::from(read_u16(table, set + 2 + rule_index * 2)?))?;
                let glyph_count = usize::from(read_u16(table, rule)?);
                let record_count = usize::from(read_u16(table, rule + 2)?);
                let Some(positions) = eligible_positions(glyphs, index, glyph_count, gdef, filter)
                else {
                    continue;
                };
                let mut matches = true;
                for input in 1..glyph_count {
                    if glyphs[positions[input]].id != read_u16(table, rule + 2 + input * 2)? {
                        matches = false;
                        break;
                    }
                }
                if matches {
                    let records = rule + 4 + (glyph_count - 1) * 2;
                    return apply_lookup_records(
                        table,
                        gdef,
                        records,
                        record_count,
                        positions,
                        glyphs,
                        depth,
                    );
                }
            }
            Some(false)
        }
        2 => {
            let coverage = subtable.checked_add(usize::from(read_u16(table, subtable + 2)?))?;
            if coverage_index(table, coverage, glyph).is_none() {
                return Some(false);
            }
            let class_def = subtable.checked_add(usize::from(read_u16(table, subtable + 4)?))?;
            let class = usize::from(class_value(table, class_def, glyph).unwrap_or(0));
            let set_count = usize::from(read_u16(table, subtable + 6)?);
            if class >= set_count {
                return None;
            }
            let set_offset = read_u16(table, subtable + 8 + class * 2)?;
            if set_offset == 0 {
                return Some(false);
            }
            let set = subtable.checked_add(usize::from(set_offset))?;
            let rule_count = usize::from(read_u16(table, set)?);
            checked_slice(table, set + 2, rule_count.checked_mul(2)?)?;
            for rule_index in 0..rule_count {
                let rule =
                    set.checked_add(usize::from(read_u16(table, set + 2 + rule_index * 2)?))?;
                let glyph_count = usize::from(read_u16(table, rule)?);
                let record_count = usize::from(read_u16(table, rule + 2)?);
                let Some(positions) = eligible_positions(glyphs, index, glyph_count, gdef, filter)
                else {
                    continue;
                };
                let mut matches = true;
                for input in 1..glyph_count {
                    let expected = read_u16(table, rule + 2 + input * 2)?;
                    if class_value(table, class_def, glyphs[positions[input]].id).unwrap_or(0)
                        != expected
                    {
                        matches = false;
                        break;
                    }
                }
                if matches {
                    let records = rule + 4 + (glyph_count - 1) * 2;
                    return apply_lookup_records(
                        table,
                        gdef,
                        records,
                        record_count,
                        positions,
                        glyphs,
                        depth,
                    );
                }
            }
            Some(false)
        }
        3 => {
            let glyph_count = usize::from(read_u16(table, subtable + 2)?);
            let record_count = usize::from(read_u16(table, subtable + 4)?);
            let Some(positions) = eligible_positions(glyphs, index, glyph_count, gdef, filter)
            else {
                return Some(false);
            };
            checked_slice(table, subtable + 6, glyph_count.checked_mul(2)?)?;
            for input in 0..glyph_count {
                let coverage = subtable
                    .checked_add(usize::from(read_u16(table, subtable + 6 + input * 2)?))?;
                if coverage_index(table, coverage, glyphs[positions[input]].id).is_none() {
                    return Some(false);
                }
            }
            apply_lookup_records(
                table,
                gdef,
                subtable + 6 + glyph_count * 2,
                record_count,
                positions,
                glyphs,
                depth,
            )
        }
        _ => Some(false),
    }
}

fn apply_chain_context_substitution(
    table: &[u8],
    gdef: Option<&[u8]>,
    filter: LookupFilter,
    subtable: usize,
    glyphs: &mut Vec<BufferGlyph>,
    index: usize,
    depth: usize,
) -> Option<bool> {
    let glyph = glyphs.get(index)?.id;
    match read_u16(table, subtable)? {
        1 => {
            let coverage = subtable.checked_add(usize::from(read_u16(table, subtable + 2)?))?;
            let Some(coverage_index) = coverage_index(table, coverage, glyph) else {
                return Some(false);
            };
            let set_count = usize::from(read_u16(table, subtable + 4)?);
            if coverage_index >= set_count {
                return None;
            }
            let set_offset = read_u16(table, subtable + 6 + coverage_index * 2)?;
            if set_offset == 0 {
                return Some(false);
            }
            let set = subtable.checked_add(usize::from(set_offset))?;
            let rule_count = usize::from(read_u16(table, set)?);
            for rule_index in 0..rule_count {
                let rule =
                    set.checked_add(usize::from(read_u16(table, set + 2 + rule_index * 2)?))?;
                if let Some((records, record_count, positions)) =
                    match_chain_rule_glyphs(table, rule, glyphs, index, gdef, filter)?
                {
                    return apply_lookup_records(
                        table,
                        gdef,
                        records,
                        record_count,
                        positions,
                        glyphs,
                        depth,
                    );
                }
            }
            Some(false)
        }
        2 => {
            let coverage = subtable.checked_add(usize::from(read_u16(table, subtable + 2)?))?;
            if coverage_index(table, coverage, glyph).is_none() {
                return Some(false);
            }
            let backtrack_def =
                subtable.checked_add(usize::from(read_u16(table, subtable + 4)?))?;
            let input_def = subtable.checked_add(usize::from(read_u16(table, subtable + 6)?))?;
            let lookahead_def =
                subtable.checked_add(usize::from(read_u16(table, subtable + 8)?))?;
            let class = usize::from(class_value(table, input_def, glyph).unwrap_or(0));
            let set_count = usize::from(read_u16(table, subtable + 10)?);
            if class >= set_count {
                return None;
            }
            let set_offset = read_u16(table, subtable + 12 + class * 2)?;
            if set_offset == 0 {
                return Some(false);
            }
            let set = subtable.checked_add(usize::from(set_offset))?;
            let rule_count = usize::from(read_u16(table, set)?);
            for rule_index in 0..rule_count {
                let rule =
                    set.checked_add(usize::from(read_u16(table, set + 2 + rule_index * 2)?))?;
                if let Some((records, record_count, positions)) = match_chain_rule_classes(
                    table,
                    rule,
                    glyphs,
                    index,
                    gdef,
                    filter,
                    backtrack_def,
                    input_def,
                    lookahead_def,
                )? {
                    return apply_lookup_records(
                        table,
                        gdef,
                        records,
                        record_count,
                        positions,
                        glyphs,
                        depth,
                    );
                }
            }
            Some(false)
        }
        3 => {
            let mut cursor = subtable + 2;
            let backtrack_count = usize::from(read_u16(table, cursor)?);
            cursor += 2;
            let Some(backtrack_positions) =
                backtrack_positions(glyphs, index, backtrack_count, gdef, filter)
            else {
                return Some(false);
            };
            for backtrack in 0..backtrack_count {
                let coverage = subtable.checked_add(usize::from(read_u16(table, cursor)?))?;
                cursor += 2;
                if coverage_index(table, coverage, glyphs[backtrack_positions[backtrack]].id)
                    .is_none()
                {
                    return Some(false);
                }
            }
            let input_count = usize::from(read_u16(table, cursor)?);
            cursor += 2;
            let Some(input_positions) =
                eligible_positions(glyphs, index, input_count, gdef, filter)
            else {
                return Some(false);
            };
            for input in 0..input_count {
                let coverage = subtable.checked_add(usize::from(read_u16(table, cursor)?))?;
                cursor += 2;
                if coverage_index(table, coverage, glyphs[input_positions[input]].id).is_none() {
                    return Some(false);
                }
            }
            let lookahead_count = usize::from(read_u16(table, cursor)?);
            cursor += 2;
            let Some(lookahead_positions) = lookahead_positions(
                glyphs,
                *input_positions.last()?,
                lookahead_count,
                gdef,
                filter,
            ) else {
                return Some(false);
            };
            for lookahead in 0..lookahead_count {
                let coverage = subtable.checked_add(usize::from(read_u16(table, cursor)?))?;
                cursor += 2;
                if coverage_index(table, coverage, glyphs[lookahead_positions[lookahead]].id)
                    .is_none()
                {
                    return Some(false);
                }
            }
            let record_count = usize::from(read_u16(table, cursor)?);
            cursor += 2;
            apply_lookup_records(
                table,
                gdef,
                cursor,
                record_count,
                input_positions,
                glyphs,
                depth,
            )
        }
        _ => Some(false),
    }
}

fn match_chain_rule_glyphs(
    table: &[u8],
    rule: usize,
    glyphs: &[BufferGlyph],
    index: usize,
    gdef: Option<&[u8]>,
    filter: LookupFilter,
) -> Option<Option<(usize, usize, Vec<usize>)>> {
    let mut cursor = rule;
    let backtrack_count = usize::from(read_u16(table, cursor)?);
    cursor += 2;
    let Some(backtrack_positions) =
        backtrack_positions(glyphs, index, backtrack_count, gdef, filter)
    else {
        return Some(None);
    };
    for backtrack in 0..backtrack_count {
        if glyphs[backtrack_positions[backtrack]].id != read_u16(table, cursor)? {
            return Some(None);
        }
        cursor += 2;
    }
    let input_count = usize::from(read_u16(table, cursor)?);
    cursor += 2;
    let Some(input_positions) = eligible_positions(glyphs, index, input_count, gdef, filter) else {
        return Some(None);
    };
    for input in 1..input_count {
        if glyphs[input_positions[input]].id != read_u16(table, cursor)? {
            return Some(None);
        }
        cursor += 2;
    }
    let lookahead_count = usize::from(read_u16(table, cursor)?);
    cursor += 2;
    let Some(lookahead_positions) = lookahead_positions(
        glyphs,
        *input_positions.last()?,
        lookahead_count,
        gdef,
        filter,
    ) else {
        return Some(None);
    };
    for lookahead in 0..lookahead_count {
        if glyphs[lookahead_positions[lookahead]].id != read_u16(table, cursor)? {
            return Some(None);
        }
        cursor += 2;
    }
    let record_count = usize::from(read_u16(table, cursor)?);
    Some(Some((cursor + 2, record_count, input_positions)))
}

#[allow(clippy::too_many_arguments)]
fn match_chain_rule_classes(
    table: &[u8],
    rule: usize,
    glyphs: &[BufferGlyph],
    index: usize,
    gdef: Option<&[u8]>,
    filter: LookupFilter,
    backtrack_def: usize,
    input_def: usize,
    lookahead_def: usize,
) -> Option<Option<(usize, usize, Vec<usize>)>> {
    let mut cursor = rule;
    let backtrack_count = usize::from(read_u16(table, cursor)?);
    cursor += 2;
    let Some(backtrack_positions) =
        backtrack_positions(glyphs, index, backtrack_count, gdef, filter)
    else {
        return Some(None);
    };
    for backtrack in 0..backtrack_count {
        let class = class_value(
            table,
            backtrack_def,
            glyphs[backtrack_positions[backtrack]].id,
        )
        .unwrap_or(0);
        if class != read_u16(table, cursor)? {
            return Some(None);
        }
        cursor += 2;
    }
    let input_count = usize::from(read_u16(table, cursor)?);
    cursor += 2;
    let Some(input_positions) = eligible_positions(glyphs, index, input_count, gdef, filter) else {
        return Some(None);
    };
    for input in 1..input_count {
        let class = class_value(table, input_def, glyphs[input_positions[input]].id).unwrap_or(0);
        if class != read_u16(table, cursor)? {
            return Some(None);
        }
        cursor += 2;
    }
    let lookahead_count = usize::from(read_u16(table, cursor)?);
    cursor += 2;
    let Some(lookahead_positions) = lookahead_positions(
        glyphs,
        *input_positions.last()?,
        lookahead_count,
        gdef,
        filter,
    ) else {
        return Some(None);
    };
    for lookahead in 0..lookahead_count {
        let class = class_value(
            table,
            lookahead_def,
            glyphs[lookahead_positions[lookahead]].id,
        )
        .unwrap_or(0);
        if class != read_u16(table, cursor)? {
            return Some(None);
        }
        cursor += 2;
    }
    let record_count = usize::from(read_u16(table, cursor)?);
    Some(Some((cursor + 2, record_count, input_positions)))
}

fn apply_lookup_records(
    table: &[u8],
    gdef: Option<&[u8]>,
    records: usize,
    record_count: usize,
    mut positions: Vec<usize>,
    glyphs: &mut Vec<BufferGlyph>,
    depth: usize,
) -> Option<bool> {
    checked_slice(table, records, record_count.checked_mul(4)?)?;
    let mut applied = false;
    for record in 0..record_count {
        let sequence_index = usize::from(read_u16(table, records + record * 4)?);
        let lookup_index = usize::from(read_u16(table, records + record * 4 + 2)?);
        let Some(&target) = positions.get(sequence_index) else {
            return None;
        };
        if target >= glyphs.len() {
            continue;
        }
        let lookup = lookup_offset(table, lookup_index)?;
        let old_len = glyphs.len();
        if apply_gsub_lookup_at(table, gdef, lookup, glyphs, target, depth + 1)? {
            applied = true;
            let new_len = glyphs.len();
            if new_len > old_len {
                let delta = new_len - old_len;
                for position in &mut positions[sequence_index + 1..] {
                    *position = position.checked_add(delta)?;
                }
            } else if old_len > new_len {
                let delta = old_len - new_len;
                for position in &mut positions[sequence_index + 1..] {
                    *position = position.saturating_sub(delta).max(target);
                }
            }
        }
    }
    Some(applied)
}

fn apply_reverse_chain_substitution(
    table: &[u8],
    gdef: Option<&[u8]>,
    filter: LookupFilter,
    subtable: usize,
    glyphs: &mut [BufferGlyph],
    index: usize,
) -> Option<bool> {
    if read_u16(table, subtable)? != 1 {
        return Some(false);
    }
    let coverage = subtable.checked_add(usize::from(read_u16(table, subtable + 2)?))?;
    let Some(substitute_index) = coverage_index(table, coverage, glyphs.get(index)?.id) else {
        return Some(false);
    };
    let mut cursor = subtable + 4;
    let backtrack_count = usize::from(read_u16(table, cursor)?);
    cursor += 2;
    let Some(backtrack_positions) =
        backtrack_positions(glyphs, index, backtrack_count, gdef, filter)
    else {
        return Some(false);
    };
    for backtrack in 0..backtrack_count {
        let coverage = subtable.checked_add(usize::from(read_u16(table, cursor)?))?;
        cursor += 2;
        if coverage_index(table, coverage, glyphs[backtrack_positions[backtrack]].id).is_none() {
            return Some(false);
        }
    }
    let lookahead_count = usize::from(read_u16(table, cursor)?);
    cursor += 2;
    let Some(lookahead_positions) =
        lookahead_positions(glyphs, index, lookahead_count, gdef, filter)
    else {
        return Some(false);
    };
    for lookahead in 0..lookahead_count {
        let coverage = subtable.checked_add(usize::from(read_u16(table, cursor)?))?;
        cursor += 2;
        if coverage_index(table, coverage, glyphs[lookahead_positions[lookahead]].id).is_none() {
            return Some(false);
        }
    }
    let glyph_count = usize::from(read_u16(table, cursor)?);
    cursor += 2;
    if substitute_index >= glyph_count {
        return None;
    }
    glyphs[index].id = read_u16(table, cursor + substitute_index * 2)?;
    Some(true)
}

fn lookup_offset(table: &[u8], lookup_index: usize) -> Option<usize> {
    let lookup_list = usize::from(read_u16(table, 8)?);
    let lookup_count = usize::from(read_u16(table, lookup_list)?);
    if lookup_index >= lookup_count {
        return None;
    }
    lookup_list.checked_add(usize::from(read_u16(
        table,
        lookup_list + 2 + lookup_index * 2,
    )?))
}

fn apply_gpos(
    table: &[u8],
    gdef: Option<&[u8]>,
    script: Tag,
    features: &[Tag],
    glyphs: &mut [BufferGlyph],
    right_to_left: bool,
) -> Option<()> {
    let lookups = feature_lookups(table, script, features)?;
    for lookup in lookups {
        apply_gpos_lookup(table, gdef, lookup, glyphs, 0, right_to_left)?;
    }
    Some(())
}

fn apply_gpos_lookup(
    table: &[u8],
    gdef: Option<&[u8]>,
    lookup_offset: usize,
    glyphs: &mut [BufferGlyph],
    depth: usize,
    right_to_left: bool,
) -> Option<()> {
    if depth > 8 {
        return None;
    }
    let mut index = 0usize;
    while index < glyphs.len() {
        apply_gpos_lookup_at(
            table,
            gdef,
            lookup_offset,
            glyphs,
            index,
            depth,
            right_to_left,
        )?;
        index += 1;
    }
    Some(())
}

#[allow(clippy::too_many_arguments)]
fn apply_gpos_lookup_at(
    table: &[u8],
    gdef: Option<&[u8]>,
    lookup_offset: usize,
    glyphs: &mut [BufferGlyph],
    index: usize,
    depth: usize,
    right_to_left: bool,
) -> Option<bool> {
    if depth > 8 || index >= glyphs.len() {
        return Some(false);
    }
    let lookup_type = read_u16(table, lookup_offset)?;
    let filter = lookup_filter(table, lookup_offset)?;
    if ignored_glyph(gdef, glyphs[index].id, glyphs[index].codepoint, filter) {
        return Some(false);
    }
    let subtable_count = usize::from(read_u16(table, lookup_offset + 4)?);
    checked_slice(table, lookup_offset + 6, subtable_count.checked_mul(2)?)?;
    for subtable_index in 0..subtable_count {
        let subtable = lookup_offset.checked_add(usize::from(read_u16(
            table,
            lookup_offset + 6 + subtable_index * 2,
        )?))?;
        if apply_gpos_subtable(
            table,
            gdef,
            lookup_type,
            filter,
            subtable,
            glyphs,
            index,
            depth,
            right_to_left,
        )? {
            return Some(true);
        }
    }
    Some(false)
}

#[allow(clippy::too_many_arguments)]
fn apply_gpos_subtable(
    table: &[u8],
    gdef: Option<&[u8]>,
    lookup_type: u16,
    filter: LookupFilter,
    subtable: usize,
    glyphs: &mut [BufferGlyph],
    index: usize,
    depth: usize,
    right_to_left: bool,
) -> Option<bool> {
    if lookup_type == 9 {
        if read_u16(table, subtable)? != 1 {
            return Some(false);
        }
        let extended_type = read_u16(table, subtable + 2)?;
        let offset = usize::try_from(read_u32(table, subtable + 4)?).ok()?;
        return apply_gpos_subtable(
            table,
            gdef,
            extended_type,
            filter,
            subtable.checked_add(offset)?,
            glyphs,
            index,
            depth + 1,
            right_to_left,
        );
    }
    match lookup_type {
        1 => apply_single_position(table, subtable, glyphs, index),
        2 => apply_pair_position(table, gdef, filter, subtable, glyphs, index),
        3 => apply_cursive_position(table, gdef, filter, subtable, glyphs, index, right_to_left),
        4 => apply_mark_to_base(table, gdef, subtable, glyphs, index, right_to_left),
        5 => apply_mark_to_ligature(table, gdef, subtable, glyphs, index, right_to_left),
        6 => apply_mark_to_mark(table, gdef, subtable, glyphs, index, right_to_left),
        7 => apply_context_position(
            table,
            gdef,
            filter,
            subtable,
            glyphs,
            index,
            depth,
            right_to_left,
        ),
        8 => apply_chain_context_position(
            table,
            gdef,
            filter,
            subtable,
            glyphs,
            index,
            depth,
            right_to_left,
        ),
        _ => Some(false),
    }
}

#[allow(clippy::too_many_arguments)]
fn apply_cursive_position(
    table: &[u8],
    gdef: Option<&[u8]>,
    filter: LookupFilter,
    subtable: usize,
    glyphs: &mut [BufferGlyph],
    index: usize,
    right_to_left: bool,
) -> Option<bool> {
    if read_u16(table, subtable)? != 1 {
        return Some(false);
    }
    let coverage = subtable.checked_add(usize::from(read_u16(table, subtable + 2)?))?;
    let Some(current_coverage) = coverage_index(table, coverage, glyphs.get(index)?.id) else {
        return Some(false);
    };
    let Some(previous) = previous_eligible(glyphs, index, gdef, filter) else {
        return Some(false);
    };
    let Some(previous_coverage) = coverage_index(table, coverage, glyphs[previous].id) else {
        return Some(false);
    };
    let record_count = usize::from(read_u16(table, subtable + 4)?);
    if current_coverage >= record_count || previous_coverage >= record_count {
        return None;
    }
    let current_record = subtable + 6 + current_coverage * 4;
    let previous_record = subtable + 6 + previous_coverage * 4;
    let entry_offset = usize::from(read_u16(table, current_record)?);
    let exit_offset = usize::from(read_u16(table, previous_record + 2)?);
    if entry_offset == 0 || exit_offset == 0 {
        return Some(false);
    }
    let entry = anchor(table, subtable.checked_add(entry_offset)?)?;
    let exit = anchor(table, subtable.checked_add(exit_offset)?)?;

    if right_to_left {
        let delta = exit.0 + glyphs[previous].x_offset;
        glyphs[previous].x_advance -= delta;
        glyphs[previous].x_offset -= delta;
        glyphs[index].x_advance = entry.0 + glyphs[index].x_offset;
    } else {
        glyphs[previous].x_advance = exit.0 + glyphs[previous].x_offset;
        let delta = entry.0 + glyphs[index].x_offset;
        glyphs[index].x_advance -= delta;
        glyphs[index].x_offset -= delta;
    }

    if filter.flags & 0x0001 != 0 {
        glyphs[previous].cursive_parent = Some(index);
        glyphs[previous].y_offset = entry.1 - exit.1;
    } else {
        glyphs[index].cursive_parent = Some(previous);
        glyphs[index].y_offset = exit.1 - entry.1;
    }
    Some(true)
}

#[allow(clippy::too_many_arguments)]
fn apply_context_position(
    table: &[u8],
    gdef: Option<&[u8]>,
    filter: LookupFilter,
    subtable: usize,
    glyphs: &mut [BufferGlyph],
    index: usize,
    depth: usize,
    right_to_left: bool,
) -> Option<bool> {
    let glyph = glyphs.get(index)?.id;
    match read_u16(table, subtable)? {
        1 => {
            let coverage = subtable.checked_add(usize::from(read_u16(table, subtable + 2)?))?;
            let Some(coverage_index) = coverage_index(table, coverage, glyph) else {
                return Some(false);
            };
            let set_count = usize::from(read_u16(table, subtable + 4)?);
            if coverage_index >= set_count {
                return None;
            }
            let set_offset = read_u16(table, subtable + 6 + coverage_index * 2)?;
            if set_offset == 0 {
                return Some(false);
            }
            let set = subtable.checked_add(usize::from(set_offset))?;
            let rule_count = usize::from(read_u16(table, set)?);
            checked_slice(table, set + 2, rule_count.checked_mul(2)?)?;
            for rule_index in 0..rule_count {
                let rule =
                    set.checked_add(usize::from(read_u16(table, set + 2 + rule_index * 2)?))?;
                let glyph_count = usize::from(read_u16(table, rule)?);
                let record_count = usize::from(read_u16(table, rule + 2)?);
                let Some(positions) = eligible_positions(glyphs, index, glyph_count, gdef, filter)
                else {
                    continue;
                };
                let mut matches = true;
                for input in 1..glyph_count {
                    if glyphs[positions[input]].id != read_u16(table, rule + 2 + input * 2)? {
                        matches = false;
                        break;
                    }
                }
                if matches {
                    return apply_position_records(
                        table,
                        gdef,
                        rule + 4 + (glyph_count - 1) * 2,
                        record_count,
                        &positions,
                        glyphs,
                        depth,
                        right_to_left,
                    );
                }
            }
            Some(false)
        }
        2 => {
            let coverage = subtable.checked_add(usize::from(read_u16(table, subtable + 2)?))?;
            if coverage_index(table, coverage, glyph).is_none() {
                return Some(false);
            }
            let class_def = subtable.checked_add(usize::from(read_u16(table, subtable + 4)?))?;
            let class = usize::from(class_value(table, class_def, glyph).unwrap_or(0));
            let set_count = usize::from(read_u16(table, subtable + 6)?);
            if class >= set_count {
                return None;
            }
            let set_offset = read_u16(table, subtable + 8 + class * 2)?;
            if set_offset == 0 {
                return Some(false);
            }
            let set = subtable.checked_add(usize::from(set_offset))?;
            let rule_count = usize::from(read_u16(table, set)?);
            checked_slice(table, set + 2, rule_count.checked_mul(2)?)?;
            for rule_index in 0..rule_count {
                let rule =
                    set.checked_add(usize::from(read_u16(table, set + 2 + rule_index * 2)?))?;
                let glyph_count = usize::from(read_u16(table, rule)?);
                let record_count = usize::from(read_u16(table, rule + 2)?);
                let Some(positions) = eligible_positions(glyphs, index, glyph_count, gdef, filter)
                else {
                    continue;
                };
                let mut matches = true;
                for input in 1..glyph_count {
                    if class_value(table, class_def, glyphs[positions[input]].id).unwrap_or(0)
                        != read_u16(table, rule + 2 + input * 2)?
                    {
                        matches = false;
                        break;
                    }
                }
                if matches {
                    return apply_position_records(
                        table,
                        gdef,
                        rule + 4 + (glyph_count - 1) * 2,
                        record_count,
                        &positions,
                        glyphs,
                        depth,
                        right_to_left,
                    );
                }
            }
            Some(false)
        }
        3 => {
            let glyph_count = usize::from(read_u16(table, subtable + 2)?);
            let record_count = usize::from(read_u16(table, subtable + 4)?);
            let Some(positions) = eligible_positions(glyphs, index, glyph_count, gdef, filter)
            else {
                return Some(false);
            };
            checked_slice(table, subtable + 6, glyph_count.checked_mul(2)?)?;
            for input in 0..glyph_count {
                let coverage = subtable
                    .checked_add(usize::from(read_u16(table, subtable + 6 + input * 2)?))?;
                if coverage_index(table, coverage, glyphs[positions[input]].id).is_none() {
                    return Some(false);
                }
            }
            apply_position_records(
                table,
                gdef,
                subtable + 6 + glyph_count * 2,
                record_count,
                &positions,
                glyphs,
                depth,
                right_to_left,
            )
        }
        _ => Some(false),
    }
}

#[allow(clippy::too_many_arguments)]
fn apply_chain_context_position(
    table: &[u8],
    gdef: Option<&[u8]>,
    filter: LookupFilter,
    subtable: usize,
    glyphs: &mut [BufferGlyph],
    index: usize,
    depth: usize,
    right_to_left: bool,
) -> Option<bool> {
    let glyph = glyphs.get(index)?.id;
    match read_u16(table, subtable)? {
        1 => {
            let coverage = subtable.checked_add(usize::from(read_u16(table, subtable + 2)?))?;
            let Some(coverage_index) = coverage_index(table, coverage, glyph) else {
                return Some(false);
            };
            let set_count = usize::from(read_u16(table, subtable + 4)?);
            if coverage_index >= set_count {
                return None;
            }
            let set_offset = read_u16(table, subtable + 6 + coverage_index * 2)?;
            if set_offset == 0 {
                return Some(false);
            }
            let set = subtable.checked_add(usize::from(set_offset))?;
            let rule_count = usize::from(read_u16(table, set)?);
            for rule_index in 0..rule_count {
                let rule =
                    set.checked_add(usize::from(read_u16(table, set + 2 + rule_index * 2)?))?;
                if let Some((records, record_count, positions)) =
                    match_chain_rule_glyphs(table, rule, glyphs, index, gdef, filter)?
                {
                    return apply_position_records(
                        table,
                        gdef,
                        records,
                        record_count,
                        &positions,
                        glyphs,
                        depth,
                        right_to_left,
                    );
                }
            }
            Some(false)
        }
        2 => {
            let coverage = subtable.checked_add(usize::from(read_u16(table, subtable + 2)?))?;
            if coverage_index(table, coverage, glyph).is_none() {
                return Some(false);
            }
            let backtrack_def =
                subtable.checked_add(usize::from(read_u16(table, subtable + 4)?))?;
            let input_def = subtable.checked_add(usize::from(read_u16(table, subtable + 6)?))?;
            let lookahead_def =
                subtable.checked_add(usize::from(read_u16(table, subtable + 8)?))?;
            let class = usize::from(class_value(table, input_def, glyph).unwrap_or(0));
            let set_count = usize::from(read_u16(table, subtable + 10)?);
            if class >= set_count {
                return None;
            }
            let set_offset = read_u16(table, subtable + 12 + class * 2)?;
            if set_offset == 0 {
                return Some(false);
            }
            let set = subtable.checked_add(usize::from(set_offset))?;
            let rule_count = usize::from(read_u16(table, set)?);
            for rule_index in 0..rule_count {
                let rule =
                    set.checked_add(usize::from(read_u16(table, set + 2 + rule_index * 2)?))?;
                if let Some((records, record_count, positions)) = match_chain_rule_classes(
                    table,
                    rule,
                    glyphs,
                    index,
                    gdef,
                    filter,
                    backtrack_def,
                    input_def,
                    lookahead_def,
                )? {
                    return apply_position_records(
                        table,
                        gdef,
                        records,
                        record_count,
                        &positions,
                        glyphs,
                        depth,
                        right_to_left,
                    );
                }
            }
            Some(false)
        }
        3 => {
            let mut cursor = subtable + 2;
            let backtrack_count = usize::from(read_u16(table, cursor)?);
            cursor += 2;
            let Some(backtracks) =
                backtrack_positions(glyphs, index, backtrack_count, gdef, filter)
            else {
                return Some(false);
            };
            for position in backtracks {
                let coverage = subtable.checked_add(usize::from(read_u16(table, cursor)?))?;
                cursor += 2;
                if coverage_index(table, coverage, glyphs[position].id).is_none() {
                    return Some(false);
                }
            }
            let input_count = usize::from(read_u16(table, cursor)?);
            cursor += 2;
            let Some(inputs) = eligible_positions(glyphs, index, input_count, gdef, filter) else {
                return Some(false);
            };
            for &position in &inputs {
                let coverage = subtable.checked_add(usize::from(read_u16(table, cursor)?))?;
                cursor += 2;
                if coverage_index(table, coverage, glyphs[position].id).is_none() {
                    return Some(false);
                }
            }
            let lookahead_count = usize::from(read_u16(table, cursor)?);
            cursor += 2;
            let Some(lookaheads) =
                lookahead_positions(glyphs, *inputs.last()?, lookahead_count, gdef, filter)
            else {
                return Some(false);
            };
            for position in lookaheads {
                let coverage = subtable.checked_add(usize::from(read_u16(table, cursor)?))?;
                cursor += 2;
                if coverage_index(table, coverage, glyphs[position].id).is_none() {
                    return Some(false);
                }
            }
            let record_count = usize::from(read_u16(table, cursor)?);
            apply_position_records(
                table,
                gdef,
                cursor + 2,
                record_count,
                &inputs,
                glyphs,
                depth,
                right_to_left,
            )
        }
        _ => Some(false),
    }
}

#[allow(clippy::too_many_arguments)]
fn apply_position_records(
    table: &[u8],
    gdef: Option<&[u8]>,
    records: usize,
    record_count: usize,
    positions: &[usize],
    glyphs: &mut [BufferGlyph],
    depth: usize,
    right_to_left: bool,
) -> Option<bool> {
    checked_slice(table, records, record_count.checked_mul(4)?)?;
    let mut applied = false;
    for record in 0..record_count {
        let sequence_index = usize::from(read_u16(table, records + record * 4)?);
        let lookup_index = usize::from(read_u16(table, records + record * 4 + 2)?);
        let Some(&target) = positions.get(sequence_index) else {
            return None;
        };
        let lookup = lookup_offset(table, lookup_index)?;
        applied |= apply_gpos_lookup_at(
            table,
            gdef,
            lookup,
            glyphs,
            target,
            depth + 1,
            right_to_left,
        )?;
    }
    Some(applied)
}

fn apply_single_position(
    table: &[u8],
    subtable: usize,
    glyphs: &mut [BufferGlyph],
    index: usize,
) -> Option<bool> {
    let format = read_u16(table, subtable)?;
    let coverage = subtable.checked_add(usize::from(read_u16(table, subtable + 2)?))?;
    let Some(coverage_index) = coverage_index(table, coverage, glyphs.get(index)?.id) else {
        return Some(false);
    };
    let value_format = read_u16(table, subtable + 4)?;
    let value_offset = if format == 1 {
        subtable + 6
    } else if format == 2 {
        let count = usize::from(read_u16(table, subtable + 6)?);
        if coverage_index >= count {
            return None;
        }
        subtable + 8 + coverage_index * value_record_size(value_format)
    } else {
        return Some(false);
    };
    apply_value_record(table, value_offset, value_format, &mut glyphs[index])?;
    Some(true)
}

fn apply_pair_position(
    table: &[u8],
    gdef: Option<&[u8]>,
    filter: LookupFilter,
    subtable: usize,
    glyphs: &mut [BufferGlyph],
    index: usize,
) -> Option<bool> {
    let Some(next) = next_eligible(glyphs, index + 1, gdef, filter) else {
        return Some(false);
    };
    let format = read_u16(table, subtable)?;
    let coverage = subtable.checked_add(usize::from(read_u16(table, subtable + 2)?))?;
    let Some(coverage_index) = coverage_index(table, coverage, glyphs[index].id) else {
        return Some(false);
    };
    let value_format_1 = read_u16(table, subtable + 4)?;
    let value_format_2 = read_u16(table, subtable + 6)?;
    let size_1 = value_record_size(value_format_1);
    let size_2 = value_record_size(value_format_2);
    let (value_1, value_2) = if format == 1 {
        let set_count = usize::from(read_u16(table, subtable + 8)?);
        if coverage_index >= set_count {
            return None;
        }
        let set = subtable.checked_add(usize::from(read_u16(
            table,
            subtable + 10 + coverage_index * 2,
        )?))?;
        let pair_count = usize::from(read_u16(table, set)?);
        let record_size = 2usize.checked_add(size_1)?.checked_add(size_2)?;
        let mut found = None;
        for pair in 0..pair_count {
            let record = set + 2 + pair * record_size;
            if read_u16(table, record)? == glyphs[next].id {
                found = Some((record + 2, record + 2 + size_1));
                break;
            }
        }
        let Some(found) = found else {
            return Some(false);
        };
        found
    } else if format == 2 {
        let class_def_1 = subtable.checked_add(usize::from(read_u16(table, subtable + 8)?))?;
        let class_def_2 = subtable.checked_add(usize::from(read_u16(table, subtable + 10)?))?;
        let class_1_count = usize::from(read_u16(table, subtable + 12)?);
        let class_2_count = usize::from(read_u16(table, subtable + 14)?);
        let class_1 = usize::from(class_value(table, class_def_1, glyphs[index].id).unwrap_or(0));
        let class_2 = usize::from(class_value(table, class_def_2, glyphs[next].id).unwrap_or(0));
        if class_1 >= class_1_count || class_2 >= class_2_count {
            return None;
        }
        let record_size = size_1.checked_add(size_2)?;
        let record_index = class_1.checked_mul(class_2_count)?.checked_add(class_2)?;
        let value_1 = subtable + 16 + record_index * record_size;
        (value_1, value_1 + size_1)
    } else {
        return Some(false);
    };
    apply_value_record(table, value_1, value_format_1, &mut glyphs[index])?;
    apply_value_record(table, value_2, value_format_2, &mut glyphs[next])?;
    Some(true)
}

fn apply_mark_to_base(
    table: &[u8],
    gdef: Option<&[u8]>,
    subtable: usize,
    glyphs: &mut [BufferGlyph],
    mark_index: usize,
    right_to_left: bool,
) -> Option<bool> {
    if read_u16(table, subtable)? != 1 {
        return Some(false);
    }
    let mark_coverage = subtable.checked_add(usize::from(read_u16(table, subtable + 2)?))?;
    let Some(mark_coverage_index) = coverage_index(table, mark_coverage, glyphs[mark_index].id)
    else {
        return Some(false);
    };
    let base_coverage = subtable.checked_add(usize::from(read_u16(table, subtable + 4)?))?;
    let candidates = 0..mark_index;
    let base_index = if combining_class(glyphs[mark_index].codepoint) == 0 {
        candidates
            .clone()
            .find(|candidate| {
                glyphs[*candidate].cluster == glyphs[mark_index].cluster
                    && !is_mark_glyph(gdef, &glyphs[*candidate])
            })
            .or_else(|| {
                candidates
                    .rev()
                    .find(|candidate| !is_mark_glyph(gdef, &glyphs[*candidate]))
            })
    } else {
        candidates
            .rev()
            .find(|candidate| !is_mark_glyph(gdef, &glyphs[*candidate]))
    };
    let Some(base_index) = base_index else {
        return Some(false);
    };
    let Some(base_coverage_index) = coverage_index(table, base_coverage, glyphs[base_index].id)
    else {
        return Some(false);
    };
    let class_count = usize::from(read_u16(table, subtable + 6)?);
    let mark_array = subtable.checked_add(usize::from(read_u16(table, subtable + 8)?))?;
    let base_array = subtable.checked_add(usize::from(read_u16(table, subtable + 10)?))?;
    let (class, mark_anchor) = mark_record(table, mark_array, mark_coverage_index)?;
    if class >= class_count {
        return None;
    }
    let base_count = usize::from(read_u16(table, base_array)?);
    if base_coverage_index >= base_count {
        return None;
    }
    let anchor_offset = read_u16(
        table,
        base_array + 2 + (base_coverage_index * class_count + class) * 2,
    )?;
    if anchor_offset == 0 {
        return Some(false);
    }
    let base_anchor = anchor(table, base_array + usize::from(anchor_offset))?;
    attach_mark(
        glyphs,
        base_index,
        mark_index,
        base_anchor,
        mark_anchor,
        right_to_left,
    );
    Some(true)
}

fn apply_mark_to_ligature(
    table: &[u8],
    gdef: Option<&[u8]>,
    subtable: usize,
    glyphs: &mut [BufferGlyph],
    mark_index: usize,
    right_to_left: bool,
) -> Option<bool> {
    if read_u16(table, subtable)? != 1 {
        return Some(false);
    }
    let mark_coverage = subtable.checked_add(usize::from(read_u16(table, subtable + 2)?))?;
    let Some(mark_coverage_index) = coverage_index(table, mark_coverage, glyphs[mark_index].id)
    else {
        return Some(false);
    };
    let ligature_coverage = subtable.checked_add(usize::from(read_u16(table, subtable + 4)?))?;
    let Some(ligature_index) = (0..mark_index)
        .rev()
        .find(|candidate| !is_mark_glyph(gdef, &glyphs[*candidate]))
    else {
        return Some(false);
    };
    let Some(ligature_coverage_index) =
        coverage_index(table, ligature_coverage, glyphs[ligature_index].id)
    else {
        return Some(false);
    };
    let class_count = usize::from(read_u16(table, subtable + 6)?);
    let mark_array = subtable.checked_add(usize::from(read_u16(table, subtable + 8)?))?;
    let ligature_array = subtable.checked_add(usize::from(read_u16(table, subtable + 10)?))?;
    let (class, mark_anchor) = mark_record(table, mark_array, mark_coverage_index)?;
    if class >= class_count {
        return None;
    }
    let ligature_count = usize::from(read_u16(table, ligature_array)?);
    if ligature_coverage_index >= ligature_count {
        return None;
    }
    let attach_offset = read_u16(table, ligature_array + 2 + ligature_coverage_index * 2)?;
    if attach_offset == 0 {
        return Some(false);
    }
    let attach = ligature_array.checked_add(usize::from(attach_offset))?;
    let component_count = usize::from(read_u16(table, attach)?);
    if component_count == 0 {
        return Some(false);
    }
    let component = if glyphs[mark_index].ligature_id != 0
        && glyphs[mark_index].ligature_id == glyphs[ligature_index].ligature_id
        && glyphs[mark_index].ligature_component != 0
    {
        usize::from(glyphs[mark_index].ligature_component - 1).min(component_count - 1)
    } else {
        component_count - 1
    };
    let anchor_offset = read_u16(table, attach + 2 + (component * class_count + class) * 2)?;
    if anchor_offset == 0 {
        return Some(false);
    }
    let ligature_anchor = anchor(table, attach + usize::from(anchor_offset))?;
    attach_mark(
        glyphs,
        ligature_index,
        mark_index,
        ligature_anchor,
        mark_anchor,
        right_to_left,
    );
    Some(true)
}

fn apply_mark_to_mark(
    table: &[u8],
    gdef: Option<&[u8]>,
    subtable: usize,
    glyphs: &mut [BufferGlyph],
    mark_index: usize,
    right_to_left: bool,
) -> Option<bool> {
    if read_u16(table, subtable)? != 1 {
        return Some(false);
    }
    let mark_1_coverage = subtable.checked_add(usize::from(read_u16(table, subtable + 2)?))?;
    let Some(mark_1_index) = coverage_index(table, mark_1_coverage, glyphs[mark_index].id) else {
        return Some(false);
    };
    let mark_2_coverage = subtable.checked_add(usize::from(read_u16(table, subtable + 4)?))?;
    let Some(parent_index) = mark_index.checked_sub(1) else {
        return Some(false);
    };
    if !is_mark_glyph(gdef, &glyphs[parent_index]) {
        return Some(false);
    }
    let Some(mark_2_index) = coverage_index(table, mark_2_coverage, glyphs[parent_index].id) else {
        return Some(false);
    };
    let class_count = usize::from(read_u16(table, subtable + 6)?);
    let mark_1_array = subtable.checked_add(usize::from(read_u16(table, subtable + 8)?))?;
    let mark_2_array = subtable.checked_add(usize::from(read_u16(table, subtable + 10)?))?;
    let (class, mark_anchor) = mark_record(table, mark_1_array, mark_1_index)?;
    if class >= class_count {
        return None;
    }
    let mark_2_count = usize::from(read_u16(table, mark_2_array)?);
    if mark_2_index >= mark_2_count {
        return None;
    }
    let anchor_offset = read_u16(
        table,
        mark_2_array + 2 + (mark_2_index * class_count + class) * 2,
    )?;
    if anchor_offset == 0 {
        return Some(false);
    }
    let parent_anchor = anchor(table, mark_2_array + usize::from(anchor_offset))?;
    attach_mark(
        glyphs,
        parent_index,
        mark_index,
        parent_anchor,
        mark_anchor,
        right_to_left,
    );
    Some(true)
}

fn is_mark_glyph(gdef: Option<&[u8]>, glyph: &BufferGlyph) -> bool {
    let class = gdef_glyph_class(gdef, glyph.id);
    class == 3 || (class == 0 && combining_class(glyph.codepoint) != 0)
}

fn attach_mark(
    glyphs: &mut [BufferGlyph],
    parent: usize,
    mark: usize,
    parent_anchor: (i32, i32),
    mark_anchor: (i32, i32),
    right_to_left: bool,
) {
    glyphs[mark].mark_parent = Some(parent);
    glyphs[mark].mark_x_delta = parent_anchor.0 - mark_anchor.0;
    glyphs[mark].mark_y_delta = parent_anchor.1 - mark_anchor.1;
    let origin = |index: usize| -> i32 {
        if right_to_left {
            glyphs[index + 1..]
                .iter()
                .map(|glyph| glyph.x_advance)
                .sum()
        } else {
            glyphs[..index].iter().map(|glyph| glyph.x_advance).sum()
        }
    };
    let parent_origin = origin(parent);
    let mark_origin = origin(mark);
    glyphs[mark].x_offset =
        parent_origin + glyphs[parent].x_offset + glyphs[mark].mark_x_delta - mark_origin;
    glyphs[mark].y_offset = glyphs[parent].y_offset + glyphs[mark].mark_y_delta;
}

fn resolve_cursive_offsets(glyphs: &mut [BufferGlyph]) {
    fn resolve(glyphs: &mut [BufferGlyph], index: usize, state: &mut [u8]) {
        if state[index] != 0 {
            return;
        }
        state[index] = 1;
        if let Some(parent) = glyphs[index].cursive_parent {
            if parent < glyphs.len() && state[parent] != 1 {
                resolve(glyphs, parent, state);
                glyphs[index].y_offset += glyphs[parent].y_offset;
            }
        }
        state[index] = 2;
    }

    let mut state = vec![0u8; glyphs.len()];
    for index in 0..glyphs.len() {
        resolve(glyphs, index, &mut state);
    }
}

fn resolve_mark_offsets(glyphs: &mut [BufferGlyph], right_to_left: bool) {
    fn resolve(glyphs: &mut [BufferGlyph], index: usize, state: &mut [u8], right_to_left: bool) {
        if state[index] != 0 {
            return;
        }
        state[index] = 1;
        if let Some(parent) = glyphs[index].mark_parent {
            if parent < glyphs.len() && state[parent] != 1 {
                resolve(glyphs, parent, state, right_to_left);
                let origin = |glyphs: &[BufferGlyph], position: usize| -> i32 {
                    if right_to_left {
                        glyphs[position + 1..]
                            .iter()
                            .map(|glyph| glyph.x_advance)
                            .sum()
                    } else {
                        glyphs[..position].iter().map(|glyph| glyph.x_advance).sum()
                    }
                };
                let parent_origin = origin(glyphs, parent);
                let mark_origin = origin(glyphs, index);
                glyphs[index].x_offset =
                    parent_origin + glyphs[parent].x_offset + glyphs[index].mark_x_delta
                        - mark_origin;
                glyphs[index].y_offset = glyphs[parent].y_offset + glyphs[index].mark_y_delta;
            }
        }
        state[index] = 2;
    }

    let mut state = vec![0u8; glyphs.len()];
    for index in 0..glyphs.len() {
        resolve(glyphs, index, &mut state, right_to_left);
    }
}

fn mark_record(table: &[u8], array: usize, index: usize) -> Option<(usize, (i32, i32))> {
    let count = usize::from(read_u16(table, array)?);
    if index >= count {
        return None;
    }
    let record = array + 2 + index * 4;
    let class = usize::from(read_u16(table, record)?);
    let offset = usize::from(read_u16(table, record + 2)?);
    Some((class, anchor(table, array + offset)?))
}

fn anchor(table: &[u8], offset: usize) -> Option<(i32, i32)> {
    match read_u16(table, offset)? {
        1..=3 => Some((
            i32::from(read_i16(table, offset + 2)?),
            i32::from(read_i16(table, offset + 4)?),
        )),
        _ => None,
    }
}

fn value_record_size(format: u16) -> usize {
    usize::try_from((format & 0x00ff).count_ones()).unwrap_or_default() * 2
}

fn apply_value_record(
    table: &[u8],
    offset: usize,
    format: u16,
    glyph: &mut BufferGlyph,
) -> Option<()> {
    let mut cursor = offset;
    for bit in 0..8 {
        if format & (1 << bit) == 0 {
            continue;
        }
        let value = read_i16(table, cursor)?;
        cursor += 2;
        match bit {
            0 => glyph.x_offset += i32::from(value),
            1 => glyph.y_offset += i32::from(value),
            2 => glyph.x_advance += i32::from(value),
            3 => glyph.y_advance += i32::from(value),
            _ => {}
        }
    }
    Some(())
}

fn next_eligible(
    glyphs: &[BufferGlyph],
    start: usize,
    gdef: Option<&[u8]>,
    filter: LookupFilter,
) -> Option<usize> {
    (start..glyphs.len())
        .find(|index| !ignored_glyph(gdef, glyphs[*index].id, glyphs[*index].codepoint, filter))
}

fn eligible_positions(
    glyphs: &[BufferGlyph],
    start: usize,
    count: usize,
    gdef: Option<&[u8]>,
    filter: LookupFilter,
) -> Option<Vec<usize>> {
    if count == 0 || start >= glyphs.len() {
        return None;
    }
    let mut positions = Vec::with_capacity(count);
    positions.push(start);
    let mut cursor = start + 1;
    while positions.len() < count {
        let position = next_eligible(glyphs, cursor, gdef, filter)?;
        positions.push(position);
        cursor = position + 1;
    }
    Some(positions)
}

fn previous_eligible(
    glyphs: &[BufferGlyph],
    before: usize,
    gdef: Option<&[u8]>,
    filter: LookupFilter,
) -> Option<usize> {
    (0..before)
        .rev()
        .find(|index| !ignored_glyph(gdef, glyphs[*index].id, glyphs[*index].codepoint, filter))
}

fn backtrack_positions(
    glyphs: &[BufferGlyph],
    before: usize,
    count: usize,
    gdef: Option<&[u8]>,
    filter: LookupFilter,
) -> Option<Vec<usize>> {
    let mut positions = Vec::with_capacity(count);
    let mut cursor = before;
    while positions.len() < count {
        let position = previous_eligible(glyphs, cursor, gdef, filter)?;
        positions.push(position);
        cursor = position;
    }
    Some(positions)
}

fn lookahead_positions(
    glyphs: &[BufferGlyph],
    after: usize,
    count: usize,
    gdef: Option<&[u8]>,
    filter: LookupFilter,
) -> Option<Vec<usize>> {
    let mut positions = Vec::with_capacity(count);
    let mut cursor = after.checked_add(1)?;
    while positions.len() < count {
        let position = next_eligible(glyphs, cursor, gdef, filter)?;
        positions.push(position);
        cursor = position.checked_add(1)?;
    }
    Some(positions)
}

fn ignored_glyph(gdef: Option<&[u8]>, glyph: u16, codepoint: u32, filter: LookupFilter) -> bool {
    let class = match gdef_glyph_class(gdef, glyph) {
        0 => {
            if combining_class(codepoint) != 0 {
                3
            } else {
                1
            }
        }
        class => class,
    };
    (filter.flags & 0x0002 != 0 && class == 1)
        || (filter.flags & 0x0004 != 0 && class == 2)
        || (filter.flags & 0x0008 != 0 && class == 3)
        || (class == 3
            && filter
                .mark_filtering_set
                .is_some_and(|set| !gdef_mark_set_contains(gdef, set, glyph)))
        || (class == 3
            && filter.mark_filtering_set.is_none()
            && filter.flags >> 8 != 0
            && gdef_mark_attachment_class(gdef, glyph) != filter.flags >> 8)
}

fn lookup_filter(table: &[u8], lookup: usize) -> Option<LookupFilter> {
    let flags = read_u16(table, lookup + 2)?;
    let subtable_count = usize::from(read_u16(table, lookup + 4)?);
    checked_slice(table, lookup + 6, subtable_count.checked_mul(2)?)?;
    let mark_filtering_set = if flags & 0x0010 != 0 {
        Some(read_u16(table, lookup + 6 + subtable_count * 2)?)
    } else {
        None
    };
    Some(LookupFilter {
        flags,
        mark_filtering_set,
    })
}

fn gdef_glyph_class(gdef: Option<&[u8]>, glyph: u16) -> u16 {
    gdef.and_then(|table| {
        let offset = read_u16(table, 4).filter(|offset| *offset != 0)?;
        class_value(table, usize::from(offset), glyph)
    })
    .unwrap_or(0)
}

fn gdef_mark_attachment_class(gdef: Option<&[u8]>, glyph: u16) -> u16 {
    gdef.and_then(|table| {
        let offset = read_u16(table, 10).filter(|offset| *offset != 0)?;
        class_value(table, usize::from(offset), glyph)
    })
    .unwrap_or(0)
}

fn gdef_mark_set_contains(gdef: Option<&[u8]>, set: u16, glyph: u16) -> bool {
    gdef.and_then(|table| {
        if read_u16(table, 0)? != 1 || read_u16(table, 2)? < 2 {
            return Some(false);
        }
        let sets_offset = usize::from(read_u16(table, 12)?);
        if sets_offset == 0 || read_u16(table, sets_offset)? != 1 {
            return Some(false);
        }
        let count = usize::from(read_u16(table, sets_offset + 2)?);
        let set = usize::from(set);
        if set >= count {
            return Some(false);
        }
        let coverage_offset = usize::try_from(read_u32(table, sets_offset + 4 + set * 4)?).ok()?;
        let coverage = sets_offset.checked_add(coverage_offset)?;
        Some(coverage_index(table, coverage, glyph).is_some())
    })
    .unwrap_or(false)
}

fn feature_lookups(table: &[u8], script: Tag, enabled: &[Tag]) -> Option<Vec<usize>> {
    let script_list = usize::from(read_u16(table, 4)?);
    let feature_list = usize::from(read_u16(table, 6)?);
    let lookup_list = usize::from(read_u16(table, 8)?);
    let langsys = select_langsys(table, script_list, script)?;
    let feature_count = usize::from(read_u16(table, langsys + 4)?);
    checked_slice(table, langsys + 6, feature_count.checked_mul(2)?)?;
    let mut feature_indices = Vec::with_capacity(feature_count + 1);
    let required = read_u16(table, langsys + 2)?;
    if required != u16::MAX {
        feature_indices.push(required);
    }
    for index in 0..feature_count {
        feature_indices.push(read_u16(table, langsys + 6 + index * 2)?);
    }

    let global_feature_count = usize::from(read_u16(table, feature_list)?);
    checked_slice(
        table,
        feature_list + 2,
        global_feature_count.checked_mul(6)?,
    )?;
    let lookup_count = usize::from(read_u16(table, lookup_list)?);
    checked_slice(table, lookup_list + 2, lookup_count.checked_mul(2)?)?;
    let mut result = Vec::new();
    for feature_tag in enabled {
        for &feature_index in &feature_indices {
            let feature_index = usize::from(feature_index);
            if feature_index >= global_feature_count {
                return None;
            }
            let record = feature_list + 2 + feature_index * 6;
            if read_tag(table, record)? != *feature_tag {
                continue;
            }
            let feature = feature_list.checked_add(usize::from(read_u16(table, record + 4)?))?;
            let count = usize::from(read_u16(table, feature + 2)?);
            checked_slice(table, feature + 4, count.checked_mul(2)?)?;
            for index in 0..count {
                let lookup_index = usize::from(read_u16(table, feature + 4 + index * 2)?);
                if lookup_index >= lookup_count {
                    return None;
                }
                let lookup = lookup_list.checked_add(usize::from(read_u16(
                    table,
                    lookup_list + 2 + lookup_index * 2,
                )?))?;
                if !result.iter().any(|(existing, _)| *existing == lookup_index) {
                    result.push((lookup_index, lookup));
                }
            }
        }
    }
    result.sort_unstable_by_key(|(lookup_index, _)| *lookup_index);
    Some(result.into_iter().map(|(_, lookup)| lookup).collect())
}

fn select_langsys(table: &[u8], script_list: usize, script: Tag) -> Option<usize> {
    let count = usize::from(read_u16(table, script_list)?);
    checked_slice(table, script_list + 2, count.checked_mul(6)?)?;
    let mut selected = None;
    let mut default = None;
    for index in 0..count {
        let record = script_list + 2 + index * 6;
        let tag = read_tag(table, record)?;
        let offset = usize::from(read_u16(table, record + 4)?);
        if tag == script {
            selected = Some(script_list.checked_add(offset)?);
            break;
        }
        if tag == *b"DFLT" {
            default = Some(script_list.checked_add(offset)?);
        }
        if script == *b"dev2" && tag == *b"deva" && selected.is_none() {
            selected = Some(script_list.checked_add(offset)?);
        }
    }
    let script_table = selected.or(default)?;
    let default_langsys = usize::from(read_u16(table, script_table)?);
    if default_langsys != 0 {
        return script_table.checked_add(default_langsys);
    }
    let language_count = usize::from(read_u16(table, script_table + 2)?);
    if language_count == 0 {
        return None;
    }
    script_table.checked_add(usize::from(read_u16(table, script_table + 8)?))
}

fn coverage_index(table: &[u8], offset: usize, glyph: u16) -> Option<usize> {
    match read_u16(table, offset)? {
        1 => {
            let count = usize::from(read_u16(table, offset + 2)?);
            checked_slice(table, offset + 4, count.checked_mul(2)?)?;
            (0..count).find(|index| read_u16(table, offset + 4 + index * 2) == Some(glyph))
        }
        2 => {
            let count = usize::from(read_u16(table, offset + 2)?);
            checked_slice(table, offset + 4, count.checked_mul(6)?)?;
            for index in 0..count {
                let record = offset + 4 + index * 6;
                let start = read_u16(table, record)?;
                let end = read_u16(table, record + 2)?;
                if (start..=end).contains(&glyph) {
                    return usize::from(read_u16(table, record + 4)?)
                        .checked_add(usize::from(glyph - start));
                }
            }
            None
        }
        _ => None,
    }
}

fn class_value(table: &[u8], offset: usize, glyph: u16) -> Option<u16> {
    match read_u16(table, offset)? {
        1 => {
            let start = read_u16(table, offset + 2)?;
            let count = read_u16(table, offset + 4)?;
            let index = glyph.checked_sub(start)?;
            if index >= count {
                return None;
            }
            read_u16(table, offset + 6 + usize::from(index) * 2)
        }
        2 => {
            let count = usize::from(read_u16(table, offset + 2)?);
            checked_slice(table, offset + 4, count.checked_mul(6)?)?;
            for index in 0..count {
                let record = offset + 4 + index * 6;
                if (read_u16(table, record)?..=read_u16(table, record + 2)?).contains(&glyph) {
                    return read_u16(table, record + 4);
                }
            }
            None
        }
        _ => None,
    }
}

fn checked_slice(data: &[u8], offset: usize, length: usize) -> Option<&[u8]> {
    data.get(offset..offset.checked_add(length)?)
}

fn read_tag(data: &[u8], offset: usize) -> Option<Tag> {
    let bytes = checked_slice(data, offset, 4)?;
    Some([bytes[0], bytes[1], bytes[2], bytes[3]])
}

fn read_u16(data: &[u8], offset: usize) -> Option<u16> {
    let bytes = checked_slice(data, offset, 2)?;
    Some(u16::from_be_bytes([bytes[0], bytes[1]]))
}

fn read_i16(data: &[u8], offset: usize) -> Option<i16> {
    read_u16(data, offset).map(|value| value as i16)
}

fn read_u32(data: &[u8], offset: usize) -> Option<u32> {
    let bytes = checked_slice(data, offset, 4)?;
    Some(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}
