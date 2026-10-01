//! Developer tool: measure the check on folders of harmless pictures (RELEASE-SPEC §4).
//! `cargo run --release -p pinhole-check --example falsepos -- <check dir> <folder>…`
//! Prints, per folder: pictures that rule 2 would block, pictures counted as intimate (what
//! rules 1 and 3 would block if the picture came from a brought-in photo of a person or a
//! "safe images only" model), and how often each half of rule 2 fires on its own. A picture
//! that would be blocked is deleted after it is measured.
//! `FALSEPOS_VERBOSE=1` prints the scores behind each intimate or sexual count.
//! `FALSEPOS_SCALE=0.2` shrinks every picture first (how the face rules behave on small faces).
fn main() {
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
