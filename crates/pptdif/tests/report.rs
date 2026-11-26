//! The HTML report: self-contained, drawn from the comparison, escaped.

use pptdif::diff::{self, CompareOptions};
use pptdif::html::{Images, Report, render};
use pptdif::model::Presentation;
use pptdif_fixtures::{RawDeck, RawSlide, content_slide, placed_box, text_shape};

const INCH: i64 = 914_400;

fn deck(slides: Vec<RawSlide>) -> Presentation {
    pptdif::read_bytes(slides.into_iter().fold(RawDeck::new(), RawDeck::slide).to_bytes()).unwrap()
}

fn pair() -> (Presentation, Presentation) {
    let a = deck(vec![
        content_slide("Plan <draft>", &["Order the timber railings this week", "Book the crane"]),
        RawSlide::new(
            1,
            &[
                text_shape(2, "Title 1", Some(("title", 0)), "Site"),
                placed_box(5, "Note", "Gate", (INCH, INCH, INCH, INCH), ""),
            ]
            .concat(),
        ),
    ]);
    let b = deck(vec![
        content_slide("Plan <draft>", &["Order the steel railings this week", "Book the crane"]),
        RawSlide::new(
            1,
            &[
                text_shape(2, "Title 1", Some(("title", 0)), "Site"),
                placed_box(5, "Note", "Gate", (3 * INCH, INCH, INCH, INCH), ""),
            ]
            .concat(),
        ),
        content_slide("Next steps", &["Inspect"]),
    ]);
    (a, b)
}

#[test]
fn the_report_is_one_self_contained_document() {
    let (a, b) = pair();
    let c = diff::compare("a.pptx", &a, "b.pptx", &b, CompareOptions::default());
    let html = render(&c, &a, &b, &Report::default());
    assert!(html.starts_with("<!doctype html>") && html.ends_with("</html>\n"));
    for external in ["src=\"http", "href=\"http", "url(http", "<link", "@import"] {
        assert!(!html.contains(external), "{external}");
    }
    // Every slide pair and every change appears, by id.
    for pair in &c.slides {
        assert!(html.contains(&format!("id=\"{}\"", pair.id)));
        for change in &pair.changes {
            assert!(html.contains(&format!("id=\"{}\"", change.id)), "{}", change.id);
        }
    }
    // Slide text is escaped; changed words are marked.
    assert!(html.contains("Plan &lt;draft&gt;") && !html.contains("Plan <draft>"));
    assert!(html.contains("<mark>steel</mark>"));
    // The moved box is outlined on both sides and points at its change.
    // The edited bullet's placeholder has no position in this synthetic layout,
    // so it is listed under the slide; the moved box is drawn in place.
    let outlined =
        html.matches(r#"class="shape changed"#).count() + html.matches(r#"class="shape unplaced changed"#).count();
    assert_eq!(outlined, 4, "text and geometry on both sides");
    assert!(html.contains("data-change=\"p2:geometry:s5:s5\""));
}

#[test]
fn slide_images_are_drawn_under_the_outlines() {
    let (a, b) = pair();
    let c = diff::compare("a.pptx", &a, "b.pptx", &b, CompareOptions::default());
    let mut images = Images::default();
    images.base.insert(2, "data:image/png;base64,AAAA".to_owned());
    let html = render(&c, &a, &b, &Report { warnings: &[], images: Some(&images) });
    assert!(html.contains("background-image:url('data:image/png;base64,AAAA')"));
    assert!(html.contains("slide imaged"));
}

#[test]
fn a_deck_against_itself_reports_no_changes() {
    let (a, _) = pair();
    let c = diff::compare("a.pptx", &a, "a.pptx", &a, CompareOptions::default());
    let html = render(&c, &a, &a, &Report::default());
    assert!(!html.contains("class=\"change "));
    assert!(html.contains("pair same"));
}

#[test]
fn group_members_are_placed_through_their_group() {
    // A group at x = 2 in, 4 in wide, whose child frame is 0..2 in: a member
    // at child x = 1 in lands at slide x = 4 in (one third of 12 in).
    let member = placed_box(5, "Member", "m", (INCH, 0, INCH, INCH), "");
    let group = format!(
        r#"<p:grpSp><p:nvGrpSpPr><p:cNvPr id="9" name="Pair"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr><a:xfrm><a:off x="{}" y="0"/><a:ext cx="{}" cy="{INCH}"/><a:chOff x="0" y="0"/><a:chExt cx="{}" cy="{INCH}"/></a:xfrm></p:grpSpPr>{member}</p:grpSp>"#,
        2 * INCH,
        4 * INCH,
        2 * INCH
    );
    let d = deck(vec![RawSlide::new(1, &group)]);
    let c = diff::compare("a.pptx", &d, "a.pptx", &d, CompareOptions::default());
    let html = render(&c, &d, &d, &Report::default());
    // The slide is 13.333 in wide: 4 in is 30%.
    assert!(html.contains(r#"title="Member""#));
    assert!(
        html.contains("left:30.000%"),
        "{}",
        &html[html.find("Member").unwrap() - 200..html.find("Member").unwrap()]
    );
}
