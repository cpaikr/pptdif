//! Writes the public synthetic decks into a directory, for the parity
//! check (`scripts/parity.py`).
//!
//! Usage: `cargo run -p pptdif-fixtures --example write_fixtures -- DIR`

use pptdif_fixtures::{RawDeck, RawSlide, basic, frame, group, picture, text_shape, uri};

fn main() {
    let dir = std::env::args().nth(1).expect("usage: write_fixtures DIR");
    let dir = std::path::Path::new(&dir);
    std::fs::create_dir_all(dir).expect("the directory can be created");
    let objects = RawDeck::new()
        .slide(
            RawSlide::new(
                1,
                &[text_shape(2, "Title 1", Some(("title", 0)), "Objects"), frame(3, "Table 2", uri::TABLE, "<a:tbl/>")]
                    .concat(),
            )
            .notes("Notes"),
        )
        .slide(
            RawSlide::new(
                1,
                &group(2, "Group 1", &[picture(3, "Picture 2"), text_shape(4, "Box", None, "x")].concat()),
            )
            .hidden(),
        )
        .section("All", &[0, 1]);
    for (name, deck) in [("basic.pptx", basic()), ("objects.pptx", objects)] {
        std::fs::write(dir.join(name), deck.to_bytes()).expect("the fixture can be written");
    }
}
