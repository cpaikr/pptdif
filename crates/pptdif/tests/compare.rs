//! The comparison on synthetic decks: slide alignment, shape pairing,
//! paragraph changes, and the listing.

// Decks a and b, changes c, and their sides read best short in these tests.
#![allow(clippy::many_single_char_names)]

use pptdif::diff::{self, AlignMode, Change, CompareOptions, Comparison, Filter, Kind, Side, Status};
use pptdif::model::Presentation;
use pptdif_fixtures::{RawDeck, RawSlide, content_slide, paragraphs_shape, picture, text_shape};

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

fn five() -> Vec<RawSlide> {
    vec![
        content_slide("Agenda", &["Scope", "Timeline", "Budget"]),
        content_slide("Scope", &["Pier", "Deck", "Railings"]),
        content_slide("Timeline", &["Spring: foundations", "Summer: framing"]),
        content_slide("Budget", &["Timber 120", "Steel 80"]),
        content_slide("Questions", &["Contact the planning team"]),
    ]
}

#[test]
fn a_deck_against_itself_shows_nothing() {
    let a = deck(five());
    let c = compare(&a, &a);
    assert!(changes(&c).is_empty(), "{:?}", changes(&c));
    assert!(c.slides.iter().all(|p| p.status == Status::Same));
    let s = diff::summarize(&c);
    assert_eq!(s.slides_changed + s.slides_only_base + s.slides_only_other + s.slides_moved, 0);
}

#[test]
fn an_inserted_slide_is_one_change() {
    let mut slides = five();
    slides.insert(2, content_slide("Risks", &["Weather", "Supply"]));
    let c = compare(&deck(five()), &deck(slides));
    let all = changes(&c);
    assert_eq!(all.len(), 1, "{all:?}");
    assert_eq!((all[0].kind, all[0].status), (Kind::Slide, Status::OnlyOther));
    let side = all[0].other.as_ref().unwrap();
    assert_eq!((side.slide, side.text.as_deref()), (3, Some("Risks")));
}

#[test]
fn a_deleted_slide_is_one_change() {
    let mut slides = five();
    slides.remove(1);
    let c = compare(&deck(five()), &deck(slides));
    let all = changes(&c);
    assert_eq!(all.len(), 1, "{all:?}");
    assert_eq!((all[0].kind, all[0].status, all[0].base.as_ref().unwrap().slide), (Kind::Slide, Status::OnlyBase, 2));
}

#[test]
fn a_reordered_slide_is_moved() {
    let mut slides = five();
    let budget = slides.remove(3);
    slides.insert(1, budget);
    let c = compare(&deck(five()), &deck(slides));
    let moved: Vec<_> = c.slides.iter().filter(|p| p.moved).collect();
    assert_eq!(moved.len(), 1);
    assert_eq!(moved[0].base.as_ref().unwrap().title.as_deref(), Some("Budget"));
    let all = changes(&c);
    assert_eq!(all.len(), 1, "{all:?}");
    assert_eq!(all[0].status, Status::Moved);
}

#[test]
fn a_slide_moved_and_edited_is_still_one_slide() {
    let mut slides = five();
    slides.remove(3);
    slides.insert(0, content_slide("Budget", &["Timber 140", "Steel 80"]));
    let c = compare(&deck(five()), &deck(slides));
    let pair = c.slides.iter().find(|p| p.base.as_ref().is_some_and(|b| b.index == 4)).unwrap();
    assert!(pair.moved);
    assert_eq!(pair.other.as_ref().unwrap().index, 1);
    let text: Vec<_> = pair.changes.iter().filter(|c| c.kind == Kind::Text).collect();
    assert_eq!(text.len(), 1);
    assert_eq!(text[0].base.as_ref().unwrap().text.as_deref(), Some("Timber 120"));
    assert_eq!(text[0].other.as_ref().unwrap().text.as_deref(), Some("Timber 140"));
}

#[test]
fn an_edited_and_an_added_bullet_are_one_change_each() {
    let mut slides = five();
    slides[1] = content_slide("Scope", &["Pier", "Deck and stairs", "Lighting", "Railings"]);
    let c = compare(&deck(five()), &deck(slides));
    let all = changes(&c);
    assert_eq!(all.len(), 2, "{all:?}");
    assert_eq!(all[0].status, Status::Changed);
    assert_eq!(all[0].base.as_ref().unwrap().paragraph, Some(2));
    assert_eq!(all[0].other.as_ref().unwrap().text.as_deref(), Some("Deck and stairs"));
    assert_eq!((all[1].status, all[1].other.as_ref().unwrap().paragraph), (Status::OnlyOther, Some(3)));
    let shape = all[0].base.as_ref().unwrap().shape.as_ref().unwrap();
    assert_eq!((shape.name.as_str(), shape.placeholder.as_deref()), ("Content Placeholder 2", Some("body:1")));
    assert_eq!(all[0].id, "p2:text:s3.2:s3.2");
}

#[test]
fn renumbered_fields_are_not_changes() {
    let footer = |n: &str| {
        let field = format!(r#"<a:fld id="{{1}}" type="slidenum"><a:t>{n}</a:t></a:fld>"#);
        RawSlide::new(1, &paragraphs_shape(4, "Slide Number", Some(("sldNum", 12)), &[&field]))
    };
    let c = compare(&deck(vec![footer("3")]), &deck(vec![footer("4")]));
    assert!(changes(&c).is_empty(), "{:?}", changes(&c));
}

#[test]
fn shapes_pair_by_placeholder_and_report_added_and_renamed() {
    let base = RawSlide::new(1, &[text_shape(2, "Title 1", Some(("title", 0)), "Site"), picture(3, "Photo")].concat());
    let other = RawSlide::new(
        1,
        &[
            text_shape(7, "Heading", Some(("title", 0)), "Site"),
            picture(3, "Photo"),
            text_shape(9, "Caption", None, "North view"),
        ]
        .concat(),
    );
    let c = compare(&deck(vec![base]), &deck(vec![other]));
    let all = changes(&c);
    assert_eq!(all.len(), 2, "{all:?}");
    let renamed = all.iter().find(|c| c.status == Status::Changed).unwrap();
    assert_eq!((renamed.kind, renamed.properties[0].name), (Kind::Shape, "name"));
    let added = all.iter().find(|c| c.status == Status::OnlyOther).unwrap();
    assert_eq!(added.other.as_ref().unwrap().text.as_deref(), Some("North view"));
}

#[test]
fn notes_and_visibility_are_compared() {
    let a = deck(vec![content_slide("Plan", &["One"]).notes("Speak slowly")]);
    let b = deck(vec![content_slide("Plan", &["One"]).notes("Speak clearly").hidden()]);
    let c = compare(&a, &b);
    let kinds: Vec<(Kind, Status)> = changes(&c).iter().map(|c| (c.kind, c.status)).collect();
    assert_eq!(kinds, [(Kind::Slide, Status::Changed), (Kind::Notes, Status::Changed)]);
    assert_eq!(changes(&c)[0].properties[0].name, "hidden");
}

#[test]
fn position_alignment_pairs_slide_n_with_slide_n() {
    let mut slides = five();
    slides.insert(0, content_slide("Cover", &["Harbour project"]));
    let (a, b) = (deck(five()), deck(slides));
    let c = diff::compare("a", &a, "b", &b, CompareOptions { align: AlignMode::Position });
    assert!(c.slides.iter().take(5).all(|p| p.base.as_ref().unwrap().index == p.other.as_ref().unwrap().index));
    assert_eq!(c.slides.last().unwrap().status, Status::OnlyOther);
}

#[test]
fn the_listing_filters_and_counts_what_the_limit_cut() {
    let mut slides = five();
    slides[1] = content_slide("Scope", &["Pier only"]);
    slides[3] = content_slide("Budget", &["Timber 200", "Steel 90"]);
    let c = compare(&deck(five()), &deck(slides));
    let total = changes(&c).len();
    let limited = diff::listing(&c, &Filter { limit: 1, ..Filter::default() });
    assert_eq!(limited.listed, 1);
    let cut: usize = limited.omitted.values().flat_map(|m| m.values()).sum();
    assert_eq!(cut, total - 1);
    let one = diff::listing(&c, &Filter { slide: Some(4), ..Filter::default() });
    assert_eq!(one.slides.len(), 1);
    let every = diff::listing(&c, &Filter { all: true, ..Filter::default() });
    assert_eq!(every.slides.len(), 5);
}

fn describe(c: &Change, swap: bool) -> String {
    let (base, other) = if swap { (&c.other, &c.base) } else { (&c.base, &c.other) };
    let status = match (c.status, swap) {
        (Status::OnlyBase, true) => Status::OnlyOther,
        (Status::OnlyOther, true) => Status::OnlyBase,
        (s, _) => s,
    };
    let loc = |s: &Option<Side>| {
        s.as_ref().map(|s| (s.slide, s.shape.as_ref().map(|r| (r.id, r.name.clone())), s.paragraph, s.text.clone()))
    };
    let props: Vec<String> = c
        .properties
        .iter()
        .map(|p| {
            if swap {
                format!("{}:{}>{}", p.name, p.other, p.base)
            } else {
                format!("{}:{}>{}", p.name, p.base, p.other)
            }
        })
        .collect();
    format!("{:?} {status:?} {:?} {:?} {props:?}", c.kind, loc(base), loc(other))
}

/// Swapping the decks mirrors every change, and every slide pair's status
/// and moved flag.
fn assert_mirrors(a: &Presentation, b: &Presentation) {
    let (ab, ba) = (compare(a, b), compare(b, a));
    let mut forward: Vec<String> = changes(&ab).iter().map(|c| describe(c, false)).collect();
    let mut backward: Vec<String> = changes(&ba).iter().map(|c| describe(c, true)).collect();
    forward.sort();
    backward.sort();
    assert_eq!(forward, backward);
    let pairs = |c: &Comparison, swap: bool| {
        let mut v: Vec<String> = c
            .slides
            .iter()
            .map(|p| {
                let (x, y) = (p.base.as_ref().map(|r| r.index), p.other.as_ref().map(|r| r.index));
                let (x, y) = if swap { (y, x) } else { (x, y) };
                let status = match (p.status, swap) {
                    (Status::OnlyBase, true) => Status::OnlyOther,
                    (Status::OnlyOther, true) => Status::OnlyBase,
                    (s, _) => s,
                };
                format!("{x:?} {y:?} {status:?} {}", p.moved)
            })
            .collect();
        v.sort();
        v
    };
    assert_eq!(pairs(&ab, false), pairs(&ba, true));
}

#[test]
fn swapping_the_decks_mirrors_the_result() {
    let mut slides = five();
    slides.insert(2, content_slide("Risks", &["Weather"]));
    slides.remove(4);
    slides[1] = content_slide("Scope", &["Pier", "Deck", "Lighting", "Railings"]);
    assert_mirrors(&deck(five()), &deck(slides));
}

mod properties {
    use proptest::prelude::*;

    use super::{assert_mirrors, changes, compare, deck};
    use pptdif_fixtures::content_slide;

    const TITLES: [&str; 4] = ["One", "Two", "Three", "Four"];
    const WORDS: [&str; 6] = ["alpha", "beta", "gamma", "delta", "pier", "deck"];

    fn spec() -> impl Strategy<Value = Vec<(usize, Vec<usize>)>> {
        prop::collection::vec((0..4usize, prop::collection::vec(0..6usize, 0..4)), 0..6)
    }

    fn build(spec: &[(usize, Vec<usize>)]) -> pptdif::model::Presentation {
        deck(
            spec.iter()
                .map(|(t, ws)| {
                    let bullets: Vec<&str> = ws.iter().map(|&w| WORDS[w]).collect();
                    content_slide(TITLES[*t], &bullets)
                })
                .collect(),
        )
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        #[test]
        fn self_compare_is_empty(s in spec()) {
            let d = build(&s);
            prop_assert!(changes(&compare(&d, &d)).is_empty());
        }

        #[test]
        fn swapping_mirrors(a in spec(), b in spec()) {
            assert_mirrors(&build(&a), &build(&b));
        }
    }
}

#[test]
fn a_field_and_its_text_typed_in_differ() {
    let slide = |p: &str| {
        let footer = paragraphs_shape(4, "Slide Number", Some(("sldNum", 12)), &[p]);
        RawSlide::new(1, &[text_shape(2, "Title 1", Some(("title", 0)), "Budget"), footer].concat())
    };
    let field = deck(vec![slide(r#"<a:fld id="{1}" type="slidenum"><a:t>3</a:t></a:fld>"#)]);
    // Braces were once how a field compared; typed in, they are text.
    let typed = deck(vec![slide("<a:r><a:t>{slidenum}</a:t></a:r>")]);
    let c = compare(&field, &typed);
    assert_eq!(changes(&c).len(), 1, "{:?}", changes(&c));
}

#[test]
fn a_field_without_text_is_still_a_field() {
    let empty = r#"<a:fld id="{1}" type="datetime1"/>"#;
    let d = deck(vec![RawSlide::new(1, &paragraphs_shape(4, "Date", Some(("dt", 10)), &[empty]))]);
    let runs = &d.slides[0].shapes[0].text.as_ref().unwrap()[0].runs;
    assert_eq!(runs.len(), 1);
    assert!(matches!(&runs[0], pptdif::model::Run::Field { kind, text } if kind == "datetime1" && text.is_empty()));
}

#[test]
fn a_fields_paragraph_properties_do_not_set_the_level() {
    let p = r#"<a:pPr lvl="2"/><a:fld id="{1}" type="slidenum"><a:pPr lvl="0"/><a:t>3</a:t></a:fld>"#;
    let d = deck(vec![RawSlide::new(1, &paragraphs_shape(4, "N", None, &[p]))]);
    assert_eq!(d.slides[0].shapes[0].text.as_ref().unwrap()[0].level, 2);
}

#[test]
fn empty_shapes_do_not_pair_by_kind_alone() {
    let blank = |id: u32, name: &str| {
        format!(r#"<p:sp><p:nvSpPr><p:cNvPr id="{id}" name="{name}"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr><p:spPr/></p:sp>"#)
    };
    let c = compare(&deck(vec![RawSlide::new(1, &blank(5, "Old"))]), &deck(vec![RawSlide::new(1, &blank(6, "New"))]));
    let mut statuses: Vec<String> = changes(&c).iter().map(|c| format!("{:?}", c.status)).collect();
    statuses.sort();
    assert_eq!(statuses, ["OnlyBase", "OnlyOther"]);
}

#[test]
fn blank_slides_pair_by_layout() {
    let blank = || RawSlide::new(1, "");
    let a = deck(vec![content_slide("Cover", &["x"]), blank(), blank()]);
    assert!(changes(&compare(&a, &a)).is_empty());
    let b = deck(vec![content_slide("Cover", &["x"]), blank(), blank(), blank()]);
    let c = compare(&a, &b);
    assert_eq!(changes(&c).len(), 1, "{:?}", changes(&c));
}

#[test]
fn grown_paragraphs_pair_in_order() {
    let a = deck(vec![content_slide("Plan", &["Pier", "Deck", "Rail"])]);
    let b =
        deck(vec![content_slide("Plan", &["Pier footings poured", "New item", "Deck boards laid", "Rail posts set"])]);
    let c = compare(&a, &b);
    let edits: Vec<(Option<usize>, Option<usize>)> = changes(&c)
        .iter()
        .filter(|c| c.status == Status::Changed)
        .map(|c| (c.base.as_ref().and_then(|s| s.paragraph), c.other.as_ref().and_then(|s| s.paragraph)))
        .collect();
    assert_eq!(edits, [(Some(1), Some(1)), (Some(2), Some(3)), (Some(3), Some(4))]);
}

#[test]
fn a_hidden_shape_is_a_shape_change() {
    let shown = text_shape(5, "Note", None, "Draft");
    let hidden = shown.replace(r#"name="Note"/>"#, r#"name="Note" hidden="1"/>"#);
    let c = compare(&deck(vec![RawSlide::new(1, &shown)]), &deck(vec![RawSlide::new(1, &hidden)]));
    assert_eq!(changes(&c)[0].properties[0].name, "hidden");
}

#[test]
fn changed_paragraphs_carry_word_marks() {
    let a = deck(vec![content_slide("Plan", &["Order the timber railings this week"])]);
    let b = deck(vec![content_slide("Plan", &["Order the steel railings this week"])]);
    let c = compare(&a, &b);
    let marks = changes(&c)[0].marks.as_ref().expect("marks");
    assert_eq!((marks.base.len(), marks.other.len()), (1, 1));
    let (s, e) = marks.other[0];
    assert_eq!(&"Order the steel railings this week"[s as usize..e as usize], "steel");
}

#[test]
fn all_lists_unchanged_slides_whatever_the_kinds() {
    let mut slides = five();
    slides[1] = content_slide("Scope", &["Pier only"]);
    let c = compare(&deck(five()), &deck(slides));
    let listing = diff::listing(&c, &Filter { all: true, kinds: vec![Kind::Notes], ..Filter::default() });
    assert_eq!(listing.slides.len(), 4);
}
