//! Developer tool: run the word check over a file of prompts, one per line, and print the
//! numbers of the lines it blocks (not their text).
//! `cargo run --release -p pinhole-engine --example wordcheck -- prompts.txt`
fn main() {
    let path = std::env::args().nth(1).expect("prompts file");
    let text = std::fs::read_to_string(path).expect("read prompts");
    let (mut n, mut blocked) = (0, 0);
    for (i, line) in text.lines().enumerate() {
        n += 1;
        if pinhole_engine::words::pairs_minor_with_sexual(line) {
            blocked += 1;
            println!("{}", i + 1);
        }
    }
    eprintln!("{blocked} of {n} lines blocked");
}
