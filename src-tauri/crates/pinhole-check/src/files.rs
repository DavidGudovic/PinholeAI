//! The check's model files. Their addresses, sizes and SHA-256 values are compiled
//! in on purpose (RELEASE-SPEC §4): there is no config file, setting or variable that
//! points the check at other files. Each file is read, hashed and only then parsed,
//! so a swapped or damaged file never runs.
use std::io::Read;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::CheckError;

/// One check file, pinned to a commit on Hugging Face.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CheckFile {
    /// Stable id (also the file name inside the check folder).
    pub id: &'static str,
    /// What it is, for the Downloads list.
    pub label: &'static str,
    pub url: &'static str,
    /// Exact size in bytes.
    pub size: u64,
    /// Lowercase hex SHA-256.
    pub sha256: &'static str,
    pub license: &'static str,
}

pub const NUDITY: CheckFile = CheckFile {
    id: "nudity-vit-384.onnx",
    label: "Safety check: image classifier",
    url: "https://huggingface.co/AdamCodd/vit-base-nsfw-detector/resolve/8587de998f441aac03fdd57a85d2e4cb808c7d64/onnx/model.onnx",
    size: 344_569_044,
    sha256: "dce8f5af8509fee39c453b78a66076ead5c97321ddcee0ddfa16f67dc8286384",
    license: "Apache-2.0",
};

pub const TAGGER: CheckFile = CheckFile {
    id: "wd-vit-tagger-v3.onnx",
    label: "Safety check: tagger",
    url: "https://huggingface.co/SmilingWolf/wd-vit-tagger-v3/resolve/7f6b584d0bd3f55c4531f14ba3d4761b2bccdc0f/model.onnx",
    size: 378_536_310,
    sha256: "35f23693620b668f4d53fd3c62bf65e40af739bc52c7eb0fbc49258b58d065b6",
    license: "Apache-2.0",
};

pub const TAGGER_TAGS: CheckFile = CheckFile {
    id: "wd-vit-tagger-v3-tags.csv",
    label: "Safety check: tagger labels",
    url: "https://huggingface.co/SmilingWolf/wd-vit-tagger-v3/resolve/7f6b584d0bd3f55c4531f14ba3d4761b2bccdc0f/selected_tags.csv",
    size: 308_468,
    sha256: "298633d94d0031d2081c0893f29c82eab7f0df00b08483ba8f29d1e979441217",
    license: "Apache-2.0",
};

pub const FACES: CheckFile = CheckFile {
    id: "yunet-2023mar.onnx",
    label: "Safety check: face finder",
    url: "https://huggingface.co/opencv/face_detection_yunet/resolve/3cc26e7f1014a5ee5d74a42acee58bafc9d0a310/face_detection_yunet_2023mar.onnx",
    size: 232_589,
    sha256: "8f2383e4dd3cfbb4553ea8718107fc0423210dc964f9f4280604804ed2552fa4",
    license: "MIT",
};

pub const AGE: CheckFile = CheckFile {
    id: "fairface-age-vit-224.onnx",
    label: "Safety check: age estimate",
    url: "https://huggingface.co/onnx-community/fairface_age_image_detection-ONNX/resolve/fd04cddf04982f6acc1ac76fb1a31b44c9285596/onnx/model.onnx",
    size: 343_423_222,
    sha256: "7f28fc7e890b4c21bf48f65d9985cefaea3430c28df1391e4f91293c0d89555a",
    license: "Apache-2.0",
};

/// Every file the check needs. All must be present and intact, or nothing is made.
pub const FILES: [CheckFile; 5] = [NUDITY, TAGGER, TAGGER_TAGS, FACES, AGE];

/// Total download size.
pub fn total_bytes() -> u64 {
    FILES.iter().map(|f| f.size).sum()
}

pub fn path(dir: &Path, f: &CheckFile) -> PathBuf {
    dir.join(f.id)
}

/// Files that are absent or have the wrong size (cheap; the hash is checked on load).
pub fn missing(dir: &Path) -> Vec<CheckFile> {
    FILES
        .iter()
        .filter(|f| {
            std::fs::metadata(path(dir, f))
                .map(|m| !m.is_file() || m.len() != f.size)
                .unwrap_or(true)
        })
        .copied()
        .collect()
}

/// Read a whole file and return its bytes only if size and SHA-256 match.
pub fn read_verified(dir: &Path, f: &CheckFile) -> Result<Vec<u8>, CheckError> {
    let p = path(dir, f);
    let file = std::fs::File::open(&p).map_err(|_| CheckError::Missing(f.label))?;
    let mut bytes = Vec::with_capacity(f.size as usize);
    // Read at most one byte more than expected, so a huge file can't fill memory.
    file.take(f.size + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| CheckError::Missing(f.label))?;
    if bytes.len() as u64 != f.size || hex::encode(Sha256::digest(&bytes)) != f.sha256 {
        return Err(CheckError::Damaged(f.label));
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pins_are_well_formed() {
        for f in FILES {
            assert_eq!(f.sha256.len(), 64, "{}", f.id);
            assert!(f
                .sha256
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
            assert!(f.url.starts_with("https://huggingface.co/"), "{}", f.id);
            // Pinned to a commit, never a moving branch.
            assert!(!f.url.contains("/resolve/main/"), "{}", f.id);
            assert!(f.size > 0);
        }
        let mut ids: Vec<_> = FILES.iter().map(|f| f.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), FILES.len());
    }

    #[test]
    fn missing_and_damaged_files_are_caught() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(missing(dir.path()).len(), FILES.len());
        assert!(matches!(
            read_verified(dir.path(), &FACES),
            Err(CheckError::Missing(_))
        ));
        // Right size, wrong content.
        std::fs::write(path(dir.path(), &FACES), vec![0u8; FACES.size as usize]).unwrap();
        assert!(!missing(dir.path()).contains(&FACES));
        assert!(matches!(
            read_verified(dir.path(), &FACES),
            Err(CheckError::Damaged(_))
        ));
        // Wrong size.
        std::fs::write(path(dir.path(), &FACES), b"short").unwrap();
        assert!(missing(dir.path()).contains(&FACES));
        assert!(matches!(
            read_verified(dir.path(), &FACES),
            Err(CheckError::Damaged(_))
        ));
    }
}
