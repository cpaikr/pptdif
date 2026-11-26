//! Inheritance: what slides take from layouts, masters, the theme, and the
//! presentation's default style, and how changes to those are reported.

use pptdif::diff::{self, Change, CompareOptions, Comparison, Kind, Status};
use pptdif::model::Presentation;
use pptdif_fixtures::{RawDeck, RawSlide, font_scheme, paragraphs_shape, placed_box, placeholder_def, text_shape};

const INCH: i64 = 914_400;

/// A deck whose content layout places the title at `title_at` (inches)
/// and whose master sets titles to `title_size` hundredths of a point, in
/// the theme's minor font `minor`.
fn deck_with(title_at: i64, title_size: u32, minor: &str, slide: RawSlide) -> Presentation {
    let layout = placeholder_def(2, "Title 1", ("title", 0), (title_at * INCH, INCH / 2, 8 * INCH, INCH), "");
    let styles = format!(
        r#"<p:titleStyle><a:lvl1pPr algn="l"><a:defRPr sz="{title_size}"><a:latin typeface="+mj-lt"/></a:defRPr></a:lvl1pPr></p:titleStyle><p:bodyStyle><a:lvl1pPr><a:defRPr sz="2800"><a:latin typeface="+mn-lt"/></a:defRPr></a:lvl1pPr><a:lvl2pPr><a:defRPr sz="2400"><a:latin typeface="+mn-lt"/></a:defRPr></a:lvl2pPr></p:bodyStyle><p:otherStyle/>"#
    );
    let master = placeholder_def(2, "Title Placeholder 1", ("title", 0), (INCH, INCH / 2, 8 * INCH, INCH), "");
    let raw = RawDeck::new()
        .layout_shapes(1, &layout)
        .master(&master, &styles)
        .theme("Plain", &font_scheme("Georgia", minor))
        .presentation_xml(r#"<p:defaultTextStyle><a:lvl1pPr><a:defRPr sz="1800"><a:latin typeface="+mn-lt"/></a:defRPr></a:lvl1pPr></p:defaultTextStyle>"#)
        .slide(slide);
    pptdif::read_bytes(raw.to_bytes()).unwrap()
}

fn title(text: &str) -> String {
    // A placeholder that writes no geometry and no size.
    format!(
        r#"<p:sp><p:nvSpPr><p:cNvPr id="2" name="Title 1"/><p:cNvSpPr/><p:nvPr><p:ph type="title"/></p:nvPr></p:nvSpPr><p:spPr/><p:txBody><a:bodyPr/><a:p><a:r><a:t>{text}</a:t></a:r></a:p></p:txBody></p:sp>"#
    )
}

fn slide(extra: &str) -> RawSlide {
    RawSlide::new(1, &[title("Harbour"), extra.to_owned()].concat())
}

fn compare(a: &Presentation, b: &Presentation) -> Comparison {
    diff::compare("a.pptx", a, "b.pptx", b, CompareOptions::default())
}

fn changes(c: &Comparison) -> Vec<&Change> {
    c.slides.iter().flat_map(|p| &p.changes).collect()
}

#[test]
fn a_placeholder_takes_geometry_size_and_font_from_its_layout_master_and_theme() {
    let d = deck_with(2, 4400, "Calibri", slide(""));
    let t = &d.slides[0].shapes[0];
    assert!(t.geometry.is_none());
    assert_eq!(t.effective_geometry.map(|g| g.x), Some(2 * INCH));
    let run = &t.text.as_ref().unwrap()[0].runs[0].effective;
    assert_eq!(run.get("size").map(String::as_str), Some("44pt"));
    assert_eq!(run.get("font").map(String::as_str), Some("Georgia"));
    assert_eq!(t.text.as_ref().unwrap()[0].effective.get("align").map(String::as_str), Some("l"));
}

#[test]
fn a_text_box_takes_the_default_style() {
    let d = deck_with(2, 4400, "Calibri", slide(&placed_box(5, "Note", "Gate", (0, 0, INCH, INCH), "")));
    let run = &d.slides[0].shapes[1].text.as_ref().unwrap()[0].runs[0].effective;
    assert_eq!(
        (run.get("size").map(String::as_str), run.get("font").map(String::as_str)),
        (Some("18pt"), Some("Calibri"))
    );
}

#[test]
fn a_moved_layout_placeholder_is_one_layout_change_not_a_slide_change() {
    let (a, b) = (deck_with(2, 4400, "Calibri", slide("")), deck_with(3, 4400, "Calibri", slide("")));
    let c = compare(&a, &b);
    assert!(changes(&c).is_empty(), "{:?}", changes(&c));
    let kinds: Vec<(Kind, Status)> = c.objects.iter().map(|o| (o.kind, o.status)).collect();
    assert_eq!(kinds, [(Kind::Layout, Status::Changed)]);
    assert_eq!(c.objects[0].properties[0].name, "placeholder");
}

#[test]
fn a_master_style_change_is_one_master_change() {
    let (a, b) = (deck_with(2, 4400, "Calibri", slide("")), deck_with(2, 4000, "Calibri", slide("")));
    let c = compare(&a, &b);
    assert!(changes(&c).is_empty(), "{:?}", changes(&c));
    assert_eq!(c.objects.len(), 1);
    assert_eq!((c.objects[0].kind, c.objects[0].properties[0].name), (Kind::Master, "titleStyle"));
}

#[test]
fn a_theme_font_change_is_one_theme_change() {
    let (a, b) = (deck_with(2, 4400, "Calibri", slide("")), deck_with(2, 4400, "Arial", slide("")));
    let c = compare(&a, &b);
    assert!(changes(&c).is_empty(), "{:?}", changes(&c));
    assert_eq!(c.objects.len(), 1);
    assert_eq!((c.objects[0].kind, c.objects[0].properties[0].name), (Kind::Theme, "minorFont"));
}

#[test]
fn writing_the_inherited_value_is_no_change_and_overriding_it_is_one() {
    let body = |run: &str| paragraphs_shape(3, "Body", Some(("body", 1)), &[run]);
    let plain = deck_with(2, 4400, "Calibri", slide(&body("<a:r><a:t>Order railings</a:t></a:r>")));
    let same = deck_with(2, 4400, "Calibri", slide(&body(r#"<a:r><a:rPr sz="2800"/><a:t>Order railings</a:t></a:r>"#)));
    assert!(changes(&compare(&plain, &same)).is_empty());
    let bigger =
        deck_with(2, 4400, "Calibri", slide(&body(r#"<a:r><a:rPr sz="3200"/><a:t>Order railings</a:t></a:r>"#)));
    let c = compare(&plain, &bigger);
    let f: Vec<&Change> = changes(&c).into_iter().filter(|c| c.kind == Kind::Format).collect();
    assert_eq!(f.len(), 1, "{:?}", changes(&c));
    let p = &f[0].properties[0];
    assert_eq!((p.name, p.base.as_str(), p.other.as_str()), ("size", Some("28pt"), Some("32pt")));
}

#[test]
fn a_slide_that_moves_its_placeholder_off_the_layout_is_a_geometry_change() {
    let moved = format!(
        r#"<p:sp><p:nvSpPr><p:cNvPr id="2" name="Title 1"/><p:cNvSpPr/><p:nvPr><p:ph type="title"/></p:nvPr></p:nvSpPr><p:spPr><a:xfrm><a:off x="{}" y="{}"/><a:ext cx="{}" cy="{}"/></a:xfrm></p:spPr><p:txBody><a:bodyPr/><a:p><a:r><a:t>Harbour</a:t></a:r></a:p></p:txBody></p:sp>"#,
        4 * INCH,
        INCH / 2,
        8 * INCH,
        INCH
    );
    let a = deck_with(2, 4400, "Calibri", slide(""));
    let b = deck_with(2, 4400, "Calibri", RawSlide::new(1, &moved));
    let c = compare(&a, &b);
    let g: Vec<&Change> = changes(&c).into_iter().filter(|c| c.kind == Kind::Geometry).collect();
    assert_eq!(g.len(), 1, "{:?}", changes(&c));
    assert_eq!((g[0].properties[0].base.as_f64(), g[0].properties[0].other.as_f64()), (Some(144.0), Some(288.0)));
    let _ = text_shape;
}
