//! The deck PowerPoint built and saved (`scripts/powerpoint_fixture.py`).
//! Each assertion is a file-format fact observed in PowerPoint's own output
//! (PowerPoint for Microsoft 365, 2026-11-26); this file is the record of
//! what was seen.

use std::path::Path;

use pptdif::model::{DeckFormat, ShapeKind};

fn deck() -> pptdif::model::Presentation {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/powerpoint/powerpoint-saved.pptx");
    pptdif::read_path(&path).unwrap()
}

#[test]
fn slides_sections_and_layouts() {
    let deck = deck();
    assert_eq!(deck.format, DeckFormat::Pptx);
    // 16:9 at 960 x 540 pt.
    assert_eq!(deck.slide_size.map(|s| (s.cx, s.cy)), Some((12_192_000, 6_858_000)));
    assert_eq!(deck.slides.iter().map(|s| s.id).collect::<Vec<_>>(), [256, 257, 258, 259]);
    // Sections sit in presentation.xml's p14 extension, each listing its
    // slides by `sldId` id.
    let sections: Vec<(&str, &[u32])> =
        deck.sections.iter().map(|s| (s.name.as_str(), s.slide_ids.as_slice())).collect();
    assert_eq!(sections, [("Opening", &[256, 257][..]), ("Details", &[258, 259][..])]);
    assert_eq!(deck.slides[0].layout.as_deref(), Some("Title Slide"));
    assert_eq!(deck.masters[0].layouts.len(), 11);
}

#[test]
fn hidden_slides_carry_show_zero() {
    let hidden: Vec<bool> = deck().slides.iter().map(|s| s.hidden).collect();
    assert_eq!(hidden, [false, false, false, true]);
}

#[test]
fn notes_comments_and_animations() {
    let deck = deck();
    assert!(deck.slides[0].notes.is_some());
    // A modern comment: relationship type ending in `comments`
    // (office/2018/10), part `ppt/comments/modernComment_*.xml`, one
    // `p188:cm` per comment.
    assert_eq!(deck.slides.iter().map(|s| s.comments.len()).collect::<Vec<_>>(), [0, 0, 0, 1]);
    // PowerPoint writes `<p:timing>` only on the slide with an animation.
    assert_eq!(deck.slides.iter().map(|s| s.animated).collect::<Vec<_>>(), [false, true, false, false]);
}

#[test]
fn text_levels_and_notes() {
    let deck = deck();
    let body = deck.slides[1].shapes[1].text.as_ref().unwrap();
    let paragraphs: Vec<(u8, String)> = body.iter().map(|p| (p.level, p.text())).collect();
    // `IndentLevel = 2` is written as `lvl="1"`: levels count from 0.
    assert_eq!(
        paragraphs,
        [
            (0, "Pier foundations done".to_owned()),
            (0, "Deck framing started".to_owned()),
            (1, "Railings ordered".to_owned())
        ]
    );
    let notes = deck.slides[0].notes.as_ref().unwrap();
    assert_eq!(notes.iter().map(pptdif::model::Paragraph::text).collect::<Vec<_>>(), ["Open with the timeline."]);
}

#[test]
fn pictures_tables_charts_and_comments() {
    let deck = deck();
    let picture = &deck.slides[1].shapes[2];
    let image = picture.image.as_ref().unwrap();
    assert_eq!(image.part.as_deref(), Some("ppt/media/image1.png"));
    assert!(image.hash.is_some());
    // AddPicture(left 600 pt, top 300 pt, 80 x 80 pt) is written in EMU.
    let g = picture.geometry.unwrap();
    assert_eq!((g.x, g.y, g.cx, g.cy), (7_620_000, 3_810_000, 1_016_000, 1_016_000));
    // A text box PowerPoint adds says it has no fill.
    assert_eq!(deck.slides[1].shapes[3].children[0].fill.as_deref(), Some("noFill{}"));

    let table = deck.slides[2].shapes[1].table.as_ref().unwrap();
    let cells: Vec<Vec<String>> = table
        .rows
        .iter()
        .map(|r| r.iter().map(|c| c.text.iter().map(pptdif::model::Paragraph::text).collect()).collect())
        .collect();
    assert_eq!(cells, [["Item", "Cost"], ["Timber", "120"], ["Steel", "80"]]);

    // AddChart2's default data: three series over four categories, cached
    // in the chart part.
    let chart = deck.slides[2].shapes[2].chart.as_ref().unwrap().data.as_ref().unwrap();
    assert_eq!(chart.kinds, ["barChart"]);
    assert_eq!(chart.series.len(), 3);
    assert_eq!(chart.series[0].name.as_deref(), Some("Series 1"));
    assert_eq!(chart.series[0].categories, ["Category 1", "Category 2", "Category 3", "Category 4"]);
    assert_eq!(chart.series[0].values, [Some(4.3), Some(2.5), Some(3.5), Some(4.5)]);

    // A modern comment's author comes from ppt/authors.xml; removing
    // personal information renames it "Author".
    let comment = &deck.slides[3].comments[0];
    assert_eq!((comment.author.as_deref(), comment.text.as_str()), (Some("Author"), "Check the totals."));
}

#[test]
fn the_deck_compares_equal_to_itself() {
    let deck = deck();
    let c = pptdif::diff::compare("a", &deck, "b", &deck, pptdif::diff::CompareOptions::default());
    assert!(c.slides.iter().all(|p| p.changes.is_empty()));
}

#[test]
fn shapes_frames_and_groups() {
    let deck = deck();
    let kinds = |i: usize| deck.slides[i].all_shapes().map(|s| s.kind).collect::<Vec<_>>();
    assert_eq!(
        kinds(1),
        [ShapeKind::Shape, ShapeKind::Shape, ShapeKind::Picture, ShapeKind::Group, ShapeKind::Shape, ShapeKind::Shape]
    );
    assert_eq!(kinds(2), [ShapeKind::Shape, ShapeKind::Table, ShapeKind::Chart]);
    assert_eq!(kinds(3), [ShapeKind::Shape, ShapeKind::Diagram]);
    let title = deck.slides[0].shapes[0].placeholder.as_ref().unwrap();
    assert_eq!(title.kind, "ctrTitle");
    assert_eq!(deck.warnings.len(), 1);
    assert_eq!(deck.warnings[0].message, "pptdif does not compare 1 SmartArt diagram, 1 slide's animations");
}
