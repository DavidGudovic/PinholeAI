//! Developer tool: measure the check on folders of harmless pictures (RELEASE-SPEC §4).
//! `cargo run --release -p pinhole-check --example falsepos -- <check dir> <folder>…`
//! Read-only: it never moves, changes or deletes a picture.
//!
//! Per folder it prints which set of pictures was measured (file count and one SHA-256 over
//! every file name and content), then the counts with their denominators: files, files that
//! couldn't be read as a picture, pictures the check failed on, and pictures measured.
//! The scores come from the same steps and rules a result goes through in the app
//! (`Checker::readings` + `rules::decide`), for three cases: a plain Create (rule 2), a
//! result made from a brought-in photo of a person (rule 1), and a result of a "safe images
//! only" model (rule 3). Every blocked picture is listed with the rule that blocked it.
//! `FALSEPOS_DIAG=1` also runs every step on every picture (`Checker::full_readings`, the dev
//! readings view) and prints how often each half of rule 2 fires on its own. Those numbers
//! are diagnostics, not what the app decides.
//! `FALSEPOS_VERBOSE=1` prints the scores behind each intimate or sexual count.
//! `FALSEPOS_SCALE=0.2` shrinks every picture first (how the face rules behave on small faces).
//! `FALSEPOS_AGES=1` only runs the face finder and age estimate (folders of face portraits,
//! e.g. sorted by labelled age): see [`ages`].
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

fn main() {
    if std::env::var_os("FALSEPOS_AGES").is_some() {
        return ages();
    }
    let mut args = std::env::args().skip(1);
    let verbose = std::env::var_os("FALSEPOS_VERBOSE").is_some();
    let diag = std::env::var_os("FALSEPOS_DIAG").is_some();
    let scale: f32 = std::env::var("FALSEPOS_SCALE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1.0);
    let c = pinhole_check::Checker::new(args.next().expect("check dir").into());
    let person = [pinhole_check::rules::Original { has_face: true }];
    for dir in args {
        let (files, id) = dataset(&dir);
        let (mut unreadable, mut errors, mut n) = (0, 0, 0);
        let (mut create, mut from_photo, mut safe_only) = (0, 0, 0);
        let (mut intimate, mut sexual) = (0, 0);
        let (mut minor_tag, mut young_face, mut with_face, mut small_face) = (0, 0, 0, 0);
        for p in &files {
            let Some(png) = load(p, scale) else {
                unreadable += 1;
                continue;
            };
            let r = match c.readings(&png) {
                Ok(r) => r,
                Err(e) => {
                    errors += 1;
                    println!("  ERROR {}: {e}", p.display());
                    continue;
                }
            };
            n += 1;
            let tags = r.tags.as_ref();
            let is_intimate = pinhole_check::rules::is_intimate(r.nudity, tags);
            let is_sexual = pinhole_check::rules::is_sexual(r.nudity, tags);
            intimate += is_intimate as u32;
            sexual += is_sexual as u32;
            if verbose && (is_intimate || is_sexual) {
                let t = tags.cloned().unwrap_or_default();
                println!(
                    "  {} nudity {:.2} questionable {:.2} explicit {:.2} nude tag {:.2} underwear tag {:.2}",
                    p.display(),
                    r.nudity,
                    t.questionable,
                    t.explicit,
                    t.nude,
                    t.underwear
                );
            }
            for (count, case, originals, safe) in [
                (&mut create, "create", &[][..], false),
                (&mut from_photo, "from a photo of a person", &person[..], false),
                (&mut safe_only, "safe-images-only model", &[][..], true),
            ] {
                if let Some(rule) = pinhole_check::rules::decide(&r, originals, safe) {
                    *count += 1;
                    println!("  BLOCK [{case}] {} ({})", p.display(), rule.key());
                }
            }
            if diag {
                let Ok(full) = c.full_readings(&png) else {
                    continue; // already counted by the readings above if it keeps failing
                };
                let t = full.tags.unwrap_or_default();
                minor_tag += (t.minor >= pinhole_check::rules::MINOR_TAG) as u32;
                let faces = full.faces.unwrap_or_default();
                let photo = pinhole_check::rules::is_photo_style(&t);
                small_face += (photo && faces.iter().any(|f| f.too_small_to_judge())) as u32;
                with_face += faces.iter().any(|f| f.judged()) as u32;
                young_face += (photo
                    && faces.iter().filter(|f| f.judged()).any(|f| {
                        f.child_face
                            .is_some_and(|u| u >= pinhole_check::rules::CHILD_FACE)
                    })) as u32;
            }
        }
        let pct = |k: u32| 100.0 * k as f32 / n.max(1) as f32;
        println!(
            "{dir}: dataset {id} · {} files · unreadable {unreadable} · check error {errors} · measured {n}",
            files.len()
        );
        println!(
            "  app decision (of {n} measured): Create blocks {create} ({:.2}%) · from a photo of a person blocks {from_photo} ({:.2}%) · safe-images-only model blocks {safe_only} ({:.2}%) · intimate {intimate} · sexual {sexual}",
            pct(create),
            pct(from_photo),
            pct(safe_only),
        );
        if diag {
            println!(
                "  diagnostics (every step on every picture, not the app's decision): minor tag ≥ {:.1}: {minor_tag} · photo-style child face ≥ {:.1}: {young_face} · with a judged face: {with_face} · photo-style face too small to judge: {small_face}",
                pinhole_check::rules::MINOR_TAG,
                pinhole_check::rules::CHILD_FACE,
            );
        }
    }
}

/// The files of a folder (sorted) and one SHA-256 over every file's name and content, so a
/// result can be tied to the exact set of pictures it came from.
fn dataset(dir: &str) -> (Vec<PathBuf>, String) {
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{dir}: {e}"))
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .collect();
    files.sort();
    let mut h = Sha256::new();
    for p in &files {
        h.update(p.file_name().unwrap_or_default().as_encoded_bytes());
        h.update([0]);
        h.update(Sha256::digest(std::fs::read(p).unwrap_or_default()));
    }
    (files, hex::encode(&h.finalize()[..8]))
}

/// One picture as PNG bytes (what the app checks), shrunk first when `scale` < 1.
/// `None`: the file isn't a picture this tool can read.
fn load(p: &Path, scale: f32) -> Option<Vec<u8>> {
    let img = image::open(p).ok()?;
    let img = if scale < 1.0 {
        let (w, h) = (img.width() as f32 * scale, img.height() as f32 * scale);
        img.resize_exact(
            (w as u32).max(1),
            (h as u32).max(1),
            image::imageops::FilterType::Triangle,
        )
    } else {
        img
    };
    let mut png = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .ok()?;
    Some(png)
}

/// The age estimate alone, on folders of face portraits. Per folder: how many judged faces
/// reach each threshold of the child groups (0–9), the 10–19 group alone and under 20 (0–19),
/// and how many the age half of rule 2 would block on a sexual photo-style result.
/// The 10–19 group includes adults aged 18 and 19, so "under 20" is not "under 18". These are
/// numbers for one part of the check, not how often the whole check catches harmful pictures.
fn ages() {
    const LEVELS: [f32; 6] = [0.5, 0.6, 0.7, 0.8, 0.85, 0.9];
    // FALSEPOS_VERBOSE=1 also prints each picture's 0–9 and 0–19 scores.
    let verbose = std::env::var_os("FALSEPOS_VERBOSE").is_some();
    let mut args = std::env::args().skip(1);
    let c = pinhole_check::Checker::new(args.next().expect("check dir").into());
    for dir in args {
        let (files, id) = dataset(&dir);
        let (mut unreadable, mut errors, mut n, mut no_face, mut judged, mut blocked) =
            (0, 0, 0, 0, 0, 0);
        let (mut child, mut teen, mut under_20) = ([0; 6], [0; 6], [0; 6]);
        for p in &files {
            let Some(png) = load(p, 1.0) else {
                unreadable += 1;
                continue;
            };
            let faces = match c.face_readings(&png) {
                Ok(f) => f,
                Err(e) => {
                    errors += 1;
                    println!("  ERROR {}: {e}", p.display());
                    continue;
                }
            };
            n += 1;
            // The most underage-looking judged face stands for the picture.
            let Some((c0, u)) = faces
                .iter()
                .filter(|f| f.judged())
                .filter_map(|f| Some((f.child_face?, f.under_20_face?)))
                .max_by(|a, b| a.1.total_cmp(&b.1))
            else {
                no_face += 1;
                continue;
            };
            judged += 1;
            if verbose {
                println!("  {} 0–9 {c0:.4} 0–19 {u:.4}", p.display());
            }
            for (i, level) in LEVELS.iter().enumerate() {
                child[i] += (c0 >= *level) as u32;
                teen[i] += (u - c0 >= *level) as u32;
                under_20[i] += (u >= *level) as u32;
            }
            if faces.iter().any(|f| f.looks_underage()) {
                blocked += 1;
            }
        }
        let row = |name: &str, v: [u32; 6]| {
            let cells: Vec<String> = LEVELS
                .iter()
                .zip(v)
                .map(|(l, k)| {
                    format!(
                        "≥{l}: {k} ({:.1}%)",
                        100.0 * k as f32 / judged.max(1) as f32
                    )
                })
                .collect();
            println!("  {name}: {}", cells.join(" · "));
        };
        println!(
            "{dir}: dataset {id} · {} files · unreadable {unreadable} · check error {errors} · measured {n} · no judged face {no_face} · judged face {judged}",
            files.len()
        );
        println!(
            "  age half of rule 2 on a sexual photo-style result would block {blocked} of {judged} judged ({:.2}%)",
            100.0 * blocked as f32 / judged.max(1) as f32
        );
        println!("  share of judged faces (age estimate only):");
        row("0–9", child);
        row("10–19 (includes 18 and 19)", teen);
        row("0–19", under_20);
    }
}
