//! Developer tool: print the check's readings for image files, to tune the rules on
//! ordinary, legal pictures. `cargo run --release -p pinhole-check --example measure -- <check dir> <image>…`
use std::time::Instant;

fn main() {
    let mut args = std::env::args().skip(1);
    let dir = args.next().expect("usage: measure <check dir> <image>…");
    let c = pinhole_check::Checker::new(dir.into());
    for p in args {
        let img = image::open(&p).expect("read image");
        let mut png = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .expect("encode");
        let t = Instant::now();
        match (c.readings(&png), c.original(&png)) {
            (Ok(r), Ok(o)) => println!(
                "{p}: {:?} verdict={:?} as-original={o:?}\n  {r:?}",
                t.elapsed(),
                pinhole_check::rules::decide(&r, &[], false),
            ),
            (Err(e), _) | (_, Err(e)) => println!("{p}: {e}"),
        }
    }
}
