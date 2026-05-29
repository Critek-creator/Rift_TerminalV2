//! GGUF header inspector — reads a GGUF model file's metadata block to
//! recover the architecture facts a VRAM estimate needs (layer count, hidden
//! size, attention head counts, expert count, total parameters) WITHOUT
//! loading any tensor weights.
//!
//! Only the metadata key/value section at the start of the file is read — the
//! parser stops before the tensor data, so inspecting a 20 GB model touches a
//! few hundred KB at most. This replaces the frontend's filename-heuristic
//! guessing (param count parsed from the filename, a hardcoded GQA factor, and
//! an architecture lookup table) with measured truth when the file is present.
//!
//! Format reference: GGUF v2/v3 (llama.cpp). Lives inside the §9 translator
//! boundary; reads a local file only (no external-system primitives).

use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

use serde::Serialize;

/// Architecture facts pulled from a GGUF header. All counts are `Option`
/// because not every model writes every key; the frontend falls back to its
/// filename heuristic for any field that is `None`.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct GgufMeta {
    /// `general.architecture`, e.g. "llama", "gemma2", "qwen3moe".
    pub architecture: Option<String>,
    /// `{arch}.block_count` — transformer layer count.
    pub n_layers: Option<u32>,
    /// `{arch}.embedding_length` — hidden size.
    pub n_embd: Option<u32>,
    /// `{arch}.attention.head_count` — query head count.
    pub n_head: Option<u32>,
    /// `{arch}.attention.head_count_kv` — KV head count (grouped-query
    /// attention: kv heads ≤ query heads). The exact GQA ratio is
    /// `n_head_kv / n_head`, replacing the frontend's 0.25 estimate.
    pub n_head_kv: Option<u32>,
    /// `{arch}.expert_count` — number of MoE experts (absent for dense models).
    pub expert_count: Option<u32>,
    /// `general.parameter_count` — total parameters, when the writer recorded
    /// it. Used for an exact weights estimate instead of the filename guess.
    pub parameter_count: Option<u64>,
}

#[derive(Debug)]
pub enum GgufError {
    Io(std::io::Error),
    NotGguf,
    Malformed(&'static str),
}

impl std::fmt::Display for GgufError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GgufError::Io(e) => write!(f, "gguf: io error: {e}"),
            GgufError::NotGguf => write!(f, "gguf: not a GGUF file (bad magic)"),
            GgufError::Malformed(m) => write!(f, "gguf: malformed header: {m}"),
        }
    }
}

impl std::error::Error for GgufError {}

impl From<std::io::Error> for GgufError {
    fn from(e: std::io::Error) -> Self {
        GgufError::Io(e)
    }
}

// GGUF metadata value type tags.
const T_UINT8: u32 = 0;
const T_INT8: u32 = 1;
const T_UINT16: u32 = 2;
const T_INT16: u32 = 3;
const T_UINT32: u32 = 4;
const T_INT32: u32 = 5;
const T_FLOAT32: u32 = 6;
const T_BOOL: u32 = 7;
const T_STRING: u32 = 8;
const T_ARRAY: u32 = 9;
const T_UINT64: u32 = 10;
const T_INT64: u32 = 11;
const T_FLOAT64: u32 = 12;

// Defensive caps so a corrupt or hostile file can't make us allocate wildly or
// loop forever. Real models stay far under these.
const MAX_KV_COUNT: u64 = 1_000_000;
const MAX_STRING_LEN: u64 = 64 * 1024 * 1024; // 64 MiB
const MAX_ARRAY_LEN: u64 = 100_000_000;

/// A single scalar value we care about. Arrays and unhandled types are skipped.
enum Scalar {
    U64(u64),
    Str(String),
    /// Parsed-and-discarded (array, bool, float we don't use, etc.).
    Skipped,
}

/// Inspect a GGUF file's metadata header. Reads only the KV section.
pub fn inspect(path: &Path) -> Result<GgufMeta, GgufError> {
    let file = File::open(path)?;
    let mut r = BufReader::new(file);

    let magic = read_u32(&mut r)?;
    // "GGUF" little-endian.
    if magic != 0x4655_4747 {
        return Err(GgufError::NotGguf);
    }
    let _version = read_u32(&mut r)?;
    let _tensor_count = read_u64(&mut r)?;
    let kv_count = read_u64(&mut r)?;
    if kv_count > MAX_KV_COUNT {
        return Err(GgufError::Malformed("kv_count exceeds sane maximum"));
    }

    let mut meta = GgufMeta::default();
    // Architecture comes first in practice, but don't rely on ordering — we
    // resolve arch-prefixed keys against whatever architecture we've seen.
    let mut arch: Option<String> = None;

    for _ in 0..kv_count {
        let key = read_string(&mut r)?;
        let value = read_value(&mut r)?;

        match (key.as_str(), value) {
            ("general.architecture", Scalar::Str(s)) => {
                arch = Some(s.clone());
                meta.architecture = Some(s);
            }
            ("general.parameter_count", Scalar::U64(n)) => {
                meta.parameter_count = Some(n);
            }
            (k, Scalar::U64(n)) => {
                // Arch-prefixed scalar keys. Match on the suffix so we don't
                // need to know the architecture name ahead of time.
                if let Some(suffix) = arch.as_deref().and_then(|a| k.strip_prefix(a)) {
                    let n32 = n as u32;
                    match suffix {
                        ".block_count" => meta.n_layers = Some(n32),
                        ".embedding_length" => meta.n_embd = Some(n32),
                        ".attention.head_count" => meta.n_head = Some(n32),
                        ".attention.head_count_kv" => meta.n_head_kv = Some(n32),
                        ".expert_count" => meta.expert_count = Some(n32),
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    Ok(meta)
}

// --- primitive readers -----------------------------------------------------

fn read_exact<R: Read>(r: &mut R, buf: &mut [u8]) -> Result<(), GgufError> {
    r.read_exact(buf).map_err(GgufError::Io)
}

fn read_u32<R: Read>(r: &mut R) -> Result<u32, GgufError> {
    let mut b = [0u8; 4];
    read_exact(r, &mut b)?;
    Ok(u32::from_le_bytes(b))
}

fn read_u64<R: Read>(r: &mut R) -> Result<u64, GgufError> {
    let mut b = [0u8; 8];
    read_exact(r, &mut b)?;
    Ok(u64::from_le_bytes(b))
}

fn read_string<R: Read>(r: &mut R) -> Result<String, GgufError> {
    let len = read_u64(r)?;
    if len > MAX_STRING_LEN {
        return Err(GgufError::Malformed("string length exceeds sane maximum"));
    }
    let mut buf = vec![0u8; len as usize];
    read_exact(r, &mut buf)?;
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

/// Discard `n` bytes from the reader without allocating the whole span.
fn skip<R: Read>(r: &mut R, n: u64) -> Result<(), GgufError> {
    let mut remaining = n;
    let mut scratch = [0u8; 8192];
    while remaining > 0 {
        let take = remaining.min(scratch.len() as u64) as usize;
        read_exact(r, &mut scratch[..take])?;
        remaining -= take as u64;
    }
    Ok(())
}

/// Byte width of a fixed-size scalar type, or `None` for variable-width types
/// (string / array) which must be read element-by-element.
fn fixed_width(t: u32) -> Option<u64> {
    match t {
        T_UINT8 | T_INT8 | T_BOOL => Some(1),
        T_UINT16 | T_INT16 => Some(2),
        T_UINT32 | T_INT32 | T_FLOAT32 => Some(4),
        T_UINT64 | T_INT64 | T_FLOAT64 => Some(8),
        _ => None,
    }
}

/// Read one metadata value, returning the scalar kinds we care about and
/// skipping everything else (including arrays — we never need them, but we
/// must consume their bytes to stay aligned for the next key).
fn read_value<R: Read>(r: &mut R) -> Result<Scalar, GgufError> {
    let t = read_u32(r)?;
    match t {
        T_UINT32 => Ok(Scalar::U64(read_u32(r)? as u64)),
        T_UINT64 => Ok(Scalar::U64(read_u64(r)?)),
        T_INT32 => {
            // Counts are non-negative; reinterpret but guard negatives to 0.
            let v = read_u32(r)? as i32;
            Ok(Scalar::U64(v.max(0) as u64))
        }
        T_INT64 => {
            let v = read_u64(r)? as i64;
            Ok(Scalar::U64(v.max(0) as u64))
        }
        T_UINT16 => Ok(Scalar::U64(read_u16(r)? as u64)),
        T_INT16 => {
            let mut b = [0u8; 2];
            read_exact(r, &mut b)?;
            Ok(Scalar::U64(i16::from_le_bytes(b).max(0) as u64))
        }
        T_UINT8 => {
            let mut b = [0u8; 1];
            read_exact(r, &mut b)?;
            Ok(Scalar::U64(b[0] as u64))
        }
        T_INT8 => {
            let mut b = [0u8; 1];
            read_exact(r, &mut b)?;
            Ok(Scalar::U64((b[0] as i8).max(0) as u64))
        }
        T_STRING => Ok(Scalar::Str(read_string(r)?)),
        T_BOOL => {
            skip(r, 1)?;
            Ok(Scalar::Skipped)
        }
        T_FLOAT32 => {
            skip(r, 4)?;
            Ok(Scalar::Skipped)
        }
        T_FLOAT64 => {
            skip(r, 8)?;
            Ok(Scalar::Skipped)
        }
        T_ARRAY => {
            read_array(r)?;
            Ok(Scalar::Skipped)
        }
        _ => Err(GgufError::Malformed("unknown value type tag")),
    }
}

fn read_u16<R: Read>(r: &mut R) -> Result<u16, GgufError> {
    let mut b = [0u8; 2];
    read_exact(r, &mut b)?;
    Ok(u16::from_le_bytes(b))
}

/// Consume an array value (element type + count + elements) without retaining
/// it. Handles the common tokenizer arrays (string arrays can be large) by
/// reading each element's bytes sequentially.
fn read_array<R: Read>(r: &mut R) -> Result<(), GgufError> {
    let elem_type = read_u32(r)?;
    let count = read_u64(r)?;
    if count > MAX_ARRAY_LEN {
        return Err(GgufError::Malformed("array length exceeds sane maximum"));
    }
    if let Some(width) = fixed_width(elem_type) {
        skip(r, width.saturating_mul(count))?;
    } else if elem_type == T_STRING {
        for _ in 0..count {
            let len = read_u64(r)?;
            if len > MAX_STRING_LEN {
                return Err(GgufError::Malformed("array string exceeds sane maximum"));
            }
            skip(r, len)?;
        }
    } else {
        // Nested arrays aren't used by real models; bail rather than guess.
        return Err(GgufError::Malformed("nested or unknown array element type"));
    }
    Ok(())
}

// --- tests -----------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    // Minimal GGUF byte-buffer builder for tests.
    struct GgufBuilder {
        kv: Vec<u8>,
        count: u64,
    }

    impl GgufBuilder {
        fn new() -> Self {
            Self {
                kv: Vec::new(),
                count: 0,
            }
        }
        fn key(&mut self, k: &str) {
            self.kv.extend_from_slice(&(k.len() as u64).to_le_bytes());
            self.kv.extend_from_slice(k.as_bytes());
        }
        fn u32_kv(&mut self, k: &str, v: u32) {
            self.key(k);
            self.kv.extend_from_slice(&T_UINT32.to_le_bytes());
            self.kv.extend_from_slice(&v.to_le_bytes());
            self.count += 1;
        }
        fn u64_kv(&mut self, k: &str, v: u64) {
            self.key(k);
            self.kv.extend_from_slice(&T_UINT64.to_le_bytes());
            self.kv.extend_from_slice(&v.to_le_bytes());
            self.count += 1;
        }
        fn str_kv(&mut self, k: &str, v: &str) {
            self.key(k);
            self.kv.extend_from_slice(&T_STRING.to_le_bytes());
            self.kv.extend_from_slice(&(v.len() as u64).to_le_bytes());
            self.kv.extend_from_slice(v.as_bytes());
            self.count += 1;
        }
        /// A string array (e.g. tokenizer tokens) the parser must skip cleanly.
        fn str_array_kv(&mut self, k: &str, items: &[&str]) {
            self.key(k);
            self.kv.extend_from_slice(&T_ARRAY.to_le_bytes());
            self.kv.extend_from_slice(&T_STRING.to_le_bytes());
            self.kv
                .extend_from_slice(&(items.len() as u64).to_le_bytes());
            for it in items {
                self.kv.extend_from_slice(&(it.len() as u64).to_le_bytes());
                self.kv.extend_from_slice(it.as_bytes());
            }
            self.count += 1;
        }
        fn build(self) -> Vec<u8> {
            let mut out = Vec::new();
            out.extend_from_slice(&0x4655_4747u32.to_le_bytes()); // "GGUF"
            out.extend_from_slice(&3u32.to_le_bytes()); // version
            out.extend_from_slice(&0u64.to_le_bytes()); // tensor_count
            out.extend_from_slice(&self.count.to_le_bytes()); // kv_count
            out.extend_from_slice(&self.kv);
            out
        }
    }

    fn write_temp(bytes: &[u8], name: &str) -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("rift-gguf-test-{name}.gguf"));
        let mut f = File::create(&p).unwrap();
        f.write_all(bytes).unwrap();
        p
    }

    #[test]
    fn parses_dense_model_metadata() {
        let mut b = GgufBuilder::new();
        b.str_kv("general.architecture", "llama");
        b.u64_kv("general.parameter_count", 8_030_000_000);
        b.u32_kv("llama.block_count", 32);
        b.u32_kv("llama.embedding_length", 4096);
        b.u32_kv("llama.attention.head_count", 32);
        b.u32_kv("llama.attention.head_count_kv", 8);
        // A tokenizer array in the middle the parser must skip to reach later keys.
        b.str_array_kv("tokenizer.ggml.tokens", &["<s>", "</s>", "hello", "world"]);
        let path = write_temp(&b.build(), "dense");

        let meta = inspect(&path).expect("parse");
        assert_eq!(meta.architecture.as_deref(), Some("llama"));
        assert_eq!(meta.parameter_count, Some(8_030_000_000));
        assert_eq!(meta.n_layers, Some(32));
        assert_eq!(meta.n_embd, Some(4096));
        assert_eq!(meta.n_head, Some(32));
        assert_eq!(meta.n_head_kv, Some(8));
        assert_eq!(meta.expert_count, None);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn parses_moe_expert_count() {
        let mut b = GgufBuilder::new();
        b.str_kv("general.architecture", "qwen3moe");
        b.u32_kv("qwen3moe.block_count", 48);
        b.u32_kv("qwen3moe.expert_count", 128);
        let path = write_temp(&b.build(), "moe");

        let meta = inspect(&path).expect("parse");
        assert_eq!(meta.architecture.as_deref(), Some("qwen3moe"));
        assert_eq!(meta.n_layers, Some(48));
        assert_eq!(meta.expert_count, Some(128));

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn rejects_non_gguf() {
        let path = write_temp(b"NOTGGUF.................", "bad");
        assert!(matches!(inspect(&path), Err(GgufError::NotGguf)));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn missing_keys_yield_none_not_error() {
        let mut b = GgufBuilder::new();
        b.str_kv("general.architecture", "phi3");
        // No block_count / embedding_length etc.
        let path = write_temp(&b.build(), "sparse");

        let meta = inspect(&path).expect("parse");
        assert_eq!(meta.architecture.as_deref(), Some("phi3"));
        assert_eq!(meta.n_layers, None);
        assert_eq!(meta.n_embd, None);

        let _ = std::fs::remove_file(&path);
    }
}
