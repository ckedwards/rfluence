//! Print the normalized form of a markdown file or stdin: `cargo run --example norm -- file.md`.
use std::io::Read;
fn main() {
    let md = match std::env::args().nth(1) {
        Some(p) => std::fs::read_to_string(p).unwrap(),
        None => { let mut s = String::new(); std::io::stdin().read_to_string(&mut s).unwrap(); s }
    };
    print!("{}", rfluence_convert::normalize(&md));
}
