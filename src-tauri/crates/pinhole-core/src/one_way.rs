//! Guards the two choke points (RELEASE-SPEC §1) against a new path that skips them. The types
//! do most of the work: `ImgGenRequest::new` takes only a word-checked `CheckedPrompt`, and
//! `Session::insert_generated` takes only an image-checked `CheckedPng`. These tests catch the
//! ways around the types: building a session picture or a checked picture by hand, calling
//! the image engine from somewhere new, or turning on the test-only constructors in the app.

use std::path::{Path, PathBuf};

/// Every non-test Rust file of the app, with its `#[cfg(test)] mod tests` part cut off.
fn product_sources() -> Vec<(PathBuf, String)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut out = Vec::new();
    let mut dirs = vec![root.join("src"), root.join("crates")];
    while let Some(dir) = dirs.pop() {
        for e in std::fs::read_dir(&dir).unwrap().flatten() {
            let p = e.path();
            let name = p.file_name().unwrap().to_string_lossy().to_string();
            if p.is_dir() {
                if !matches!(name.as_str(), "tests" | "examples" | "benches" | "target") {
                    dirs.push(p);
                }
            } else if name.ends_with(".rs")
                && !matches!(name.as_str(), "testing.rs" | "testutil.rs" | "one_way.rs")
                && !name.ends_with("_tests.rs")
            {
                let text = std::fs::read_to_string(&p).unwrap();
                let product = match text.find("#[cfg(test)]\nmod tests") {
                    Some(i) => text[..i].to_string(),
                    None => text,
                };
                out.push((p, product));
            }
        }
    }
    assert!(out.len() > 50, "found only {} source files", out.len());
    out
}

/// Files (by name) whose product code contains `needle`.
fn files_with(needle: &str) -> Vec<String> {
    let mut v: Vec<String> = product_sources()
        .into_iter()
        .filter(|(_, t)| t.contains(needle))
        .map(|(p, _)| p.file_name().unwrap().to_string_lossy().to_string())
        .collect();
    v.sort();
    v.dedup();
    v
}

#[test]
fn session_pictures_are_built_only_by_the_session() {
    assert_eq!(files_with("SessionImage {"), ["session.rs"]);
}

#[test]
fn checked_pictures_are_built_only_by_the_image_check() {
    assert_eq!(files_with("CheckedPng {"), ["imagecheck.rs"]);
    assert_eq!(files_with("CheckedPrompt("), ["words.rs"]);
}

#[test]
fn the_image_engine_is_asked_for_pictures_only_by_generate() {
    // `submit` (img_gen) and `upscale` are the SdClient calls that return pictures; both
    // results go through `imagecheck::check_results` in generate.rs.
    assert_eq!(files_with(".submit("), ["generate.rs"]);
    assert_eq!(files_with(".upscale("), ["generate.rs"]);
    assert_eq!(files_with(".insert_generated("), ["generate.rs"]);
    assert_eq!(
        files_with("check_results("),
        ["generate.rs", "imagecheck.rs"]
    );
}

#[test]
fn the_app_never_turns_on_test_only_constructors() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let app = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
    assert!(!app.contains("test-util"));
    assert!(files_with("unchecked_for_tests")
        .iter()
        .all(|f| f == "imagecheck.rs"));
}
