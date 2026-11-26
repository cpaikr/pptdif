//! Prints the model of a deck, for checking the reader by eye.
//!
//! Usage: `cargo run -p pptdif --example dump -- DECK`

fn main() {
    let path = std::env::args().nth(1).expect("usage: dump DECK");
    let deck = pptdif::read_path(std::path::Path::new(&path)).expect("the deck reads");
    println!("{deck:#?}");
}
