//! What a slide holds beyond text: geometry, formatting, pictures, tables,
//! charts, arrangement, comments, and the deck's sections.

use pptdif::diff::{self, Change, CompareOptions, Comparison, Kind, Status};
use pptdif::model::Presentation;
use pptdif_fixtures::{
    RawDeck, RawSlide, chart_frame, chart_part, content_slide, group, paragraphs_shape, picture_of, placed_box, table,
    text_shape,
};

fn deck(slides: Vec<RawSlide>) -> Presentation {
    let raw = slides.into_iter().fold(RawDeck::new(), RawDeck::slide);
    pptdif::read_bytes(raw.to_bytes()).unwrap()
}

fn compare(a: &Presentation, b: &Presentation) -> Comparison {
    diff::compare("a.pptx", a, "b.pptx", b, CompareOptions::default())
}

fn changes(c: &Comparison) -> Vec<&Change> {
    c.slides.iter().flat_map(|p| &p.changes).collect()
}

fn of(c: &Comparison, kind: Kind) -> Vec<&Change> {
    changes(c).into_iter().filter(|x| x.kind == kind).collect()
}

fn one(shapes: &str) -> RawSlide {
    RawSlide::new(1, &[text_shape(2, "Title 1", Some(("title", 0)), "Site plan"), shapes.to_owned()].concat())
}

const INCH: i64 = 914_400;

#[test]
fn a_moved_box_is_a_geometry_change_beyond_the_tolerance() {
    let at = |x: i64| deck(vec![one(&placed_box(5, "Note", "North gate", (x, INCH, 2 * INCH, INCH), ""))]);
    let c = compare(&at(INCH), &at(INCH + 254_000));
    let g = of(&c, Kind::Geometry);
    assert_eq!(g.len(), 1, "{:?}", changes(&c));
    assert_eq!(g[0].properties[0].name, "x");
    assert_eq!((g[0].properties[0].base.as_f64(), g[0].properties[0].other.as_f64()), (Some(72.0), Some(92.0)));
    // Within half a point: no change.
    assert!(changes(&compare(&at(INCH), &at(INCH + 3_000))).is_empty());
}

#[test]
fn bold_on_one_word_is_one_format_change_on_that_word() {
    let plain = paragraphs_shape(3, "Body", Some(("body", 1)), &["Order the railings"]);
    let bold = paragraphs_shape(
        3,
        "Body",
        Some(("body", 1)),
        &["<a:r><a:t>Order the </a:t></a:r><a:r><a:rPr b=\"1\"/><a:t>railings</a:t></a:r>"],
    );
    let c = compare(&deck(vec![one(&plain)]), &deck(vec![one(&bold)]));
    let f = of(&c, Kind::Format);
    assert_eq!(f.len(), 1, "{:?}", changes(&c));
    assert_eq!(f[0].base.as_ref().unwrap().text.as_deref(), Some("railings"));
    assert_eq!(f[0].properties[0].name, "bold");
    assert_eq!(f[0].properties[0].other, "1");
    assert!(of(&c, Kind::Text).is_empty());
}

#[test]
fn paragraph_alignment_and_shape_fill_are_format_changes() {
    let left = paragraphs_shape(3, "Body", Some(("body", 1)), &["<a:pPr algn=\"l\"/><a:r><a:t>Phase one</a:t></a:r>"]);
    let centre =
        paragraphs_shape(3, "Body", Some(("body", 1)), &["<a:pPr algn=\"ctr\"/><a:r><a:t>Phase one</a:t></a:r>"]);
    let fill = |rgb: &str| {
        placed_box(
            5,
            "Badge",
            "New",
            (0, 0, INCH, INCH),
            &format!("<a:solidFill><a:srgbClr val=\"{rgb}\"/></a:solidFill>"),
        )
    };
    let a = deck(vec![one(&[left, fill("FF0000")].concat())]);
    let b = deck(vec![one(&[centre, fill("00FF00")].concat())]);
    let names: Vec<&str> = of(&compare(&a, &b), Kind::Format).iter().map(|c| c.properties[0].name).collect();
    assert_eq!(names, ["align", "fill"]);
}

#[test]
fn a_replaced_image_is_a_picture_change_and_a_renamed_part_is_not() {
    let slide = |name: &str, bytes: &[u8]| one(&picture_of(4, "Photo", "rIdP")).part("rIdP", "image", name, bytes);
    let a = deck(vec![slide("media/image1.png", b"first image")]);
    let same_bytes = deck(vec![slide("media/image7.png", b"first image")]);
    let other_bytes = deck(vec![slide("media/image1.png", b"second image")]);
    assert!(changes(&compare(&a, &same_bytes)).is_empty());
    let c = compare(&a, &other_bytes);
    let p = of(&c, Kind::Picture);
    assert_eq!((p.len(), p[0].properties[0].name), (1, "image"));
}

#[test]
fn tables_report_cells_rows_and_columns() {
    let base: &[&[&str]] = &[&["Item", "Cost"], &["Timber", "120"], &["Steel", "80"]];
    let t = |rows: &[&[&str]]| deck(vec![one(&table(6, "Table 5", rows))]);

    let edited: &[&[&str]] = &[&["Item", "Cost"], &["Timber", "140"], &["Steel", "80"]];
    let c = compare(&t(base), &t(edited));
    let cells = of(&c, Kind::Table);
    assert_eq!(cells.len(), 1, "{:?}", changes(&c));
    let side = cells[0].other.as_ref().unwrap();
    assert_eq!((side.row, side.column, side.text.as_deref()), (Some(2), Some(2), Some("140")));

    let row_added: &[&[&str]] = &[&["Item", "Cost"], &["Timber", "120"], &["Glass", "60"], &["Steel", "80"]];
    let c = compare(&t(base), &t(row_added));
    let rows = of(&c, Kind::Table);
    assert_eq!(rows.len(), 1, "{:?}", changes(&c));
    assert_eq!((rows[0].status, rows[0].other.as_ref().unwrap().row), (Status::OnlyOther, Some(3)));

    let column_added: &[&[&str]] =
        &[&["Item", "Supplier", "Cost"], &["Timber", "Mill", "120"], &["Steel", "Forge", "80"]];
    let c = compare(&t(base), &t(column_added));
    let cols = of(&c, Kind::Table);
    assert_eq!(cols.len(), 1, "{:?}", changes(&c));
    assert_eq!((cols[0].status, cols[0].other.as_ref().unwrap().column), (Status::OnlyOther, Some(2)));
}

#[test]
fn chart_series_values_and_additions_are_reported() {
    let slide = |series: &[(&str, &[f64])]| {
        one(&chart_frame(7, "Chart 6", "rIdC")).part(
            "rIdC",
            "chart",
            "charts/chart1.xml",
            chart_part(&["Q1", "Q2"], series).as_bytes(),
        )
    };
    let a = deck(vec![slide(&[("Revenue", &[10.0, 12.0])])]);
    let b = deck(vec![slide(&[("Revenue", &[10.0, 15.0]), ("Cost", &[8.0, 9.0])])]);
    let c = compare(&a, &b);
    let ch = of(&c, Kind::Chart);
    assert_eq!(ch.len(), 2, "{:?}", changes(&c));
    let added = ch.iter().find(|x| x.status == Status::OnlyOther).unwrap();
    assert_eq!(added.other.as_ref().unwrap().text.as_deref(), Some("Cost"));
    let edited = ch.iter().find(|x| x.status == Status::Changed).unwrap();
    assert_eq!(edited.properties[0].name, "values");
}

#[test]
fn stacking_and_grouping_are_shape_changes() {
    let (b1, b2) = (
        placed_box(5, "Back", "one", (0, 0, INCH, INCH), ""),
        placed_box(6, "Front", "two", (2 * INCH, 0, INCH, INCH), ""),
    );
    let a = deck(vec![one(&[b1.clone(), b2.clone()].concat())]);
    let swapped = deck(vec![one(&[b2.clone(), b1.clone()].concat())]);
    let z: Vec<&str> = of(&compare(&a, &swapped), Kind::Shape).iter().map(|c| c.properties[0].name).collect();
    assert_eq!(z, ["zOrder"]);
    let grouped = deck(vec![one(&group(9, "Pair", &[b1, b2].concat()))]);
    let c = compare(&a, &grouped);
    let g = of(&c, Kind::Shape);
    assert_eq!(g.len(), 2, "{:?}", changes(&c));
    assert!(g.iter().all(|x| x.properties[0].name == "group"));
}

#[test]
fn comments_are_compared_by_text_and_author() {
    let authors = r#"<p:cmAuthorLst xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"><p:cmAuthor id="0" name="Reviewer" initials="R" lastIdx="2" clrIdx="0"/></p:cmAuthorLst>"#;
    let with = |slide: RawSlide| {
        let raw = RawDeck::new()
            .slide(slide)
            .text_part("ppt/commentAuthors.xml", authors)
            .presentation_rel("commentAuthors", "commentAuthors.xml");
        pptdif::read_bytes(raw.to_bytes()).unwrap()
    };
    let a = with(content_slide("Plan", &["One"]).comments(1));
    assert_eq!(a.slides[0].comments[0].author.as_deref(), Some("Reviewer"));
    let b = with(content_slide("Plan", &["One"]).comments(2));
    let c = compare(&a, &b);
    let cm = of(&c, Kind::Comment);
    assert_eq!(cm.len(), 1, "{:?}", changes(&c));
    assert_eq!(cm[0].other.as_ref().unwrap().text.as_deref(), Some("Reviewer: Comment 1"));
}

#[test]
fn sections_report_renames_and_moved_slides() {
    let slides = || (0..3).map(|i| content_slide(["One", "Two", "Three"][i], &["x"])).collect::<Vec<_>>();
    let build = |sections: &[(&str, &[usize])]| {
        let raw = sections
            .iter()
            .fold(slides().into_iter().fold(RawDeck::new(), RawDeck::slide), |d, (n, s)| d.section(n, s));
        pptdif::read_bytes(raw.to_bytes()).unwrap()
    };
    let a = build(&[("Intro", &[0]), ("Body", &[1, 2])]);
    let moved = build(&[("Intro", &[0, 1]), ("Body", &[2])]);
    let c = compare(&a, &moved);
    assert_eq!(c.objects.len(), 2, "{:?}", c.objects);
    assert!(c.objects.iter().all(|o| o.status == Status::Changed && o.properties[0].name == "slides"));
    let renamed = build(&[("Opening", &[0]), ("Body", &[1, 2])]);
    let statuses: Vec<Status> = compare(&a, &renamed).objects.iter().map(|o| o.status).collect();
    assert_eq!(statuses, [Status::OnlyBase, Status::OnlyOther]);
}

#[test]
fn object_changes_mirror_when_the_decks_swap() {
    let base: &[&[&str]] = &[&["Item", "Cost"], &["Timber", "120"]];
    let other: &[&[&str]] = &[&["Item", "Cost"], &["Timber", "125"], &["Steel", "80"]];
    let a = deck(vec![one(&[table(6, "T", base), placed_box(5, "N", "gate", (0, 0, INCH, INCH), "")].concat())]);
    let b = deck(vec![one(&[table(6, "T", other), placed_box(5, "N", "gate", (INCH, 0, INCH, INCH), "")].concat())]);
    let (ab, ba) = (compare(&a, &b), compare(&b, &a));
    let describe = |c: &Change, swap: bool| {
        let (x, y) = if swap { (&c.other, &c.base) } else { (&c.base, &c.other) };
        format!(
            "{:?} {:?} {:?}",
            c.kind,
            x.as_ref().map(|s| (s.row, s.column, s.text.clone())),
            y.as_ref().map(|s| (s.row, s.column, s.text.clone()))
        )
    };
    let mut f: Vec<String> = changes(&ab).iter().map(|c| describe(c, false)).collect();
    let mut r: Vec<String> = changes(&ba).iter().map(|c| describe(c, true)).collect();
    f.sort();
    r.sort();
    assert_eq!(f, r);
}
