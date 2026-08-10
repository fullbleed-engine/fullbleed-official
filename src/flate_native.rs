use std::cell::RefCell;
const ADLER_BASE: u32 = 65_521;
const DEFAULT_ADLER_CHUNK: usize = 1 << 20;
const ADLER_NMAX: usize = 5_552;

const LZ77_CHUNK_BYTES: usize = 128 * 1024;
const MIN_MATCH: usize = 3;
const MAX_MATCH: usize = 258;
const MAX_DISTANCE: usize = 32 * 1024;
const MAX_CHAIN_STEPS: usize = 64;
const DEFAULT_COMPILED_FLOW_CHAIN_STEPS: usize = 4;
const COMPILED_FLOW_THROUGHPUT_MIN_BYTES: usize = 4 * 1024;
const HASH_BITS: usize = 15;
const HASH_SIZE: usize = 1 << HASH_BITS;

const LENGTH_BASE: [usize; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];

const LENGTH_EXTRA_BITS: [u8; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];

const DIST_BASE: [usize; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];

const DIST_EXTRA_BITS: [u8; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];

#[derive(Clone, Copy, Debug)]
struct AdlerPartial {
    a: u32,
    b: u32,
    len: usize,
}

impl AdlerPartial {
    fn identity() -> Self {
        Self { a: 1, b: 0, len: 0 }
    }

    fn for_bytes(data: &[u8]) -> Self {
        let mut a = 1_u64;
        let mut b = 0_u64;
        for chunk in data.chunks(ADLER_NMAX) {
            for &byte in chunk {
                a += u64::from(byte);
                b += a;
            }
            a %= u64::from(ADLER_BASE);
            b %= u64::from(ADLER_BASE);
        }
        Self {
            a: a as u32,
            b: b as u32,
            len: data.len(),
        }
    }

    fn combine(self, rhs: Self) -> Self {
        if self.len == 0 {
            return rhs;
        }
        if rhs.len == 0 {
            return self;
        }
        let a = (self.a + rhs.a + ADLER_BASE - 1) % ADLER_BASE;
        let b = (self.b as u64
            + rhs.b as u64
            + ((rhs.len as u64 % ADLER_BASE as u64) * ((self.a + ADLER_BASE - 1) as u64)))
            % ADLER_BASE as u64;
        Self {
            a,
            b: b as u32,
            len: self.len + rhs.len,
        }
    }

    fn to_adler32(self) -> u32 {
        (self.b << 16) | self.a
    }
}

#[derive(Clone, Copy, Debug)]
enum Token {
    Literal(u8),
    Match { len: u16, dist: u16 },
}

#[derive(Clone, Debug)]
struct ChunkPlan {
    tokens: Vec<Token>,
}

#[derive(Default)]
struct BitWriter {
    out: Vec<u8>,
    bit_buf: u64,
    bit_count: u8,
}

impl BitWriter {
    fn with_capacity(capacity: usize) -> Self {
        Self {
            out: Vec::with_capacity(capacity),
            bit_buf: 0,
            bit_count: 0,
        }
    }

    fn write_bits(&mut self, bits: u32, count: u8) {
        if count == 0 {
            return;
        }
        self.bit_buf |= (bits as u64) << self.bit_count;
        self.bit_count += count;
        while self.bit_count >= 8 {
            self.out.push((self.bit_buf & 0xFF) as u8);
            self.bit_buf >>= 8;
            self.bit_count -= 8;
        }
    }

    fn finish(mut self) -> Vec<u8> {
        if self.bit_count > 0 {
            self.out.push((self.bit_buf & 0xFF) as u8);
            self.bit_buf = 0;
            self.bit_count = 0;
        }
        self.out
    }
}

fn chunk_ranges(total_len: usize, chunk_size: usize) -> Vec<(usize, usize)> {
    if total_len == 0 {
        return vec![(0, 0)];
    }
    let chunk_size = chunk_size.max(1);
    let mut out = Vec::with_capacity((total_len + chunk_size - 1) / chunk_size);
    let mut start = 0usize;
    while start < total_len {
        let end = (start + chunk_size).min(total_len);
        out.push((start, end));
        start = end;
    }
    out
}

fn adler32_parallel(data: &[u8], chunk_size: usize) -> u32 {
    let ranges = chunk_ranges(data.len(), chunk_size.max(1));
    let partials = crate::parallel::map_ordered(&ranges, |(start, end)| {
        AdlerPartial::for_bytes(&data[*start..*end])
    });

    let merged = partials
        .into_iter()
        .fold(AdlerPartial::identity(), AdlerPartial::combine);
    merged.to_adler32()
}

fn hash3(data: &[u8], i: usize) -> usize {
    let v = ((data[i] as u32) << 16) ^ ((data[i + 1] as u32) << 8) ^ (data[i + 2] as u32);
    (v.wrapping_mul(0x1E35_A7BD) >> (32 - HASH_BITS)) as usize
}

fn match_len(data: &[u8], a: usize, b: usize, max_len: usize) -> usize {
    let mut l = 0usize;
    while l + 8 <= max_len {
        let left = u64::from_le_bytes(
            data[a + l..a + l + 8]
                .try_into()
                .expect("eight-byte match window"),
        );
        let right = u64::from_le_bytes(
            data[b + l..b + l + 8]
                .try_into()
                .expect("eight-byte match window"),
        );
        let difference = left ^ right;
        if difference != 0 {
            return l + (difference.trailing_zeros() as usize >> 3);
        }
        l += 8;
    }
    while l < max_len && data[a + l] == data[b + l] {
        l += 1;
    }
    l
}

struct Lz77Workspace {
    head: Vec<i32>,
    head_epoch: Vec<u32>,
    prev: Vec<i32>,
    epoch: u32,
}

impl Lz77Workspace {
    fn new() -> Self {
        Self {
            head: vec![-1; HASH_SIZE],
            head_epoch: vec![0; HASH_SIZE],
            prev: Vec::new(),
            epoch: 0,
        }
    }

    fn begin_page(&mut self, len: usize) -> u32 {
        self.epoch = self.epoch.wrapping_add(1);
        if self.epoch == 0 {
            self.head_epoch.fill(0);
            self.epoch = 1;
        }
        if self.prev.len() < len {
            self.prev.resize(len, -1);
        }
        self.epoch
    }

    fn plan(&mut self, data: &[u8], max_chain_steps: usize) -> ChunkPlan {
        let n = data.len();
        if n == 0 {
            return ChunkPlan { tokens: Vec::new() };
        }
        let epoch = self.begin_page(n);
        let mut tokens = Vec::with_capacity(n / 2);

        let mut i = 0usize;
        while i < n {
            if i + MIN_MATCH > n {
                tokens.push(Token::Literal(data[i]));
                i += 1;
                continue;
            }

            let h = hash3(data, i);
            let mut cand = if self.head_epoch[h] == epoch {
                self.head[h]
            } else {
                -1
            };
            self.prev[i] = cand;
            self.head[h] = i as i32;
            self.head_epoch[h] = epoch;

            let mut best_len = 0usize;
            let mut best_dist = 0usize;
            let mut steps = 0usize;

            while cand >= 0 && steps < max_chain_steps {
                let c = cand as usize;
                let dist = i - c;
                if dist > MAX_DISTANCE {
                    break;
                }

                if data[c] == data[i] && data[c + 1] == data[i + 1] && data[c + 2] == data[i + 2] {
                    let max_len = MAX_MATCH.min(n - i);
                    let len = match_len(data, c, i, max_len);
                    if len >= MIN_MATCH && (len > best_len || (len == best_len && dist < best_dist))
                    {
                        best_len = len;
                        best_dist = dist;
                        if best_len == MAX_MATCH {
                            break;
                        }
                    }
                }

                cand = self.prev[c];
                steps += 1;
            }

            if best_len >= MIN_MATCH {
                tokens.push(Token::Match {
                    len: best_len as u16,
                    dist: best_dist as u16,
                });

                let end = (i + best_len).min(n);
                let mut j = i + 1;
                while j < end {
                    if j + MIN_MATCH <= n {
                        let hj = hash3(data, j);
                        self.prev[j] = if self.head_epoch[hj] == epoch {
                            self.head[hj]
                        } else {
                            -1
                        };
                        self.head[hj] = j as i32;
                        self.head_epoch[hj] = epoch;
                    }
                    j += 1;
                }

                i += best_len;
            } else {
                tokens.push(Token::Literal(data[i]));
                i += 1;
            }
        }

        ChunkPlan { tokens }
    }
}

thread_local! {
    static LZ77_WORKSPACE: RefCell<Lz77Workspace> = RefCell::new(Lz77Workspace::new());
}

fn plan_lz77_chunk_with_chain(data: &[u8], max_chain_steps: usize) -> ChunkPlan {
    LZ77_WORKSPACE.with(|workspace| {
        workspace
            .borrow_mut()
            .plan(data, max_chain_steps.clamp(1, MAX_CHAIN_STEPS))
    })
}

fn reverse_bits(mut value: u16, len: u8) -> u16 {
    let mut out = 0u16;
    for _ in 0..len {
        out = (out << 1) | (value & 1);
        value >>= 1;
    }
    out
}

fn fixed_litlen_code(sym: u16) -> (u16, u8) {
    match sym {
        0..=143 => (0x30 + sym, 8),
        144..=255 => (0x190 + (sym - 144), 9),
        256..=279 => (sym - 256, 7),
        280..=287 => (0x0C0 + (sym - 280), 8),
        _ => (0, 0),
    }
}

fn write_fixed_litlen(bw: &mut BitWriter, sym: u16) {
    let (code, len) = fixed_litlen_code(sym);
    let bits = reverse_bits(code, len) as u32;
    bw.write_bits(bits, len);
}

fn write_fixed_dist(bw: &mut BitWriter, sym: u16) {
    let bits = reverse_bits(sym, 5) as u32;
    bw.write_bits(bits, 5);
}

fn length_to_symbol(len: usize) -> (u16, u8, u16) {
    for (idx, (&base, &extra)) in LENGTH_BASE.iter().zip(LENGTH_EXTRA_BITS.iter()).enumerate() {
        let max = if extra == 0 {
            base
        } else {
            base + ((1usize << extra) - 1)
        };
        if len <= max {
            let sym = 257 + idx as u16;
            let extra_val = (len - base) as u16;
            return (sym, extra, extra_val);
        }
    }
    (285, 0, 0)
}

fn dist_to_symbol(dist: usize) -> (u16, u8, u16) {
    for (idx, (&base, &extra)) in DIST_BASE.iter().zip(DIST_EXTRA_BITS.iter()).enumerate() {
        let max = if extra == 0 {
            base
        } else {
            base + ((1usize << extra) - 1)
        };
        if dist <= max {
            let sym = idx as u16;
            let extra_val = (dist - base) as u16;
            return (sym, extra, extra_val);
        }
    }
    (0, 0, 0)
}

fn encode_chunk_fixed_huffman(bw: &mut BitWriter, chunk: &ChunkPlan, final_block: bool) {
    // BFINAL + BTYPE(01=fixed Huffman), packed LSB-first.
    let header = (if final_block { 1u32 } else { 0u32 }) | (0b01 << 1);
    bw.write_bits(header, 3);

    for token in &chunk.tokens {
        match *token {
            Token::Literal(byte) => {
                write_fixed_litlen(bw, byte as u16);
            }
            Token::Match { len, dist } => {
                let (len_sym, len_extra_bits, len_extra_val) = length_to_symbol(len as usize);
                write_fixed_litlen(bw, len_sym);
                if len_extra_bits > 0 {
                    bw.write_bits(len_extra_val as u32, len_extra_bits);
                }

                let (dist_sym, dist_extra_bits, dist_extra_val) = dist_to_symbol(dist as usize);
                write_fixed_dist(bw, dist_sym);
                if dist_extra_bits > 0 {
                    bw.write_bits(dist_extra_val as u32, dist_extra_bits);
                }
            }
        }
    }

    // End-of-block symbol.
    write_fixed_litlen(bw, 256);
}

fn estimate_deflate_capacity(input_len: usize) -> usize {
    // Empirical upper-bound-ish heuristic for fixed-Huffman + literals.
    // We can emit roughly <= 2x input bits on very small chunks plus headers.
    2 + input_len.saturating_mul(2) + 64
}

pub(crate) fn zlib_deflate_parallel(data: &[u8]) -> Vec<u8> {
    zlib_deflate_with_chain(data, MAX_CHAIN_STEPS)
}

pub(crate) fn zlib_deflate_compiled_flow(
    data: &[u8],
    compression: crate::pdf::CompiledFlowCompression,
) -> Vec<u8> {
    let steps = match compression {
        crate::pdf::CompiledFlowCompression::Throughput => DEFAULT_COMPILED_FLOW_CHAIN_STEPS,
        crate::pdf::CompiledFlowCompression::Compact => MAX_CHAIN_STEPS,
    };
    let steps = if data.len() < COMPILED_FLOW_THROUGHPUT_MIN_BYTES {
        MAX_CHAIN_STEPS
    } else {
        steps
    };
    zlib_deflate_with_chain(data, steps)
}

fn zlib_deflate_with_chain(data: &[u8], max_chain_steps: usize) -> Vec<u8> {
    let ranges = chunk_ranges(data.len(), LZ77_CHUNK_BYTES);

    let plans = crate::parallel::map_ordered(&ranges, |(start, end)| {
        plan_lz77_chunk_with_chain(&data[*start..*end], max_chain_steps)
    });

    let adler = adler32_parallel(data, DEFAULT_ADLER_CHUNK);

    let mut bw = BitWriter::with_capacity(estimate_deflate_capacity(data.len()));
    // zlib header: CMF=0x78 (deflate + 32K window), FLG=0x01 (valid FCHECK, fast hint).
    bw.out.extend_from_slice(&[0x78, 0x01]);

    for (idx, plan) in plans.iter().enumerate() {
        let final_block = idx + 1 == plans.len();
        encode_chunk_fixed_huffman(&mut bw, plan, final_block);
    }

    let mut out = bw.finish();
    out.extend_from_slice(&adler.to_be_bytes());
    out
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct InflateError {
    offset: usize,
    reason: &'static str,
}

impl std::fmt::Display for InflateError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "invalid DEFLATE stream at byte {}: {}",
            self.offset, self.reason
        )
    }
}

impl std::error::Error for InflateError {}

pub(crate) fn zlib_inflate(data: &[u8], max_output: usize) -> Result<Vec<u8>, InflateError> {
    if data.len() < 6 {
        return Err(inflate_error(0, "zlib stream is too short"));
    }
    let cmf = data[0];
    let flg = data[1];
    if cmf & 0x0f != 8 {
        return Err(inflate_error(0, "zlib stream does not use DEFLATE"));
    }
    if cmf >> 4 > 7 {
        return Err(inflate_error(0, "zlib window size exceeds 32 KiB"));
    }
    if (u16::from(cmf) * 256 + u16::from(flg)) % 31 != 0 {
        return Err(inflate_error(1, "zlib header check bits are invalid"));
    }
    if flg & 0x20 != 0 {
        return Err(inflate_error(1, "preset dictionaries are not supported"));
    }

    let checksum_offset = data.len() - 4;
    let mut output = deflate_inflate(&data[2..checksum_offset], max_output)?;
    let expected = u32::from_be_bytes(
        data[checksum_offset..]
            .try_into()
            .expect("four-byte Adler-32 trailer"),
    );
    let actual = AdlerPartial::for_bytes(&output).to_adler32();
    if actual != expected {
        output.clear();
        return Err(inflate_error(checksum_offset, "Adler-32 checksum mismatch"));
    }
    Ok(output)
}

pub(crate) fn deflate_inflate(data: &[u8], max_output: usize) -> Result<Vec<u8>, InflateError> {
    let mut reader = BitReader::new(data);
    let mut output = Vec::new();
    loop {
        let final_block = reader.read_bits(1)? != 0;
        let block_type = reader.read_bits(2)?;
        match block_type {
            0 => decode_stored_block(&mut reader, &mut output, max_output)?,
            1 => {
                let (literal_lengths, distances) = fixed_huffman_trees();
                decode_huffman_block(
                    &mut reader,
                    &mut output,
                    max_output,
                    literal_lengths,
                    distances,
                )?;
            }
            2 => {
                let (literal_lengths, distances) = read_dynamic_huffman_trees(&mut reader)?;
                decode_huffman_block(
                    &mut reader,
                    &mut output,
                    max_output,
                    &literal_lengths,
                    &distances,
                )?;
            }
            _ => return Err(reader.error("reserved DEFLATE block type")),
        }
        if final_block {
            break;
        }
    }
    if reader.byte_offset() != data.len() {
        return Err(reader.error("trailing bytes after final DEFLATE block"));
    }
    Ok(output)
}

struct BitReader<'a> {
    data: &'a [u8],
    offset: usize,
    bits: u64,
    bit_count: u8,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            offset: 0,
            bits: 0,
            bit_count: 0,
        }
    }

    fn read_bits(&mut self, count: u8) -> Result<u32, InflateError> {
        debug_assert!(count <= 24);
        while self.bit_count < count {
            let Some(byte) = self.data.get(self.offset).copied() else {
                return Err(self.error("unexpected end of compressed data"));
            };
            self.offset += 1;
            self.bits |= u64::from(byte) << self.bit_count;
            self.bit_count += 8;
        }
        let mask = if count == 0 { 0 } else { (1u64 << count) - 1 };
        let value = (self.bits & mask) as u32;
        self.bits >>= count;
        self.bit_count -= count;
        Ok(value)
    }

    fn align_to_byte(&mut self) {
        self.bits = 0;
        self.bit_count = 0;
    }

    fn read_aligned_u16(&mut self) -> Result<u16, InflateError> {
        if self.bit_count != 0 {
            return Err(self.error("internal byte-alignment error"));
        }
        let Some(bytes) = self.data.get(self.offset..self.offset.saturating_add(2)) else {
            return Err(self.error("unexpected end of stored block header"));
        };
        self.offset += 2;
        Ok(u16::from_le_bytes(
            bytes.try_into().expect("two-byte stored length"),
        ))
    }

    fn take_aligned(&mut self, length: usize) -> Result<&'a [u8], InflateError> {
        if self.bit_count != 0 {
            return Err(self.error("internal byte-alignment error"));
        }
        let Some(bytes) = self
            .data
            .get(self.offset..self.offset.saturating_add(length))
        else {
            return Err(self.error("stored block exceeds compressed input"));
        };
        self.offset += length;
        Ok(bytes)
    }

    fn byte_offset(&self) -> usize {
        self.offset
    }

    fn error(&self, reason: &'static str) -> InflateError {
        inflate_error(self.offset, reason)
    }
}

#[derive(Clone, Debug)]
struct Huffman {
    codes_by_length: Vec<Vec<(u16, u16)>>,
    max_length: u8,
}

impl Huffman {
    fn from_lengths(lengths: &[u8]) -> Result<Self, InflateError> {
        const MAX_BITS: usize = 15;
        let mut counts = [0u16; MAX_BITS + 1];
        for &length in lengths {
            if usize::from(length) > MAX_BITS {
                return Err(inflate_error(0, "Huffman code length exceeds 15 bits"));
            }
            if length != 0 {
                counts[usize::from(length)] += 1;
            }
        }
        if counts[1..].iter().all(|count| *count == 0) {
            return Err(inflate_error(0, "Huffman alphabet has no symbols"));
        }

        let mut remaining = 1i32;
        for count in counts.iter().skip(1) {
            remaining = (remaining << 1) - i32::from(*count);
            if remaining < 0 {
                return Err(inflate_error(0, "oversubscribed Huffman alphabet"));
            }
        }

        let mut next_code = [0u16; MAX_BITS + 1];
        let mut code = 0u16;
        for bits in 1..=MAX_BITS {
            code = (code + counts[bits - 1]) << 1;
            next_code[bits] = code;
        }

        let max_length = lengths.iter().copied().max().unwrap_or(0);
        let mut codes_by_length = vec![Vec::new(); usize::from(max_length) + 1];
        for (symbol, &length) in lengths.iter().enumerate() {
            if length == 0 {
                continue;
            }
            let canonical = next_code[usize::from(length)];
            next_code[usize::from(length)] += 1;
            codes_by_length[usize::from(length)]
                .push((reverse_bits(canonical, length), symbol as u16));
        }
        Ok(Self {
            codes_by_length,
            max_length,
        })
    }

    fn decode(&self, reader: &mut BitReader<'_>) -> Result<u16, InflateError> {
        let mut code = 0u16;
        for length in 1..=self.max_length {
            code |= (reader.read_bits(1)? as u16) << (length - 1);
            if let Some((_, symbol)) = self.codes_by_length[usize::from(length)]
                .iter()
                .find(|(candidate, _)| *candidate == code)
            {
                return Ok(*symbol);
            }
        }
        Err(reader.error("compressed data uses an undefined Huffman code"))
    }
}

fn fixed_huffman_trees() -> &'static (Huffman, Huffman) {
    static TREES: std::sync::OnceLock<(Huffman, Huffman)> = std::sync::OnceLock::new();
    TREES.get_or_init(|| {
        let mut literal_lengths = vec![0u8; 288];
        literal_lengths[0..=143].fill(8);
        literal_lengths[144..=255].fill(9);
        literal_lengths[256..=279].fill(7);
        literal_lengths[280..=287].fill(8);
        let distance_lengths = vec![5u8; 32];
        (
            Huffman::from_lengths(&literal_lengths).expect("valid fixed literal tree"),
            Huffman::from_lengths(&distance_lengths).expect("valid fixed distance tree"),
        )
    })
}

fn read_dynamic_huffman_trees(
    reader: &mut BitReader<'_>,
) -> Result<(Huffman, Huffman), InflateError> {
    const ORDER: [usize; 19] = [
        16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
    ];
    let literal_count = reader.read_bits(5)? as usize + 257;
    let distance_count = reader.read_bits(5)? as usize + 1;
    let code_length_count = reader.read_bits(4)? as usize + 4;
    if literal_count > 286 || distance_count > 32 {
        return Err(reader.error("dynamic Huffman alphabet size is invalid"));
    }

    let mut code_length_lengths = [0u8; 19];
    for &symbol in ORDER.iter().take(code_length_count) {
        code_length_lengths[symbol] = reader.read_bits(3)? as u8;
    }
    let code_length_tree = Huffman::from_lengths(&code_length_lengths)
        .map_err(|_| reader.error("invalid code-length Huffman alphabet"))?;

    let total = literal_count + distance_count;
    let mut lengths = Vec::with_capacity(total);
    while lengths.len() < total {
        match code_length_tree.decode(reader)? {
            symbol @ 0..=15 => lengths.push(symbol as u8),
            16 => {
                let Some(previous) = lengths.last().copied() else {
                    return Err(reader.error("repeat code has no previous length"));
                };
                let repeat = reader.read_bits(2)? as usize + 3;
                if lengths.len() + repeat > total {
                    return Err(reader.error("repeat code exceeds Huffman alphabet"));
                }
                lengths.extend(std::iter::repeat_n(previous, repeat));
            }
            17 => {
                let repeat = reader.read_bits(3)? as usize + 3;
                if lengths.len() + repeat > total {
                    return Err(reader.error("zero repeat exceeds Huffman alphabet"));
                }
                lengths.extend(std::iter::repeat_n(0, repeat));
            }
            18 => {
                let repeat = reader.read_bits(7)? as usize + 11;
                if lengths.len() + repeat > total {
                    return Err(reader.error("long zero repeat exceeds Huffman alphabet"));
                }
                lengths.extend(std::iter::repeat_n(0, repeat));
            }
            _ => return Err(reader.error("invalid code-length symbol")),
        }
    }
    if lengths.get(256).copied().unwrap_or(0) == 0 {
        return Err(reader.error("literal alphabet omits end-of-block symbol"));
    }
    let literal_lengths = Huffman::from_lengths(&lengths[..literal_count])
        .map_err(|_| reader.error("invalid literal/length Huffman alphabet"))?;
    let distances = Huffman::from_lengths(&lengths[literal_count..])
        .map_err(|_| reader.error("invalid distance Huffman alphabet"))?;
    Ok((literal_lengths, distances))
}

fn decode_stored_block(
    reader: &mut BitReader<'_>,
    output: &mut Vec<u8>,
    max_output: usize,
) -> Result<(), InflateError> {
    reader.align_to_byte();
    let length = reader.read_aligned_u16()?;
    let complement = reader.read_aligned_u16()?;
    if length != !complement {
        return Err(reader.error("stored block length complement is invalid"));
    }
    ensure_output_capacity(output.len(), usize::from(length), max_output, reader)?;
    output.extend_from_slice(reader.take_aligned(usize::from(length))?);
    Ok(())
}

fn decode_huffman_block(
    reader: &mut BitReader<'_>,
    output: &mut Vec<u8>,
    max_output: usize,
    literal_lengths: &Huffman,
    distances: &Huffman,
) -> Result<(), InflateError> {
    loop {
        let symbol = literal_lengths.decode(reader)?;
        match symbol {
            0..=255 => {
                ensure_output_capacity(output.len(), 1, max_output, reader)?;
                output.push(symbol as u8);
            }
            256 => return Ok(()),
            257..=285 => {
                let length_index = usize::from(symbol - 257);
                let mut length = LENGTH_BASE[length_index];
                let length_extra = LENGTH_EXTRA_BITS[length_index];
                if length_extra != 0 {
                    length += reader.read_bits(length_extra)? as usize;
                }

                let distance_symbol = distances.decode(reader)?;
                if distance_symbol >= 30 {
                    return Err(reader.error("reserved distance symbol"));
                }
                let distance_index = usize::from(distance_symbol);
                let mut distance = DIST_BASE[distance_index];
                let distance_extra = DIST_EXTRA_BITS[distance_index];
                if distance_extra != 0 {
                    distance += reader.read_bits(distance_extra)? as usize;
                }
                if distance == 0 || distance > output.len() {
                    return Err(reader.error("back-reference exceeds decoded history"));
                }
                ensure_output_capacity(output.len(), length, max_output, reader)?;
                for _ in 0..length {
                    let byte = output[output.len() - distance];
                    output.push(byte);
                }
            }
            _ => return Err(reader.error("reserved literal/length symbol")),
        }
    }
}

fn ensure_output_capacity(
    current: usize,
    additional: usize,
    maximum: usize,
    reader: &BitReader<'_>,
) -> Result<(), InflateError> {
    if current
        .checked_add(additional)
        .is_none_or(|length| length > maximum)
    {
        Err(reader.error("decoded output exceeds configured limit"))
    } else {
        Ok(())
    }
}

fn inflate_error(offset: usize, reason: &'static str) -> InflateError {
    InflateError { offset, reason }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pdf_native::{Dictionary, Stream};

    fn decode_with_pdf_stream(data: &[u8]) -> Vec<u8> {
        let mut dict = Dictionary::new();
        dict.set("Filter", "FlateDecode");
        dict.set("Length", data.len() as i64);
        let stream = Stream::new(dict, data.to_vec());
        stream.get_plain_content().expect("decompress")
    }

    fn stored_wrapper_size(len: usize) -> usize {
        let blocks = if len == 0 {
            1
        } else {
            (len + 65_535 - 1) / 65_535
        };
        2 + 4 + len + blocks * 5
    }

    #[test]
    fn zlib_lz77_roundtrip_small() {
        let src = b"hello native flate";
        let encoded = zlib_deflate_parallel(src);
        let decoded = decode_with_pdf_stream(&encoded);
        assert_eq!(decoded, src);
        assert_eq!(
            zlib_inflate(&encoded, src.len()).expect("native inflate"),
            src
        );
    }

    #[test]
    fn zlib_lz77_roundtrip_large_repetitive() {
        let src = vec![0xAB; 200_000];
        let encoded = zlib_deflate_parallel(&src);
        let decoded = decode_with_pdf_stream(&encoded);
        assert_eq!(decoded, src);
        assert_eq!(
            zlib_inflate(&encoded, src.len()).expect("native inflate"),
            src
        );
    }

    #[test]
    fn zlib_compiled_flow_chain_levels_roundtrip_deterministically() {
        let src = b"BT /F1 10 Tf 72 720 Td [(REFLOW-000001)] TJ ET\n".repeat(2_000);
        for steps in [1, 4, 8, 16, 32, 64] {
            let a = zlib_deflate_with_chain(&src, steps);
            let b = zlib_deflate_with_chain(&src, steps);
            assert_eq!(a, b, "deterministic at {steps} chain steps");
            assert_eq!(
                zlib_inflate(&a, src.len()).expect("native inflate"),
                src,
                "roundtrip at {steps} chain steps",
            );
        }
    }

    #[test]
    fn compiled_flow_compression_modes_are_per_call_and_deterministic() {
        use crate::pdf::CompiledFlowCompression::{Compact, Throughput};

        let src = (0..8_000)
            .map(|index| format!("BT /F1 10 Tf 72 720 Td [(RECORD-{index:05})] TJ ET\n"))
            .collect::<String>()
            .into_bytes();
        let throughput = zlib_deflate_compiled_flow(&src, Throughput);
        let compact = zlib_deflate_compiled_flow(&src, Compact);
        assert_eq!(throughput, zlib_deflate_compiled_flow(&src, Throughput));
        assert_eq!(compact, zlib_deflate_compiled_flow(&src, Compact));
        assert_eq!(
            zlib_inflate(&throughput, src.len()).expect("throughput roundtrip"),
            src
        );
        assert_eq!(
            zlib_inflate(&compact, src.len()).expect("compact roundtrip"),
            src
        );
        assert!(
            compact.len() <= throughput.len(),
            "compact search should not produce a larger representative stream"
        );
    }

    #[test]
    fn zlib_lz77_roundtrip_empty() {
        let src: Vec<u8> = Vec::new();
        let encoded = zlib_deflate_parallel(&src);
        let decoded = decode_with_pdf_stream(&encoded);
        assert_eq!(decoded, src);
        assert_eq!(zlib_inflate(&encoded, 0).expect("native inflate"), src);
    }

    #[test]
    fn zlib_lz77_beats_stored_on_repetitive_payload() {
        let src = vec![b'X'; 80_000];
        let encoded = zlib_deflate_parallel(&src);
        let stored = stored_wrapper_size(src.len());
        assert!(
            encoded.len() < stored,
            "expected compressed({}) < stored({})",
            encoded.len(),
            stored
        );
    }

    #[test]
    fn zlib_lz77_is_deterministic() {
        let src: Vec<u8> = (0..250_000).map(|i| (i % 251) as u8).collect();
        let a = zlib_deflate_parallel(&src);
        let b = zlib_deflate_parallel(&src);
        assert_eq!(a, b);
    }

    #[test]
    fn zlib_lz77_is_deterministic_across_thread_counts() {
        let src: Vec<u8> = (0..320_000).map(|i| (i % 239) as u8).collect();
        let run_with_threads = |threads: usize| -> Vec<u8> {
            crate::parallel::with_thread_count(threads, || zlib_deflate_parallel(&src))
        };
        let a = run_with_threads(1);
        let b = run_with_threads(4);
        assert_eq!(a, b);
    }

    #[test]
    fn adler_combine_matches_serial() {
        let data: Vec<u8> = (0..200_000).map(|i| (i % 251) as u8).collect();
        let serial = AdlerPartial::for_bytes(&data).to_adler32();
        let parallel = adler32_parallel(&data, 4096);
        assert_eq!(parallel, serial);
    }

    #[test]
    fn inflate_reads_dynamic_huffman_streams() {
        let compressed = decode_hex(concat!(
            "78daedcadb1182301400d1566e05f4a4100451a221c147f53ab6e0eff9dcd953",
            "a714f736f74b1c4b7eac31e6679cdbf5b645de5389fadd97c3fb15433e75bf82",
            "6118866118866118866118866118866118866118866118866118866118866118",
            "8661188661188661188661188661188661188661188661188661188661188661",
            "1886611886611886611886611886611886611886611886611886611886611886",
            "6118866118866118866118866118866118866118866118866118866118866118",
            "86e13ff00720c7990a",
        ));
        let expected = b"the quick brown fox jumps over the lazy dog. ".repeat(1000);
        assert_eq!(
            zlib_inflate(&compressed, expected.len()).expect("dynamic Huffman inflate"),
            expected
        );
    }

    #[test]
    fn inflate_rejects_checksum_corruption_and_output_bombs() {
        let source = b"bounded output".repeat(100);
        let mut compressed = zlib_deflate_parallel(&source);
        assert!(zlib_inflate(&compressed, source.len() - 1).is_err());
        let last = compressed.len() - 1;
        compressed[last] ^= 1;
        assert!(zlib_inflate(&compressed, source.len()).is_err());
    }

    fn decode_hex(input: &str) -> Vec<u8> {
        input
            .as_bytes()
            .chunks_exact(2)
            .map(|pair| {
                let text = std::str::from_utf8(pair).expect("ASCII hex");
                u8::from_str_radix(text, 16).expect("hex byte")
            })
            .collect()
    }
}
