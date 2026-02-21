use crate::canvas::{Command, Document, Page};
use crate::flowable::PaintFilterSpec;
use crate::types::{Color, MixBlendMode, Pt, Shading, ShadingStop, Size};
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub struct SpillStore {
    dir: PathBuf,
    counter: AtomicU64,
    files: AtomicU64,
    bytes: AtomicU64,
}

impl SpillStore {
    pub fn new(dir: impl AsRef<Path>) -> io::Result<Self> {
        let dir = dir.as_ref().to_path_buf();
        fs::create_dir_all(&dir)?;
        Ok(Self {
            dir,
            counter: AtomicU64::new(0),
            files: AtomicU64::new(0),
            bytes: AtomicU64::new(0),
        })
    }

    pub fn spill(&self, doc: &Document) -> io::Result<PathBuf> {
        let id = self.counter.fetch_add(1, Ordering::Relaxed);
        let path = self.dir.join(format!("fullbleed_spill_{id}.bin"));
        let mut file = File::create(&path)?;
        write_document(&mut file, doc)?;
        let size = file.metadata().map(|m| m.len()).unwrap_or(0);
        self.files.fetch_add(1, Ordering::Relaxed);
        self.bytes.fetch_add(size, Ordering::Relaxed);
        Ok(path)
    }

    pub fn load(&self, path: &Path) -> io::Result<Document> {
        let mut file = File::open(path)?;
        let doc = read_document(&mut file)?;
        fs::remove_file(path)?;
        Ok(doc)
    }

    pub fn metrics(&self) -> (u64, u64) {
        (
            self.files.load(Ordering::Relaxed),
            self.bytes.load(Ordering::Relaxed),
        )
    }
}

fn write_document<W: Write>(out: &mut W, doc: &Document) -> io::Result<()> {
    write_size(out, doc.page_size)?;
    write_u32(out, doc.pages.len() as u32)?;
    for page in &doc.pages {
        write_page(out, page)?;
    }
    Ok(())
}

fn read_document<R: Read>(input: &mut R) -> io::Result<Document> {
    let page_size = read_size(input)?;
    let pages_len = read_u32(input)? as usize;
    let mut pages = Vec::with_capacity(pages_len);
    for _ in 0..pages_len {
        pages.push(read_page(input)?);
    }
    Ok(Document { page_size, pages })
}

fn write_page<W: Write>(out: &mut W, page: &Page) -> io::Result<()> {
    write_u32(out, page.commands.len() as u32)?;
    for command in &page.commands {
        write_command(out, command)?;
    }
    Ok(())
}

fn read_page<R: Read>(input: &mut R) -> io::Result<Page> {
    let len = read_u32(input)? as usize;
    let mut commands = Vec::with_capacity(len);
    for _ in 0..len {
        commands.push(read_command(input)?);
    }
    Ok(Page { commands })
}

fn write_command<W: Write>(out: &mut W, command: &Command) -> io::Result<()> {
    match command {
        Command::SaveState => write_u8(out, 1),
        Command::RestoreState => write_u8(out, 2),
        Command::Translate(x, y) => {
            write_u8(out, 3)?;
            write_pt(out, *x)?;
            write_pt(out, *y)
        }
        Command::Scale(x, y) => {
            write_u8(out, 4)?;
            write_f32(out, *x)?;
            write_f32(out, *y)
        }
        Command::Rotate(angle) => {
            write_u8(out, 5)?;
            write_f32(out, *angle)
        }
        Command::ConcatMatrix { a, b, c, d, e, f } => {
            write_u8(out, 41)?;
            write_f32(out, *a)?;
            write_f32(out, *b)?;
            write_f32(out, *c)?;
            write_f32(out, *d)?;
            write_pt(out, *e)?;
            write_pt(out, *f)
        }
        Command::Meta { key, value } => {
            write_u8(out, 6)?;
            write_string(out, key)?;
            write_string(out, value)
        }
        Command::SetFillColor(color) => {
            write_u8(out, 7)?;
            write_color(out, *color)
        }
        Command::SetStrokeColor(color) => {
            write_u8(out, 8)?;
            write_color(out, *color)
        }
        Command::SetLineWidth(width) => {
            write_u8(out, 9)?;
            write_pt(out, *width)
        }
        Command::SetLineCap(cap) => {
            write_u8(out, 10)?;
            write_u8(out, *cap)
        }
        Command::SetLineJoin(join) => {
            write_u8(out, 11)?;
            write_u8(out, *join)
        }
        Command::SetMiterLimit(limit) => {
            write_u8(out, 12)?;
            write_pt(out, *limit)
        }
        Command::SetDash { pattern, phase } => {
            write_u8(out, 13)?;
            write_u32(out, pattern.len() as u32)?;
            for value in pattern {
                write_pt(out, *value)?;
            }
            write_pt(out, *phase)
        }
        Command::SetOpacity { fill, stroke } => {
            write_u8(out, 14)?;
            write_f32(out, *fill)?;
            write_f32(out, *stroke)
        }
        Command::SetBlendMode { mode } => {
            write_u8(out, 42)?;
            write_u8(
                out,
                match mode {
                    MixBlendMode::Normal => 0,
                    MixBlendMode::Multiply => 1,
                    MixBlendMode::Screen => 2,
                },
            )
        }
        Command::ApplyBackdropFilter {
            x,
            y,
            width,
            height,
            radius,
            filter,
        } => {
            write_u8(out, 43)?;
            write_pt(out, *x)?;
            write_pt(out, *y)?;
            write_pt(out, *width)?;
            write_pt(out, *height)?;
            write_pt(out, *radius)?;
            write_paint_filter(out, *filter)
        }
        Command::SetFontName(name) => {
            write_u8(out, 15)?;
            write_string(out, name)
        }
        Command::SetFontSize(size) => {
            write_u8(out, 16)?;
            write_pt(out, *size)
        }
        Command::ClipRect {
            x,
            y,
            width,
            height,
        } => {
            write_u8(out, 17)?;
            write_pt(out, *x)?;
            write_pt(out, *y)?;
            write_pt(out, *width)?;
            write_pt(out, *height)
        }
        Command::ClipPath { evenodd } => {
            write_u8(out, 18)?;
            write_bool(out, *evenodd)
        }
        Command::ShadingFill(shading) => {
            write_u8(out, 19)?;
            write_shading(out, shading)
        }
        Command::MoveTo { x, y } => {
            write_u8(out, 20)?;
            write_pt(out, *x)?;
            write_pt(out, *y)
        }
        Command::LineTo { x, y } => {
            write_u8(out, 21)?;
            write_pt(out, *x)?;
            write_pt(out, *y)
        }
        Command::CurveTo {
            x1,
            y1,
            x2,
            y2,
            x,
            y,
        } => {
            write_u8(out, 22)?;
            write_pt(out, *x1)?;
            write_pt(out, *y1)?;
            write_pt(out, *x2)?;
            write_pt(out, *y2)?;
            write_pt(out, *x)?;
            write_pt(out, *y)
        }
        Command::ClosePath => write_u8(out, 23),
        Command::Fill => write_u8(out, 24),
        Command::FillEvenOdd => write_u8(out, 25),
        Command::Stroke => write_u8(out, 26),
        Command::FillStroke => write_u8(out, 27),
        Command::FillStrokeEvenOdd => write_u8(out, 28),
        Command::DrawString { x, y, text } => {
            write_u8(out, 29)?;
            write_pt(out, *x)?;
            write_pt(out, *y)?;
            write_string(out, text)
        }
        Command::DrawStringTransformed {
            x,
            y,
            text,
            m00,
            m01,
            m10,
            m11,
        } => {
            write_u8(out, 40)?;
            write_pt(out, *x)?;
            write_pt(out, *y)?;
            write_string(out, text)?;
            write_f32(out, *m00)?;
            write_f32(out, *m01)?;
            write_f32(out, *m10)?;
            write_f32(out, *m11)
        }
        Command::DrawGlyphRun {
            x,
            y,
            glyph_ids,
            advances,
            m00,
            m01,
            m10,
            m11,
        } => {
            write_u8(out, 39)?;
            write_pt(out, *x)?;
            write_pt(out, *y)?;
            write_u32(out, glyph_ids.len() as u32)?;
            for gid in glyph_ids {
                write_u16(out, *gid)?;
            }
            write_u32(out, advances.len() as u32)?;
            for (dx, dy) in advances {
                write_pt(out, *dx)?;
                write_pt(out, *dy)?;
            }
            write_f32(out, *m00)?;
            write_f32(out, *m01)?;
            write_f32(out, *m10)?;
            write_f32(out, *m11)?;
            Ok(())
        }
        Command::DrawRect {
            x,
            y,
            width,
            height,
        } => {
            write_u8(out, 30)?;
            write_pt(out, *x)?;
            write_pt(out, *y)?;
            write_pt(out, *width)?;
            write_pt(out, *height)
        }
        Command::DrawImage {
            x,
            y,
            width,
            height,
            resource_id,
        } => {
            write_u8(out, 31)?;
            write_pt(out, *x)?;
            write_pt(out, *y)?;
            write_pt(out, *width)?;
            write_pt(out, *height)?;
            write_string(out, resource_id)
        }
        Command::BeginTag {
            role,
            mcid,
            alt,
            scope,
            table_id,
            col_index,
            group_only,
        } => {
            write_u8(out, 32)?;
            write_string(out, role)?;
            write_option_u32(out, *mcid)?;
            write_option_string(out, alt.as_deref())?;
            write_option_string(out, scope.as_deref())?;
            write_option_u32(out, table_id.map(|v| v as u32))?;
            write_option_u16(out, *col_index)?;
            write_bool(out, *group_only)
        }
        Command::EndTag => write_u8(out, 33),
        Command::DefineForm {
            resource_id,
            width,
            height,
            commands,
        } => {
            write_u8(out, 34)?;
            write_string(out, resource_id)?;
            write_pt(out, *width)?;
            write_pt(out, *height)?;
            write_u32(out, commands.len() as u32)?;
            for cmd in commands {
                write_command(out, cmd)?;
            }
            Ok(())
        }
        Command::DrawForm {
            x,
            y,
            width,
            height,
            resource_id,
        } => {
            write_u8(out, 35)?;
            write_pt(out, *x)?;
            write_pt(out, *y)?;
            write_pt(out, *width)?;
            write_pt(out, *height)?;
            write_string(out, resource_id)
        }
        Command::BeginArtifact { subtype } => {
            write_u8(out, 36)?;
            write_option_string(out, subtype.as_deref())
        }
        Command::BeginOptionalContent { name } => {
            write_u8(out, 37)?;
            write_string(out, name)
        }
        Command::EndMarkedContent => write_u8(out, 38),
    }
}

fn read_command<R: Read>(input: &mut R) -> io::Result<Command> {
    let tag = read_u8(input)?;
    let command = match tag {
        1 => Command::SaveState,
        2 => Command::RestoreState,
        3 => Command::Translate(read_pt(input)?, read_pt(input)?),
        4 => Command::Scale(read_f32(input)?, read_f32(input)?),
        5 => Command::Rotate(read_f32(input)?),
        41 => Command::ConcatMatrix {
            a: read_f32(input)?,
            b: read_f32(input)?,
            c: read_f32(input)?,
            d: read_f32(input)?,
            e: read_pt(input)?,
            f: read_pt(input)?,
        },
        6 => Command::Meta {
            key: read_string(input)?,
            value: read_string(input)?,
        },
        7 => Command::SetFillColor(read_color(input)?),
        8 => Command::SetStrokeColor(read_color(input)?),
        9 => Command::SetLineWidth(read_pt(input)?),
        10 => Command::SetLineCap(read_u8(input)?),
        11 => Command::SetLineJoin(read_u8(input)?),
        12 => Command::SetMiterLimit(read_pt(input)?),
        13 => {
            let len = read_u32(input)? as usize;
            let mut pattern = Vec::with_capacity(len);
            for _ in 0..len {
                pattern.push(read_pt(input)?);
            }
            let phase = read_pt(input)?;
            Command::SetDash { pattern, phase }
        }
        14 => Command::SetOpacity {
            fill: read_f32(input)?,
            stroke: read_f32(input)?,
        },
        42 => {
            let mode = match read_u8(input)? {
                1 => MixBlendMode::Multiply,
                2 => MixBlendMode::Screen,
                _ => MixBlendMode::Normal,
            };
            Command::SetBlendMode { mode }
        }
        43 => Command::ApplyBackdropFilter {
            x: read_pt(input)?,
            y: read_pt(input)?,
            width: read_pt(input)?,
            height: read_pt(input)?,
            radius: read_pt(input)?,
            filter: read_paint_filter(input)?,
        },
        15 => Command::SetFontName(read_string(input)?),
        16 => Command::SetFontSize(read_pt(input)?),
        17 => Command::ClipRect {
            x: read_pt(input)?,
            y: read_pt(input)?,
            width: read_pt(input)?,
            height: read_pt(input)?,
        },
        18 => Command::ClipPath {
            evenodd: read_bool(input)?,
        },
        19 => Command::ShadingFill(read_shading(input)?),
        20 => Command::MoveTo {
            x: read_pt(input)?,
            y: read_pt(input)?,
        },
        21 => Command::LineTo {
            x: read_pt(input)?,
            y: read_pt(input)?,
        },
        22 => Command::CurveTo {
            x1: read_pt(input)?,
            y1: read_pt(input)?,
            x2: read_pt(input)?,
            y2: read_pt(input)?,
            x: read_pt(input)?,
            y: read_pt(input)?,
        },
        23 => Command::ClosePath,
        24 => Command::Fill,
        25 => Command::FillEvenOdd,
        26 => Command::Stroke,
        27 => Command::FillStroke,
        28 => Command::FillStrokeEvenOdd,
        29 => Command::DrawString {
            x: read_pt(input)?,
            y: read_pt(input)?,
            text: read_string(input)?,
        },
        40 => Command::DrawStringTransformed {
            x: read_pt(input)?,
            y: read_pt(input)?,
            text: read_string(input)?,
            m00: read_f32(input)?,
            m01: read_f32(input)?,
            m10: read_f32(input)?,
            m11: read_f32(input)?,
        },
        39 => {
            let x = read_pt(input)?;
            let y = read_pt(input)?;
            let glyph_len = read_u32(input)? as usize;
            let mut glyph_ids = Vec::with_capacity(glyph_len);
            for _ in 0..glyph_len {
                glyph_ids.push(read_u16(input)?);
            }
            let adv_len = read_u32(input)? as usize;
            let mut advances = Vec::with_capacity(adv_len);
            for _ in 0..adv_len {
                advances.push((read_pt(input)?, read_pt(input)?));
            }
            let m00 = read_f32(input)?;
            let m01 = read_f32(input)?;
            let m10 = read_f32(input)?;
            let m11 = read_f32(input)?;
            Command::DrawGlyphRun {
                x,
                y,
                glyph_ids,
                advances,
                m00,
                m01,
                m10,
                m11,
            }
        }
        30 => Command::DrawRect {
            x: read_pt(input)?,
            y: read_pt(input)?,
            width: read_pt(input)?,
            height: read_pt(input)?,
        },
        31 => Command::DrawImage {
            x: read_pt(input)?,
            y: read_pt(input)?,
            width: read_pt(input)?,
            height: read_pt(input)?,
            resource_id: read_string(input)?,
        },
        32 => Command::BeginTag {
            role: read_string(input)?,
            mcid: read_option_u32(input)?,
            alt: read_option_string(input)?,
            scope: read_option_string(input)?,
            table_id: read_option_u32(input)?,
            col_index: read_option_u16(input)?,
            group_only: read_bool(input)?,
        },
        33 => Command::EndTag,
        34 => {
            let resource_id = read_string(input)?;
            let width = read_pt(input)?;
            let height = read_pt(input)?;
            let len = read_u32(input)? as usize;
            let mut commands = Vec::with_capacity(len);
            for _ in 0..len {
                commands.push(read_command(input)?);
            }
            Command::DefineForm {
                resource_id,
                width,
                height,
                commands,
            }
        }
        35 => Command::DrawForm {
            x: read_pt(input)?,
            y: read_pt(input)?,
            width: read_pt(input)?,
            height: read_pt(input)?,
            resource_id: read_string(input)?,
        },
        36 => Command::BeginArtifact {
            subtype: read_option_string(input)?,
        },
        37 => Command::BeginOptionalContent {
            name: read_string(input)?,
        },
        38 => Command::EndMarkedContent,
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unknown spill command tag {tag}"),
            ));
        }
    };
    Ok(command)
}

fn write_shading<W: Write>(out: &mut W, shading: &Shading) -> io::Result<()> {
    match shading {
        Shading::Axial {
            x0,
            y0,
            x1,
            y1,
            stops,
        } => {
            write_u8(out, 1)?;
            write_f32(out, *x0)?;
            write_f32(out, *y0)?;
            write_f32(out, *x1)?;
            write_f32(out, *y1)?;
            write_stops(out, stops)
        }
        Shading::Radial {
            x0,
            y0,
            r0,
            x1,
            y1,
            r1,
            stops,
        } => {
            write_u8(out, 2)?;
            write_f32(out, *x0)?;
            write_f32(out, *y0)?;
            write_f32(out, *r0)?;
            write_f32(out, *x1)?;
            write_f32(out, *y1)?;
            write_f32(out, *r1)?;
            write_stops(out, stops)
        }
    }
}

fn read_shading<R: Read>(input: &mut R) -> io::Result<Shading> {
    let tag = read_u8(input)?;
    let shading = match tag {
        1 => {
            let x0 = read_f32(input)?;
            let y0 = read_f32(input)?;
            let x1 = read_f32(input)?;
            let y1 = read_f32(input)?;
            let stops = read_stops(input)?;
            Shading::Axial {
                x0,
                y0,
                x1,
                y1,
                stops,
            }
        }
        2 => {
            let x0 = read_f32(input)?;
            let y0 = read_f32(input)?;
            let r0 = read_f32(input)?;
            let x1 = read_f32(input)?;
            let y1 = read_f32(input)?;
            let r1 = read_f32(input)?;
            let stops = read_stops(input)?;
            Shading::Radial {
                x0,
                y0,
                r0,
                x1,
                y1,
                r1,
                stops,
            }
        }
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unknown shading tag {tag}"),
            ));
        }
    };
    Ok(shading)
}

fn write_stops<W: Write>(out: &mut W, stops: &[ShadingStop]) -> io::Result<()> {
    write_u32(out, stops.len() as u32)?;
    for stop in stops {
        write_f32(out, stop.offset)?;
        write_color(out, stop.color)?;
    }
    Ok(())
}

fn read_stops<R: Read>(input: &mut R) -> io::Result<Vec<ShadingStop>> {
    let len = read_u32(input)? as usize;
    let mut stops = Vec::with_capacity(len);
    for _ in 0..len {
        let offset = read_f32(input)?;
        let color = read_color(input)?;
        stops.push(ShadingStop { offset, color });
    }
    Ok(stops)
}

fn write_size<W: Write>(out: &mut W, size: Size) -> io::Result<()> {
    write_pt(out, size.width)?;
    write_pt(out, size.height)
}

fn read_size<R: Read>(input: &mut R) -> io::Result<Size> {
    Ok(Size {
        width: read_pt(input)?,
        height: read_pt(input)?,
    })
}

fn write_color<W: Write>(out: &mut W, color: Color) -> io::Result<()> {
    write_f32(out, color.r)?;
    write_f32(out, color.g)?;
    write_f32(out, color.b)
}

fn read_color<R: Read>(input: &mut R) -> io::Result<Color> {
    Ok(Color {
        r: read_f32(input)?,
        g: read_f32(input)?,
        b: read_f32(input)?,
    })
}

fn write_pt<W: Write>(out: &mut W, value: Pt) -> io::Result<()> {
    write_i64(out, value.to_milli_i64())
}

fn read_pt<R: Read>(input: &mut R) -> io::Result<Pt> {
    let milli = read_i64(input)?;
    Ok(Pt::from_milli_i64(milli))
}

fn write_paint_filter<W: Write>(out: &mut W, filter: PaintFilterSpec) -> io::Result<()> {
    write_f32(out, filter.saturate)?;
    write_pt(out, filter.blur_radius)
}

fn read_paint_filter<R: Read>(input: &mut R) -> io::Result<PaintFilterSpec> {
    Ok(PaintFilterSpec {
        saturate: read_f32(input)?,
        blur_radius: read_pt(input)?,
    })
}

fn write_string<W: Write>(out: &mut W, value: &str) -> io::Result<()> {
    let bytes = value.as_bytes();
    write_u32(out, bytes.len() as u32)?;
    out.write_all(bytes)
}

fn read_string<R: Read>(input: &mut R) -> io::Result<String> {
    let len = read_u32(input)? as usize;
    let mut buf = vec![0u8; len];
    input.read_exact(&mut buf)?;
    String::from_utf8(buf).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
}

fn write_option_string<W: Write>(out: &mut W, value: Option<&str>) -> io::Result<()> {
    match value {
        Some(v) => {
            write_u8(out, 1)?;
            write_string(out, v)
        }
        None => write_u8(out, 0),
    }
}

fn read_option_string<R: Read>(input: &mut R) -> io::Result<Option<String>> {
    let flag = read_u8(input)?;
    if flag == 0 {
        Ok(None)
    } else {
        read_string(input).map(Some)
    }
}

fn write_option_u32<W: Write>(out: &mut W, value: Option<u32>) -> io::Result<()> {
    match value {
        Some(v) => {
            write_u8(out, 1)?;
            write_u32(out, v)
        }
        None => write_u8(out, 0),
    }
}

fn read_option_u32<R: Read>(input: &mut R) -> io::Result<Option<u32>> {
    let flag = read_u8(input)?;
    if flag == 0 {
        Ok(None)
    } else {
        read_u32(input).map(Some)
    }
}

fn write_option_u16<W: Write>(out: &mut W, value: Option<u16>) -> io::Result<()> {
    match value {
        Some(v) => {
            write_u8(out, 1)?;
            write_u16(out, v)
        }
        None => write_u8(out, 0),
    }
}

fn read_option_u16<R: Read>(input: &mut R) -> io::Result<Option<u16>> {
    let flag = read_u8(input)?;
    if flag == 0 {
        Ok(None)
    } else {
        read_u16(input).map(Some)
    }
}

fn write_bool<W: Write>(out: &mut W, value: bool) -> io::Result<()> {
    write_u8(out, if value { 1 } else { 0 })
}

fn read_bool<R: Read>(input: &mut R) -> io::Result<bool> {
    Ok(read_u8(input)? != 0)
}

fn write_u8<W: Write>(out: &mut W, value: u8) -> io::Result<()> {
    out.write_all(&[value])
}

fn read_u8<R: Read>(input: &mut R) -> io::Result<u8> {
    let mut buf = [0u8; 1];
    input.read_exact(&mut buf)?;
    Ok(buf[0])
}

fn write_u16<W: Write>(out: &mut W, value: u16) -> io::Result<()> {
    out.write_all(&value.to_le_bytes())
}

fn read_u16<R: Read>(input: &mut R) -> io::Result<u16> {
    let mut buf = [0u8; 2];
    input.read_exact(&mut buf)?;
    Ok(u16::from_le_bytes(buf))
}

fn write_u32<W: Write>(out: &mut W, value: u32) -> io::Result<()> {
    out.write_all(&value.to_le_bytes())
}

fn read_u32<R: Read>(input: &mut R) -> io::Result<u32> {
    let mut buf = [0u8; 4];
    input.read_exact(&mut buf)?;
    Ok(u32::from_le_bytes(buf))
}

fn write_i64<W: Write>(out: &mut W, value: i64) -> io::Result<()> {
    out.write_all(&value.to_le_bytes())
}

fn read_i64<R: Read>(input: &mut R) -> io::Result<i64> {
    let mut buf = [0u8; 8];
    input.read_exact(&mut buf)?;
    Ok(i64::from_le_bytes(buf))
}

fn write_f32<W: Write>(out: &mut W, value: f32) -> io::Result<()> {
    write_u32(out, value.to_bits())
}

fn read_f32<R: Read>(input: &mut R) -> io::Result<f32> {
    Ok(f32::from_bits(read_u32(input)?))
}
