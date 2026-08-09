use crate::flate_native::{zlib_deflate_parallel, zlib_inflate};
use std::fmt;
use std::ops::Index;

const PNG_SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
const MAX_IMAGE_BYTES: usize = 1 << 30;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ImageFormat {
    Png,
    Jpeg,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ImageColor {
    Gray,
    GrayAlpha,
    Rgb,
    Rgba,
    Cmyk,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ImageError {
    offset: usize,
    reason: String,
}

impl ImageError {
    fn new(offset: usize, reason: impl Into<String>) -> Self {
        Self {
            offset,
            reason: reason.into(),
        }
    }
}

impl fmt::Display for ImageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "invalid image at byte {}: {}",
            self.offset, self.reason
        )
    }
}

impl std::error::Error for ImageError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Rgba(pub [u8; 4]);

impl Index<usize> for Rgba {
    type Output = u8;

    fn index(&self, index: usize) -> &Self::Output {
        &self.0[index]
    }
}

#[cfg(feature = "python")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Luma(pub [u8; 1]);

#[cfg(feature = "python")]
impl Index<usize> for Luma {
    type Output = u8;

    fn index(&self, index: usize) -> &Self::Output {
        &self.0[index]
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RgbaImage {
    width: u32,
    height: u32,
    data: Vec<u8>,
}

impl RgbaImage {
    #[cfg(test)]
    pub(crate) fn new(width: u32, height: u32) -> Self {
        let length = image_buffer_len(width, height, 4).unwrap_or(0);
        Self {
            width,
            height,
            data: vec![0; length],
        }
    }

    pub(crate) fn from_raw(width: u32, height: u32, data: Vec<u8>) -> Option<Self> {
        (image_buffer_len(width, height, 4)? == data.len()).then_some(Self {
            width,
            height,
            data,
        })
    }

    #[cfg(test)]
    pub(crate) fn width(&self) -> u32 {
        self.width
    }

    #[cfg(test)]
    pub(crate) fn height(&self) -> u32 {
        self.height
    }

    pub(crate) fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    pub(crate) fn as_raw(&self) -> &Vec<u8> {
        &self.data
    }

    #[cfg(test)]
    pub(crate) fn as_bytes(&self) -> &[u8] {
        &self.data
    }

    pub(crate) fn into_raw(self) -> Vec<u8> {
        self.data
    }

    pub(crate) fn pixels(&self) -> impl Iterator<Item = Rgba> + '_ {
        self.data
            .chunks_exact(4)
            .map(|pixel| Rgba([pixel[0], pixel[1], pixel[2], pixel[3]]))
    }

    #[cfg(test)]
    pub(crate) fn enumerate_pixels(&self) -> impl Iterator<Item = (u32, u32, Rgba)> + '_ {
        let width = self.width;
        self.pixels().enumerate().map(move |(index, pixel)| {
            let index = index as u32;
            (index % width, index / width, pixel)
        })
    }

    #[cfg(test)]
    pub(crate) fn get_pixel(&self, x: u32, y: u32) -> Rgba {
        assert!(y < self.height, "pixel coordinate in bounds");
        let offset = pixel_offset(self.width, x, y, 4).expect("pixel coordinate in bounds");
        Rgba([
            self.data[offset],
            self.data[offset + 1],
            self.data[offset + 2],
            self.data[offset + 3],
        ])
    }

    #[cfg(test)]
    pub(crate) fn put_pixel(&mut self, x: u32, y: u32, pixel: Rgba) {
        assert!(y < self.height, "pixel coordinate in bounds");
        let offset = pixel_offset(self.width, x, y, 4).expect("pixel coordinate in bounds");
        self.data[offset..offset + 4].copy_from_slice(&pixel.0);
    }
}

#[cfg(any(feature = "python", test))]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct GrayImage {
    width: u32,
    height: u32,
    data: Vec<u8>,
}

#[cfg(any(feature = "python", test))]
impl GrayImage {
    #[cfg(test)]
    pub(crate) fn from_raw(width: u32, height: u32, data: Vec<u8>) -> Option<Self> {
        (image_buffer_len(width, height, 1)? == data.len()).then_some(Self {
            width,
            height,
            data,
        })
    }

    pub(crate) fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    #[cfg(feature = "python")]
    pub(crate) fn pixels(&self) -> impl Iterator<Item = Luma> + '_ {
        self.data.iter().copied().map(|value| Luma([value]))
    }

    #[cfg(feature = "python")]
    pub(crate) fn get_pixel(&self, x: u32, y: u32) -> Luma {
        assert!(y < self.height, "pixel coordinate in bounds");
        let offset = pixel_offset(self.width, x, y, 1).expect("pixel coordinate in bounds");
        Luma([self.data[offset]])
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DecodedImage {
    pixels: RgbaImage,
    source_color: ImageColor,
    jpeg_adobe_transform: Option<u8>,
}

impl DecodedImage {
    pub(crate) fn dimensions(&self) -> (u32, u32) {
        self.pixels.dimensions()
    }

    pub(crate) fn color(&self) -> ImageColor {
        self.source_color
    }

    pub(crate) fn jpeg_adobe_transform(&self) -> Option<u8> {
        self.jpeg_adobe_transform
    }

    pub(crate) fn to_rgba8(&self) -> RgbaImage {
        self.pixels.clone()
    }

    #[cfg(any(feature = "python", test))]
    pub(crate) fn into_rgba8(self) -> RgbaImage {
        self.pixels
    }

    #[cfg(feature = "python")]
    pub(crate) fn to_luma8(&self) -> GrayImage {
        let mut gray = Vec::with_capacity(self.pixels.data.len() / 4);
        for pixel in self.pixels.data.chunks_exact(4) {
            // ITU-R BT.601 luma, matching conventional 8-bit image conversion.
            let y = (u32::from(pixel[0]) * 299
                + u32::from(pixel[1]) * 587
                + u32::from(pixel[2]) * 114
                + 500)
                / 1000;
            gray.push(y as u8);
        }
        GrayImage {
            width: self.pixels.width,
            height: self.pixels.height,
            data: gray,
        }
    }
}

pub(crate) fn guess_format(data: &[u8]) -> Result<ImageFormat, ImageError> {
    if data.starts_with(PNG_SIGNATURE) {
        return Ok(ImageFormat::Png);
    }
    if data.starts_with(&[0xff, 0xd8]) {
        return Ok(ImageFormat::Jpeg);
    }
    Err(ImageError::new(0, "unrecognized image signature"))
}

pub(crate) fn dimensions(data: &[u8]) -> Result<(u32, u32), ImageError> {
    match guess_format(data)? {
        ImageFormat::Png => png_dimensions(data),
        ImageFormat::Jpeg => jpeg_dimensions(data),
    }
}

pub(crate) fn load_from_memory(data: &[u8]) -> Result<DecodedImage, ImageError> {
    load_from_memory_with_format(data, guess_format(data)?)
}

pub(crate) fn load_from_memory_with_format(
    data: &[u8],
    format: ImageFormat,
) -> Result<DecodedImage, ImageError> {
    match format {
        ImageFormat::Png => decode_png(data),
        ImageFormat::Jpeg => {
            let decoded = crate::jpeg_native::decode(data)
                .map_err(|error| ImageError::new(error.offset, error.to_string()))?;
            let source_color = match decoded.color {
                crate::jpeg_native::JpegColor::Gray => ImageColor::Gray,
                crate::jpeg_native::JpegColor::Rgb => ImageColor::Rgb,
                crate::jpeg_native::JpegColor::Cmyk => ImageColor::Cmyk,
            };
            Ok(DecodedImage {
                pixels: RgbaImage::from_raw(decoded.width, decoded.height, decoded.rgba)
                    .ok_or_else(|| ImageError::new(0, "invalid decoded JPEG buffer length"))?,
                source_color,
                jpeg_adobe_transform: decoded.adobe_transform,
            })
        }
    }
}

fn image_buffer_len(width: u32, height: u32, channels: usize) -> Option<usize> {
    let length = usize::try_from(width)
        .ok()?
        .checked_mul(usize::try_from(height).ok()?)?
        .checked_mul(channels)?;
    (length <= MAX_IMAGE_BYTES).then_some(length)
}

fn pixel_offset(width: u32, x: u32, y: u32, channels: usize) -> Option<usize> {
    if x >= width {
        return None;
    }
    usize::try_from(y)
        .ok()?
        .checked_mul(usize::try_from(width).ok()?)?
        .checked_add(usize::try_from(x).ok()?)?
        .checked_mul(channels)
}

#[derive(Clone, Copy, Debug)]
struct PngHeader {
    width: u32,
    height: u32,
    bit_depth: u8,
    color_type: u8,
    interlace: u8,
}

impl PngHeader {
    fn channels(self) -> usize {
        match self.color_type {
            0 | 3 => 1,
            2 => 3,
            4 => 2,
            6 => 4,
            _ => unreachable!("validated PNG color type"),
        }
    }

    fn source_color(self) -> ImageColor {
        match self.color_type {
            0 => ImageColor::Gray,
            2 | 3 => ImageColor::Rgb,
            4 => ImageColor::GrayAlpha,
            6 => ImageColor::Rgba,
            _ => unreachable!("validated PNG color type"),
        }
    }
}

fn png_dimensions(data: &[u8]) -> Result<(u32, u32), ImageError> {
    let header = parse_png_header(data)?;
    Ok((header.width, header.height))
}

fn parse_png_header(data: &[u8]) -> Result<PngHeader, ImageError> {
    if !data.starts_with(PNG_SIGNATURE) {
        return Err(ImageError::new(0, "invalid PNG signature"));
    }
    let length = read_be_u32(data, 8)? as usize;
    if length != 13 || data.get(12..16) != Some(b"IHDR".as_slice()) {
        return Err(ImageError::new(
            8,
            "PNG must begin with a 13-byte IHDR chunk",
        ));
    }
    let chunk_end = 16usize
        .checked_add(length)
        .ok_or_else(|| ImageError::new(8, "PNG chunk length overflow"))?;
    let crc_end = chunk_end
        .checked_add(4)
        .ok_or_else(|| ImageError::new(8, "PNG chunk length overflow"))?;
    if crc_end > data.len() {
        return Err(ImageError::new(8, "truncated PNG IHDR chunk"));
    }
    validate_png_crc(data, 12, chunk_end)?;

    let width = read_be_u32(data, 16)?;
    let height = read_be_u32(data, 20)?;
    let bit_depth = data[24];
    let color_type = data[25];
    let compression = data[26];
    let filter = data[27];
    let interlace = data[28];
    if width == 0 || height == 0 {
        return Err(ImageError::new(16, "PNG dimensions must be nonzero"));
    }
    image_buffer_len(width, height, 4)
        .ok_or_else(|| ImageError::new(16, "PNG dimensions exceed the decoded image limit"))?;
    if !valid_png_depth(color_type, bit_depth) {
        return Err(ImageError::new(
            24,
            "invalid PNG bit-depth/color-type combination",
        ));
    }
    if compression != 0 {
        return Err(ImageError::new(26, "unsupported PNG compression method"));
    }
    if filter != 0 {
        return Err(ImageError::new(27, "unsupported PNG filter method"));
    }
    if interlace > 1 {
        return Err(ImageError::new(28, "unsupported PNG interlace method"));
    }
    Ok(PngHeader {
        width,
        height,
        bit_depth,
        color_type,
        interlace,
    })
}

fn valid_png_depth(color_type: u8, bit_depth: u8) -> bool {
    match color_type {
        0 => matches!(bit_depth, 1 | 2 | 4 | 8 | 16),
        2 => matches!(bit_depth, 8 | 16),
        3 => matches!(bit_depth, 1 | 2 | 4 | 8),
        4 | 6 => matches!(bit_depth, 8 | 16),
        _ => false,
    }
}

#[derive(Default)]
struct PngAuxiliary {
    palette: Option<Vec<[u8; 3]>>,
    palette_alpha: Vec<u8>,
    transparent_gray: Option<u16>,
    transparent_rgb: Option<[u16; 3]>,
}

fn decode_png(data: &[u8]) -> Result<DecodedImage, ImageError> {
    let header = parse_png_header(data)?;
    let mut auxiliary = PngAuxiliary::default();
    let mut compressed = Vec::new();
    let mut offset = 8usize;
    let mut saw_ihdr = false;
    let mut saw_idat = false;
    let mut ended_idat = false;
    let mut saw_iend = false;

    while offset < data.len() {
        let length = usize::try_from(read_be_u32(data, offset)?)
            .map_err(|_| ImageError::new(offset, "PNG chunk length exceeds address space"))?;
        let type_offset = offset
            .checked_add(4)
            .ok_or_else(|| ImageError::new(offset, "PNG chunk offset overflow"))?;
        let payload_offset = type_offset
            .checked_add(4)
            .ok_or_else(|| ImageError::new(offset, "PNG chunk offset overflow"))?;
        let payload_end = payload_offset
            .checked_add(length)
            .ok_or_else(|| ImageError::new(offset, "PNG chunk length overflow"))?;
        let chunk_end = payload_end
            .checked_add(4)
            .ok_or_else(|| ImageError::new(offset, "PNG chunk length overflow"))?;
        if chunk_end > data.len() {
            return Err(ImageError::new(offset, "truncated PNG chunk"));
        }
        let chunk_type: [u8; 4] = data[type_offset..payload_offset]
            .try_into()
            .expect("four-byte PNG chunk type");
        if !chunk_type.iter().all(u8::is_ascii_alphabetic) {
            return Err(ImageError::new(type_offset, "invalid PNG chunk type"));
        }
        validate_png_crc(data, type_offset, payload_end)?;
        let payload = &data[payload_offset..payload_end];

        if saw_idat && chunk_type != *b"IDAT" {
            ended_idat = true;
        }
        match &chunk_type {
            b"IHDR" => {
                if saw_ihdr || offset != 8 {
                    return Err(ImageError::new(type_offset, "duplicate or misplaced IHDR"));
                }
                saw_ihdr = true;
            }
            b"PLTE" => {
                if !saw_ihdr || saw_idat || auxiliary.palette.is_some() {
                    return Err(ImageError::new(type_offset, "duplicate or misplaced PLTE"));
                }
                if matches!(header.color_type, 0 | 4) {
                    return Err(ImageError::new(
                        type_offset,
                        "PLTE is prohibited for grayscale PNGs",
                    ));
                }
                if payload.is_empty() || payload.len() % 3 != 0 || payload.len() > 768 {
                    return Err(ImageError::new(type_offset, "invalid PNG palette length"));
                }
                let entries: Vec<[u8; 3]> = payload
                    .chunks_exact(3)
                    .map(|entry| [entry[0], entry[1], entry[2]])
                    .collect();
                if header.color_type == 3 && entries.len() > (1usize << header.bit_depth) {
                    return Err(ImageError::new(
                        type_offset,
                        "PNG palette exceeds indexed bit depth",
                    ));
                }
                auxiliary.palette = Some(entries);
            }
            b"tRNS" => {
                if !saw_ihdr
                    || saw_idat
                    || auxiliary.transparent_gray.is_some()
                    || auxiliary.transparent_rgb.is_some()
                    || !auxiliary.palette_alpha.is_empty()
                {
                    return Err(ImageError::new(type_offset, "duplicate or misplaced tRNS"));
                }
                match header.color_type {
                    0 if payload.len() == 2 => {
                        auxiliary.transparent_gray =
                            Some(u16::from_be_bytes([payload[0], payload[1]]));
                    }
                    2 if payload.len() == 6 => {
                        auxiliary.transparent_rgb = Some([
                            u16::from_be_bytes([payload[0], payload[1]]),
                            u16::from_be_bytes([payload[2], payload[3]]),
                            u16::from_be_bytes([payload[4], payload[5]]),
                        ]);
                    }
                    3 => {
                        let palette_len = auxiliary
                            .palette
                            .as_ref()
                            .ok_or_else(|| {
                                ImageError::new(type_offset, "indexed tRNS precedes PLTE")
                            })?
                            .len();
                        if payload.is_empty() || payload.len() > palette_len {
                            return Err(ImageError::new(
                                type_offset,
                                "invalid indexed tRNS length",
                            ));
                        }
                        auxiliary.palette_alpha.extend_from_slice(payload);
                    }
                    4 | 6 => {
                        return Err(ImageError::new(
                            type_offset,
                            "tRNS is prohibited for PNGs with an alpha channel",
                        ));
                    }
                    _ => return Err(ImageError::new(type_offset, "invalid tRNS length")),
                }
            }
            b"IDAT" => {
                if !saw_ihdr || ended_idat {
                    return Err(ImageError::new(
                        type_offset,
                        "non-consecutive PNG IDAT chunks",
                    ));
                }
                if header.color_type == 3 && auxiliary.palette.is_none() {
                    return Err(ImageError::new(type_offset, "indexed PNG omits PLTE"));
                }
                saw_idat = true;
                compressed
                    .len()
                    .checked_add(payload.len())
                    .filter(|length| *length <= data.len())
                    .ok_or_else(|| ImageError::new(payload_offset, "PNG IDAT length overflow"))?;
                compressed.extend_from_slice(payload);
            }
            b"IEND" => {
                if !saw_ihdr || !saw_idat || !payload.is_empty() {
                    return Err(ImageError::new(type_offset, "invalid or misplaced IEND"));
                }
                saw_iend = true;
                offset = chunk_end;
                break;
            }
            _ => {
                if chunk_type[0] & 0x20 == 0 {
                    return Err(ImageError::new(
                        type_offset,
                        format!(
                            "unsupported critical PNG chunk {}",
                            String::from_utf8_lossy(&chunk_type)
                        ),
                    ));
                }
            }
        }
        offset = chunk_end;
    }

    if !saw_iend {
        return Err(ImageError::new(offset, "PNG omits IEND"));
    }
    if offset != data.len() {
        return Err(ImageError::new(offset, "trailing bytes after PNG IEND"));
    }

    let expected = png_filtered_length(header)?;
    let filtered = zlib_inflate(&compressed, expected)
        .map_err(|error| ImageError::new(0, format!("invalid PNG zlib stream: {error}")))?;
    if filtered.len() != expected {
        return Err(ImageError::new(
            0,
            "PNG decompressed data has an unexpected length",
        ));
    }

    let mut rgba = vec![0u8; image_buffer_len(header.width, header.height, 4).unwrap()];
    if header.interlace == 0 {
        decode_png_pass(
            &filtered,
            header,
            header.width,
            header.height,
            (0, 0, 1, 1),
            &auxiliary,
            &mut rgba,
        )?;
    } else {
        let mut source_offset = 0usize;
        for &(start_x, start_y, step_x, step_y) in &ADAM7_PASSES {
            let pass_width = pass_dimension(header.width, start_x, step_x);
            let pass_height = pass_dimension(header.height, start_y, step_y);
            if pass_width == 0 || pass_height == 0 {
                continue;
            }
            let pass_length = filtered_pass_length(header, pass_width, pass_height)?;
            let pass_end = source_offset
                .checked_add(pass_length)
                .ok_or_else(|| ImageError::new(source_offset, "Adam7 pass length overflow"))?;
            decode_png_pass(
                &filtered[source_offset..pass_end],
                header,
                pass_width,
                pass_height,
                (start_x, start_y, step_x, step_y),
                &auxiliary,
                &mut rgba,
            )?;
            source_offset = pass_end;
        }
        debug_assert_eq!(source_offset, filtered.len());
    }

    Ok(DecodedImage {
        pixels: RgbaImage {
            width: header.width,
            height: header.height,
            data: rgba,
        },
        source_color: header.source_color(),
        jpeg_adobe_transform: None,
    })
}

const ADAM7_PASSES: [(u32, u32, u32, u32); 7] = [
    (0, 0, 8, 8),
    (4, 0, 8, 8),
    (0, 4, 4, 8),
    (2, 0, 4, 4),
    (0, 2, 2, 4),
    (1, 0, 2, 2),
    (0, 1, 1, 2),
];

fn pass_dimension(full: u32, start: u32, step: u32) -> u32 {
    if full <= start {
        0
    } else {
        (full - start).div_ceil(step)
    }
}

fn png_filtered_length(header: PngHeader) -> Result<usize, ImageError> {
    if header.interlace == 0 {
        return filtered_pass_length(header, header.width, header.height);
    }
    let mut total = 0usize;
    for &(start_x, start_y, step_x, step_y) in &ADAM7_PASSES {
        let width = pass_dimension(header.width, start_x, step_x);
        let height = pass_dimension(header.height, start_y, step_y);
        if width == 0 || height == 0 {
            continue;
        }
        total = total
            .checked_add(filtered_pass_length(header, width, height)?)
            .ok_or_else(|| ImageError::new(0, "PNG filtered data length overflow"))?;
    }
    Ok(total)
}

fn filtered_pass_length(header: PngHeader, width: u32, height: u32) -> Result<usize, ImageError> {
    let row = png_row_bytes(header, width)?;
    row.checked_add(1)
        .and_then(|stride| stride.checked_mul(height as usize))
        .filter(|length| *length <= MAX_IMAGE_BYTES)
        .ok_or_else(|| ImageError::new(0, "PNG filtered data exceeds the image limit"))
}

fn png_row_bytes(header: PngHeader, width: u32) -> Result<usize, ImageError> {
    usize::try_from(width)
        .ok()
        .and_then(|width| width.checked_mul(header.channels()))
        .and_then(|samples| samples.checked_mul(usize::from(header.bit_depth)))
        .and_then(|bits| bits.checked_add(7))
        .map(|bits| bits / 8)
        .ok_or_else(|| ImageError::new(0, "PNG scanline length overflow"))
}

#[allow(clippy::too_many_arguments)]
fn decode_png_pass(
    filtered: &[u8],
    header: PngHeader,
    pass_width: u32,
    pass_height: u32,
    placement: (u32, u32, u32, u32),
    auxiliary: &PngAuxiliary,
    rgba: &mut [u8],
) -> Result<(), ImageError> {
    let row_bytes = png_row_bytes(header, pass_width)?;
    let bytes_per_pixel = ((header.channels() * usize::from(header.bit_depth)).div_ceil(8)).max(1);
    let mut previous = vec![0u8; row_bytes];
    let mut current = vec![0u8; row_bytes];
    let mut source_offset = 0usize;
    let (start_x, start_y, step_x, step_y) = placement;

    for pass_y in 0..pass_height {
        let filter = filtered[source_offset];
        source_offset += 1;
        let row_end = source_offset + row_bytes;
        unfilter_png_row(
            filter,
            &filtered[source_offset..row_end],
            &previous,
            bytes_per_pixel,
            &mut current,
            source_offset - 1,
        )?;
        for pass_x in 0..pass_width {
            let pixel = png_pixel(&current, pass_x, header, auxiliary, source_offset)?;
            let x = start_x + pass_x * step_x;
            let y = start_y + pass_y * step_y;
            let destination = pixel_offset(header.width, x, y, 4)
                .ok_or_else(|| ImageError::new(source_offset, "Adam7 pixel is out of bounds"))?;
            rgba[destination..destination + 4].copy_from_slice(&pixel);
        }
        std::mem::swap(&mut current, &mut previous);
        source_offset = row_end;
    }
    Ok(())
}

fn unfilter_png_row(
    filter: u8,
    source: &[u8],
    previous: &[u8],
    bytes_per_pixel: usize,
    destination: &mut [u8],
    offset: usize,
) -> Result<(), ImageError> {
    if source.len() != previous.len() || source.len() != destination.len() {
        return Err(ImageError::new(
            offset,
            "internal PNG scanline length mismatch",
        ));
    }
    for index in 0..source.len() {
        let left = if index >= bytes_per_pixel {
            destination[index - bytes_per_pixel]
        } else {
            0
        };
        let up = previous[index];
        let upper_left = if index >= bytes_per_pixel {
            previous[index - bytes_per_pixel]
        } else {
            0
        };
        let predictor = match filter {
            0 => 0,
            1 => left,
            2 => up,
            3 => ((u16::from(left) + u16::from(up)) / 2) as u8,
            4 => paeth_predictor(left, up, upper_left),
            _ => return Err(ImageError::new(offset, "invalid PNG scanline filter")),
        };
        destination[index] = source[index].wrapping_add(predictor);
    }
    Ok(())
}

fn paeth_predictor(left: u8, up: u8, upper_left: u8) -> u8 {
    let left = i32::from(left);
    let up = i32::from(up);
    let upper_left = i32::from(upper_left);
    let estimate = left + up - upper_left;
    let left_distance = (estimate - left).abs();
    let up_distance = (estimate - up).abs();
    let upper_left_distance = (estimate - upper_left).abs();
    if left_distance <= up_distance && left_distance <= upper_left_distance {
        left as u8
    } else if up_distance <= upper_left_distance {
        up as u8
    } else {
        upper_left as u8
    }
}

fn png_pixel(
    row: &[u8],
    x: u32,
    header: PngHeader,
    auxiliary: &PngAuxiliary,
    offset: usize,
) -> Result<[u8; 4], ImageError> {
    let channels = header.channels();
    let sample_base = usize::try_from(x)
        .ok()
        .and_then(|x| x.checked_mul(channels))
        .ok_or_else(|| ImageError::new(offset, "PNG sample offset overflow"))?;
    let mut sample = [0u16; 4];
    for (channel, value) in sample.iter_mut().enumerate().take(channels) {
        *value = png_sample(row, sample_base + channel, header.bit_depth, offset)?;
    }
    let scale = |value: u16| scale_png_sample(value, header.bit_depth);
    Ok(match header.color_type {
        0 => {
            let gray = scale(sample[0]);
            let alpha = if auxiliary.transparent_gray == Some(sample[0]) {
                0
            } else {
                255
            };
            [gray, gray, gray, alpha]
        }
        2 => {
            let alpha = if auxiliary.transparent_rgb == Some([sample[0], sample[1], sample[2]]) {
                0
            } else {
                255
            };
            [scale(sample[0]), scale(sample[1]), scale(sample[2]), alpha]
        }
        3 => {
            let index = usize::from(sample[0]);
            let color = auxiliary
                .palette
                .as_ref()
                .and_then(|palette| palette.get(index))
                .ok_or_else(|| ImageError::new(offset, "PNG palette index is out of range"))?;
            let alpha = auxiliary.palette_alpha.get(index).copied().unwrap_or(255);
            [color[0], color[1], color[2], alpha]
        }
        4 => {
            let gray = scale(sample[0]);
            [gray, gray, gray, scale(sample[1])]
        }
        6 => [
            scale(sample[0]),
            scale(sample[1]),
            scale(sample[2]),
            scale(sample[3]),
        ],
        _ => unreachable!("validated PNG color type"),
    })
}

fn png_sample(row: &[u8], index: usize, bit_depth: u8, offset: usize) -> Result<u16, ImageError> {
    match bit_depth {
        1 | 2 | 4 => {
            let depth = usize::from(bit_depth);
            let bit = index
                .checked_mul(depth)
                .ok_or_else(|| ImageError::new(offset, "PNG packed sample offset overflow"))?;
            let byte = *row
                .get(bit / 8)
                .ok_or_else(|| ImageError::new(offset, "truncated PNG packed sample"))?;
            let shift = 8 - depth - (bit % 8);
            Ok(u16::from((byte >> shift) & ((1u8 << depth) - 1)))
        }
        8 => row
            .get(index)
            .copied()
            .map(u16::from)
            .ok_or_else(|| ImageError::new(offset, "truncated PNG sample")),
        16 => {
            let byte = index
                .checked_mul(2)
                .ok_or_else(|| ImageError::new(offset, "PNG sample offset overflow"))?;
            let sample = row
                .get(byte..byte + 2)
                .ok_or_else(|| ImageError::new(offset, "truncated 16-bit PNG sample"))?;
            Ok(u16::from_be_bytes([sample[0], sample[1]]))
        }
        _ => unreachable!("validated PNG bit depth"),
    }
}

fn scale_png_sample(value: u16, bit_depth: u8) -> u8 {
    if bit_depth == 8 {
        return value as u8;
    }
    if bit_depth == 16 {
        return ((u32::from(value) * 255 + 32_767) / 65_535) as u8;
    }
    let maximum = (1u32 << bit_depth) - 1;
    ((u32::from(value) * 255 + maximum / 2) / maximum) as u8
}

pub(crate) fn encode_png_rgba8(
    data: &[u8],
    width: u32,
    height: u32,
) -> Result<Vec<u8>, ImageError> {
    let expected = image_buffer_len(width, height, 4)
        .ok_or_else(|| ImageError::new(0, "PNG dimensions exceed the image limit"))?;
    if width == 0 || height == 0 {
        return Err(ImageError::new(0, "PNG dimensions must be nonzero"));
    }
    if data.len() != expected {
        return Err(ImageError::new(
            0,
            "RGBA buffer length does not match dimensions",
        ));
    }
    let row_bytes = usize::try_from(width)
        .ok()
        .and_then(|width| width.checked_mul(4))
        .ok_or_else(|| ImageError::new(0, "PNG row length overflow"))?;
    let filtered_capacity = row_bytes
        .checked_add(1)
        .and_then(|stride| stride.checked_mul(height as usize))
        .ok_or_else(|| ImageError::new(0, "PNG filtered data length overflow"))?;
    let mut filtered = Vec::with_capacity(filtered_capacity);
    let mut candidates = [Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new()];
    let zero_row = vec![0u8; row_bytes];
    for y in 0..height as usize {
        let row = &data[y * row_bytes..(y + 1) * row_bytes];
        let previous = if y == 0 {
            zero_row.as_slice()
        } else {
            &data[(y - 1) * row_bytes..y * row_bytes]
        };
        let mut best_filter = 0usize;
        let mut best_score = u64::MAX;
        for (filter, candidate) in candidates.iter_mut().enumerate() {
            candidate.clear();
            candidate.reserve(row_bytes);
            let mut score = 0u64;
            for index in 0..row_bytes {
                let left = if index >= 4 { row[index - 4] } else { 0 };
                let up = previous[index];
                let upper_left = if index >= 4 { previous[index - 4] } else { 0 };
                let predictor = match filter {
                    0 => 0,
                    1 => left,
                    2 => up,
                    3 => ((u16::from(left) + u16::from(up)) / 2) as u8,
                    4 => paeth_predictor(left, up, upper_left),
                    _ => unreachable!(),
                };
                let value = row[index].wrapping_sub(predictor);
                candidate.push(value);
                score += u64::from(value.min(value.wrapping_neg()));
            }
            if score < best_score {
                best_filter = filter;
                best_score = score;
            }
        }
        filtered.push(best_filter as u8);
        filtered.extend_from_slice(&candidates[best_filter]);
    }

    let compressed = zlib_deflate_parallel(&filtered);
    let mut png = Vec::with_capacity(compressed.len().saturating_add(57));
    png.extend_from_slice(PNG_SIGNATURE);
    let mut ihdr = [0u8; 13];
    ihdr[0..4].copy_from_slice(&width.to_be_bytes());
    ihdr[4..8].copy_from_slice(&height.to_be_bytes());
    ihdr[8] = 8;
    ihdr[9] = 6;
    write_png_chunk(&mut png, *b"IHDR", &ihdr)?;
    for chunk in compressed.chunks(1 << 20) {
        write_png_chunk(&mut png, *b"IDAT", chunk)?;
    }
    write_png_chunk(&mut png, *b"IEND", &[])?;
    Ok(png)
}

pub(crate) fn encode_png_premultiplied_rgba8(
    data: &[u8],
    width: u32,
    height: u32,
) -> Result<Vec<u8>, ImageError> {
    let expected = image_buffer_len(width, height, 4)
        .ok_or_else(|| ImageError::new(0, "PNG dimensions exceed the image limit"))?;
    if data.len() != expected {
        return Err(ImageError::new(
            0,
            "RGBA buffer length does not match dimensions",
        ));
    }
    let mut straight = data.to_vec();
    for pixel in straight.chunks_exact_mut(4) {
        let alpha = pixel[3];
        if alpha == 0 {
            pixel[0..3].fill(0);
        } else if alpha != 255 {
            for channel in &mut pixel[0..3] {
                *channel = ((u32::from(*channel) * 255 + u32::from(alpha) / 2) / u32::from(alpha))
                    .min(255) as u8;
            }
        }
    }
    encode_png_rgba8(&straight, width, height)
}

pub(crate) fn blur_rgba(image: &RgbaImage, sigma: f32) -> RgbaImage {
    blur_rgba_with_support(image, sigma, 2.0)
}

/// Chrome's raster PDF path lowers SVG Gaussian blur through Skia's
/// three-pass PlanGauss approximation. Keep the entry point explicit so the
/// filter VM can select the linear-time shader without changing legacy
/// callers of the compact scalar convolution.
pub(crate) fn blur_rgba_svg_filter(image: &RgbaImage, sigma: f32) -> RgbaImage {
    if !sigma.is_finite() || sigma <= 0.0 || image.width == 0 || image.height == 0 {
        return image.clone();
    }

    // A filter surface has transparent pixels beyond its current raster
    // bounds. Running each PlanGauss pass directly against a cropped buffer
    // incorrectly discards intermediate samples that leave the surface and
    // would feed back into later passes. Pad by the complete three-pass support,
    // execute the linear kernel once, then crop back to the virtual surface.
    // This keeps edge behavior equivalent to an unbounded transparent shader
    // without making the retained PDF image any larger.
    const SKIA_WINDOW_FACTOR: f64 = 1.879_971_205_973_250_3;
    let window = ((f64::from(sigma) * SKIA_WINDOW_FACTOR + 0.5).floor() as usize).max(1);
    if window <= 1 {
        return image.clone();
    }
    let padding = 3usize.saturating_mul((window + 1) / 2);
    let Some(padded_width) = (image.width as usize).checked_add(padding.saturating_mul(2)) else {
        return image.clone();
    };
    let Some(padded_height) = (image.height as usize).checked_add(padding.saturating_mul(2)) else {
        return image.clone();
    };
    let Ok(padded_width_u32) = u32::try_from(padded_width) else {
        return image.clone();
    };
    let Ok(padded_height_u32) = u32::try_from(padded_height) else {
        return image.clone();
    };
    let Some(padded_len) = image_buffer_len(padded_width_u32, padded_height_u32, 4) else {
        return image.clone();
    };
    let mut padded = vec![0u8; padded_len];
    let source_stride = image.width as usize * 4;
    let padded_stride = padded_width * 4;
    for row in 0..image.height as usize {
        let source_start = row * source_stride;
        let target_start = (row + padding) * padded_stride + padding * 4;
        padded[target_start..target_start + source_stride]
            .copy_from_slice(&image.data[source_start..source_start + source_stride]);
    }
    let padded = RgbaImage {
        width: padded_width_u32,
        height: padded_height_u32,
        data: padded,
    };
    let blurred = blur_rgba_plan_gauss(&padded, sigma);
    let mut data = vec![0u8; image.data.len()];
    for row in 0..image.height as usize {
        let source_start = (row + padding) * padded_stride + padding * 4;
        let target_start = row * source_stride;
        data[target_start..target_start + source_stride]
            .copy_from_slice(&blurred.data[source_start..source_start + source_stride]);
    }
    RgbaImage {
        width: image.width,
        height: image.height,
        data,
    }
}

fn blur_rgba_with_support(image: &RgbaImage, sigma: f32, support: f32) -> RgbaImage {
    if !sigma.is_finite() || sigma <= 0.0 || image.width == 0 || image.height == 0 {
        return image.clone();
    }
    let radius = (support * sigma).ceil().max(1.0) as usize;
    let denominator = 2.0 * f64::from(sigma) * f64::from(sigma);
    let mut kernel = Vec::with_capacity(radius * 2 + 1);
    let mut kernel_sum = 0.0f64;
    for offset in -(radius as isize)..=(radius as isize) {
        let distance = offset as f64;
        let weight = (-(distance * distance) / denominator).exp();
        kernel.push(weight);
        kernel_sum += weight;
    }
    for weight in &mut kernel {
        *weight /= kernel_sum;
    }

    let width = image.width as usize;
    let height = image.height as usize;
    let mut horizontal = vec![0f64; image.data.len()];
    for y in 0..height {
        for x in 0..width {
            let destination = (y * width + x) * 4;
            for (kernel_index, &weight) in kernel.iter().enumerate() {
                let offset = kernel_index as isize - radius as isize;
                let source_x = (x as isize + offset).clamp(0, width as isize - 1) as usize;
                let source = (y * width + source_x) * 4;
                for channel in 0..4 {
                    horizontal[destination + channel] +=
                        f64::from(image.data[source + channel]) * weight;
                }
            }
        }
    }

    let mut output = vec![0u8; image.data.len()];
    for y in 0..height {
        for x in 0..width {
            let destination = (y * width + x) * 4;
            let mut sum = [0.0f64; 4];
            for (kernel_index, &weight) in kernel.iter().enumerate() {
                let offset = kernel_index as isize - radius as isize;
                let source_y = (y as isize + offset).clamp(0, height as isize - 1) as usize;
                let source = (source_y * width + x) * 4;
                for channel in 0..4 {
                    sum[channel] += horizontal[source + channel] * weight;
                }
            }
            for channel in 0..4 {
                output[destination + channel] = sum[channel].round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    RgbaImage {
        width: image.width,
        height: image.height,
        data: output,
    }
}

#[derive(Clone, Copy)]
struct CssShadowPlanGauss {
    pass_sizes: [usize; 3],
    border: usize,
    sliding_window: usize,
    weight: u64,
}

impl CssShadowPlanGauss {
    fn new(sigma: f32) -> Option<Self> {
        // SkMaskBlurFilter::PlanGauss:
        // floor(sigma * 3 * sqrt(2*pi) / 4 + 0.5).
        const SKIA_WINDOW_FACTOR: f64 = 1.879_971_205_973_250_3;
        let window = ((f64::from(sigma) * SKIA_WINDOW_FACTOR + 0.5).floor() as usize).max(1);
        if window <= 1 {
            return None;
        }

        let pass_sizes = [
            window - 1,
            window - 1,
            if window & 1 == 1 { window - 1 } else { window },
        ];
        let border = if window & 1 == 1 {
            3 * ((window - 1) / 2)
        } else {
            3 * (window / 2) - 1
        };
        let window_squared = (window as u64) * (window as u64);
        let window_cubed = window_squared * (window as u64);
        let divisor = if window & 1 == 1 {
            window_cubed
        } else {
            window_cubed + window_squared
        };
        let weight = ((1u64 << 32) + divisor / 2) / divisor;
        Some(Self {
            pass_sizes,
            border,
            sliding_window: border * 2 + 1,
            weight,
        })
    }

    fn buffer_size(self) -> usize {
        self.pass_sizes.into_iter().sum()
    }
}

fn css_shadow_plan_gauss_step(
    leading_edge: u8,
    plan: CssShadowPlanGauss,
    sums: &mut [u32; 3],
    buffers: &mut [u32],
    starts: [usize; 3],
    ends: [usize; 3],
    cursors: &mut [usize; 3],
) -> u8 {
    sums[0] += u32::from(leading_edge);
    sums[1] += sums[0];
    sums[2] += sums[1];
    let output = ((plan.weight * u64::from(sums[2]) + (1u64 << 31)) >> 32) as u8;

    let cursor = cursors[2];
    sums[2] -= buffers[cursor];
    buffers[cursor] = sums[1];
    cursors[2] = if cursor + 1 < ends[2] {
        cursor + 1
    } else {
        starts[2]
    };

    let cursor = cursors[1];
    sums[1] -= buffers[cursor];
    buffers[cursor] = sums[0];
    cursors[1] = if cursor + 1 < ends[1] {
        cursor + 1
    } else {
        starts[1]
    };

    let cursor = cursors[0];
    sums[0] -= buffers[cursor];
    buffers[cursor] = u32::from(leading_edge);
    cursors[0] = if cursor + 1 < ends[0] {
        cursor + 1
    } else {
        starts[0]
    };

    output
}

fn css_shadow_plan_gauss_scan(
    source: &[u8],
    plan: CssShadowPlanGauss,
    destination: &mut Vec<u8>,
    buffers: &mut [u32],
) {
    let destination_length = source.len() + plan.border * 2;
    destination.resize(destination_length, 0);
    buffers.fill(0);
    let starts = [
        0,
        plan.pass_sizes[0],
        plan.pass_sizes[0] + plan.pass_sizes[1],
    ];
    let ends = [
        plan.pass_sizes[0],
        plan.pass_sizes[0] + plan.pass_sizes[1],
        plan.buffer_size(),
    ];
    let mut cursors = starts;
    let mut sums = [0u32; 3];
    let mut destination_index = 0usize;

    for &leading_edge in source {
        destination[destination_index] = css_shadow_plan_gauss_step(
            leading_edge,
            plan,
            &mut sums,
            buffers,
            starts,
            ends,
            &mut cursors,
        );
        destination_index += 1;
    }
    for _ in 0..plan.sliding_window.saturating_sub(source.len()) {
        destination[destination_index] =
            css_shadow_plan_gauss_step(0, plan, &mut sums, buffers, starts, ends, &mut cursors);
        destination_index += 1;
    }

    // The forward scan has emitted the left side of the expanded mask. Scan
    // backward from the other edge to fill the remaining pixels without ever
    // materializing three intermediate box-filter surfaces.
    buffers.fill(0);
    cursors = starts;
    sums = [0u32; 3];
    let mut source_index = source.len();
    let mut reverse_destination = destination_length;
    while reverse_destination > destination_index {
        reverse_destination -= 1;
        source_index -= 1;
        destination[reverse_destination] = css_shadow_plan_gauss_step(
            source[source_index],
            plan,
            &mut sums,
            buffers,
            starts,
            ends,
            &mut cursors,
        );
    }
}

/// Execute Skia's three-box PlanGauss approximation independently over all
/// premultiplied RGBA channels. Unlike a shadow mask, a CSS/SVG filter source
/// can contain unrelated colours and descendants, so reducing it to one alpha
/// plane plus a representative colour would destroy the source graphic.
fn blur_rgba_plan_gauss(image: &RgbaImage, sigma: f32) -> RgbaImage {
    if !sigma.is_finite() || sigma <= 0.0 || image.width == 0 || image.height == 0 {
        return image.clone();
    }
    let Some(plan) = CssShadowPlanGauss::new(sigma) else {
        return image.clone();
    };

    let width = image.width as usize;
    let height = image.height as usize;
    let mut horizontal = vec![0u8; image.data.len()];
    let mut output = vec![0u8; image.data.len()];
    let mut line = vec![0u8; width.max(height)];
    let mut expanded = Vec::with_capacity(width.max(height) + plan.border * 2);
    let mut buffers = vec![0u32; plan.buffer_size()];

    for channel in 0..4 {
        for y in 0..height {
            for (x, sample) in line[..width].iter_mut().enumerate() {
                *sample = image.data[(y * width + x) * 4 + channel];
            }
            css_shadow_plan_gauss_scan(&line[..width], plan, &mut expanded, &mut buffers);
            for x in 0..width {
                horizontal[(y * width + x) * 4 + channel] = expanded[plan.border + x];
            }
        }

        for x in 0..width {
            for (y, sample) in line[..height].iter_mut().enumerate() {
                *sample = horizontal[(y * width + x) * 4 + channel];
            }
            css_shadow_plan_gauss_scan(&line[..height], plan, &mut expanded, &mut buffers);
            for y in 0..height {
                output[(y * width + x) * 4 + channel] = expanded[plan.border + y];
            }
        }
    }

    RgbaImage {
        width: image.width,
        height: image.height,
        data: output,
    }
}

/// Chromium's CSS shadow masks use Skia's three-pass box approximation rather
/// than the image-filter Gaussian used by `filter: blur()`. Compile the solid
/// shadow paint to one A8 mask and execute Skia's three cascaded windows as a
/// single fixed-point scan per axis. Besides matching its axis quantization,
/// this avoids six full-size floating-point RGBA buffers.
pub(crate) fn blur_rgba_css_shadow(image: &RgbaImage, sigma: f32) -> RgbaImage {
    if !sigma.is_finite() || sigma <= 0.0 || image.width == 0 || image.height == 0 {
        return image.clone();
    }
    let Some(plan) = CssShadowPlanGauss::new(sigma) else {
        return image.clone();
    };

    let width = image.width as usize;
    let height = image.height as usize;
    let mut alpha = Vec::with_capacity(width * height);
    let mut strongest = [0u8; 4];
    for pixel in image.data.chunks_exact(4) {
        alpha.push(pixel[3]);
        if pixel[3] > strongest[3] {
            strongest.copy_from_slice(pixel);
        }
    }
    if strongest[3] == 0 {
        return image.clone();
    }
    let straight_color = [
        ((u32::from(strongest[0]) * 255 + u32::from(strongest[3]) / 2) / u32::from(strongest[3]))
            .min(255) as u8,
        ((u32::from(strongest[1]) * 255 + u32::from(strongest[3]) / 2) / u32::from(strongest[3]))
            .min(255) as u8,
        ((u32::from(strongest[2]) * 255 + u32::from(strongest[3]) / 2) / u32::from(strongest[3]))
            .min(255) as u8,
    ];

    let mut horizontal = vec![0u8; width * height];
    let mut expanded = Vec::with_capacity(width.max(height) + plan.border * 2);
    let mut buffers = vec![0u32; plan.buffer_size()];
    for y in 0..height {
        let row_start = y * width;
        css_shadow_plan_gauss_scan(
            &alpha[row_start..row_start + width],
            plan,
            &mut expanded,
            &mut buffers,
        );
        horizontal[row_start..row_start + width]
            .copy_from_slice(&expanded[plan.border..plan.border + width]);
    }

    let mut blurred_alpha = vec![0u8; width * height];
    let mut column = vec![0u8; height];
    for x in 0..width {
        for y in 0..height {
            column[y] = horizontal[y * width + x];
        }
        css_shadow_plan_gauss_scan(&column, plan, &mut expanded, &mut buffers);
        for y in 0..height {
            blurred_alpha[y * width + x] = expanded[plan.border + y];
        }
    }

    let mut data = Vec::with_capacity(image.data.len());
    for alpha in blurred_alpha {
        data.push(((u16::from(straight_color[0]) * u16::from(alpha) + 127) / 255) as u8);
        data.push(((u16::from(straight_color[1]) * u16::from(alpha) + 127) / 255) as u8);
        data.push(((u16::from(straight_color[2]) * u16::from(alpha) + 127) / 255) as u8);
        data.push(alpha);
    }
    RgbaImage {
        width: image.width,
        height: image.height,
        data,
    }
}

#[cfg(any(feature = "python", test))]
pub(crate) fn resize_triangle_gray(
    image: &GrayImage,
    destination_width: u32,
    destination_height: u32,
) -> GrayImage {
    if destination_width == 0 || destination_height == 0 || image.width == 0 || image.height == 0 {
        return GrayImage {
            width: destination_width,
            height: destination_height,
            data: Vec::new(),
        };
    }
    if image.dimensions() == (destination_width, destination_height) {
        return image.clone();
    }

    let horizontal = triangle_contributions(image.width, destination_width);
    let vertical = triangle_contributions(image.height, destination_height);
    let mut intermediate = vec![0f64; destination_width as usize * image.height as usize];
    for y in 0..image.height as usize {
        for (destination_x, contributors) in horizontal.iter().enumerate() {
            let mut sum = 0.0;
            for &(source_x, weight) in contributors {
                sum += f64::from(image.data[y * image.width as usize + source_x]) * weight;
            }
            intermediate[y * destination_width as usize + destination_x] = sum;
        }
    }

    let mut output = vec![0u8; destination_width as usize * destination_height as usize];
    for (destination_y, contributors) in vertical.iter().enumerate() {
        for x in 0..destination_width as usize {
            let mut sum = 0.0;
            for &(source_y, weight) in contributors {
                sum += intermediate[source_y * destination_width as usize + x] * weight;
            }
            output[destination_y * destination_width as usize + x] =
                sum.round().clamp(0.0, 255.0) as u8;
        }
    }
    GrayImage {
        width: destination_width,
        height: destination_height,
        data: output,
    }
}

#[cfg(any(feature = "python", test))]
fn triangle_contributions(source: u32, destination: u32) -> Vec<Vec<(usize, f64)>> {
    let scale = f64::from(source) / f64::from(destination);
    let filter_scale = scale.max(1.0);
    let support = filter_scale;
    let mut output = Vec::with_capacity(destination as usize);
    for destination_index in 0..destination {
        let center = (f64::from(destination_index) + 0.5) * scale - 0.5;
        let first = (center - support).ceil() as i64;
        let last = (center + support).floor() as i64;
        let mut combined: Vec<(usize, f64)> = Vec::new();
        let mut sum = 0.0;
        for source_index in first..=last {
            let distance = (center - source_index as f64).abs() / filter_scale;
            let weight = (1.0 - distance).max(0.0);
            if weight == 0.0 {
                continue;
            }
            let clamped = source_index.clamp(0, i64::from(source) - 1) as usize;
            if let Some((_, accumulated)) =
                combined.last_mut().filter(|(index, _)| *index == clamped)
            {
                *accumulated += weight;
            } else {
                combined.push((clamped, weight));
            }
            sum += weight;
        }
        if sum == 0.0 {
            combined.push((
                center.round().clamp(0.0, f64::from(source - 1)) as usize,
                1.0,
            ));
        } else {
            for (_, weight) in &mut combined {
                *weight /= sum;
            }
        }
        output.push(combined);
    }
    output
}

fn write_png_chunk(
    output: &mut Vec<u8>,
    chunk_type: [u8; 4],
    payload: &[u8],
) -> Result<(), ImageError> {
    let length = u32::try_from(payload.len())
        .map_err(|_| ImageError::new(output.len(), "PNG chunk exceeds 4 GiB"))?;
    output.extend_from_slice(&length.to_be_bytes());
    output.extend_from_slice(&chunk_type);
    output.extend_from_slice(payload);
    let crc_start = output.len() - payload.len() - 4;
    let checksum = crc32(&output[crc_start..]);
    output.extend_from_slice(&checksum.to_be_bytes());
    Ok(())
}

fn validate_png_crc(data: &[u8], type_offset: usize, payload_end: usize) -> Result<(), ImageError> {
    let expected = read_be_u32(data, payload_end)?;
    let actual = crc32(&data[type_offset..payload_end]);
    if actual != expected {
        return Err(ImageError::new(payload_end, "PNG chunk CRC mismatch"));
    }
    Ok(())
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & (0u32.wrapping_sub(crc & 1)));
        }
    }
    !crc
}

fn read_be_u32(data: &[u8], offset: usize) -> Result<u32, ImageError> {
    let bytes = data
        .get(offset..offset.saturating_add(4))
        .ok_or_else(|| ImageError::new(offset, "unexpected end of image data"))?;
    Ok(u32::from_be_bytes(
        bytes.try_into().expect("four-byte integer"),
    ))
}

fn jpeg_dimensions(data: &[u8]) -> Result<(u32, u32), ImageError> {
    if !data.starts_with(&[0xff, 0xd8]) {
        return Err(ImageError::new(0, "invalid JPEG start marker"));
    }
    let mut offset = 2usize;
    while offset < data.len() {
        if data[offset] != 0xff {
            return Err(ImageError::new(offset, "expected JPEG marker"));
        }
        while data.get(offset) == Some(&0xff) {
            offset += 1;
        }
        let marker = *data
            .get(offset)
            .ok_or_else(|| ImageError::new(offset, "truncated JPEG marker"))?;
        offset += 1;
        if marker == 0xd9 || marker == 0xda {
            break;
        }
        if marker == 0x01 || (0xd0..=0xd7).contains(&marker) {
            continue;
        }
        let length_bytes = data
            .get(offset..offset.saturating_add(2))
            .ok_or_else(|| ImageError::new(offset, "truncated JPEG segment length"))?;
        let length = usize::from(u16::from_be_bytes([length_bytes[0], length_bytes[1]]));
        if length < 2 {
            return Err(ImageError::new(offset, "invalid JPEG segment length"));
        }
        let segment_end = offset
            .checked_add(length)
            .ok_or_else(|| ImageError::new(offset, "JPEG segment length overflow"))?;
        if segment_end > data.len() {
            return Err(ImageError::new(offset, "truncated JPEG segment"));
        }
        if is_jpeg_frame_marker(marker) {
            let segment = &data[offset + 2..segment_end];
            if segment.len() < 6 {
                return Err(ImageError::new(offset, "truncated JPEG frame header"));
            }
            let height = u32::from(u16::from_be_bytes([segment[1], segment[2]]));
            let width = u32::from(u16::from_be_bytes([segment[3], segment[4]]));
            if width == 0 || height == 0 {
                return Err(ImageError::new(offset, "JPEG dimensions must be nonzero"));
            }
            image_buffer_len(width, height, 4).ok_or_else(|| {
                ImageError::new(offset, "JPEG dimensions exceed the decoded image limit")
            })?;
            return Ok((width, height));
        }
        offset = segment_end;
    }
    Err(ImageError::new(
        offset,
        "JPEG omits a supported frame header",
    ))
}

fn is_jpeg_frame_marker(marker: u8) -> bool {
    matches!(
        marker,
        0xc0 | 0xc1 | 0xc2 | 0xc3 | 0xc5 | 0xc6 | 0xc7 | 0xc9 | 0xca | 0xcb | 0xcd | 0xce | 0xcf
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_png(
        width: u32,
        height: u32,
        depth: u8,
        color_type: u8,
        interlace: u8,
        palette: Option<&[u8]>,
        transparency: Option<&[u8]>,
        filtered: &[u8],
    ) -> Vec<u8> {
        let mut png = PNG_SIGNATURE.to_vec();
        let mut ihdr = [0u8; 13];
        ihdr[0..4].copy_from_slice(&width.to_be_bytes());
        ihdr[4..8].copy_from_slice(&height.to_be_bytes());
        ihdr[8] = depth;
        ihdr[9] = color_type;
        ihdr[12] = interlace;
        write_png_chunk(&mut png, *b"IHDR", &ihdr).unwrap();
        if let Some(palette) = palette {
            write_png_chunk(&mut png, *b"PLTE", palette).unwrap();
        }
        if let Some(transparency) = transparency {
            write_png_chunk(&mut png, *b"tRNS", transparency).unwrap();
        }
        let compressed = zlib_deflate_parallel(filtered);
        write_png_chunk(&mut png, *b"IDAT", &compressed).unwrap();
        write_png_chunk(&mut png, *b"IEND", &[]).unwrap();
        png
    }

    #[test]
    fn png_rgba_encoder_roundtrips_all_filters() {
        let width = 19;
        let height = 11;
        let mut rgba = Vec::new();
        for y in 0..height {
            for x in 0..width {
                rgba.extend_from_slice(&[
                    (x * 11 + y * 3) as u8,
                    (x * 2 + y * 17) as u8,
                    (x * 7 ^ y * 13) as u8,
                    (255 - x * 5 - y) as u8,
                ]);
            }
        }
        let encoded = encode_png_rgba8(&rgba, width, height).expect("encode PNG");
        assert_eq!(dimensions(&encoded).unwrap(), (width, height));
        let decoded = load_from_memory(&encoded).expect("decode PNG");
        assert_eq!(decoded.color(), ImageColor::Rgba);
        assert_eq!(decoded.to_rgba8().as_bytes(), rgba);
    }

    #[test]
    fn png_decodes_packed_palette_and_transparency() {
        let png = make_png(
            4,
            1,
            2,
            3,
            0,
            Some(&[255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255]),
            Some(&[0, 85, 170]),
            &[0, 0b00_01_10_11],
        );
        let pixels = load_from_memory(&png).unwrap().into_rgba8();
        assert_eq!(
            pixels.as_bytes(),
            &[
                255, 0, 0, 0, 0, 255, 0, 85, 0, 0, 255, 170, 255, 255, 255, 255,
            ]
        );
    }

    #[test]
    fn png_decodes_low_depth_gray_and_sixteen_bit_rgba() {
        let gray = make_png(4, 1, 2, 0, 0, None, Some(&[0, 2]), &[0, 0b00_01_10_11]);
        assert_eq!(
            load_from_memory(&gray).unwrap().into_rgba8().as_bytes(),
            &[
                0, 0, 0, 255, 85, 85, 85, 255, 170, 170, 170, 0, 255, 255, 255, 255
            ]
        );

        let rgba16 = make_png(
            1,
            1,
            16,
            6,
            0,
            None,
            None,
            &[0, 0x12, 0x34, 0xab, 0xcd, 0xff, 0xff, 0x80, 0x00],
        );
        assert_eq!(
            load_from_memory(&rgba16).unwrap().into_rgba8().as_bytes(),
            &[0x12, 0xab, 0xff, 0x80]
        );
    }

    #[test]
    fn png_unfilters_all_five_filter_types() {
        let expected = [10u8, 20, 30, 40, 50, 60, 70, 80];
        for filter in 0..=4 {
            let mut filtered = vec![filter];
            for (index, &value) in expected.iter().enumerate() {
                let left = if index >= 4 { expected[index - 4] } else { 0 };
                let predictor = match filter {
                    0 | 2 => 0,
                    1 => left,
                    3 => left / 2,
                    4 => left,
                    _ => unreachable!(),
                };
                filtered.push(value.wrapping_sub(predictor));
            }
            let png = make_png(2, 1, 8, 6, 0, None, None, &filtered);
            assert_eq!(
                load_from_memory(&png).unwrap().into_rgba8().as_bytes(),
                expected
            );
        }
    }

    #[test]
    fn png_decodes_adam7_rgba() {
        let width = 5u32;
        let height = 5u32;
        let pixel = |x: u32, y: u32| [x as u8, y as u8, (x + y) as u8, 255];
        let mut filtered = Vec::new();
        for &(start_x, start_y, step_x, step_y) in &ADAM7_PASSES {
            let pass_width = pass_dimension(width, start_x, step_x);
            let pass_height = pass_dimension(height, start_y, step_y);
            for py in 0..pass_height {
                filtered.push(0);
                for px in 0..pass_width {
                    filtered
                        .extend_from_slice(&pixel(start_x + px * step_x, start_y + py * step_y));
                }
            }
        }
        let png = make_png(width, height, 8, 6, 1, None, None, &filtered);
        let decoded = load_from_memory(&png).unwrap().into_rgba8();
        for y in 0..height {
            for x in 0..width {
                assert_eq!(decoded.get_pixel(x, y).0, pixel(x, y));
            }
        }
    }

    #[test]
    fn png_rejects_crc_trailing_data_and_decompression_bombs() {
        let mut crc = encode_png_rgba8(&[1, 2, 3, 4], 1, 1).unwrap();
        crc[29] ^= 1;
        assert!(
            load_from_memory(&crc)
                .unwrap_err()
                .to_string()
                .contains("CRC")
        );

        let mut trailing = encode_png_rgba8(&[1, 2, 3, 4], 1, 1).unwrap();
        trailing.push(0);
        assert!(
            load_from_memory(&trailing)
                .unwrap_err()
                .to_string()
                .contains("trailing")
        );

        let bomb = make_png(1, 1, 8, 6, 0, None, None, &[0; 100]);
        assert!(
            load_from_memory(&bomb)
                .unwrap_err()
                .to_string()
                .contains("configured limit")
        );
    }

    #[test]
    fn premultiplied_png_encoding_restores_straight_color() {
        let png = encode_png_premultiplied_rgba8(&[50, 25, 0, 128], 1, 1).unwrap();
        assert_eq!(
            load_from_memory(&png).unwrap().into_rgba8().as_bytes(),
            &[100, 50, 0, 128]
        );
    }

    #[test]
    fn gaussian_blur_preserves_constants_and_impulse_symmetry() {
        let constant = RgbaImage::from_raw(7, 5, [20, 40, 60, 80].repeat(35)).unwrap();
        assert_eq!(blur_rgba(&constant, 2.0), constant);

        let mut impulse = RgbaImage::new(7, 1);
        impulse.put_pixel(3, 0, Rgba([255, 255, 255, 255]));
        let blurred = blur_rgba(&impulse, 1.0);
        assert_eq!(blurred.get_pixel(0, 0), blurred.get_pixel(6, 0));
        assert_eq!(blurred.get_pixel(1, 0), blurred.get_pixel(5, 0));
        assert_eq!(blurred.get_pixel(2, 0), blurred.get_pixel(4, 0));
        assert!(blurred.get_pixel(3, 0)[0] > blurred.get_pixel(2, 0)[0]);
    }

    #[test]
    fn svg_gaussian_blur_uses_skia_plan_gauss_profile() {
        let mut step = RgbaImage::new(161, 161);
        for y in 0..161 {
            for x in 80..161 {
                step.put_pixel(x, y, Rgba([255, 255, 255, 255]));
            }
        }
        let svg = blur_rgba_svg_filter(&step, 18.75);

        assert_eq!(svg.get_pixel(40, 80)[3], 2);
        assert_eq!(svg.get_pixel(50, 80)[3], 12);
        assert_eq!(svg.get_pixel(80, 80)[3], 130);
        assert_eq!(svg.get_pixel(120, 80)[3], 252);
    }

    #[test]
    fn css_shadow_box_blur_is_centered_and_conserves_an_impulse() {
        let mut impulse = RgbaImage::new(41, 41);
        impulse.put_pixel(20, 20, Rgba([128, 64, 32, 255]));
        let blurred = blur_rgba_css_shadow(&impulse, 2.0);

        assert_eq!(blurred.get_pixel(17, 20), blurred.get_pixel(23, 20));
        assert_eq!(blurred.get_pixel(20, 17), blurred.get_pixel(20, 23));
        let alpha_sum: u32 = blurred.pixels().map(|pixel| u32::from(pixel[3])).sum();
        // Skia rounds once after each separable axis. Its fixed-point window
        // scale therefore retains 248/255 of this small impulse's alpha.
        assert_eq!(alpha_sum, 248);
        assert!(
            blurred.pixels().all(|pixel| {
                pixel[0] <= pixel[3] && pixel[1] <= pixel[3] && pixel[2] <= pixel[3]
            })
        );
    }

    #[test]
    fn triangle_resize_is_antialiased_and_preserves_constants() {
        let constant = GrayImage::from_raw(13, 7, vec![91; 91]).unwrap();
        assert_eq!(
            resize_triangle_gray(&constant, 5, 3),
            GrayImage::from_raw(5, 3, vec![91; 15]).unwrap()
        );

        let ramp = GrayImage::from_raw(2, 1, vec![0, 255]).unwrap();
        let resized = resize_triangle_gray(&ramp, 4, 1);
        assert_eq!(resized.data, vec![0, 64, 191, 255]);
    }

    #[test]
    fn jpeg_dimensions_scan_markers_without_decoding_pixels() {
        let jpeg = [
            0xff, 0xd8, 0xff, 0xe0, 0, 4, 0, 0, 0xff, 0xc0, 0, 11, 8, 0, 7, 0, 9, 1, 1, 0x11, 0,
            0xff, 0xd9,
        ];
        assert_eq!(dimensions(&jpeg).unwrap(), (9, 7));
    }
}
