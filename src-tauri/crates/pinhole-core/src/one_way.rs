//! Checks that the two choke points (RELEASE-SPEC §1) are the only paths. The types do most of
//! the work: `ImgGenRequest::new` takes only a word-checked `CheckedPrompt`, and
//! `Session::insert_generated` takes only an image-checked `CheckedPng`. These tests check that
//! only the owning modules build those values, that only generate.rs calls the image engine,
//! and that the app doesn't enable the test-only constructors.

use std::path::{Path, PathBuf};

/// Every non-test Rust file of the app, with its inline test modules cut out.
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
                let text = std::fs::read_to_string(&p).unwrap().replace("\r\n", "\n");
                // Comments don't call anything; drop them so docs can name the endpoints.
                let code: String = without_test_modules(&text)
                    .lines()
                    .filter(|l| !l.trim_start().starts_with("//"))
                    .map(|l| format!("{l}\n"))
                    .collect();
                out.push((p, code));
            }
        }
    }
    assert!(out.len() > 50, "found only {} source files", out.len());
    out
}

/// `text` without its inline `#[cfg(test)] mod … { … }` blocks (brace-matched, so product
/// code after a test module is still scanned; `mod tests;` pointing at a file is kept).
fn without_test_modules(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find("#[cfg(test)]\nmod ") {
        let after = &rest[i..];
        let line_end = after.find('\n').map_or(after.len(), |n| n + 1);
        let header_end = line_end
            + after[line_end..]
                .find('\n')
                .unwrap_or(after.len() - line_end);
        let header = &after[line_end..header_end];
        out.push_str(&rest[..i]);
        if header.trim_end().ends_with(';') || !header.contains('{') {
            out.push_str(&after[..header_end]);
            rest = &after[header_end..];
            continue;
        }
        let open = line_end + header.find('{').unwrap();
        let mut depth = 0usize;
        let mut end = after.len();
        for (k, c) in after[open..].char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = open + k + 1;
                        break;
                    }
                }
                _ => {}
            }
        }
        rest = &after[end..];
    }
    out.push_str(rest);
    out
}

#[test]
fn test_modules_are_cut_but_code_after_them_is_kept() {
    let t = "a\n#[cfg(test)]\nmod tests;\nb\n#[cfg(test)]\nmod tests {\n    fn x() { y() }\n}\nc\n";
    let kept = without_test_modules(t);
    assert!(kept.contains('a') && kept.contains('b') && kept.contains('c'));
    assert!(kept.contains("mod tests;"));
    assert!(!kept.contains("fn x"));
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
    assert_eq!(files_with("::submit("), Vec::<String>::new());
    assert_eq!(files_with("::upscale("), Vec::<String>::new());
    // Job results hold the pictures too: read only by generate.rs.
    assert_eq!(files_with(".job("), ["generate.rs"]);
    // No request to the engine's picture endpoints except through SdClient.
    assert_eq!(files_with("/sdcpp/v1/img_gen"), ["sdapi.rs"]);
    assert_eq!(files_with("/sdcpp/v1/upscale"), ["sdapi.rs"]);
    assert_eq!(files_with("/sdcpp/v1/jobs"), ["sdapi.rs"]);
    assert_eq!(files_with(".insert_generated("), ["generate.rs"]);
    assert_eq!(
        files_with("check_results("),
        ["generate.rs", "imagecheck.rs"]
    );
}

/// The image check works out every rule input itself from how a result was made
/// (`imagecheck::MadeBy`): no feature passes or picks one.
#[test]
fn rule_inputs_are_worked_out_only_by_the_image_check() {
    assert_eq!(files_with("rules::decide("), ["imagecheck.rs"]);
    // The "safe images only" flag of files and pictures is read for the check only there
    // (the session keeps it with a saved picture; the model lists show it as a badge).
    assert_eq!(
        files_with(".safe_images_only"),
        ["imagecheck.rs", "inventory.rs", "session.rs"]
    );
    assert!(files_with("safe_images_only")
        .iter()
        .all(|f| f != "generate.rs"));
}

/// The product code of pinhole-core's generate.rs.
fn generate_rs() -> String {
    product_sources()
        .into_iter()
        .find(|(p, _)| p.ends_with(Path::new("pinhole-core/src/generate.rs")))
        .unwrap()
        .1
}

/// A job reads session pictures only through `Inputs` in generate.rs, so every picture sent to
/// the engine is an input of the result for the image check (a mask goes as its shape only);
/// Upscale reads its one source, which it hands to the check.
#[test]
fn a_job_reads_session_pictures_only_as_declared_inputs() {
    // Without whitespace: a call can be split over lines.
    let code: String = generate_rs().split_whitespace().collect();
    // `session_image` and `upscale_image`.
    assert_eq!(code.matches("session.get(").count(), 2);
    // Its definition, `Inputs::take` and `Inputs::mask`.
    assert_eq!(code.matches("session_image(").count(), 3);
    assert!(!code.contains("session::get("));
    assert!(!code.contains("session::decode_rgba("));
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
