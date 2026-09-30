//! Developer tool: measure the check on folders of harmless pictures (RELEASE-SPEC §4).
//! `cargo run --release -p pinhole-check --example falsepos -- <check dir> <folder>…`
//! Prints, per folder: pictures that rule 2 would block, pictures counted as intimate (what
//! rules 1 and 3 would block if the picture came from a brought-in photo of a person or a
//! "safe images only" model), and how often each half of rule 2 fires on its own. A picture
//! that would be blocked is deleted after it is measured.
fn main() {
    let mut args = std::env::args().skip(1);
    let c = pinhole_check::Checker::new(args.next().expect("check dir").into());
    for dir in args {
        let mut files: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.path()).collect();
        files.sort();
        let (mut n, mut blocked, mut intimate, mut sexual, mut child_tag, mut young_face, mut adult_faces) =
            (0, 0, 0, 0, 0, 0, 0);
        for p in files {
            let Ok(img) = image::open(&p) else { continue };
            let mut png = Vec::new();
            img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
            let r = c.full_readings(&png).expect("readings");
            n += 1;
            let t = r.tags.unwrap_or_default();
            let tags = Some(&t);
            if pinhole_check::rules::is_intimate(r.nudity, tags) {
                intimate += 1;
            }
            if pinhole_check::rules::is_sexual(r.nudity, tags) {
                sexual += 1;
            }
            if t.loli.max(t.shota).max(t.child) >= pinhole_check::rules::CHILD_TAG {
                child_tag += 1;
            }
            let faces: Vec<_> = r.faces.iter().flatten().filter(|f| f.counts()).collect();
            if !faces.is_empty() {
                adult_faces += 1;
            }
            if pinhole_check::rules::is_photo_style(&t)
                && faces.iter().any(|f| f.under_ten.is_some_and(|u| u >= pinhole_check::rules::UNDER_TEN))
            {
                young_face += 1;
            }
            if let Some(rule) = pinhole_check::rules::decide(&r, &[], false) {
                blocked += 1;
                println!("  BLOCK {} ({})", p.display(), rule.key());
                let _ = std::fs::remove_file(&p);
            }
        }
        println!(
            "{dir}: {n} pictures · rule 2 blocks {blocked} · intimate {intimate} · sexual {sexual} · child tag ≥ {:.1}: {child_tag} · photo-style face under-10 ≥ {:.1}: {young_face} · with a face: {adult_faces}",
            pinhole_check::rules::CHILD_TAG,
            pinhole_check::rules::UNDER_TEN,
        );
    }
}
