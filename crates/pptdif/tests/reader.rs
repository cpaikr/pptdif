//! The reader on synthetic decks assembled from XML parts.

use pptdif::model::{DeckFormat, ShapeKind};
use pptdif::{ErrorCode, Limits, read_bytes, read_bytes_with};
use pptdif_fixtures::{MACRO_PRESENTATION_TYPE, RawDeck, RawSlide, basic, frame, group, picture, text_shape, uri};

#[test]
fn slides_read_in_show_order_with_their_layouts() {
    let deck = read_bytes(basic().to_bytes()).unwrap();
    assert_eq!(deck.format, DeckFormat::Pptx);
    assert_eq!(deck.slide_size.map(|s| (s.cx, s.cy)), Some((12_192_000, 6_858_000)));
    let layouts: Vec<_> = deck.slides.iter().map(|s| s.layout.as_deref()).collect();
    assert_eq!(layouts, [Some("Title Slide"), Some("Title and Content")]);
    assert_eq!(deck.slides.iter().map(|s| s.id).collect::<Vec<_>>(), [256, 257]);
    assert_eq!(deck.masters.len(), 1);
    let names: Vec<Option<&str>> = deck.masters[0].layouts.iter().map(|l| l.as_deref()).collect();
    assert_eq!(names, [Some("Title Slide"), Some("Title and Content")]);
    assert!(deck.warnings.is_empty());
}

#[test]
fn shapes_keep_their_identity_and_placeholder() {
    let deck = read_bytes(basic().to_bytes()).unwrap();
    let shapes = &deck.slides[1].shapes;
    assert_eq!(
        shapes.iter().map(|s| s.kind).collect::<Vec<_>>(),
        [ShapeKind::Shape, ShapeKind::Shape, ShapeKind::Picture]
    );
    assert_eq!(shapes[1].id, Some(3));
    assert_eq!(shapes[1].name, "Content Placeholder 2");
    let ph = shapes[1].placeholder.as_ref().unwrap();
    assert_eq!((ph.kind.as_str(), ph.index), ("body", 1));
    assert!(shapes[2].placeholder.is_none());
}

#[test]
fn a_placeholder_without_type_or_index_takes_the_schema_defaults() {
    let shape = r#"<p:sp><p:nvSpPr><p:cNvPr id="2" name="Content"/><p:cNvSpPr/><p:nvPr><p:ph/></p:nvPr></p:nvSpPr><p:spPr/></p:sp>"#;
    let deck = read_bytes(RawDeck::new().slide(RawSlide::new(1, shape)).to_bytes()).unwrap();
    let ph = deck.slides[0].shapes[0].placeholder.as_ref().unwrap();
    assert_eq!((ph.kind.as_str(), ph.index), ("obj", 0));
}

#[test]
fn groups_hold_their_members_and_nest() {
    let inner = group(5, "Inner", &text_shape(6, "C", None, "c"));
    let outer = group(2, "Outer", &[text_shape(3, "A", None, "a"), picture(4, "B"), inner].concat());
    let deck = read_bytes(RawDeck::new().slide(RawSlide::new(1, &outer)).to_bytes()).unwrap();
    let slide = &deck.slides[0];
    assert_eq!(slide.shapes.len(), 1);
    let g = &slide.shapes[0];
    assert_eq!((g.kind, g.name.as_str(), g.children.len()), (ShapeKind::Group, "Outer", 3));
    assert_eq!(g.children[2].children[0].name, "C");
    let names: Vec<&str> = slide.all_shapes().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["Outer", "A", "B", "Inner", "C"]);
}

#[test]
fn graphic_frames_take_their_kind_from_the_graphic_data() {
    // An OLE frame carries a preview picture of its own; it is part of the
    // frame, not another shape.
    let preview = picture(9, "Preview");
    let shapes = [
        frame(2, "Table 1", uri::TABLE, "<a:tbl/>"),
        frame(3, "Chart 2", uri::CHART, ""),
        frame(4, "Diagram 3", uri::DIAGRAM, ""),
        frame(5, "Object 4", uri::OLE, &format!("<p:oleObj><p:embed/>{preview}</p:oleObj>")),
        frame(6, "Odd 5", "urn:example:unknown", ""),
        frame(7, "Strict table", "http://purl.oclc.org/ooxml/drawingml/table", ""),
    ]
    .concat();
    let deck = read_bytes(RawDeck::new().slide(RawSlide::new(1, &shapes)).to_bytes()).unwrap();
    let kinds: Vec<ShapeKind> = deck.slides[0].shapes.iter().map(|s| s.kind).collect();
    assert_eq!(
        kinds,
        [
            ShapeKind::Table,
            ShapeKind::Chart,
            ShapeKind::Diagram,
            ShapeKind::OleObject,
            ShapeKind::OtherFrame,
            ShapeKind::Table
        ]
    );
    assert_eq!(deck.slides[0].shapes[3].name, "Object 4");
    let warning = &deck.warnings[0];
    assert_eq!(warning.code, "not_compared");
    assert!(warning.message.contains("1 SmartArt diagram"), "{}", warning.message);
    assert!(warning.message.contains("1 embedded or linked object"), "{}", warning.message);
    assert!(warning.message.contains("1 other graphic frame"), "{}", warning.message);
}

#[test]
fn markup_compatibility_reads_the_choice_and_skips_the_fallback() {
    let shapes = format!(
        r#"<mc:AlternateContent xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006"><mc:Choice Requires="p14">{}</mc:Choice><mc:Fallback>{}</mc:Fallback></mc:AlternateContent>"#,
        text_shape(2, "New form", None, "x"),
        picture(2, "Old form")
    );
    let deck = read_bytes(RawDeck::new().slide(RawSlide::new(1, &shapes)).to_bytes()).unwrap();
    let names: Vec<&str> = deck.slides[0].all_shapes().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["New form"]);
}

#[test]
fn hidden_slides_notes_comments_and_animations() {
    let deck = RawDeck::new()
        .slide(RawSlide::new(0, "").notes("Speak slowly"))
        .slide(RawSlide::new(1, "").hidden().comments(2).animated());
    let deck = read_bytes(deck.to_bytes()).unwrap();
    let (a, b) = (&deck.slides[0], &deck.slides[1]);
    assert!(a.has_notes && !a.hidden && a.comments == 0 && !a.animated);
    assert!(!b.has_notes && b.hidden && b.comments == 2 && b.animated);
    assert!(deck.warnings[0].message.contains("1 slide's animations"), "{}", deck.warnings[0].message);
}

#[test]
fn sections_list_their_slides_apart_from_the_show_order() {
    let deck = RawDeck::new()
        .slide(RawSlide::new(0, ""))
        .slide(RawSlide::new(1, ""))
        .slide(RawSlide::new(1, ""))
        .section("Intro", &[0])
        .section("Body & end", &[1, 2]);
    let deck = read_bytes(deck.to_bytes()).unwrap();
    // The `sldId` entries inside sections must not add slides.
    assert_eq!(deck.slides.len(), 3);
    let sections: Vec<(&str, &[u32])> =
        deck.sections.iter().map(|s| (s.name.as_str(), s.slide_ids.as_slice())).collect();
    assert_eq!(sections, [("Intro", &[256][..]), ("Body & end", &[257, 258][..])]);
}

#[test]
fn macro_enabled_decks_and_their_macros_are_named() {
    let deck = RawDeck::new().main_type(MACRO_PRESENTATION_TYPE).slide(RawSlide::new(0, ""));
    let read = read_bytes(deck.to_bytes()).unwrap();
    assert_eq!(read.format, DeckFormat::Pptm);
    let custom = RawDeck::new()
        .slide(RawSlide::new(0, ""))
        .presentation_xml(r#"<p:custShowLst><p:custShow name="Short" id="0"><p:sldLst/></p:custShow></p:custShowLst>"#);
    let read = read_bytes(custom.to_bytes()).unwrap();
    assert!(read.warnings[0].message.contains("custom shows"), "{}", read.warnings[0].message);
}

#[test]
fn other_office_files_are_refused_by_kind() {
    let workbook =
        RawDeck::new().main_type("application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml");
    let err = read_bytes(workbook.to_bytes()).unwrap_err();
    assert_eq!(err.code, ErrorCode::UnsupportedFormat);
    assert!(err.hint.as_deref().unwrap_or_default().contains("xldif"));

    let err = read_bytes(b"not a zip at all".to_vec()).unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidArchive);
}

#[test]
fn the_record_budget_bounds_shapes() {
    let shapes: String = (2..50).map(|i| text_shape(i, "S", None, "x")).collect();
    let bytes = RawDeck::new().slide(RawSlide::new(1, &shapes)).to_bytes();
    let limits = Limits { max_records: 10, ..Limits::default() };
    let err = read_bytes_with(bytes, &limits).unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidArchive);
}

#[test]
fn only_the_first_of_several_choices_is_read() {
    let shapes = format!(
        r#"<mc:AlternateContent xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006"><mc:Choice Requires="p15">{}</mc:Choice><mc:Choice Requires="p14">{}</mc:Choice><mc:Fallback>{}</mc:Fallback></mc:AlternateContent>{}"#,
        text_shape(2, "A", None, "x"),
        text_shape(2, "B", None, "x"),
        text_shape(2, "C", None, "x"),
        text_shape(3, "After", None, "y"),
    );
    let deck = read_bytes(RawDeck::new().slide(RawSlide::new(1, &shapes)).to_bytes()).unwrap();
    let names: Vec<&str> = deck.slides[0].all_shapes().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["A", "After"]);
}

#[test]
fn media_ink_and_macros_are_named_as_not_compared() {
    let video = r#"<p:pic><p:nvPicPr><p:cNvPr id="2" name="Clip"/><p:cNvPicPr/><p:nvPr><a:videoFile r:link="rIdV"/></p:nvPr></p:nvPicPr><p:blipFill/><p:spPr/></p:pic>"#;
    // PowerPoint wraps ink in a markup-compatibility choice.
    let ink = r#"<mc:AlternateContent xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006"><mc:Choice Requires="p14"><p14:contentPart xmlns:p14="http://schemas.microsoft.com/office/powerpoint/2010/main" r:id="rIdI"/></mc:Choice></mc:AlternateContent>"#;
    let deck = RawDeck::new()
        .slide(RawSlide::new(1, &format!("{video}{ink}")))
        .presentation_rel("vbaProject", "vbaProject.bin");
    let read = read_bytes(deck.to_bytes()).unwrap();
    let kinds: Vec<ShapeKind> = read.slides[0].shapes.iter().map(|s| s.kind).collect();
    assert_eq!(kinds, [ShapeKind::Picture, ShapeKind::Ink]);
    assert!(read.slides[0].shapes[0].media);
    let message = &read.warnings[0].message;
    for part in ["1 video or audio clip", "1 ink drawing", "VBA macros"] {
        assert!(message.contains(part), "{message}");
    }
}

#[test]
fn an_empty_custom_show_list_defines_no_show() {
    let empty = RawDeck::new().slide(RawSlide::new(0, "")).presentation_xml("<p:custShowLst/>");
    assert!(read_bytes(empty.to_bytes()).unwrap().warnings.is_empty());
}

#[test]
fn legacy_replies_are_not_counted_as_comments() {
    let deck = RawDeck::new().slide(RawSlide::new(0, "").comments(1).replies(2));
    assert_eq!(read_bytes(deck.to_bytes()).unwrap().slides[0].comments, 1);
}

#[test]
fn a_slide_entry_must_name_a_slide_part_once() {
    let twice = RawDeck::new().slide(RawSlide::new(0, "")).slide_entry_xml(r#"<p:sldId id="300" r:id="rId2"/>"#);
    assert_eq!(read_bytes(twice.to_bytes()).unwrap_err().code, ErrorCode::InvalidXml);

    let layout = RawDeck::new()
        .slide(RawSlide::new(0, ""))
        .presentation_rel("slideLayout", "slideLayouts/slideLayout1.xml")
        .slide_entry_xml(r#"<p:sldId id="300" r:id="rIdP1"/>"#);
    assert_eq!(read_bytes(layout.to_bytes()).unwrap_err().code, ErrorCode::InvalidXml);
}

#[test]
fn the_record_budget_bounds_slides_sections_and_comments() {
    let limits = Limits { max_records: 10, ..Limits::default() };
    let slides = (0..12).fold(RawDeck::new(), |deck, _| deck.slide(RawSlide::new(0, "")));
    assert_eq!(read_bytes_with(slides.to_bytes(), &limits).unwrap_err().code, ErrorCode::InvalidArchive);

    let sections = (0..12).fold(RawDeck::new().slide(RawSlide::new(0, "")), |deck, _| deck.section("S", &[0]));
    assert_eq!(read_bytes_with(sections.to_bytes(), &limits).unwrap_err().code, ErrorCode::InvalidArchive);

    let comments = RawDeck::new().slide(RawSlide::new(0, "").comments(12));
    assert_eq!(read_bytes_with(comments.to_bytes(), &limits).unwrap_err().code, ErrorCode::InvalidArchive);
}
