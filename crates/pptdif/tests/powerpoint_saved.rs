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
    assert_eq!(deck.slides.iter().map(|s| s.comments).collect::<Vec<_>>(), [0, 0, 0, 1]);
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
