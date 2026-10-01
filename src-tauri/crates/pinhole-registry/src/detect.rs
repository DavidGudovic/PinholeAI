//! Header sniffing (SPEC §6 step 3). Reads ONLY the safetensors JSON header or
//! GGUF metadata/tensor-info — never tensor data. Mirrors `get_sd_version()` in
//! stable-diffusion.cpp `src/model_loader.cpp` (pinned engine version). The
//! loader prefixes standalone diffusion files with `model.diffusion_model.`, so
//! rules match on tensor-name suffixes as well as prefixes.
//!
//! The per-family rules live in `config/models.yaml → families.*.detect`
//! (see [`crate::DetectRules`] for the pattern syntax). This module only
//! parses headers and evaluates those rules; it also recognises LoRAs,
//! all-in-one checkpoints and standalone components with fixed heuristics.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::{DetectRules, Family, Layout, Registry};

/// Hard cap for a safetensors JSON header.
/// Also caps the number of bytes read for a GGUF header (metadata + tensor infos).
pub const MAX_HEADER_BYTES: u64 = 100 * 1024 * 1024;

/// Longest GGUF key / tensor name we accept.
const MAX_GGUF_NAME: u64 = 64 * 1024;
/// Longest single GGUF string value (chat templates are ~10 KB).
const MAX_GGUF_STRING: u64 = 16 * 1024 * 1024;
const MAX_GGUF_TENSORS: u64 = 1_000_000;
const MAX_GGUF_KV: u64 = 1_000_000;
/// Tensors have at most 4 dims in ggml; GGUF files may stack a few more.
const MAX_DIMS: usize = 16;
/// GGUF arrays of arrays: nesting limit.
const MAX_ARRAY_DEPTH: u32 = 4;
/// Metadata values are kept for display/diagnostics only; long ones are cut.
const MAX_META_VALUE_CHARS: usize = 1024;

/// Strip this before matching rules (the engine adds it to `--diffusion-model` files).
const DIFFUSION_PREFIX: &str = "model.diffusion_model.";

#[derive(Debug, thiserror::Error)]
pub enum DetectError {
    #[error("could not read file: {0}")]
    Io(#[from] std::io::Error),
    #[error("not a safetensors or GGUF file")]
    UnknownFormat,
    #[error("header too large or malformed: {0}")]
    Malformed(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileFormat {
    Safetensors,
    Gguf,
}

/// One tensor from the header (no data).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TensorInfo {
    pub name: String,
    /// Lowercase dtype / ggml type name (`f16`, `bf16`, `f8_e4m3`, `q4_k`…).
    pub dtype: String,
    /// Dimensions in ggml order: `ne[0]` is the innermost (last PyTorch) dim.
    pub ne: Vec<u64>,
    /// Size of the tensor data in bytes.
    pub bytes: u64,
}

/// What we learned from the header.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeaderInfo {
    pub format: FileFormat,
    pub tensor_names: Vec<String>,
    /// Dominant dtype / quant: `f32`, `f16`, `bf16`, `f8_e4m3`, `q8_0`, `q4_k`…
    pub dtype: String,
    pub file_size: u64,
    /// Sum of tensor byte sizes (≈ weights size).
    pub tensor_bytes: u64,
    /// GGUF `general.architecture` etc., when present.
    #[serde(default)]
    pub metadata: std::collections::BTreeMap<String, String>,
    /// Per-tensor dtype/shape/size, same order as `tensor_names`. May be empty
    /// when a caller builds a `HeaderInfo` from names only; shape rules then pass.
    #[serde(default)]
    pub tensors: Vec<TensorInfo>,
}

/// Result of matching a header against every family's `detect` rules.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Detection {
    /// Candidate family ids, best first. Empty = unknown. More than one = ask the user
    /// (after known-hash and CivitAI lookups failed).
    pub candidates: Vec<String>,
    pub layout: Layout,
    pub has_vae: bool,
    pub has_text_encoders: bool,
    pub dtype: String,
    /// `true` when the file is a LoRA rather than a base model.
    pub is_lora: bool,
    /// `Some(kind)` when the file looks like a standalone component (best effort):
    /// `vae` | `taesd` | `clip_l` | `clip_g` | `t5xxl` | `llm` | `llm_vision`.
    pub is_component: Option<String>,
}

// ============================================================================
// Reading
// ============================================================================

/// Read and parse just the header of a `.safetensors` or `.gguf` file.
/// Reads at most `8 + MAX_HEADER_BYTES` bytes; never touches tensor data.
pub fn read_header(path: &Path) -> Result<HeaderInfo, DetectError> {
    let mut f = File::open(path)?;
    let file_size = f.metadata()?.len();
    if file_size < 8 {
        return Err(DetectError::UnknownFormat);
    }
    let mut head = [0u8; 9];
    let head_len = if file_size >= 9 { 9 } else { 8 };
    f.read_exact(&mut head[..head_len])?;

    if &head[..4] == b"GGUF" {
        // Re-read from the start through a bounded, buffered reader.
        let chain = (&head[..head_len]).chain(BufReader::with_capacity(64 * 1024, f));
        return parse_gguf(chain, file_size);
    }

    let n = safetensors_precheck(&head[..head_len], file_size)?;
    let total =
        usize::try_from(8 + n).map_err(|_| DetectError::Malformed("header too large".into()))?;
    let mut buf = vec![0u8; total];
    buf[..head_len].copy_from_slice(&head[..head_len]);
    f.read_exact(&mut buf[head_len..])
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::UnexpectedEof => {
                DetectError::Malformed("safetensors header is truncated".into())
            }
            _ => DetectError::Io(e),
        })?;
    parse_safetensors(&buf, file_size)
}

/// Parse a header from bytes (tests / already-open files). `bytes` starts at
/// file offset 0 and holds at least the whole header; `file_size` is the size
/// of the whole file (`0` = unknown: data-range checks are skipped).
pub fn parse_header_bytes(bytes: &[u8], file_size: u64) -> Result<HeaderInfo, DetectError> {
    if bytes.len() >= 4 && &bytes[..4] == b"GGUF" {
        return parse_gguf(bytes, file_size);
    }
    parse_safetensors(bytes, file_size)
}

/// Checks the 8-byte length prefix and the opening `{`. Returns the header length.
fn safetensors_precheck(head: &[u8], file_size: u64) -> Result<u64, DetectError> {
    if head.len() < 9 || head[8] != b'{' {
        return Err(DetectError::UnknownFormat);
    }
    let mut len = [0u8; 8];
    len.copy_from_slice(&head[..8]);
    let n = u64::from_le_bytes(len);
    if n > MAX_HEADER_BYTES {
        return Err(DetectError::Malformed(format!(
            "safetensors header is {n} bytes (limit {MAX_HEADER_BYTES})"
        )));
    }
    if n < 2 {
        return Err(DetectError::Malformed("safetensors header is empty".into()));
    }
    if file_size > 0 && n > file_size.saturating_sub(8) {
        return Err(DetectError::Malformed(
            "safetensors header runs past the end of the file".into(),
        ));
    }
    Ok(n)
}

fn parse_safetensors(bytes: &[u8], file_size: u64) -> Result<HeaderInfo, DetectError> {
    let n = safetensors_precheck(bytes, file_size)?;
    let end = 8usize
        .checked_add(
            usize::try_from(n).map_err(|_| DetectError::Malformed("header too large".into()))?,
        )
        .ok_or_else(|| DetectError::Malformed("header too large".into()))?;
    if bytes.len() < end {
        return Err(DetectError::Malformed(
            "safetensors header is truncated".into(),
        ));
    }
    let json: serde_json::Value = serde_json::from_slice(&bytes[8..end]).map_err(|e| {
        DetectError::Malformed(format!("safetensors header is not valid JSON: {e}"))
    })?;
    let obj = json
        .as_object()
        .ok_or_else(|| DetectError::Malformed("safetensors header is not a JSON object".into()))?;

    // Bytes available for tensor data, when the file size is known.
    let data_len = if file_size > 0 {
        Some(file_size - 8 - n)
    } else {
        None
    };

    let mut metadata = BTreeMap::new();
    let mut tensors = Vec::with_capacity(obj.len());
    for (name, entry) in obj {
        if name == "__metadata__" {
            if let Some(m) = entry.as_object() {
                for (k, v) in m {
                    if let Some(s) = v.as_str() {
                        metadata.insert(k.clone(), truncate_chars(s, MAX_META_VALUE_CHARS));
                    }
                }
            }
            continue;
        }
        let bad = |what: &str| {
            DetectError::Malformed(format!("tensor `{}`: {what}", truncate_chars(name, 120)))
        };
        let t = entry
            .as_object()
            .ok_or_else(|| bad("entry is not an object"))?;
        let dtype = t
            .get("dtype")
            .and_then(|d| d.as_str())
            .ok_or_else(|| bad("missing dtype"))?;
        let shape = t
            .get("shape")
            .and_then(|s| s.as_array())
            .ok_or_else(|| bad("missing shape"))?;
        if shape.len() > MAX_DIMS {
            return Err(bad("too many dimensions"));
        }
        let mut dims = Vec::with_capacity(shape.len());
        for d in shape {
            dims.push(
                d.as_u64()
                    .ok_or_else(|| bad("shape is not a list of non-negative integers"))?,
            );
        }
        let offsets = t
            .get("data_offsets")
            .and_then(|o| o.as_array())
            .ok_or_else(|| bad("missing data_offsets"))?;
        if offsets.len() != 2 {
            return Err(bad("data_offsets must have two entries"));
        }
        let begin = offsets[0].as_u64().ok_or_else(|| bad("bad data_offsets"))?;
        let stop = offsets[1].as_u64().ok_or_else(|| bad("bad data_offsets"))?;
        if stop < begin {
            return Err(bad("data_offsets end before they begin"));
        }
        if let Some(avail) = data_len {
            if stop > avail {
                return Err(bad("tensor data runs past the end of the file"));
            }
        }
        dims.iter()
            .try_fold(1u64, |acc, d| acc.checked_mul(*d))
            .ok_or_else(|| bad("shape overflows"))?;
        dims.reverse(); // PyTorch order → ggml `ne` order
        tensors.push(TensorInfo {
            name: name.clone(),
            dtype: dtype.to_ascii_lowercase(),
            ne: dims,
            bytes: stop - begin,
        });
    }
    Ok(finish(
        FileFormat::Safetensors,
        tensors,
        metadata,
        file_size,
    ))
}

fn finish(
    format: FileFormat,
    tensors: Vec<TensorInfo>,
    metadata: BTreeMap<String, String>,
    file_size: u64,
) -> HeaderInfo {
    let mut by_dtype: BTreeMap<&str, u64> = BTreeMap::new();
    let mut total = 0u64;
    for t in &tensors {
        total = total.saturating_add(t.bytes);
        let e = by_dtype.entry(t.dtype.as_str()).or_default();
        *e = e.saturating_add(t.bytes);
    }
    // Largest share of bytes wins; ties go to the alphabetically first name.
    let dtype = by_dtype
        .iter()
        .fold(None::<(&str, u64)>, |best, (d, b)| match best {
            Some((_, bb)) if bb >= *b => best,
            _ => Some((d, *b)),
        })
        .map(|(d, _)| d.to_owned())
        .unwrap_or_else(|| "unknown".to_owned());
    HeaderInfo {
        format,
        tensor_names: tensors.iter().map(|t| t.name.clone()).collect(),
        dtype,
        file_size,
        tensor_bytes: total,
        metadata,
        tensors,
    }
}

fn truncate_chars(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((i, _)) => format!("{}…", &s[..i]),
        None => s.to_owned(),
    }
}

// ---------------------------------------------------------------------------- GGUF

/// A reader that refuses to consume more than `limit` bytes.
struct Bounded<R: Read> {
    r: R,
    used: u64,
    limit: u64,
}

impl<R: Read> Bounded<R> {
    fn remaining(&self) -> u64 {
        self.limit - self.used
    }

    fn take_budget(&mut self, n: u64) -> Result<(), DetectError> {
        if n > self.remaining() {
            return Err(DetectError::Malformed(if self.limit >= MAX_HEADER_BYTES {
                format!("GGUF header is larger than {MAX_HEADER_BYTES} bytes")
            } else {
                "GGUF header runs past the end of the file".into()
            }));
        }
        self.used += n;
        Ok(())
    }

    fn fill(&mut self, buf: &mut [u8]) -> Result<(), DetectError> {
        self.take_budget(buf.len() as u64)?;
        self.r.read_exact(buf).map_err(|e| match e.kind() {
            std::io::ErrorKind::UnexpectedEof => {
                DetectError::Malformed("GGUF header is truncated".into())
            }
            _ => DetectError::Io(e),
        })
    }

    fn skip(&mut self, n: u64) -> Result<(), DetectError> {
        self.take_budget(n)?;
        let copied = std::io::copy(&mut (&mut self.r).take(n), &mut std::io::sink())?;
        if copied != n {
            return Err(DetectError::Malformed("GGUF header is truncated".into()));
        }
        Ok(())
    }

    fn u8(&mut self) -> Result<u8, DetectError> {
        let mut b = [0u8; 1];
        self.fill(&mut b)?;
        Ok(b[0])
    }
    fn u16(&mut self) -> Result<u16, DetectError> {
        let mut b = [0u8; 2];
        self.fill(&mut b)?;
        Ok(u16::from_le_bytes(b))
    }
    fn u32(&mut self) -> Result<u32, DetectError> {
        let mut b = [0u8; 4];
        self.fill(&mut b)?;
        Ok(u32::from_le_bytes(b))
    }
    fn u64(&mut self) -> Result<u64, DetectError> {
        let mut b = [0u8; 8];
        self.fill(&mut b)?;
        Ok(u64::from_le_bytes(b))
    }

    fn string(&mut self, max: u64, what: &str) -> Result<String, DetectError> {
        let len = self.u64()?;
        if len > max {
            return Err(DetectError::Malformed(format!(
                "GGUF {what} is {len} bytes long (limit {max})"
            )));
        }
        if len > self.remaining() {
            return Err(DetectError::Malformed("GGUF header is truncated".into()));
        }
        let mut buf = vec![0u8; len as usize];
        self.fill(&mut buf)?;
        Ok(String::from_utf8_lossy(&buf).into_owned())
    }
}

/// GGUF metadata value types (gguf.h `gguf_type`).
mod gguf_type {
    pub const UINT8: u32 = 0;
    pub const INT8: u32 = 1;
    pub const UINT16: u32 = 2;
    pub const INT16: u32 = 3;
    pub const UINT32: u32 = 4;
    pub const INT32: u32 = 5;
    pub const FLOAT32: u32 = 6;
    pub const BOOL: u32 = 7;
    pub const STRING: u32 = 8;
    pub const ARRAY: u32 = 9;
    pub const UINT64: u32 = 10;
    pub const INT64: u32 = 11;
    pub const FLOAT64: u32 = 12;

    pub fn name(t: u32) -> &'static str {
        match t {
            UINT8 => "u8",
            INT8 => "i8",
            UINT16 => "u16",
            INT16 => "i16",
            UINT32 => "u32",
            INT32 => "i32",
            FLOAT32 => "f32",
            BOOL => "bool",
            STRING => "string",
            ARRAY => "array",
            UINT64 => "u64",
            INT64 => "i64",
            FLOAT64 => "f64",
            _ => "?",
        }
    }

    pub fn fixed_size(t: u32) -> Option<u64> {
        match t {
            UINT8 | INT8 | BOOL => Some(1),
            UINT16 | INT16 => Some(2),
            UINT32 | INT32 | FLOAT32 => Some(4),
            UINT64 | INT64 | FLOAT64 => Some(8),
            _ => None,
        }
    }
}

/// `(name, block size, bytes per block)` for a ggml type id. Mirrors the
/// `type_traits` table in ggml/src/ggml.c (leejet/ggml @ 89c4413, the
/// stable-diffusion.cpp submodule) and the block structs in ggml-common.h.
pub fn ggml_type_info(t: u32) -> Option<(&'static str, u64, u64)> {
    Some(match t {
        0 => ("f32", 1, 4),
        1 => ("f16", 1, 2),
        2 => ("q4_0", 32, 18),
        3 => ("q4_1", 32, 20),
        6 => ("q5_0", 32, 22),
        7 => ("q5_1", 32, 24),
        8 => ("q8_0", 32, 34),
        9 => ("q8_1", 32, 36),
        10 => ("q2_k", 256, 84),
        11 => ("q3_k", 256, 110),
        12 => ("q4_k", 256, 144),
        13 => ("q5_k", 256, 176),
        14 => ("q6_k", 256, 210),
        15 => ("q8_k", 256, 292),
        16 => ("iq2_xxs", 256, 66),
        17 => ("iq2_xs", 256, 74),
        18 => ("iq3_xxs", 256, 98),
        19 => ("iq1_s", 256, 50),
        20 => ("iq4_nl", 32, 18),
        21 => ("iq3_s", 256, 110),
        22 => ("iq2_s", 256, 82),
        23 => ("iq4_xs", 256, 136),
        24 => ("i8", 1, 1),
        25 => ("i16", 1, 2),
        26 => ("i32", 1, 4),
        27 => ("i64", 1, 8),
        28 => ("f64", 1, 8),
        29 => ("iq1_m", 256, 56),
        30 => ("bf16", 1, 2),
        34 => ("tq1_0", 256, 54),
        35 => ("tq2_0", 256, 66),
        39 => ("mxfp4", 32, 17),
        40 => ("nvfp4", 64, 36),
        41 => ("q1_0", 128, 18),
        42 => ("q2_0", 64, 18),
        43 => ("f8_e4m3", 1, 1),
        44 => ("f8_e5m2", 1, 1),
        _ => return None,
    })
}

fn parse_gguf<R: Read>(r: R, file_size: u64) -> Result<HeaderInfo, DetectError> {
    let limit = if file_size > 0 {
        file_size.min(MAX_HEADER_BYTES)
    } else {
        MAX_HEADER_BYTES
    };
    let mut rd = Bounded { r, used: 0, limit };

    let mut magic = [0u8; 4];
    rd.fill(&mut magic)
        .map_err(|_| DetectError::UnknownFormat)?;
    if &magic != b"GGUF" {
        return Err(DetectError::UnknownFormat);
    }
    let version = rd.u32()?;
    if !(2..=3).contains(&version) {
        return Err(DetectError::Malformed(format!(
            "unsupported GGUF version {version}"
        )));
    }
    let n_tensors = rd.u64()?;
    let n_kv = rd.u64()?;
    if n_tensors > MAX_GGUF_TENSORS {
        return Err(DetectError::Malformed(format!(
            "GGUF claims {n_tensors} tensors"
        )));
    }
    if n_kv > MAX_GGUF_KV {
        return Err(DetectError::Malformed(format!(
            "GGUF claims {n_kv} metadata entries"
        )));
    }

    let mut metadata = BTreeMap::new();
    let mut alignment: u64 = 32;
    for _ in 0..n_kv {
        let key = rd.string(MAX_GGUF_NAME, "metadata key")?;
        let vtype = rd.u32()?;
        let value = read_gguf_value(&mut rd, vtype, 0)?;
        if key == "general.alignment" {
            if let Ok(a) = value.parse::<u64>() {
                if a != 0 && a.is_power_of_two() {
                    alignment = a;
                }
            }
        }
        metadata.insert(key, value);
    }

    let mut tensors = Vec::with_capacity(n_tensors.min(65_536) as usize);
    let mut ranges: Vec<(u64, u64)> = Vec::with_capacity(tensors.capacity());
    for _ in 0..n_tensors {
        let name = rd.string(MAX_GGUF_NAME, "tensor name")?;
        let n_dims = rd.u32()? as usize;
        if n_dims > MAX_DIMS {
            return Err(DetectError::Malformed(format!(
                "GGUF tensor with {n_dims} dimensions"
            )));
        }
        let mut ne = Vec::with_capacity(n_dims);
        for _ in 0..n_dims {
            ne.push(rd.u64()?);
        }
        let ttype = rd.u32()?;
        let offset = rd.u64()?;
        let numel = ne
            .iter()
            .try_fold(1u64, |acc, d| acc.checked_mul(*d))
            .ok_or_else(|| DetectError::Malformed("GGUF tensor shape overflows".into()))?;
        let (dtype, bytes) = match ggml_type_info(ttype) {
            Some((name, blck, size)) => {
                let bytes = numel
                    .div_ceil(blck)
                    .checked_mul(size)
                    .ok_or_else(|| DetectError::Malformed("GGUF tensor size overflows".into()))?;
                (name.to_owned(), bytes)
            }
            // A ggml type newer than this table: keep going, size unknown.
            None => (format!("type_{ttype}"), 0),
        };
        ranges.push((offset, bytes));
        tensors.push(TensorInfo {
            name,
            dtype,
            ne,
            bytes,
        });
    }

    // Tensor data must fit in the file (catches truncated downloads).
    if file_size > 0 {
        let data_start = rd.used.div_ceil(alignment) * alignment;
        let avail = file_size.saturating_sub(data_start);
        for (offset, bytes) in ranges {
            if offset.checked_add(bytes).is_none_or(|end| end > avail) {
                return Err(DetectError::Malformed(
                    "GGUF tensor data runs past the end of the file".into(),
                ));
            }
        }
    }
    Ok(finish(FileFormat::Gguf, tensors, metadata, file_size))
}

/// Read one GGUF value; returns a short display string (arrays are summarised).
fn read_gguf_value<R: Read>(
    rd: &mut Bounded<R>,
    vtype: u32,
    depth: u32,
) -> Result<String, DetectError> {
    use gguf_type::*;
    Ok(match vtype {
        UINT8 => rd.u8()?.to_string(),
        INT8 => (rd.u8()? as i8).to_string(),
        UINT16 => rd.u16()?.to_string(),
        INT16 => (rd.u16()? as i16).to_string(),
        UINT32 => rd.u32()?.to_string(),
        INT32 => (rd.u32()? as i32).to_string(),
        FLOAT32 => f32::from_bits(rd.u32()?).to_string(),
        BOOL => (rd.u8()? != 0).to_string(),
        UINT64 => rd.u64()?.to_string(),
        INT64 => (rd.u64()? as i64).to_string(),
        FLOAT64 => f64::from_bits(rd.u64()?).to_string(),
        STRING => truncate_chars(
            &rd.string(MAX_GGUF_STRING, "string value")?,
            MAX_META_VALUE_CHARS,
        ),
        ARRAY => {
            if depth >= MAX_ARRAY_DEPTH {
                return Err(DetectError::Malformed(
                    "GGUF arrays nested too deeply".into(),
                ));
            }
            let etype = rd.u32()?;
            let count = rd.u64()?;
            if let Some(size) = fixed_size(etype) {
                let total = count
                    .checked_mul(size)
                    .ok_or_else(|| DetectError::Malformed("GGUF array size overflows".into()))?;
                rd.skip(total)?;
            } else if etype == STRING {
                // Every string costs at least its 8-byte length.
                if count > rd.remaining() / 8 {
                    return Err(DetectError::Malformed(
                        "GGUF array is longer than the header".into(),
                    ));
                }
                for _ in 0..count {
                    let len = rd.u64()?;
                    if len > MAX_GGUF_STRING {
                        return Err(DetectError::Malformed(
                            "GGUF string in array is too long".into(),
                        ));
                    }
                    rd.skip(len)?;
                }
            } else if etype == ARRAY {
                if count > rd.remaining() / 12 {
                    return Err(DetectError::Malformed(
                        "GGUF array is longer than the header".into(),
                    ));
                }
                for _ in 0..count {
                    read_gguf_value(rd, ARRAY, depth + 1)?;
                }
            } else {
                return Err(DetectError::Malformed(format!(
                    "unknown GGUF array element type {etype}"
                )));
            }
            format!("[{}; {count}]", name(etype))
        }
        other => {
            return Err(DetectError::Malformed(format!(
                "unknown GGUF value type {other}"
            )))
        }
    })
}

// ============================================================================
// Detection
// ============================================================================

/// Tensor names prepared for rule matching.
struct Names<'a> {
    /// `(raw, without "model.diffusion_model.")`
    names: Vec<(&'a str, &'a str)>,
    /// raw name → ne, when the header has shapes.
    ne: BTreeMap<&'a str, &'a [u64]>,
}

impl<'a> Names<'a> {
    fn new(header: &'a HeaderInfo) -> Self {
        let names = header
            .tensor_names
            .iter()
            .map(|n| (n.as_str(), n.strip_prefix(DIFFUSION_PREFIX).unwrap_or(n)))
            .collect();
        let ne = header
            .tensors
            .iter()
            .map(|t| (t.name.as_str(), t.ne.as_slice()))
            .collect();
        Self { names, ne }
    }

    fn any(&self, pattern: &str) -> bool {
        self.names
            .iter()
            .any(|(raw, stripped)| name_matches(pattern, raw, stripped))
    }
}

fn pattern_matches(pattern: &str, name: &str) -> bool {
    let (start, p) = match pattern.strip_prefix('^') {
        Some(p) => (true, p),
        None => (false, pattern),
    };
    let (end, p) = match p.strip_suffix('$') {
        Some(p) => (true, p),
        None => (false, p),
    };
    match (start, end) {
        (true, true) => name == p,
        (true, false) => name.starts_with(p),
        (false, true) => name.ends_with(p),
        (false, false) => name.contains(p),
    }
}

fn name_matches(pattern: &str, raw: &str, stripped: &str) -> bool {
    pattern_matches(pattern, raw)
        || (stripped.len() != raw.len() && pattern_matches(pattern, stripped))
}

/// Shown when [`unsupported_weights`] refuses a file.
pub const UNSUPPORTED_WEIGHTS: &str = "This file uses a compressed format (such as NVFP4 or INT4) that Pinhole can't run, so Pinhole removed it. Look for a version with fp16, bf16, fp8, int8 or GGUF files.";

/// Safetensors weights the pinned engine cannot load
/// (`src/model_io/safetensors_io.cpp`): it skips every U8 tensor, so a
/// `.weight` packed as U8 (ComfyUI NVFP4 / INT4 / MXFP4) would be missing at
/// load time; I8 weights load only as ComfyUI `int8_tensorwise`, i.e. with a
/// `<module>.comfy_quant` config next to them (docs/int8_convrot.md).
/// `Some(message)` = refuse the file. GGUF files always pass.
pub fn unsupported_weights(header: &HeaderInfo) -> Option<&'static str> {
    if header.format != FileFormat::Safetensors {
        return None;
    }
    let names: std::collections::HashSet<&str> =
        header.tensor_names.iter().map(String::as_str).collect();
    header
        .tensors
        .iter()
        .filter_map(|t| Some((t, t.name.strip_suffix(".weight")?)))
        .any(|(t, module)| match t.dtype.as_str() {
            "u8" => true,
            "i8" => !names.contains(format!("{module}.comfy_quant").as_str()),
            _ => false,
        })
        .then_some(UNSUPPORTED_WEIGHTS)
}

/// Evaluate one family's rules against a header.
pub fn rules_match(rules: &DetectRules, header: &HeaderInfo) -> bool {
    rules_match_names(rules, &Names::new(header))
}

fn rules_match_names(rules: &DetectRules, names: &Names<'_>) -> bool {
    if !rules.has_positive_rule() {
        return false;
    }
    if !rules.any_tensor.is_empty() && !rules.any_tensor.iter().any(|p| names.any(p)) {
        return false;
    }
    if !rules.all_tensor.iter().all(|p| names.any(p)) {
        return false;
    }
    if !rules
        .all_of_any
        .iter()
        .filter(|g| !g.is_empty())
        .all(|g| g.iter().any(|p| names.any(p)))
    {
        return false;
    }
    if rules.none_tensor.iter().any(|p| names.any(p)) {
        return false;
    }
    for (pattern, want) in &rules.tensor_ne0 {
        for (raw, stripped) in &names.names {
            if !name_matches(pattern, raw, stripped) {
                continue;
            }
            if let Some(ne) = names.ne.get(raw) {
                if ne.first() != Some(want) {
                    return false;
                }
            }
        }
    }
    true
}

/// Match a header against the registry's `detect` rules.
///
/// Candidates are ordered: families whose `decisive_tensor` matched (if any,
/// the others are dropped), then base families before derived ones (`inherits`
/// / `same_as`), then YAML order. LoRA files get no candidates: their tensor
/// names describe the layers they patch, not a full architecture (use the
/// CivitAI `baseModel` for LoRAs).
pub fn detect(registry: &Registry, header: &HeaderInfo) -> Detection {
    let names = Names::new(header);
    let raw: Vec<&str> = names.names.iter().map(|(r, _)| *r).collect();

    let is_lora = looks_like_lora(&raw);
    let has_diffusion = raw.iter().any(|n| n.starts_with(DIFFUSION_PREFIX));
    let has_vae_prefixed = raw.iter().any(|n| is_vae_name(n));
    let has_te_prefixed = raw.iter().any(|n| is_text_encoder_name(n));

    let candidates = if is_lora {
        Vec::new()
    } else {
        match_families(registry, &names)
    };
    let is_component = if is_lora || has_diffusion || !candidates.is_empty() {
        None
    } else {
        component_kind(header, &raw)
    };

    // Families like HiDream-O1 carry their text encoder under their own names
    // (`detect.whole_checkpoint`): such files are always all-in-one.
    let whole_checkpoint = candidates
        .first()
        .and_then(|id| registry.family(id))
        .is_some_and(|f| f.detect.whole_checkpoint);
    let layout = if !is_lora
        && (whole_checkpoint
            || ((has_diffusion || !candidates.is_empty()) && (has_vae_prefixed || has_te_prefixed)))
    {
        Layout::AllInOne
    } else {
        Layout::DiffusionOnly
    };
    let comp = is_component.as_deref();
    Detection {
        candidates,
        layout,
        has_vae: !is_lora && (has_vae_prefixed || comp == Some("vae")),
        has_text_encoders: !is_lora
            && (whole_checkpoint
                || has_te_prefixed
                || matches!(comp, Some("clip_l" | "clip_g" | "t5xxl" | "llm"))),
        dtype: header.dtype.clone(),
        is_lora,
        is_component,
    }
}

fn is_derived(f: &Family) -> bool {
    f.inherits.is_some() || f.detect.same_as.is_some()
}

fn match_families(registry: &Registry, names: &Names<'_>) -> Vec<String> {
    let mut hits: Vec<(bool, bool, usize, &str)> = Vec::new(); // (decisive, derived, order, id)
    for (idx, fam) in registry.families_in_order().enumerate() {
        if rules_match_names(&fam.detect, names) {
            let decisive = fam.detect.decisive_tensor.iter().any(|p| names.any(p));
            hits.push((decisive, is_derived(fam), idx, fam.id.as_str()));
        }
    }
    if hits.iter().any(|h| h.0) {
        hits.retain(|h| h.0);
    }
    hits.sort_by_key(|h| (h.1, h.2));
    hits.into_iter().map(|h| h.3.to_owned()).collect()
}

/// Share of tensors that carry LoRA / LyCORIS naming. ≥ 50 % = LoRA.
fn looks_like_lora(names: &[&str]) -> bool {
    if names.is_empty() {
        return false;
    }
    let lora = names.iter().filter(|n| is_lora_tensor(n)).count();
    // `.alpha` scalars only count when real LoRA tensors are present.
    lora > 0
        && lora * 2 >= names.len()
        && names
            .iter()
            .any(|n| is_lora_tensor(n) && !n.ends_with(".alpha"))
}

fn is_lora_tensor(n: &str) -> bool {
    const MARKERS: &[&str] = &[
        "lora_down",
        "lora_up",
        "lora_mid",
        ".lora_A.",
        ".lora_B.",
        ".lora.down.",
        ".lora.up.",
        "lora_A.weight",
        "lora_B.weight",
        ".hada_w1",
        ".hada_w2",
        ".hada_t1",
        ".lokr_w1",
        ".lokr_w2",
        ".dora_scale",
        ".diff_b",
        ".lora_linear_layer.",
    ];
    n.starts_with("lora_unet_")
        || n.starts_with("lora_te")
        || n.starts_with("lycoris_")
        || n.ends_with(".alpha")
        || MARKERS.iter().any(|m| n.contains(m))
}

/// VAE tensors inside a checkpoint (name_conversion.cpp `first_stage_model_prefix_vec`).
fn is_vae_name(n: &str) -> bool {
    n.starts_with("first_stage_model.") || n.starts_with("vae.")
}

/// Text-encoder tensors inside a checkpoint (`cond_stage_model_prefix_vec` +
/// the prefixes `convert_tensor_name()` maps onto it).
fn is_text_encoder_name(n: &str) -> bool {
    const PREFIXES: &[&str] = &[
        "cond_stage_model.",
        "conditioner.embedders.",
        "text_encoders.",
        "text_encoder.",
        "text_encoder_2.",
        "te.",
        "te1.",
        "te2.",
        "te3.",
        "clip_l.",
        "clip_g.",
        "t5xxl.",
    ];
    PREFIXES.iter().any(|p| n.starts_with(p))
}

/// Best-effort kind of a standalone component file
/// (`vae` | `taesd` | `clip_l` | `clip_g` | `t5xxl` | `llm` | `llm_vision`).
fn component_kind(header: &HeaderInfo, names: &[&str]) -> Option<String> {
    if names.is_empty() {
        return None;
    }
    let arch = header
        .metadata
        .get("general.architecture")
        .map(|s| s.to_ascii_lowercase());
    let any = |p: &str| names.iter().any(|n| n.contains(p));
    let share = |pred: &dyn Fn(&str) -> bool| {
        names.iter().filter(|n| pred(n)).count() * 10 >= names.len() * 9
    };

    // GGUF companions (llama.cpp naming).
    if let Some(a) = arch.as_deref() {
        if a == "clip" || a.contains("mmproj") {
            return Some("llm_vision".into());
        }
        if a.starts_with("t5") {
            return Some("t5xxl".into());
        }
        if any("token_embd.weight") || any("blk.0.attn_q") {
            return Some("llm".into());
        }
    }
    if any("v.blk.") && any("mm.") {
        return Some("llm_vision".into());
    }
    if any("enc.blk.") && any("token_embd") {
        return Some("t5xxl".into());
    }

    // Safetensors companions (HF / ComfyUI naming). T5 first: its `encoder.block.*`
    // names would otherwise pass the VAE prefix test.
    if any("encoder.block.") && (any("SelfAttention") || any("layer.0.")) {
        return Some("t5xxl".into());
    }
    let vae_like = share(&|n: &str| {
        [
            "encoder.",
            "decoder.",
            "quant_conv.",
            "post_quant_conv.",
            "conv1.",
            "conv2.",
            "first_stage_model.",
            "vae.",
            "taesd_",
        ]
        .iter()
        .any(|p| n.starts_with(p))
    });
    if vae_like && any("decoder") {
        // TAESD / TAEF1 / TAEHV are a few MB; real VAEs are 80+ MB.
        return Some(
            if header.tensor_bytes > 0 && header.tensor_bytes < 50 * 1024 * 1024 {
                "taesd"
            } else {
                "vae"
            }
            .into(),
        );
    }
    if any("model.layers.") && any("self_attn") {
        return Some("llm".into());
    }
    if any("visual.blocks.") || (any("vision_model.encoder.") && !any("text_model.")) {
        return Some("llm_vision".into());
    }
    if any("text_model.encoder.layers.") || any("transformer.resblocks.") {
        return Some(clip_kind(header, names).into());
    }
    None
}

/// CLIP-L (hidden 768, 12 layers) vs CLIP-G (hidden 1280, 32 layers).
fn clip_kind(header: &HeaderInfo, names: &[&str]) -> &'static str {
    let hidden = header
        .tensors
        .iter()
        .find(|t| t.name.ends_with("token_embedding.weight"))
        .and_then(|t| t.ne.first().copied());
    match hidden {
        Some(768) => return "clip_l",
        Some(1280) => return "clip_g",
        _ => {}
    }
    let layers = names
        .iter()
        .filter_map(|n| {
            let rest = n
                .split("encoder.layers.")
                .nth(1)
                .or_else(|| n.split("resblocks.").nth(1))?;
            rest.split('.').next()?.parse::<u32>().ok()
        })
        .max()
        .map_or(0, |m| m + 1);
    if layers > 12 {
        "clip_g"
    } else {
        "clip_l"
    }
}
