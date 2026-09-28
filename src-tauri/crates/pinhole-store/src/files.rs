//! Shared helpers for the one-YAML-file-per-item folders (styles, presets).

use std::fs::{self, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_yaml::{Mapping, Value};

use crate::{is_valid_slug, StoreError};

/// Id prefix of read-only items shipped in `config/` (`builtin:<file-stem>`).
pub(crate) const BUILTIN_PREFIX: &str = "builtin:";

/// Library files are small; anything bigger is not ours (defensive cap).
const MAX_FILE_BYTES: u64 = 1024 * 1024;

/// `builtin:<stem>` → `Some(stem)`.
pub(crate) fn builtin_stem(id: &str) -> Option<&str> {
    id.strip_prefix(BUILTIN_PREFIX)
}

/// `*.yaml` files in `dir` whose stem is a valid slug, sorted by stem.
/// A missing directory is empty; other I/O errors are reported.
pub(crate) fn yaml_files(dir: &Path) -> Result<Vec<(String, PathBuf)>, StoreError> {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("yaml") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else { continue };
        if !is_valid_slug(stem) || !path.is_file() {
            continue;
        }
        out.push((stem.to_string(), path));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

/// Read a small text file (capped at 1 MB).
pub(crate) fn read_text(path: &Path) -> Result<String, StoreError> {
    let file = fs::File::open(path)?;
    let mut text = String::new();
    file.take(MAX_FILE_BYTES + 1).read_to_string(&mut text).map_err(|e| StoreError::Parse {
        path: path.display().to_string(),
        msg: e.to_string(),
    })?;
    if text.len() as u64 > MAX_FILE_BYTES {
        return Err(StoreError::Parse { path: path.display().to_string(), msg: "file is too large".into() });
    }
    Ok(text)
}

/// Read a file, mapping "does not exist" to `NotFound(what)`.
pub(crate) fn read_text_or_not_found(path: &Path, what: &str) -> Result<String, StoreError> {
    match fs::metadata(path) {
        Ok(m) if m.is_file() => read_text(path),
        Ok(_) => Err(StoreError::NotFound(what.to_string())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(StoreError::NotFound(what.to_string())),
        Err(e) => Err(e.into()),
    }
}

/// Reserve `<dir>/<base>.yaml`, or `<base>-2.yaml`, `<base>-3.yaml`… by creating
/// it exclusively (so two concurrent saves never pick the same id). Returns the id.
pub(crate) fn reserve_unique(dir: &Path, base: &str) -> Result<String, StoreError> {
    fs::create_dir_all(dir)?;
    for n in 1u32..10_000 {
        let id = if n == 1 { base.to_string() } else { format!("{base}-{n}") };
        match OpenOptions::new().write(true).create_new(true).open(dir.join(format!("{id}.yaml"))) {
            Ok(_) => return Ok(id),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.into()),
        }
    }
    Err(StoreError::Invalid("Too many items with this name. Pick a different name.".into()))
}

/// Serialize `value` as human-editable YAML for a library file: the `id` and
/// `builtin` keys are dropped (the file name is the id), and nulls / empty
/// lists / empty maps are left out.
pub(crate) fn to_library_yaml<T: Serialize>(value: &T, header: &str) -> Result<String, StoreError> {
    let v = serde_yaml::to_value(value).map_err(|e| StoreError::Invalid(format!("could not encode: {e}")))?;
    let mut v = prune(v).unwrap_or(Value::Mapping(Mapping::new()));
    if let Value::Mapping(m) = &mut v {
        m.remove("id");
        m.remove("builtin");
    }
    let body = serde_yaml::to_string(&v).map_err(|e| StoreError::Invalid(format!("could not encode: {e}")))?;
    Ok(format!("{header}{body}"))
}

fn prune(v: Value) -> Option<Value> {
    match v {
        Value::Null => None,
        Value::Sequence(s) => {
            let s: Vec<Value> = s.into_iter().filter_map(prune).collect();
            (!s.is_empty()).then_some(Value::Sequence(s))
        }
        Value::Mapping(m) => {
            let m: Mapping = m.into_iter().filter_map(|(k, v)| prune(v).map(|v| (k, v))).collect();
            (!m.is_empty()).then_some(Value::Mapping(m))
        }
        other => Some(other),
    }
}

pub(crate) fn parse_error(path: &Path, e: impl std::fmt::Display) -> StoreError {
    StoreError::Parse { path: path.display().to_string(), msg: e.to_string() }
}

/// Trim; empty → None.
pub(crate) fn clean_opt(s: Option<String>) -> Option<String> {
    s.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

/// Case-insensitive name order, id as tie-breaker.
pub(crate) fn name_order(a_name: &str, a_id: &str, b_name: &str, b_id: &str) -> std::cmp::Ordering {
    a_name.to_lowercase().cmp(&b_name.to_lowercase()).then_with(|| a_id.cmp(b_id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prune_drops_empty() {
        #[derive(Serialize)]
        struct S {
            id: String,
            builtin: bool,
            name: String,
            a: Option<u32>,
            list: Vec<u32>,
            inner: Inner,
        }
        #[derive(Serialize)]
        struct Inner {
            x: Option<u32>,
        }
        let s = S { id: "x".into(), builtin: true, name: "N".into(), a: None, list: vec![], inner: Inner { x: None } };
        let y = to_library_yaml(&s, "# h\n").unwrap();
        assert_eq!(y, "# h\nname: N\n");
    }

    #[test]
    fn reserve_unique_counts_up() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(reserve_unique(tmp.path(), "a").unwrap(), "a");
        assert_eq!(reserve_unique(tmp.path(), "a").unwrap(), "a-2");
        assert_eq!(reserve_unique(tmp.path(), "a").unwrap(), "a-3");
    }

    #[test]
    fn yaml_files_filters() {
        let tmp = tempfile::tempdir().unwrap();
        for n in ["b.yaml", "a.yaml", "x.yml", "Bad Name.yaml", ".a.yaml.1.0.tmp", "c.txt"] {
            fs::write(tmp.path().join(n), "name: x").unwrap();
        }
        fs::create_dir(tmp.path().join("d.yaml")).unwrap();
        let stems: Vec<_> = yaml_files(tmp.path()).unwrap().into_iter().map(|(s, _)| s).collect();
        assert_eq!(stems, vec!["a", "b"]);
        assert!(yaml_files(&tmp.path().join("missing")).unwrap().is_empty());
    }
}
