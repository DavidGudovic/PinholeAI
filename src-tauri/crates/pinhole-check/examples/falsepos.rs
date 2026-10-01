//! Developer tool: measure the check on folders of harmless pictures (RELEASE-SPEC §4).
//! `cargo run --release -p pinhole-check --example falsepos -- <check dir> <folder>…`
//! Prints, per folder: pictures that rule 2 would block, pictures counted as intimate (what
//! rules 1 and 3 would block if the picture came from a brought-in photo of a person or a
//! "safe images only" model), and how often each half of rule 2 fires on its own. A picture
//! that would be blocked is deleted after it is measured.
//! `FALSEPOS_VERBOSE=1` prints the scores behind each intimate or sexual count.
//! `FALSEPOS_SCALE=0.2` shrinks every picture first (how the face rules behave on small faces).
//! `FALSEPOS_AGES=1` only runs the face finder and age estimate (folders of face portraits,
//! e.g. sorted by labelled age): per folder, how many judged faces reach each threshold of the
//! child groups (0–9), the 10–19 group alone and under 20 (0–19), and how many the age half
//! of rule 2 would block on a sexual photo.
fn main() {
    if std::env::var_os("FALSEPOS_AGES").is_some() {
        return ages();
    }
    let mut args = std::env::args().skip(1);
    // FALSEPOS_VERBOSE=1 also prints the scores of pictures counted as intimate or sexual.
    let verbose = std::env::var_os("FALSEPOS_VERBOSE").is_some();
    let scale: f32 = std::env::var("FALSEPOS_SCALE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1.0);
    let c = pinhole_check::Checker::new(args.next().expect("check dir").into());
    for dir in args {
        let mut files: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .collect();
        files.sort();
        let (
            mut n,
            mut blocked,
            mut intimate,
            mut sexual,
            mut minor_tag,
            mut young_face,
            mut adult_faces,
            mut small_face,
        ) = (0, 0, 0, 0, 0, 0, 0, 0);
        for p in files {
            let Ok(img) = image::open(&p) else { continue };
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
                .unwrap();
            let r = c.full_readings(&png).expect("readings");
            n += 1;
            let t = r.tags.unwrap_or_default();
            let tags = Some(&t);
            if verbose
                && (pinhole_check::rules::is_intimate(r.nudity, tags)
                    || pinhole_check::rules::is_sexual(r.nudity, tags))
            {
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
            if pinhole_check::rules::is_intimate(r.nudity, tags) {
                intimate += 1;
            }
            if pinhole_check::rules::is_sexual(r.nudity, tags) {
                sexual += 1;
            }
            if t.minor >= pinhole_check::rules::MINOR_TAG {
                minor_tag += 1;
            }
            let faces: Vec<_> = r.faces.iter().flatten().filter(|f| f.judged()).collect();
            if pinhole_check::rules::is_photo_style(&t)
                && r.faces.iter().flatten().any(|f| f.too_small_to_judge())
            {
                small_face += 1;
            }
            if !faces.is_empty() {
                adult_faces += 1;
            }
            if pinhole_check::rules::is_photo_style(&t)
                && faces.iter().any(|f| {
                    f.child_face
                        .is_some_and(|u| u >= pinhole_check::rules::CHILD_FACE)
                })
            {
                young_face += 1;
            }
            if let Some(rule) = pinhole_check::rules::decide(&r, &[], false) {
                blocked += 1;
                println!("  BLOCK {} ({})", p.display(), rule.key());
                if scale >= 1.0 {
                    let _ = std::fs::remove_file(&p);
                }
            }
        }
        println!(
            "{dir}: {n} pictures · rule 2 blocks {blocked} · intimate {intimate} · sexual {sexual} · minor tag ≥ {:.1}: {minor_tag} · photo-style child face ≥ {:.1}: {young_face} · with a face: {adult_faces} · photo-style face too small to judge: {small_face}",
            pinhole_check::rules::MINOR_TAG,
            pinhole_check::rules::CHILD_FACE,
        );
    }
}

fn ages() {
    const LEVELS: [f32; 6] = [0.5, 0.6, 0.7, 0.8, 0.85, 0.9];
    // FALSEPOS_VERBOSE=1 also prints each picture's 0–9 and 0–19 scores.
    let verbose = std::env::var_os("FALSEPOS_VERBOSE").is_some();
    let mut args = std::env::args().skip(1);
    let c = pinhole_check::Checker::new(args.next().expect("check dir").into());
    for dir in args {
        let mut files: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .collect();
        files.sort();
        let (mut n, mut judged, mut blocked) = (0, 0, 0);
        let (mut child, mut teen, mut under_20) = ([0; 6], [0; 6], [0; 6]);
        for p in files {
            let Ok(bytes) = std::fs::read(&p) else {
                continue;
            };
            let Ok(img) = image::load_from_memory(&bytes) else {
                continue;
            };
            let mut png = Vec::new();
            img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
                .unwrap();
            n += 1;
            let faces = c.face_readings(&png).expect("readings");
            // The most underage-looking judged face stands for the picture.
            let Some((c0, u)) = faces
                .iter()
                .filter(|f| f.judged())
                .filter_map(|f| Some((f.child_face?, f.under_20_face?)))
                .max_by(|a, b| a.1.total_cmp(&b.1))
            else {
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
        println!("{dir}: {n} pictures · judged face {judged} · rule 2 on a sexual photo would block {blocked} ({:.2}%)", 100.0 * blocked as f32 / judged.max(1) as f32);
        row("0–9", child);
        row("10–19", teen);
        row("0–19", under_20);
    }
}
