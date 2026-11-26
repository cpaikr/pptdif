//! Writes a pair of invented decks that differ in most of the ways pptdif
//! reports, for looking at the HTML report.
//!
//! Usage: `cargo run -p pptdif-fixtures --example write_demo -- DIR`

use pptdif_fixtures::{RawDeck, RawSlide, chart_frame, chart_part, content_slide, placed_box, table, text_shape};

const INCH: i64 = 914_400;

fn cover(subtitle: &str) -> RawSlide {
    RawSlide::new(
        0,
        &[
            text_shape(2, "Title 1", Some(("ctrTitle", 0)), "Harbour project review"),
            placed_box(3, "Subtitle", subtitle, (3 * INCH, 4 * INCH, 7 * INCH, INCH), ""),
        ]
        .concat(),
    )
}

fn budget(steel: &str, values: &[f64]) -> RawSlide {
    let rows: &[&[&str]] = &[&["Item", "Cost"], &["Timber", "120"], &["Steel", steel]];
    let chart = chart_part(&["Q1", "Q2", "Q3"], &[("Spend", values)]);
    RawSlide::new(
        1,
        &[
            text_shape(2, "Title 1", Some(("title", 0)), "Budget"),
            table(4, "Costs", rows),
            chart_frame(5, "Spend chart", "rIdC"),
        ]
        .concat(),
    )
    .part("rIdC", "chart", "charts/chart1.xml", chart.as_bytes())
}

fn note(x: i64) -> String {
    placed_box(6, "Note", "Check the pier load figures", (x * INCH, 5 * INCH, 4 * INCH, INCH / 2), "")
}

fn main() {
    let dir = std::env::args().nth(1).expect("usage: write_demo DIR");
    let dir = std::path::Path::new(&dir);
    std::fs::create_dir_all(dir).expect("the directory can be created");
    let v1 = RawDeck::new()
        .slide(cover("Planning team"))
        .slide(content_slide("Progress", &["Pier foundations done", "Deck framing started", "Railings ordered"]))
        .slide(RawSlide::new(1, &[text_shape(2, "Title 1", Some(("title", 0)), "Site notes"), note(1)].concat()))
        .slide(budget("80", &[10.0, 12.0, 9.0]))
        .slide(content_slide("Risks", &["Weather delays", "Steel prices"]))
        .slide(content_slide("Questions", &["Contact the planning team"]));
    let v2 = RawDeck::new()
        .slide(cover("Planning and delivery teams"))
        .slide(content_slide(
            "Progress",
            &["Pier foundations done", "Deck framing finished", "Lighting designed", "Railings ordered"],
        ))
        .slide(RawSlide::new(1, &[text_shape(2, "Title 1", Some(("title", 0)), "Site notes"), note(4)].concat()))
        .slide(content_slide("Questions", &["Contact the planning team"]))
        .slide(budget("95", &[10.0, 14.0, 9.0]))
        .slide(content_slide("Next steps", &["Order the lighting", "Book the inspection"]));
    std::fs::write(dir.join("demo-v1.pptx"), v1.to_bytes()).expect("written");
    std::fs::write(dir.join("demo-v2.pptx"), v2.to_bytes()).expect("written");
}
