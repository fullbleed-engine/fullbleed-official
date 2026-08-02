use std::fmt;
use std::sync::OnceLock;

const MAX_WORK_BYTES: usize = 1 << 30;
const ZIGZAG: [usize; 64] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20,
    13, 6, 7, 14, 21, 28, 35, 42, 49, 56, 57, 50, 43, 36, 29, 22, 15, 23, 30, 37, 44, 51, 58, 59,
    52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63,
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum JpegColor {
    Gray,
    Rgb,
    Cmyk,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct JpegDecoded {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) rgba: Vec<u8>,
    pub(crate) color: JpegColor,
    pub(crate) adobe_transform: Option<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct JpegError {
    pub(crate) offset: usize,
    reason: String,
}

impl JpegError {
    fn new(offset: usize, reason: impl Into<String>) -> Self {
        Self {
            offset,
            reason: reason.into(),
        }
    }
}

impl fmt::Display for JpegError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "invalid JPEG at byte {}: {}",
            self.offset, self.reason
        )
    }
}

impl std::error::Error for JpegError {}

#[derive(Clone, Debug)]
struct HuffmanTable {
    minimum_code: [i32; 17],
    maximum_code: [i32; 17],
    value_offset: [i32; 17],
    symbols: Vec<u8>,
}

impl HuffmanTable {
    fn new(counts: &[u8; 16], symbols: &[u8], offset: usize) -> Result<Self, JpegError> {
        let symbol_count: usize = counts.iter().map(|count| usize::from(*count)).sum();
        if symbol_count == 0 || symbol_count != symbols.len() {
            return Err(JpegError::new(
                offset,
                "invalid empty or truncated Huffman table",
            ));
        }

        let mut minimum_code = [-1i32; 17];
        let mut maximum_code = [-1i32; 17];
        let mut value_offset = [0i32; 17];
        let mut code = 0i32;
        let mut symbol_index = 0i32;
        for length in 1..=16 {
            let count = i32::from(counts[length - 1]);
            if count != 0 {
                minimum_code[length] = code;
                maximum_code[length] = code + count - 1;
                value_offset[length] = symbol_index - code;
                symbol_index += count;
                code += count;
            }
            if code > (1 << length) {
                return Err(JpegError::new(offset, "oversubscribed JPEG Huffman table"));
            }
            code <<= 1;
        }
        Ok(Self {
            minimum_code,
            maximum_code,
            value_offset,
            symbols: symbols.to_vec(),
        })
    }

    fn decode(&self, reader: &mut EntropyReader<'_>) -> Result<u8, JpegError> {
        let mut code = 0i32;
        for length in 1..=16 {
            code = (code << 1) | i32::from(reader.read_bit()?);
            if self.maximum_code[length] >= 0
                && code >= self.minimum_code[length]
                && code <= self.maximum_code[length]
            {
                let index = code + self.value_offset[length];
                return self
                    .symbols
                    .get(index as usize)
                    .copied()
                    .ok_or_else(|| JpegError::new(reader.offset, "Huffman symbol index overflow"));
            }
        }
        Err(JpegError::new(
            reader.offset,
            "entropy data uses an undefined Huffman code",
        ))
    }
}

#[derive(Debug)]
struct Component {
    id: u8,
    horizontal_sampling: usize,
    vertical_sampling: usize,
    quantization_table: usize,
    block_width: usize,
    block_height: usize,
    padded_block_width: usize,
    padded_block_height: usize,
    coefficients: Vec<i32>,
    successive_low: [i8; 64],
    sequential_seen: bool,
}

#[derive(Debug)]
struct Frame {
    width: u32,
    height: u32,
    progressive: bool,
    maximum_horizontal_sampling: usize,
    maximum_vertical_sampling: usize,
    mcu_columns: usize,
    mcu_rows: usize,
    components: Vec<Component>,
}

#[derive(Clone, Debug)]
struct ScanComponent {
    component_index: usize,
    dc_table: usize,
    ac_table: usize,
}

#[derive(Clone, Debug)]
struct Scan {
    components: Vec<ScanComponent>,
    spectral_start: usize,
    spectral_end: usize,
    successive_high: u8,
    successive_low: u8,
}

struct Decoder<'a> {
    data: &'a [u8],
    quantization_tables: [Option<[u16; 64]>; 4],
    dc_tables: [Option<HuffmanTable>; 4],
    ac_tables: [Option<HuffmanTable>; 4],
    restart_interval: usize,
    adobe_transform: Option<u8>,
    jfif: bool,
    frame: Option<Frame>,
    saw_scan: bool,
}

impl<'a> Decoder<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            quantization_tables: [None, None, None, None],
            dc_tables: std::array::from_fn(|_| None),
            ac_tables: std::array::from_fn(|_| None),
            restart_interval: 0,
            adobe_transform: None,
            jfif: false,
            frame: None,
            saw_scan: false,
        }
    }

    fn decode(mut self) -> Result<JpegDecoded, JpegError> {
        if !self.data.starts_with(&[0xff, 0xd8]) {
            return Err(JpegError::new(0, "missing SOI marker"));
        }
        let mut offset = 2usize;
        let mut saw_eoi = false;
        while offset < self.data.len() {
            let (marker, marker_offset, after_marker) = read_marker(self.data, offset)?;
            offset = after_marker;
            match marker {
                0xd8 => return Err(JpegError::new(marker_offset, "duplicate SOI marker")),
                0xd9 => {
                    saw_eoi = true;
                    break;
                }
                0xc0 | 0xc1 | 0xc2 => {
                    let (payload, next) = segment(self.data, offset, marker_offset)?;
                    if self.frame.is_some() || self.saw_scan {
                        return Err(JpegError::new(
                            marker_offset,
                            "duplicate or misplaced frame",
                        ));
                    }
                    self.frame = Some(parse_frame(payload, marker == 0xc2, marker_offset)?);
                    offset = next;
                }
                marker if is_unsupported_frame(marker) => {
                    return Err(JpegError::new(
                        marker_offset,
                        "unsupported lossless, differential, or arithmetic JPEG frame",
                    ));
                }
                0xdb => {
                    let (payload, next) = segment(self.data, offset, marker_offset)?;
                    parse_quantization_tables(
                        payload,
                        &mut self.quantization_tables,
                        marker_offset,
                    )?;
                    offset = next;
                }
                0xc4 => {
                    let (payload, next) = segment(self.data, offset, marker_offset)?;
                    parse_huffman_tables(
                        payload,
                        &mut self.dc_tables,
                        &mut self.ac_tables,
                        marker_offset,
                    )?;
                    offset = next;
                }
                0xdd => {
                    let (payload, next) = segment(self.data, offset, marker_offset)?;
                    if payload.len() != 2 {
                        return Err(JpegError::new(marker_offset, "invalid DRI segment length"));
                    }
                    self.restart_interval =
                        usize::from(u16::from_be_bytes([payload[0], payload[1]]));
                    offset = next;
                }
                0xe0 => {
                    let (payload, next) = segment(self.data, offset, marker_offset)?;
                    if payload.starts_with(b"JFIF\0") {
                        self.jfif = true;
                    }
                    offset = next;
                }
                0xee => {
                    let (payload, next) = segment(self.data, offset, marker_offset)?;
                    if payload.len() >= 12 && payload.starts_with(b"Adobe") {
                        self.adobe_transform = Some(payload[11]);
                    }
                    offset = next;
                }
                0xda => {
                    let (payload, entropy_offset) = segment(self.data, offset, marker_offset)?;
                    let frame = self.frame.as_ref().ok_or_else(|| {
                        JpegError::new(marker_offset, "scan precedes frame header")
                    })?;
                    let scan = parse_scan(payload, frame, marker_offset)?;
                    self.validate_scan(&scan, marker_offset)?;
                    offset = self.decode_scan(scan, entropy_offset)?;
                    self.saw_scan = true;
                }
                0x01 | 0xd0..=0xd7 => {
                    return Err(JpegError::new(
                        marker_offset,
                        "misplaced standalone JPEG marker",
                    ));
                }
                0x00 | 0xff => {
                    return Err(JpegError::new(marker_offset, "invalid JPEG marker"));
                }
                _ => {
                    let (_, next) = segment(self.data, offset, marker_offset)?;
                    offset = next;
                }
            }
        }
        if !saw_eoi {
            return Err(JpegError::new(offset, "JPEG omits EOI marker"));
        }
        if !self.saw_scan {
            return Err(JpegError::new(offset, "JPEG contains no image scan"));
        }
        if offset != self.data.len() {
            return Err(JpegError::new(offset, "trailing bytes after JPEG EOI"));
        }
        self.finish_image()
    }

    fn validate_scan(&mut self, scan: &Scan, offset: usize) -> Result<(), JpegError> {
        let frame = self.frame.as_mut().expect("frame validated before scan");
        if !frame.progressive {
            if scan.spectral_start != 0
                || scan.spectral_end != 63
                || scan.successive_high != 0
                || scan.successive_low != 0
            {
                return Err(JpegError::new(
                    offset,
                    "invalid sequential JPEG scan parameters",
                ));
            }
            for scan_component in &scan.components {
                let component = &mut frame.components[scan_component.component_index];
                if component.sequential_seen {
                    return Err(JpegError::new(
                        offset,
                        "component appears in multiple sequential scans",
                    ));
                }
                component.sequential_seen = true;
            }
            return Ok(());
        }

        if scan.successive_high > 13 || scan.successive_low > 13 {
            return Err(JpegError::new(
                offset,
                "progressive approximation bit exceeds 13",
            ));
        }
        if scan.successive_high != 0
            && scan.successive_high != scan.successive_low.saturating_add(1)
        {
            return Err(JpegError::new(
                offset,
                "invalid progressive successive-approximation transition",
            ));
        }
        if scan.spectral_start == 0 {
            if scan.spectral_end != 0 {
                return Err(JpegError::new(
                    offset,
                    "progressive DC scan must end at zero",
                ));
            }
        } else if scan.components.len() != 1 || scan.spectral_end < scan.spectral_start {
            return Err(JpegError::new(
                offset,
                "progressive AC scans must contain exactly one component and a valid band",
            ));
        }

        for scan_component in &scan.components {
            let component = &mut frame.components[scan_component.component_index];
            for spectral in scan.spectral_start..=scan.spectral_end {
                let natural = ZIGZAG[spectral];
                let prior = component.successive_low[natural];
                if scan.successive_high == 0 {
                    if prior != -1 {
                        return Err(JpegError::new(offset, "duplicate progressive first scan"));
                    }
                } else if prior != scan.successive_high as i8 {
                    return Err(JpegError::new(
                        offset,
                        "progressive refinement does not follow the prior approximation",
                    ));
                }
                component.successive_low[natural] = scan.successive_low as i8;
            }
        }
        Ok(())
    }

    fn decode_scan(&mut self, scan: Scan, entropy_offset: usize) -> Result<usize, JpegError> {
        let frame = self.frame.as_mut().expect("frame validated before scan");
        let interleaved = scan.components.len() > 1;
        let unit_count = if interleaved {
            frame
                .mcu_columns
                .checked_mul(frame.mcu_rows)
                .ok_or_else(|| JpegError::new(entropy_offset, "JPEG MCU count overflow"))?
        } else {
            let component = &frame.components[scan.components[0].component_index];
            component
                .block_width
                .checked_mul(component.block_height)
                .ok_or_else(|| JpegError::new(entropy_offset, "JPEG block count overflow"))?
        };

        let mut reader = EntropyReader::new(self.data, entropy_offset);
        let mut dc_predictors = vec![0i32; frame.components.len()];
        let mut eob_run = 0usize;
        let mut expected_restart = 0u8;

        for unit in 0..unit_count {
            for scan_component in &scan.components {
                let component_index = scan_component.component_index;
                let (block_origin_x, block_origin_y, horizontal, vertical) = if interleaved {
                    let mcu_x = unit % frame.mcu_columns;
                    let mcu_y = unit / frame.mcu_columns;
                    let component = &frame.components[component_index];
                    (
                        mcu_x * component.horizontal_sampling,
                        mcu_y * component.vertical_sampling,
                        component.horizontal_sampling,
                        component.vertical_sampling,
                    )
                } else {
                    let component = &frame.components[component_index];
                    (
                        unit % component.block_width,
                        unit / component.block_width,
                        1,
                        1,
                    )
                };

                for vertical_index in 0..vertical {
                    for horizontal_index in 0..horizontal {
                        let block_x = block_origin_x + horizontal_index;
                        let block_y = block_origin_y + vertical_index;
                        let component = &mut frame.components[component_index];
                        let coefficient_offset = block_y
                            .checked_mul(component.padded_block_width)
                            .and_then(|row| row.checked_add(block_x))
                            .and_then(|block| block.checked_mul(64))
                            .ok_or_else(|| {
                                JpegError::new(reader.offset, "JPEG coefficient offset overflow")
                            })?;
                        let block = component
                            .coefficients
                            .get_mut(coefficient_offset..coefficient_offset + 64)
                            .ok_or_else(|| {
                                JpegError::new(reader.offset, "JPEG block lies outside frame")
                            })?;

                        if frame.progressive {
                            decode_progressive_block(
                                &mut reader,
                                block,
                                &scan,
                                scan_component,
                                &self.dc_tables,
                                &self.ac_tables,
                                &mut dc_predictors[component_index],
                                &mut eob_run,
                            )?;
                        } else {
                            decode_sequential_block(
                                &mut reader,
                                block,
                                scan_component,
                                &self.dc_tables,
                                &self.ac_tables,
                                &mut dc_predictors[component_index],
                            )?;
                        }
                    }
                }
            }

            if self.restart_interval != 0
                && (unit + 1) % self.restart_interval == 0
                && unit + 1 < unit_count
            {
                reader.align_to_byte();
                reader.consume_restart(expected_restart)?;
                expected_restart = (expected_restart + 1) & 7;
                dc_predictors.fill(0);
                eob_run = 0;
            }
        }
        reader.align_to_byte();
        reader.marker_offset()
    }

    fn finish_image(self) -> Result<JpegDecoded, JpegError> {
        let frame = self
            .frame
            .ok_or_else(|| JpegError::new(0, "missing JPEG frame"))?;
        for component in &frame.components {
            if frame.progressive {
                if component.successive_low[0] < 0 {
                    return Err(JpegError::new(0, "progressive component omits its DC scan"));
                }
            } else if !component.sequential_seen {
                return Err(JpegError::new(0, "sequential component omits its scan"));
            }
        }

        let mut planes = Vec::with_capacity(frame.components.len());
        for component in &frame.components {
            let quantization = self.quantization_tables[component.quantization_table]
                .as_ref()
                .ok_or_else(|| {
                    JpegError::new(0, "component references an undefined quantization table")
                })?;
            planes.push(reconstruct_component(component, quantization)?);
        }
        compose_pixels(&frame, &planes, self.adobe_transform, self.jfif)
    }
}

pub(crate) fn decode(data: &[u8]) -> Result<JpegDecoded, JpegError> {
    Decoder::new(data).decode()
}

fn segment<'a>(
    data: &'a [u8],
    offset: usize,
    marker_offset: usize,
) -> Result<(&'a [u8], usize), JpegError> {
    let length_bytes = data
        .get(offset..offset.saturating_add(2))
        .ok_or_else(|| JpegError::new(offset, "truncated JPEG segment length"))?;
    let length = usize::from(u16::from_be_bytes([length_bytes[0], length_bytes[1]]));
    if length < 2 {
        return Err(JpegError::new(
            marker_offset,
            "JPEG segment length is less than two",
        ));
    }
    let end = offset
        .checked_add(length)
        .ok_or_else(|| JpegError::new(offset, "JPEG segment length overflow"))?;
    let payload = data
        .get(offset + 2..end)
        .ok_or_else(|| JpegError::new(offset, "truncated JPEG segment"))?;
    Ok((payload, end))
}

fn read_marker(data: &[u8], mut offset: usize) -> Result<(u8, usize, usize), JpegError> {
    let marker_offset = offset;
    if data.get(offset) != Some(&0xff) {
        return Err(JpegError::new(offset, "expected JPEG marker prefix"));
    }
    while data.get(offset) == Some(&0xff) {
        offset += 1;
    }
    let marker = *data
        .get(offset)
        .ok_or_else(|| JpegError::new(offset, "truncated JPEG marker"))?;
    if marker == 0x00 {
        return Err(JpegError::new(offset, "stuffed byte outside entropy data"));
    }
    Ok((marker, marker_offset, offset + 1))
}

fn parse_frame(payload: &[u8], progressive: bool, offset: usize) -> Result<Frame, JpegError> {
    if payload.len() < 6 {
        return Err(JpegError::new(offset, "truncated JPEG frame header"));
    }
    if payload[0] != 8 {
        return Err(JpegError::new(
            offset,
            "only 8-bit JPEG precision is supported",
        ));
    }
    let height = u32::from(u16::from_be_bytes([payload[1], payload[2]]));
    let width = u32::from(u16::from_be_bytes([payload[3], payload[4]]));
    if width == 0 || height == 0 {
        return Err(JpegError::new(offset, "JPEG dimensions must be nonzero"));
    }
    let component_count = usize::from(payload[5]);
    if !matches!(component_count, 1 | 3 | 4) || payload.len() != 6 + component_count * 3 {
        return Err(JpegError::new(
            offset,
            "JPEG must have one, three, or four complete components",
        ));
    }
    let mut specifications = Vec::with_capacity(component_count);
    let mut maximum_horizontal_sampling = 0usize;
    let mut maximum_vertical_sampling = 0usize;
    for index in 0..component_count {
        let base = 6 + index * 3;
        let id = payload[base];
        if specifications.iter().any(|(prior, _, _, _)| *prior == id) {
            return Err(JpegError::new(
                offset,
                "duplicate JPEG component identifier",
            ));
        }
        let horizontal = usize::from(payload[base + 1] >> 4);
        let vertical = usize::from(payload[base + 1] & 0x0f);
        let quantization = usize::from(payload[base + 2]);
        if horizontal == 0 || vertical == 0 || horizontal > 4 || vertical > 4 {
            return Err(JpegError::new(
                offset,
                "invalid JPEG component sampling factor",
            ));
        }
        if quantization >= 4 {
            return Err(JpegError::new(
                offset,
                "JPEG quantization table selector exceeds three",
            ));
        }
        maximum_horizontal_sampling = maximum_horizontal_sampling.max(horizontal);
        maximum_vertical_sampling = maximum_vertical_sampling.max(vertical);
        specifications.push((id, horizontal, vertical, quantization));
    }
    if specifications
        .iter()
        .map(|(_, horizontal, vertical, _)| horizontal * vertical)
        .sum::<usize>()
        > 10
    {
        return Err(JpegError::new(
            offset,
            "JPEG MCU contains more than ten blocks",
        ));
    }

    let mcu_width = maximum_horizontal_sampling * 8;
    let mcu_height = maximum_vertical_sampling * 8;
    let mcu_columns = (width as usize).div_ceil(mcu_width);
    let mcu_rows = (height as usize).div_ceil(mcu_height);
    let mut work_bytes = 0usize;
    let mut components = Vec::with_capacity(component_count);
    for (id, horizontal, vertical, quantization) in specifications {
        let block_width = (width as usize * horizontal).div_ceil(maximum_horizontal_sampling * 8);
        let block_height = (height as usize * vertical).div_ceil(maximum_vertical_sampling * 8);
        let padded_block_width = mcu_columns * horizontal;
        let padded_block_height = mcu_rows * vertical;
        let coefficient_count = padded_block_width
            .checked_mul(padded_block_height)
            .and_then(|blocks| blocks.checked_mul(64))
            .ok_or_else(|| JpegError::new(offset, "JPEG coefficient allocation overflow"))?;
        work_bytes =
            work_bytes
                .checked_add(coefficient_count.checked_mul(4).ok_or_else(|| {
                    JpegError::new(offset, "JPEG coefficient allocation overflow")
                })?)
                .ok_or_else(|| JpegError::new(offset, "JPEG working set overflow"))?;
        if work_bytes > MAX_WORK_BYTES {
            return Err(JpegError::new(
                offset,
                "JPEG working set exceeds the image limit",
            ));
        }
        components.push(Component {
            id,
            horizontal_sampling: horizontal,
            vertical_sampling: vertical,
            quantization_table: quantization,
            block_width,
            block_height,
            padded_block_width,
            padded_block_height,
            coefficients: vec![0; coefficient_count],
            successive_low: [-1; 64],
            sequential_seen: false,
        });
    }
    Ok(Frame {
        width,
        height,
        progressive,
        maximum_horizontal_sampling,
        maximum_vertical_sampling,
        mcu_columns,
        mcu_rows,
        components,
    })
}

fn parse_quantization_tables(
    mut payload: &[u8],
    tables: &mut [Option<[u16; 64]>; 4],
    offset: usize,
) -> Result<(), JpegError> {
    while !payload.is_empty() {
        let information = payload[0];
        payload = &payload[1..];
        let precision = information >> 4;
        let identifier = usize::from(information & 0x0f);
        if precision > 1 || identifier >= 4 {
            return Err(JpegError::new(
                offset,
                "invalid JPEG quantization table header",
            ));
        }
        let bytes_per_value = usize::from(precision) + 1;
        let table_bytes = payload
            .get(..64 * bytes_per_value)
            .ok_or_else(|| JpegError::new(offset, "truncated JPEG quantization table"))?;
        let mut table = [0u16; 64];
        for zigzag in 0..64 {
            let value = if bytes_per_value == 1 {
                u16::from(table_bytes[zigzag])
            } else {
                let index = zigzag * 2;
                u16::from_be_bytes([table_bytes[index], table_bytes[index + 1]])
            };
            if value == 0 {
                return Err(JpegError::new(offset, "JPEG quantization value is zero"));
            }
            table[ZIGZAG[zigzag]] = value;
        }
        tables[identifier] = Some(table);
        payload = &payload[64 * bytes_per_value..];
    }
    Ok(())
}

fn parse_huffman_tables(
    mut payload: &[u8],
    dc_tables: &mut [Option<HuffmanTable>; 4],
    ac_tables: &mut [Option<HuffmanTable>; 4],
    offset: usize,
) -> Result<(), JpegError> {
    while !payload.is_empty() {
        let information = payload[0];
        payload = &payload[1..];
        let class = information >> 4;
        let identifier = usize::from(information & 0x0f);
        if class > 1 || identifier >= 4 {
            return Err(JpegError::new(offset, "invalid JPEG Huffman table header"));
        }
        let count_bytes = payload
            .get(..16)
            .ok_or_else(|| JpegError::new(offset, "truncated JPEG Huffman code counts"))?;
        let counts: [u8; 16] = count_bytes.try_into().expect("sixteen Huffman counts");
        let symbol_count: usize = counts.iter().map(|count| usize::from(*count)).sum();
        let symbols = payload
            .get(16..16 + symbol_count)
            .ok_or_else(|| JpegError::new(offset, "truncated JPEG Huffman symbols"))?;
        let table = HuffmanTable::new(&counts, symbols, offset)?;
        if class == 0 {
            if symbols.iter().any(|symbol| *symbol > 11) {
                return Err(JpegError::new(offset, "JPEG DC category exceeds eleven"));
            }
            dc_tables[identifier] = Some(table);
        } else {
            if symbols.iter().any(|symbol| (symbol & 0x0f) > 10) {
                return Err(JpegError::new(offset, "invalid JPEG AC run/size symbol"));
            }
            ac_tables[identifier] = Some(table);
        }
        payload = &payload[16 + symbol_count..];
    }
    Ok(())
}

fn parse_scan(payload: &[u8], frame: &Frame, offset: usize) -> Result<Scan, JpegError> {
    let component_count = payload.first().copied().map(usize::from).unwrap_or(0);
    if component_count == 0 || component_count > 4 || payload.len() != 1 + component_count * 2 + 3 {
        return Err(JpegError::new(offset, "invalid JPEG scan header length"));
    }
    let mut components = Vec::with_capacity(component_count);
    for index in 0..component_count {
        let id = payload[1 + index * 2];
        let component_index = frame
            .components
            .iter()
            .position(|component| component.id == id)
            .ok_or_else(|| JpegError::new(offset, "scan references an unknown component"))?;
        if components
            .iter()
            .any(|prior: &ScanComponent| prior.component_index == component_index)
        {
            return Err(JpegError::new(offset, "duplicate component in JPEG scan"));
        }
        let selectors = payload[2 + index * 2];
        let dc_table = usize::from(selectors >> 4);
        let ac_table = usize::from(selectors & 0x0f);
        if dc_table >= 4 || ac_table >= 4 {
            return Err(JpegError::new(
                offset,
                "scan Huffman selector exceeds three",
            ));
        }
        components.push(ScanComponent {
            component_index,
            dc_table,
            ac_table,
        });
    }
    let parameter_offset = 1 + component_count * 2;
    let spectral_start = usize::from(payload[parameter_offset]);
    let spectral_end = usize::from(payload[parameter_offset + 1]);
    let successive_high = payload[parameter_offset + 2] >> 4;
    let successive_low = payload[parameter_offset + 2] & 0x0f;
    if spectral_start > 63 || spectral_end > 63 {
        return Err(JpegError::new(offset, "JPEG spectral selection exceeds 63"));
    }
    Ok(Scan {
        components,
        spectral_start,
        spectral_end,
        successive_high,
        successive_low,
    })
}

fn decode_sequential_block(
    reader: &mut EntropyReader<'_>,
    block: &mut [i32],
    scan_component: &ScanComponent,
    dc_tables: &[Option<HuffmanTable>; 4],
    ac_tables: &[Option<HuffmanTable>; 4],
    predictor: &mut i32,
) -> Result<(), JpegError> {
    let dc = dc_tables[scan_component.dc_table]
        .as_ref()
        .ok_or_else(|| JpegError::new(reader.offset, "scan references an undefined DC table"))?;
    let ac = ac_tables[scan_component.ac_table]
        .as_ref()
        .ok_or_else(|| JpegError::new(reader.offset, "scan references an undefined AC table"))?;
    let category = dc.decode(reader)?;
    let difference = reader.receive_extend(category)?;
    *predictor = predictor
        .checked_add(difference)
        .ok_or_else(|| JpegError::new(reader.offset, "JPEG DC predictor overflow"))?;
    block[0] = *predictor;

    let mut spectral = 1usize;
    while spectral <= 63 {
        let symbol = ac.decode(reader)?;
        let run = usize::from(symbol >> 4);
        let size = symbol & 0x0f;
        if size == 0 {
            if run == 0 {
                break;
            }
            if run != 15 {
                return Err(JpegError::new(
                    reader.offset,
                    "invalid sequential JPEG zero-run symbol",
                ));
            }
            spectral += 16;
            if spectral > 64 {
                return Err(JpegError::new(reader.offset, "JPEG zero run exceeds block"));
            }
            continue;
        }
        spectral = spectral
            .checked_add(run)
            .ok_or_else(|| JpegError::new(reader.offset, "JPEG AC run overflow"))?;
        if spectral > 63 {
            return Err(JpegError::new(reader.offset, "JPEG AC run exceeds block"));
        }
        block[ZIGZAG[spectral]] = reader.receive_extend(size)?;
        spectral += 1;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn decode_progressive_block(
    reader: &mut EntropyReader<'_>,
    block: &mut [i32],
    scan: &Scan,
    scan_component: &ScanComponent,
    dc_tables: &[Option<HuffmanTable>; 4],
    ac_tables: &[Option<HuffmanTable>; 4],
    predictor: &mut i32,
    eob_run: &mut usize,
) -> Result<(), JpegError> {
    if scan.spectral_start == 0 {
        if scan.successive_high == 0 {
            let dc = dc_tables[scan_component.dc_table].as_ref().ok_or_else(|| {
                JpegError::new(reader.offset, "scan references an undefined DC table")
            })?;
            let category = dc.decode(reader)?;
            let difference = reader.receive_extend(category)?;
            *predictor = predictor
                .checked_add(difference)
                .ok_or_else(|| JpegError::new(reader.offset, "JPEG DC predictor overflow"))?;
            block[0] = *predictor << scan.successive_low;
        } else {
            block[0] |= i32::from(reader.read_bit()?) << scan.successive_low;
        }
        return Ok(());
    }

    let ac = ac_tables[scan_component.ac_table]
        .as_ref()
        .ok_or_else(|| JpegError::new(reader.offset, "scan references an undefined AC table"))?;
    if scan.successive_high == 0 {
        decode_progressive_ac_first(reader, block, scan, ac, eob_run)
    } else {
        decode_progressive_ac_refinement(reader, block, scan, ac, eob_run)
    }
}

fn decode_progressive_ac_first(
    reader: &mut EntropyReader<'_>,
    block: &mut [i32],
    scan: &Scan,
    table: &HuffmanTable,
    eob_run: &mut usize,
) -> Result<(), JpegError> {
    if *eob_run != 0 {
        *eob_run -= 1;
        return Ok(());
    }
    let mut spectral = scan.spectral_start;
    while spectral <= scan.spectral_end {
        let symbol = table.decode(reader)?;
        let run = usize::from(symbol >> 4);
        let size = symbol & 0x0f;
        if size == 0 {
            if run == 15 {
                spectral += 16;
                if spectral > scan.spectral_end + 1 {
                    return Err(JpegError::new(
                        reader.offset,
                        "progressive zero run exceeds band",
                    ));
                }
                continue;
            }
            let extra = if run == 0 {
                0
            } else {
                reader.read_bits(run as u8)? as usize
            };
            *eob_run = (1usize << run)
                .checked_add(extra)
                .and_then(|count| count.checked_sub(1))
                .ok_or_else(|| JpegError::new(reader.offset, "progressive EOB run overflow"))?;
            break;
        }
        spectral += run;
        if spectral > scan.spectral_end {
            return Err(JpegError::new(
                reader.offset,
                "progressive AC run exceeds band",
            ));
        }
        block[ZIGZAG[spectral]] = reader
            .receive_extend(size)?
            .checked_shl(u32::from(scan.successive_low))
            .ok_or_else(|| JpegError::new(reader.offset, "progressive coefficient overflow"))?;
        spectral += 1;
    }
    Ok(())
}

fn decode_progressive_ac_refinement(
    reader: &mut EntropyReader<'_>,
    block: &mut [i32],
    scan: &Scan,
    table: &HuffmanTable,
    eob_run: &mut usize,
) -> Result<(), JpegError> {
    let bit = 1i32 << scan.successive_low;
    let mut spectral = scan.spectral_start;
    if *eob_run == 0 {
        loop {
            let symbol = table.decode(reader)?;
            let mut zero_run = usize::from(symbol >> 4);
            let size = symbol & 0x0f;
            let new_coefficient = if size == 0 {
                if zero_run != 15 {
                    let extra = if zero_run == 0 {
                        0
                    } else {
                        reader.read_bits(zero_run as u8)? as usize
                    };
                    *eob_run = (1usize << zero_run).checked_add(extra).ok_or_else(|| {
                        JpegError::new(reader.offset, "progressive EOB run overflow")
                    })?;
                    break;
                }
                zero_run = 16;
                0
            } else if size == 1 {
                if reader.read_bit()? == 1 { bit } else { -bit }
            } else {
                return Err(JpegError::new(
                    reader.offset,
                    "progressive AC refinement symbol has size greater than one",
                ));
            };

            loop {
                if spectral > scan.spectral_end {
                    return Err(JpegError::new(
                        reader.offset,
                        "progressive refinement run exceeds band",
                    ));
                }
                let coefficient = &mut block[ZIGZAG[spectral]];
                if *coefficient != 0 {
                    refine_nonzero(reader, coefficient, bit)?;
                } else if zero_run == 0 {
                    break;
                } else {
                    zero_run -= 1;
                }
                spectral += 1;
            }
            if new_coefficient != 0 {
                block[ZIGZAG[spectral]] = new_coefficient;
                spectral += 1;
            }
            if spectral > scan.spectral_end {
                break;
            }
        }
    }

    while spectral <= scan.spectral_end {
        let coefficient = &mut block[ZIGZAG[spectral]];
        if *coefficient != 0 {
            refine_nonzero(reader, coefficient, bit)?;
        }
        spectral += 1;
    }
    if *eob_run != 0 {
        *eob_run -= 1;
    }
    Ok(())
}

fn refine_nonzero(
    reader: &mut EntropyReader<'_>,
    coefficient: &mut i32,
    bit: i32,
) -> Result<(), JpegError> {
    if reader.read_bit()? != 0 && coefficient.abs() & bit == 0 {
        if *coefficient > 0 {
            *coefficient += bit;
        } else {
            *coefficient -= bit;
        }
    }
    Ok(())
}

struct EntropyReader<'a> {
    data: &'a [u8],
    offset: usize,
    current_byte: u8,
    remaining_bits: u8,
}

impl<'a> EntropyReader<'a> {
    fn new(data: &'a [u8], offset: usize) -> Self {
        Self {
            data,
            offset,
            current_byte: 0,
            remaining_bits: 0,
        }
    }

    fn read_bit(&mut self) -> Result<u8, JpegError> {
        if self.remaining_bits == 0 {
            self.current_byte = self.read_entropy_byte()?;
            self.remaining_bits = 8;
        }
        self.remaining_bits -= 1;
        Ok((self.current_byte >> self.remaining_bits) & 1)
    }

    fn read_bits(&mut self, count: u8) -> Result<u32, JpegError> {
        let mut value = 0u32;
        for _ in 0..count {
            value = (value << 1) | u32::from(self.read_bit()?);
        }
        Ok(value)
    }

    fn receive_extend(&mut self, count: u8) -> Result<i32, JpegError> {
        if count == 0 {
            return Ok(0);
        }
        if count > 16 {
            return Err(JpegError::new(
                self.offset,
                "JPEG coefficient category exceeds 16",
            ));
        }
        let value = self.read_bits(count)? as i32;
        let threshold = 1i32 << (count - 1);
        if value < threshold {
            Ok(value + 1 - (1i32 << count))
        } else {
            Ok(value)
        }
    }

    fn read_entropy_byte(&mut self) -> Result<u8, JpegError> {
        let byte = *self
            .data
            .get(self.offset)
            .ok_or_else(|| JpegError::new(self.offset, "truncated JPEG entropy data"))?;
        self.offset += 1;
        if byte != 0xff {
            return Ok(byte);
        }
        let marker_offset = self.offset - 1;
        while self.data.get(self.offset) == Some(&0xff) {
            self.offset += 1;
        }
        let following = *self
            .data
            .get(self.offset)
            .ok_or_else(|| JpegError::new(self.offset, "truncated marker in entropy data"))?;
        if following == 0x00 {
            self.offset += 1;
            return Ok(0xff);
        }
        Err(JpegError::new(
            marker_offset,
            "unexpected marker before scan data was complete",
        ))
    }

    fn align_to_byte(&mut self) {
        self.remaining_bits = 0;
    }

    fn consume_restart(&mut self, expected: u8) -> Result<(), JpegError> {
        let marker_offset = self.offset;
        if self.data.get(self.offset) != Some(&0xff) {
            return Err(JpegError::new(self.offset, "missing JPEG restart marker"));
        }
        while self.data.get(self.offset) == Some(&0xff) {
            self.offset += 1;
        }
        let marker = *self
            .data
            .get(self.offset)
            .ok_or_else(|| JpegError::new(self.offset, "truncated JPEG restart marker"))?;
        self.offset += 1;
        if marker != 0xd0 + expected {
            return Err(JpegError::new(
                marker_offset,
                "out-of-sequence JPEG restart marker",
            ));
        }
        Ok(())
    }

    fn marker_offset(&self) -> Result<usize, JpegError> {
        if self.data.get(self.offset) != Some(&0xff) {
            return Err(JpegError::new(
                self.offset,
                "extra entropy bytes remain after the expected scan units",
            ));
        }
        Ok(self.offset)
    }
}

fn reconstruct_component(
    component: &Component,
    quantization: &[u16; 64],
) -> Result<Vec<u8>, JpegError> {
    let width = component
        .padded_block_width
        .checked_mul(8)
        .ok_or_else(|| JpegError::new(0, "JPEG component width overflow"))?;
    let height = component
        .padded_block_height
        .checked_mul(8)
        .ok_or_else(|| JpegError::new(0, "JPEG component height overflow"))?;
    let length = width
        .checked_mul(height)
        .filter(|length| *length <= MAX_WORK_BYTES)
        .ok_or_else(|| JpegError::new(0, "JPEG component plane exceeds the image limit"))?;
    let mut plane = vec![0u8; length];
    for block_y in 0..component.padded_block_height {
        for block_x in 0..component.padded_block_width {
            let coefficient_offset = (block_y * component.padded_block_width + block_x) * 64;
            let block = &component.coefficients[coefficient_offset..coefficient_offset + 64];
            let pixels = inverse_dct(block, quantization);
            for y in 0..8 {
                let destination = (block_y * 8 + y) * width + block_x * 8;
                plane[destination..destination + 8].copy_from_slice(&pixels[y * 8..y * 8 + 8]);
            }
        }
    }
    Ok(plane)
}

fn inverse_dct(coefficients: &[i32], quantization: &[u16; 64]) -> [u8; 64] {
    if coefficients[1..]
        .iter()
        .all(|coefficient| *coefficient == 0)
    {
        let value = ((f64::from(coefficients[0]) * f64::from(quantization[0]) / 8.0) + 128.0)
            .round()
            .clamp(0.0, 255.0) as u8;
        return [value; 64];
    }
    let basis = idct_basis();
    let mut intermediate = [[0.0f64; 8]; 8];
    for vertical_frequency in 0..8 {
        for x in 0..8 {
            let mut sum = 0.0;
            for horizontal_frequency in 0..8 {
                let index = vertical_frequency * 8 + horizontal_frequency;
                sum += basis[x][horizontal_frequency]
                    * f64::from(coefficients[index])
                    * f64::from(quantization[index]);
            }
            intermediate[vertical_frequency][x] = sum;
        }
    }
    let mut output = [0u8; 64];
    for y in 0..8 {
        for x in 0..8 {
            let mut sum = 0.0;
            for vertical_frequency in 0..8 {
                sum += basis[y][vertical_frequency] * intermediate[vertical_frequency][x];
            }
            output[y * 8 + x] = (sum * 0.25 + 128.0).round().clamp(0.0, 255.0) as u8;
        }
    }
    output
}

fn idct_basis() -> &'static [[f64; 8]; 8] {
    static BASIS: OnceLock<[[f64; 8]; 8]> = OnceLock::new();
    BASIS.get_or_init(|| {
        let mut basis = [[0.0f64; 8]; 8];
        for spatial in 0..8 {
            for frequency in 0..8 {
                let normalization = if frequency == 0 {
                    std::f64::consts::FRAC_1_SQRT_2
                } else {
                    1.0
                };
                basis[spatial][frequency] = normalization
                    * (((2 * spatial + 1) * frequency) as f64 * std::f64::consts::PI / 16.0).cos();
            }
        }
        basis
    })
}

fn compose_pixels(
    frame: &Frame,
    planes: &[Vec<u8>],
    adobe_transform: Option<u8>,
    jfif: bool,
) -> Result<JpegDecoded, JpegError> {
    let output_length = frame
        .width
        .checked_mul(frame.height)
        .and_then(|pixels| pixels.checked_mul(4))
        .and_then(|bytes| usize::try_from(bytes).ok())
        .filter(|bytes| *bytes <= MAX_WORK_BYTES)
        .ok_or_else(|| JpegError::new(0, "JPEG output exceeds the image limit"))?;
    let mut rgba = Vec::with_capacity(output_length);
    let component_ids: Vec<u8> = frame
        .components
        .iter()
        .map(|component| component.id)
        .collect();
    let rgb_components = component_ids == b"RGB";
    let ycbcr = frame.components.len() == 3
        && (adobe_transform == Some(1) || jfif || (!rgb_components && adobe_transform != Some(0)));

    for y in 0..frame.height {
        for x in 0..frame.width {
            let mut samples = [0u8; 4];
            for (index, component) in frame.components.iter().enumerate() {
                samples[index] = sample_component(frame, component, &planes[index], x, y);
            }
            let (red, green, blue) = match frame.components.len() {
                1 => (samples[0], samples[0], samples[0]),
                3 if ycbcr => ycbcr_to_rgb(samples[0], samples[1], samples[2]),
                3 => (samples[0], samples[1], samples[2]),
                4 if adobe_transform == Some(2) => {
                    let (red, green, blue) = ycbcr_to_rgb(samples[0], samples[1], samples[2]);
                    (
                        multiply_u8(red, samples[3]),
                        multiply_u8(green, samples[3]),
                        multiply_u8(blue, samples[3]),
                    )
                }
                4 if adobe_transform.is_some() => (
                    multiply_u8(samples[0], samples[3]),
                    multiply_u8(samples[1], samples[3]),
                    multiply_u8(samples[2], samples[3]),
                ),
                4 => (
                    255u8.saturating_sub(samples[0].saturating_add(samples[3])),
                    255u8.saturating_sub(samples[1].saturating_add(samples[3])),
                    255u8.saturating_sub(samples[2].saturating_add(samples[3])),
                ),
                _ => unreachable!("validated JPEG component count"),
            };
            rgba.extend_from_slice(&[red, green, blue, 255]);
        }
    }
    let color = match frame.components.len() {
        1 => JpegColor::Gray,
        3 => JpegColor::Rgb,
        4 => JpegColor::Cmyk,
        _ => unreachable!(),
    };
    Ok(JpegDecoded {
        width: frame.width,
        height: frame.height,
        rgba,
        color,
        adobe_transform,
    })
}

fn sample_component(frame: &Frame, component: &Component, plane: &[u8], x: u32, y: u32) -> u8 {
    let plane_stride = component.padded_block_width * 8;
    let logical_width = (frame.width as usize * component.horizontal_sampling)
        .div_ceil(frame.maximum_horizontal_sampling);
    let logical_height = (frame.height as usize * component.vertical_sampling)
        .div_ceil(frame.maximum_vertical_sampling);
    let source_x = (f64::from(x) + 0.5) * component.horizontal_sampling as f64
        / frame.maximum_horizontal_sampling as f64
        - 0.5;
    let source_y = (f64::from(y) + 0.5) * component.vertical_sampling as f64
        / frame.maximum_vertical_sampling as f64
        - 0.5;
    let x0 = source_x.floor().clamp(0.0, (logical_width - 1) as f64) as usize;
    let y0 = source_y.floor().clamp(0.0, (logical_height - 1) as f64) as usize;
    let x1 = (x0 + 1).min(logical_width - 1);
    let y1 = (y0 + 1).min(logical_height - 1);
    let horizontal_weight = source_x.clamp(0.0, (logical_width - 1) as f64) - x0 as f64;
    let vertical_weight = source_y.clamp(0.0, (logical_height - 1) as f64) - y0 as f64;
    let top = f64::from(plane[y0 * plane_stride + x0]) * (1.0 - horizontal_weight)
        + f64::from(plane[y0 * plane_stride + x1]) * horizontal_weight;
    let bottom = f64::from(plane[y1 * plane_stride + x0]) * (1.0 - horizontal_weight)
        + f64::from(plane[y1 * plane_stride + x1]) * horizontal_weight;
    (top * (1.0 - vertical_weight) + bottom * vertical_weight)
        .round()
        .clamp(0.0, 255.0) as u8
}

fn ycbcr_to_rgb(y: u8, cb: u8, cr: u8) -> (u8, u8, u8) {
    let y = f64::from(y);
    let cb = f64::from(cb) - 128.0;
    let cr = f64::from(cr) - 128.0;
    (
        (y + 1.402 * cr).round().clamp(0.0, 255.0) as u8,
        (y - 0.344_136 * cb - 0.714_136 * cr)
            .round()
            .clamp(0.0, 255.0) as u8,
        (y + 1.772 * cb).round().clamp(0.0, 255.0) as u8,
    )
}

fn multiply_u8(left: u8, right: u8) -> u8 {
    ((u16::from(left) * u16::from(right) + 127) / 255) as u8
}

fn is_unsupported_frame(marker: u8) -> bool {
    matches!(
        marker,
        0xc3 | 0xc5 | 0xc6 | 0xc7 | 0xc9 | 0xca | 0xcb | 0xcd | 0xce | 0xcf
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idct_dc_only_matches_level_shift_contract() {
        let mut coefficients = [0i32; 64];
        let quantization = [1u16; 64];
        assert_eq!(inverse_dct(&coefficients, &quantization), [128; 64]);
        coefficients[0] = 80;
        assert_eq!(inverse_dct(&coefficients, &quantization), [138; 64]);
        coefficients[0] = -80;
        assert_eq!(inverse_dct(&coefficients, &quantization), [118; 64]);
    }

    #[test]
    fn colorspace_conversions_cover_neutral_and_primaries() {
        assert_eq!(ycbcr_to_rgb(0, 128, 128), (0, 0, 0));
        assert_eq!(ycbcr_to_rgb(255, 128, 128), (255, 255, 255));
        let red = ycbcr_to_rgb(76, 85, 255);
        assert!(red.0 > 250 && red.1 < 5 && red.2 < 5, "{red:?}");
        assert_eq!(multiply_u8(255, 128), 128);
    }

    fn decode_hex(input: &str) -> Vec<u8> {
        input
            .as_bytes()
            .chunks_exact(2)
            .map(|pair| {
                let digit = |byte: u8| match byte {
                    b'0'..=b'9' => byte - b'0',
                    b'a'..=b'f' => byte - b'a' + 10,
                    _ => panic!("invalid fixture hex"),
                };
                (digit(pair[0]) << 4) | digit(pair[1])
            })
            .collect()
    }

    fn assert_fixture(input: &str, color: JpegColor, expected_digest: &str) -> JpegDecoded {
        assert_fixture_size(input, color, expected_digest, 19, 13)
    }

    fn assert_fixture_size(
        input: &str,
        color: JpegColor,
        expected_digest: &str,
        width: u32,
        height: u32,
    ) -> JpegDecoded {
        let decoded = decode(&decode_hex(input)).expect("decode independent JPEG fixture");
        assert_eq!(decoded.width, width);
        assert_eq!(decoded.height, height);
        assert_eq!(decoded.color, color);
        let digest = fullbleed_audit_contract::sha256::Sha256::digest(&decoded.rgba)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        assert_eq!(digest, expected_digest);
        decoded
    }

    #[test]
    fn decodes_baseline_progressive_and_restart_marker_jpegs() {
        let expected = "de8a8f7911b3c78397eeaf4713dfb48701f92bf429601184d0b13c17d0112707";
        let baseline = assert_fixture(
            concat!(
                "ffd8ffe000104a46494600010100000100010000ffdb0043000302020302020303020303030303040705040404040906",
                "0705070a090b0b0a090a0a0c0d110e0c0c100c0a0a0e140f1011121313130b0e141614121611121312ffdb0043010303",
                "0304040408050508120c0a0c121212121212121212121212121212121212121212121212121212121212121212121212",
                "1212121212121212121212121212ffc0001108000d001303012200021101031101ffc400170000030100000000000000",
                "00000000000000070806ffc4002d10000101050603090100000000000000000102000304051106070812142142516115",
                "2223242532335481d4ffc400160101010100000000000000000000000000050304ffc400261101000102040407000000",
                "0000000000000102031100040521060731511213224142d1e1ffda000c03010002110311003f009c6eeae07e2f2dcb85",
                "a94b2d71ba295a9e07145a939114ee9a9daa3a8dcfe33e2eeaece55e16dcb85b01899c4f425c53e9f59b9758f1317d2c",
                "4b8098fed630ea25f3b72732521d1ca521f9a1cc77483d18fd1f8bf58a9a0669d362cab10b1b8599ca34c97a90da531f",
                "cbe2bcbdd7f52cff00992ca526a3469caac8188f8205db3243b1d6fbe15315a797c4bd8597c835d0f0eb2ed11222c243",
                "da1a150010a196b5a1aee2876ad1864ec062c2d6e8dd7a2d8df6fd089fe861b1d1e5dea65388d18ad8f9bf782eaf1e73",
                "9ea4d9c33508c56e014903d8169ab6e9babdf1ffd9",
            ),
            JpegColor::Rgb,
            expected,
        );
        let progressive = assert_fixture(
            concat!(
                "ffd8ffe000104a46494600010100000100010000ffdb0043000302020302020303020303030303040705040404040906",
                "0705070a090b0b0a090a0a0c0d110e0c0c100c0a0a0e140f1011121313130b0e141614121611121312ffdb0043010303",
                "0304040408050508120c0a0c121212121212121212121212121212121212121212121212121212121212121212121212",
                "1212121212121212121212121212ffc2001108000d001303012200021101031101ffc400160001010100000000000000",
                "000000000000060007ffc400160101010100000000000000000000000000040203ffda000c03010002100310000001ce",
                "34a787cf4521d625ffc4001b10000105010100000000000000000000000500020306120401ffda00080101000105021c",
                "017283c44ecc6e1c322566b3b417acb675e3ffc400201100010205050000000000000000000000010203000406123105",
                "117182d2ffda0008010301013f01a7a7e65fb8b49bac4951c60730ad7ab351dc3a00ebe63fffc4002211000102030900",
                "000000000000000000000102040003110506121421226191b1ffda0008010201013f01676bbc5309b96155d3d2135d79",
                "308bbae708d83b8fffc4002210000005030403000000000000000000000001020312041113212231612392d2ffda0008",
                "010100063f022da2f60696d89a53a4a5c820fd3374790dbb6fcb1e48baec17868fd15f43ffc4001c1000010403010000",
                "00000000000000000000011121314161f171ffda0008010100013f21e20e96aa32609858c87d52c1c019b0db3b35128d",
                "be0d4327ffda000c0301000200030000001017dfffc4001a110101000301010000000000000000000001110021315141",
                "ffda0008010301013f10aa681086758a0f0edde3574680303e02a59cdabee7ffc4001a11010003000300000000000000",
                "000000000100112141d1e1ffda0008010201013f105240a9a1491a060be5c0828d1cfb9fffc4001d1001010101000105",
                "000000000000000000011121006110415181a1ffda0008010100013f10fcbe4a2727aae53c9afd768c88c896200b5637",
                "48e59ded7f1c2c456a45103a0e35a1f1e951bfffd9",
            ),
            JpegColor::Rgb,
            expected,
        );
        assert_fixture_size(
            concat!(
                "ffd8ffe000104a46494600010100000100010000ffdb0043000302020302020303020303030303040705040404040906",
                "0705070a090b0b0a090a0a0c0d110e0c0c100c0a0a0e140f1011121313130b0e141614121611121312ffdb0043010303",
                "0304040408050508120c0a0c121212121212121212121212121212121212121212121212121212121212121212121212",
                "1212121212121212121212121212ffc00011080011002903012200021101031101ffc400190000020301000000000000",
                "000000000000000705060804ffc4003710000102050300040a0b00000000000000010203000405061107122114224381",
                "081315173135425164a21623243341617475a1b2b3ffc400190100020301000000000000000000000000020506070800",
                "ffc400301100010204050007090100000000000000010211030405060012212231071334367173b12332354151617491",
                "b2b3ffdd00040002ffda000c03010002110311003f00ce3a75a03f75f66f77b31a52d6d0de854b53818c2d49d88c754e",
                "4f191f98e4f74485b5ad1a67469c97669cc57ebada9a0b3334fa6842107246c21f536adc300f09230a1ce720486aef86",
                "2d3acda1892b56c59b9a33e42249e9baa26526065385b85b4217b76e56014ad5cf8b2719c414ad5ee695b6e6234bc052",
                "6229395198849ccb2120ee2350f99be80966070e6d48971cb4a2a7a34a28438692b39884381f2de53a92c00e49385755",
                "f4d9376dc4cb5232f990a5ee659575541d5eeebb808f64ed481c9e120f19221a543d0a54bd250112c417d696f238c0f4",
                "9fe063be129696b7ea454e9a65a4a9f6cd25e736ed9c93a738a75ac2813b43ae2d1c8041ca4f04e307045ea6aedd52ba",
                "95274e9fbaaa4d3528a0eeea7b2dc839bc82305c6108514e083b738ce09190305372b57a1d93193d7a219581093b8b92",
                "bd14432487cb98f20e878e7096d4b7aedb92b6aa8d463c34aa229d5b944800066012cc0324024683f7ffd06fd4b4bbe8",
                "a5a87a337b27ea1f512bb7852723aee0c1046d4fa08ce1451ef8a0f981f86f9633b5c36cd77576f1f295cd53ab56d992",
                "4f46a739509a72614db0093d52be40528a9783c8dd8fc227fcc0fc37cb1617473622e4e8c81167c088ad54c876fb3e71",
                "c7872f8825fd4f55d155311154f630b621a1b82c77281eb07bc750581ca12e1f1cda75d977475ebb7aced5fd3bff00d9",
                "104109eb3f0187e627d158d2775773273c21ff00ac3c7fffd1abe9cf65dd0ceab7a9ee2fdb1dff0008208ee917bb925f",
                "910ff8898915add8667ca5fa62b7a75d977436e0822e0b6fb18c674a2f6618ffd9",
            ),
            JpegColor::Rgb,
            "6c72070aa48e69c157ba3ceefd1caf7771277a12b2a29914b73ef0e5518ae05a",
            41,
            17,
        );
        assert_eq!(baseline.rgba, progressive.rgba);
    }

    #[test]
    fn decodes_grayscale_and_adobe_cmyk_jpegs() {
        assert_fixture(
            concat!(
                "ffd8ffe000104a46494600010100000100010000ffdb0043000202020202010202020203020203030604030303030705",
                "0504060807090808070808090a0d0b090a0c0a08080b0f0b0c0d0e0e0f0e090b1011100e110d0e0e0effc0000b08000d",
                "001301011100ffc40017000100030000000000000000000000000008060709ffc4002d10000102040405010900000000",
                "0000000001020403050711000612210814152251422324263233556181d4ffda0008010100003f0015d3aa03f43dcbc7",
                "a30ddcad43792cb0b8c1a6988a48443b769d4adae0f91b9fd6228eb9397cc9c3197e50eaacdbc430a1bc0fd2811b49b1",
                "524250a1a4906c751b8b1daf60dea754ce41ec3b7c7a062a2e26789e97d0a779b724cba9c09d389621b25334ebc5b289",
                "710a01d494a6028a1490e5401d477483f8c67530e2c2a1f476ff000c64df93ed8eff00ab1fffd9",
            ),
            JpegColor::Gray,
            "95bcd4e407c7458b15f3c0c52e62d52c35badd385044b282909c1f0909261410",
        );
        let cmyk = assert_fixture(
            concat!(
                "ffd8ffee000e41646f626500640000000000ffdb0043000302020302020303030304030304050805050404050a070706",
                "080c0a0c0c0b0a0b0b0d0e12100d0e110e0b0b1016101113141515150c0f171816141812141514ffc0001408000d0013",
                "044311004d11005911004b1100ffc4001a000100020301000000000000000000000008000703060904ffc40034100001",
                "0106020607090100000000000000010200030405071106211217234143512232336481a1a20813141524313776b471ff",
                "da000e0443004d0059004b00003f0076564e3f8b3675edde3cd9a1502a0769b4e7bdba118ba77d7e93052b271fc5a6bd",
                "bbc79b1aaa0540ed369cf7b5298ba77d7e933c68c7e1bc01faecbbf95db4d7b778f36a323aa07d5bdda6fe6db1c9fd9d",
                "62e6b2b858d8e9ff00cb62a21d87ab84302565cdc5c2544ad27480b5c5b23719dae70d64e3f8b01358f33e7ea640d40c",
                "4d15b4cf9ef6f762e983ce9b052b271fc5a6b1e67cfd4d56d3cc2e6b25629060f8e8e7d2f8298bd7a5fbf87014f03b76",
                "e56f5494df20a5076521441092abd956b1d6e984821b18d400ea62843f8482875c62e1de27492f8a4a52949cc6414b0a",
                "cee0e8d88b1678d18fc37803f5d977f2bb69ac799f3f53745240e60b02c9e12412080732b944023dd43c2b8be8a13f72",
                "4939a944924a89254492492496bca2a60f3e21e7fadfffd9",
            ),
            JpegColor::Cmyk,
            "e4a93ad1db0ccec1e43686b1dd505959d9369157851997c33958edd15fad5aaa",
        );
        assert_eq!(cmyk.adobe_transform, Some(0));
    }
}
