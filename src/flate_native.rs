use rayon::prelude::*;

const ADLER_BASE: u32 = 65_521;
const DEFAULT_ADLER_CHUNK: usize = 1 << 20;

const LZ77_CHUNK_BYTES: usize = 128 * 1024;
const MIN_MATCH: usize = 3;
const MAX_MATCH: usize = 258;
const MAX_DISTANCE: usize = 32 * 1024;
const MAX_CHAIN_STEPS: usize = 64;
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
        let mut a: u32 = 1;
        let mut b: u32 = 0;
        for &byte in data {
            a += byte as u32;
            if a >= ADLER_BASE {
                a -= ADLER_BASE;
            }
            b += a;
            b %= ADLER_BASE;
        }
        Self {
            a,
            b,
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
    let partials: Vec<AdlerPartial> = ranges
        .par_iter()
        .map(|(start, end)| AdlerPartial::for_bytes(&data[*start..*end]))
        .collect();

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
    while l < max_len && data[a + l] == data[b + l] {
        l += 1;
    }
    l
}

fn plan_lz77_chunk(data: &[u8]) -> ChunkPlan {
    let n = data.len();
    if n == 0 {
        return ChunkPlan { tokens: Vec::new() };
    }

    let mut head = vec![-1_i32; HASH_SIZE];
    let mut prev = vec![-1_i32; n];
    let mut tokens = Vec::with_capacity(n / 2);

    let mut i = 0usize;
    while i < n {
        if i + MIN_MATCH > n {
            tokens.push(Token::Literal(data[i]));
            i += 1;
            continue;
        }

        let h = hash3(data, i);
        let mut cand = head[h];
        prev[i] = cand;
        head[h] = i as i32;

        let mut best_len = 0usize;
        let mut best_dist = 0usize;
        let mut steps = 0usize;

        while cand >= 0 && steps < MAX_CHAIN_STEPS {
            let c = cand as usize;
            let dist = i - c;
            if dist > MAX_DISTANCE {
                break;
            }

            if data[c] == data[i] && data[c + 1] == data[i + 1] && data[c + 2] == data[i + 2] {
                let max_len = MAX_MATCH.min(n - i);
                let len = match_len(data, c, i, max_len);
                if len >= MIN_MATCH && (len > best_len || (len == best_len && dist < best_dist)) {
                    best_len = len;
                    best_dist = dist;
                    if best_len == MAX_MATCH {
                        break;
                    }
                }
            }

            cand = prev[c];
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
                    prev[j] = head[hj];
                    head[hj] = j as i32;
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
    let ranges = chunk_ranges(data.len(), LZ77_CHUNK_BYTES);

    let plans: Vec<ChunkPlan> = ranges
        .par_iter()
        .map(|(start, end)| plan_lz77_chunk(&data[*start..*end]))
        .collect();

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

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::{Dictionary, Stream};

    fn decode_with_lopdf_zlib(data: &[u8]) -> Vec<u8> {
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
        let decoded = decode_with_lopdf_zlib(&encoded);
        assert_eq!(decoded, src);
    }

    #[test]
    fn zlib_lz77_roundtrip_large_repetitive() {
        let src = vec![0xAB; 200_000];
        let encoded = zlib_deflate_parallel(&src);
        let decoded = decode_with_lopdf_zlib(&encoded);
        assert_eq!(decoded, src);
    }

    #[test]
    fn zlib_lz77_roundtrip_empty() {
        let src: Vec<u8> = Vec::new();
        let encoded = zlib_deflate_parallel(&src);
        let decoded = decode_with_lopdf_zlib(&encoded);
        assert_eq!(decoded, src);
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
            let pool = rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .build()
                .expect("thread pool");
            pool.install(|| zlib_deflate_parallel(&src))
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
}
