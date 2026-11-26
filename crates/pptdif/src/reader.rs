//! Reads a presentation package into the [`Presentation`] model: slide
//! order, sections, masters and layouts, and each slide's shape tree,
//! notes, and comments.
//!
//! Content pptdif does not compare (SmartArt, OLE objects, media, ink,
//! animations, custom shows, VBA) is counted while reading and reported as
//! one `not_compared` warning, so "no changes" never silently covers it.

use std::collections::{HashMap, HashSet};
use std::io::{Read, Seek};
use std::path::Path;
use std::sync::Arc;

use crate::model::{
    DeckFormat, Master, Paragraph, Placeholder, Presentation, Run, Section, Shape, ShapeKind, Size, Slide, Warning,
};
use crate::package::{Limits, Package, Relationship};
use crate::xml::{Element, Event, Parser};
use crate::{Error, ErrorCode};

pub fn read_path(path: &Path) -> Result<Presentation, Error> {
    read_path_with(path, &Limits::default())
}

pub fn read_path_with(path: &Path, limits: &Limits) -> Result<Presentation, Error> {
    read_package(&mut Package::open(path, limits)?, limits)
}

pub fn read_bytes(bytes: Vec<u8>) -> Result<Presentation, Error> {
    read_bytes_with(bytes, &Limits::default())
}

pub fn read_bytes_with(bytes: Vec<u8>, limits: &Limits) -> Result<Presentation, Error> {
    read_package(&mut Package::from_bytes(bytes, limits)?, limits)
}

fn missing(part: &str) -> Error {
    Error::new(ErrorCode::InvalidXml, format!("the presentation has no {part} part"))
}

/// Counts records (slides, shapes, comments) against
/// [`Limits::max_records`], so memory stays proportional to the limit
/// whatever the file's compression ratio.
struct Budget {
    left: usize,
    limit: usize,
}

impl Budget {
    fn take(&mut self) -> Result<(), Error> {
        self.left = self.left.checked_sub(1).ok_or_else(|| {
            Error::new(ErrorCode::InvalidArchive, format!("the presentation holds more than {} records", self.limit))
        })?;
        Ok(())
    }
}

/// What the model has no place for, counted for the `not_compared` warning.
#[derive(Default)]
struct Unread {
    diagrams: usize,
    ole_objects: usize,
    media: usize,
    ink: usize,
    other_frames: usize,
    animated_slides: usize,
    custom_shows: bool,
    vba: bool,
}

impl Unread {
    fn count_shapes(&mut self, slide: &Slide) {
        for shape in slide.all_shapes() {
            match shape.kind {
                ShapeKind::Diagram => self.diagrams += 1,
                ShapeKind::OleObject => self.ole_objects += 1,
                ShapeKind::Ink => self.ink += 1,
                ShapeKind::OtherFrame => self.other_frames += 1,
                _ => {}
            }
            if shape.media {
                self.media += 1;
            }
        }
        if slide.animated {
            self.animated_slides += 1;
        }
    }

    fn warning(&self) -> Option<Warning> {
        let plural = |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
        let mut parts = Vec::new();
        for (n, one, many) in [
            (self.diagrams, "SmartArt diagram", "SmartArt diagrams"),
            (self.ole_objects, "embedded or linked object", "embedded or linked objects"),
            (self.media, "video or audio clip", "video or audio clips"),
            (self.ink, "ink drawing", "ink drawings"),
            (self.other_frames, "other graphic frame", "other graphic frames"),
            (self.animated_slides, "slide's animations", "slides' animations"),
        ] {
            if n > 0 {
                parts.push(plural(n, one, many));
            }
        }
        if self.custom_shows {
            parts.push("custom shows".to_owned());
        }
        if self.vba {
            parts.push("VBA macros".to_owned());
        }
        (!parts.is_empty())
            .then(|| Warning { code: "not_compared", message: format!("pptdif does not compare {}", parts.join(", ")) })
    }
}

fn format_of(content_type: Option<&str>) -> Result<DeckFormat, Error> {
    let format = match content_type {
        Some("application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml") => DeckFormat::Pptx,
        Some("application/vnd.ms-powerpoint.presentation.macroEnabled.main+xml") => DeckFormat::Pptm,
        Some("application/vnd.openxmlformats-officedocument.presentationml.template.main+xml") => DeckFormat::Potx,
        Some("application/vnd.ms-powerpoint.template.macroEnabled.main+xml") => DeckFormat::Potm,
        Some("application/vnd.openxmlformats-officedocument.presentationml.slideshow.main+xml") => DeckFormat::Ppsx,
        Some("application/vnd.ms-powerpoint.slideshow.macroEnabled.main+xml") => DeckFormat::Ppsm,
        Some(other) if other.contains("spreadsheetml") || other.contains("ms-excel") => {
            return Err(Error::new(ErrorCode::UnsupportedFormat, "this is an Excel workbook, not a presentation")
                .with_hint("compare workbooks with xldif"));
        }
        Some(other) => {
            return Err(Error::new(
                ErrorCode::UnsupportedFormat,
                format!("the main part is {other}, not a PowerPoint presentation"),
            ));
        }
        None => return Err(Error::new(ErrorCode::UnsupportedFormat, "the main part declares no content type")),
    };
    Ok(format)
}

fn read_package<R: Read + Seek>(pkg: &mut Package<R>, limits: &Limits) -> Result<Presentation, Error> {
    let mut budget = Budget { left: limits.max_records, limit: limits.max_records };
    let root = pkg.relationships("")?;
    let main = root
        .iter()
        .find(|r| r.kind == "officeDocument" && !r.external)
        .ok_or_else(|| Error::new(ErrorCode::InvalidArchive, "the file is not an Office Open XML package"))?
        .target
        .clone();
    let format = format_of(pkg.content_type(&main)?.as_deref())?;
    let text = pkg.read_text(&main)?.ok_or_else(|| missing(&main))?;
    let listing = read_presentation_part(&text, &main, &mut budget)?;
    let rels: HashMap<String, Relationship> =
        pkg.relationships(&main)?.into_iter().map(|r| (r.id.clone(), r)).collect();

    let mut unread = Unread { custom_shows: listing.custom_shows, ..Unread::default() };
    unread.vba = rels.values().any(|r| r.kind == "vbaProject");

    let mut layouts = LayoutNames::default();
    let mut masters = Vec::new();
    for rid in &listing.master_rids {
        let Some(rel) = rels.get(rid).filter(|r| r.kind == "slideMaster" && !r.external) else { continue };
        masters.push(read_master(pkg, &rel.target, &mut layouts, &mut budget)?);
    }

    let mut slides = Vec::new();
    // PowerPoint lists each slide part once; a part listed again would be
    // read again, so it is refused.
    let mut seen = HashSet::new();
    for (id, rid) in &listing.slides {
        budget.take()?;
        let rel = rels.get(rid).filter(|r| r.kind == "slide" && !r.external).ok_or_else(|| {
            Error::new(ErrorCode::InvalidXml, format!("{main}: slide {id} names no slide part ({rid})"))
        })?;
        if !seen.insert(rel.target.as_str()) {
            return Err(Error::new(
                ErrorCode::InvalidXml,
                format!("{main}: {} is listed as more than one slide", rel.target),
            ));
        }
        let slide = read_slide(pkg, &rel.target, *id, &mut layouts, &mut budget)?;
        unread.count_shapes(&slide);
        slides.push(slide);
    }

    Ok(Presentation {
        format,
        slide_size: listing.slide_size,
        slides,
        sections: listing.sections,
        masters,
        warnings: unread.warning().into_iter().collect(),
    })
}

/// What `presentation.xml` lists.
struct Listing {
    slide_size: Option<Size>,
    /// `(sldId id, r:id)` in show order.
    slides: Vec<(u32, String)>,
    master_rids: Vec<String>,
    sections: Vec<Section>,
    custom_shows: bool,
}

fn number<T: std::str::FromStr>(e: &Element, name: &str) -> Option<T> {
    e.plain_attr(name).and_then(|v| v.parse().ok())
}

fn read_presentation_part(text: &str, part: &str, budget: &mut Budget) -> Result<Listing, Error> {
    let mut parser = Parser::new(text, part);
    let mut listing = Listing {
        slide_size: None,
        slides: Vec::new(),
        master_rids: Vec::new(),
        sections: Vec::new(),
        custom_shows: false,
    };
    // `sldId` appears both in the show order and, with the same local
    // name, inside each section of the `p14` extension.
    let mut in_slide_list = false;
    let mut in_sections = false;
    loop {
        match parser.next()? {
            Event::Start(e) => match e.name.as_str() {
                "sldSz" => {
                    if let (Some(cx), Some(cy)) = (number(&e, "cx"), number(&e, "cy")) {
                        listing.slide_size = Some(Size { cx, cy });
                    }
                }
                "sldMasterId" => {
                    if let Some(rid) = e.prefixed_attr("id") {
                        listing.master_rids.push(rid.to_owned());
                    }
                }
                "sldIdLst" if !in_sections => in_slide_list = true,
                "sectionLst" => in_sections = true,
                "section" if in_sections => {
                    budget.take()?;
                    let name = e.plain_attr("name").unwrap_or_default().to_owned();
                    listing.sections.push(Section { name, slide_ids: Vec::new() });
                }
                "sldId" if in_sections => {
                    if let (Some(section), Some(id)) = (listing.sections.last_mut(), number(&e, "id")) {
                        budget.take()?;
                        section.slide_ids.push(id);
                    }
                }
                "sldId" if in_slide_list => {
                    let (Some(id), Some(rid)) = (number(&e, "id"), e.prefixed_attr("id")) else {
                        return Err(Error::new(
                            ErrorCode::InvalidXml,
                            format!("{part}: a slide entry lacks id or r:id"),
                        ));
                    };
                    listing.slides.push((id, rid.to_owned()));
                }
                // An empty list defines no show.
                "custShow" => listing.custom_shows = true,
                _ => {}
            },
            Event::End(name) => match name.as_str() {
                "sldIdLst" => in_slide_list = false,
                "sectionLst" => in_sections = false,
                _ => {}
            },
            Event::Text(_) => {}
            Event::Eof => return Ok(listing),
        }
    }
}

/// The name in a slide, layout, or master part's `<p:cSld name>`.
fn common_slide_name(text: &str, part: &str) -> Result<Option<Arc<str>>, Error> {
    let mut parser = Parser::new(text, part);
    loop {
        match parser.next()? {
            Event::Start(e) if e.name == "cSld" => {
                return Ok(e.plain_attr("name").filter(|n| !n.is_empty()).map(Arc::from));
            }
            Event::Eof => return Ok(None),
            _ => {}
        }
    }
}

/// Layout names by part, read once however many slides use a layout and
/// shared, so a name is never copied per use.
#[derive(Default)]
struct LayoutNames(HashMap<String, Option<Arc<str>>>);

impl LayoutNames {
    fn get<R: Read + Seek>(&mut self, pkg: &mut Package<R>, part: &str) -> Result<Option<Arc<str>>, Error> {
        if let Some(name) = self.0.get(part) {
            return Ok(name.clone());
        }
        let name = match pkg.read_text(part)? {
            Some(text) => common_slide_name(&text, part)?,
            None => None,
        };
        self.0.insert(part.to_owned(), name.clone());
        Ok(name)
    }
}

fn read_master<R: Read + Seek>(
    pkg: &mut Package<R>,
    part: &str,
    names: &mut LayoutNames,
    budget: &mut Budget,
) -> Result<Master, Error> {
    let text = pkg.read_text(part)?.ok_or_else(|| missing(part))?;
    let name = common_slide_name(&text, part)?;
    let rels: HashMap<String, Relationship> = pkg.relationships(part)?.into_iter().map(|r| (r.id.clone(), r)).collect();
    // The master lists its layouts in order (`sldLayoutIdLst`); its
    // relationships alone have no order.
    let mut layout_rids = Vec::new();
    let mut parser = Parser::new(&text, part);
    loop {
        match parser.next()? {
            Event::Start(e) if e.name == "sldLayoutId" => {
                if let Some(rid) = e.prefixed_attr("id") {
                    budget.take()?;
                    layout_rids.push(rid.to_owned());
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    let mut layouts = Vec::new();
    for rid in layout_rids {
        if let Some(rel) = rels.get(&rid).filter(|r| r.kind == "slideLayout" && !r.external) {
            layouts.push(names.get(pkg, &rel.target)?);
        }
    }
    Ok(Master { name, layouts })
}

fn read_slide<R: Read + Seek>(
    pkg: &mut Package<R>,
    part: &str,
    id: u32,
    layouts: &mut LayoutNames,
    budget: &mut Budget,
) -> Result<Slide, Error> {
    let text = pkg.read_text(part)?.ok_or_else(|| missing(part))?;
    let rels = pkg.relationships(part)?;
    let mut slide = Slide {
        part: part.to_owned(),
        id,
        hidden: false,
        layout: None,
        shapes: Vec::new(),
        notes: None,
        comments: 0,
        animated: false,
    };
    let mut parser = Parser::new(&text, part);
    loop {
        match parser.next()? {
            Event::Start(e) => match e.name.as_str() {
                // Hidden slides carry `show="0"` on the root.
                "sld" => slide.hidden = matches!(e.plain_attr("show"), Some("0" | "false")),
                "spTree" => slide.shapes = read_shape(&mut parser, ShapeKind::Group, budget)?.children,
                "timing" => slide.animated = true,
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
    }
    for rel in rels.iter().filter(|r| !r.external) {
        match rel.kind.as_str() {
            "slideLayout" => slide.layout = layouts.get(pkg, &rel.target)?,
            "notesSlide" => {
                let notes = match pkg.read_text(&rel.target)? {
                    Some(text) => read_notes(&text, &rel.target, budget)?,
                    None => Vec::new(),
                };
                slide.notes = Some(notes);
            }
            // Legacy comment lists and modern comments both end their
            // relationship type in `comments` (modern observed, legacy
            // unverified; TODO.md).
            "comments" => {
                if let Some(text) = pkg.read_text(&rel.target)? {
                    slide.comments += count_comments(&text, &rel.target, budget)?;
                }
            }
            _ => {}
        }
    }
    Ok(slide)
}

/// Comments without their replies. A modern comment keeps its replies in
/// `p188:replyLst`; a legacy reply is a `cm` of its own whose
/// `p15:threadingInfo` names a `parentCm` (unverified; TODO.md).
fn count_comments(text: &str, part: &str, budget: &mut Budget) -> Result<usize, Error> {
    let mut parser = Parser::new(text, part);
    let mut n = 0;
    let mut reply = false;
    loop {
        match parser.next()? {
            Event::Start(e) if e.name == "cm" => {
                budget.take()?;
                reply = false;
            }
            Event::Start(e) if e.name == "parentCm" => reply = true,
            Event::End(name) if name == "cm" && !reply => n += 1,
            Event::Eof => return Ok(n),
            _ => {}
        }
    }
}

/// The element names that are one shape in a shape tree.
fn shape_kind(name: &str) -> Option<ShapeKind> {
    Some(match name {
        "sp" => ShapeKind::Shape,
        "grpSp" => ShapeKind::Group,
        "pic" => ShapeKind::Picture,
        "cxnSp" => ShapeKind::Connector,
        // A graphic frame's kind is settled by its `graphicData`.
        "graphicFrame" => ShapeKind::OtherFrame,
        "contentPart" => ShapeKind::Ink,
        _ => return None,
    })
}

fn frame_kind(uri: &str) -> ShapeKind {
    // Transitional and Strict namespaces differ only before the last segment.
    match uri.rsplit('/').next().unwrap_or(uri) {
        "table" => ShapeKind::Table,
        "chart" | "chartex" => ShapeKind::Chart,
        "diagram" => ShapeKind::Diagram,
        "ole" => ShapeKind::OleObject,
        _ => ShapeKind::OtherFrame,
    }
}

/// One shape, its start tag just read, through its end tag. A group's
/// members are read into `children`; the slide's `spTree` is read as a
/// group whose members are the slide's shapes. A markup compatibility
/// block contributes its first choice, whatever it `Requires`: later
/// choices and the fallback repeat the content for other readers
/// (unverified on a PowerPoint-saved file; TODO.md).
fn read_shape(parser: &mut Parser<'_>, mut kind: ShapeKind, budget: &mut Budget) -> Result<Shape, Error> {
    let mut shape = Shape {
        kind,
        id: None,
        name: String::new(),
        hidden: false,
        placeholder: None,
        media: false,
        text: None,
        children: Vec::new(),
    };
    let mut seen_identity = false;
    let mut depth = 1usize;
    // For each open `mc:AlternateContent`, whether a choice was taken.
    let mut alternates: Vec<bool> = Vec::new();
    loop {
        match parser.next()? {
            Event::Start(e) => {
                let later_choice = e.name == "Choice" && alternates.last() == Some(&true);
                if e.name == "Fallback" || later_choice {
                    parser.skip_element()?;
                    continue;
                }
                match e.name.as_str() {
                    "AlternateContent" => alternates.push(false),
                    "Choice" => {
                        if let Some(taken) = alternates.last_mut() {
                            *taken = true;
                        }
                    }
                    _ => {}
                }
                // Inside a group every shape element is a member; inside any
                // other shape it is part of that shape (an OLE frame's
                // preview picture).
                if kind == ShapeKind::Group
                    && let Some(member) = shape_kind(&e.name)
                {
                    budget.take()?;
                    shape.children.push(read_shape(parser, member, budget)?);
                    continue;
                }
                // A table cell's text body belongs to the table (M2); only
                // an autoshape's own body is the shape's text.
                if e.name == "txBody" && kind == ShapeKind::Shape && shape.text.is_none() {
                    shape.text = Some(read_text_body(parser, budget)?);
                    continue;
                }
                depth += 1;
                match e.name.as_str() {
                    // The first `cNvPr` is the shape's own.
                    "cNvPr" if !seen_identity => {
                        seen_identity = true;
                        shape.id = number(&e, "id");
                        e.plain_attr("name").unwrap_or_default().clone_into(&mut shape.name);
                        shape.hidden = matches!(e.plain_attr("hidden"), Some("1" | "true"));
                    }
                    "ph" if shape.placeholder.is_none() => {
                        shape.placeholder = Some(Placeholder {
                            kind: e.plain_attr("type").unwrap_or("obj").to_owned(),
                            index: number(&e, "idx").unwrap_or(0),
                        });
                    }
                    "graphicData" if kind == ShapeKind::OtherFrame => {
                        kind = frame_kind(e.plain_attr("uri").unwrap_or_default());
                        shape.kind = kind;
                    }
                    "videoFile" | "audioFile" | "quickTimeFile" | "media" if kind == ShapeKind::Picture => {
                        shape.media = true;
                    }
                    _ => {}
                }
            }
            Event::End(name) => {
                if name == "AlternateContent" {
                    alternates.pop();
                }
                depth -= 1;
                if depth == 0 {
                    return Ok(shape);
                }
            }
            Event::Text(_) => {}
            // The parser reports a part that ends inside an open element.
            Event::Eof => return Ok(shape),
        }
    }
}

/// The paragraphs of the text body just started, through its end tag.
/// Runs inside markup-compatibility blocks (equations) are skipped like
/// any other unknown element: their text is not read.
fn read_text_body(parser: &mut Parser<'_>, budget: &mut Budget) -> Result<Vec<Paragraph>, Error> {
    let mut paragraphs: Vec<Paragraph> = Vec::new();
    // A field's text is the `a:t` inside its `a:fld`.
    let mut field: Option<String> = None;
    let mut depth = 1usize;
    loop {
        match parser.next()? {
            Event::Start(e) => match e.name.as_str() {
                "AlternateContent" => parser.skip_element()?,
                "p" => {
                    budget.take()?;
                    depth += 1;
                    paragraphs.push(Paragraph { level: 0, runs: Vec::new() });
                }
                "pPr" => {
                    depth += 1;
                    if let Some(p) = paragraphs.last_mut() {
                        p.level = number(&e, "lvl").unwrap_or(0);
                    }
                }
                "t" => {
                    budget.take()?;
                    let text = parser.text_until_end(&[])?;
                    let Some(p) = paragraphs.last_mut() else { continue };
                    match &field {
                        Some(kind) => p.runs.push(Run::Field { kind: kind.clone(), text }),
                        None => p.runs.push(Run::Text(text)),
                    }
                }
                "br" => {
                    depth += 1;
                    if let Some(p) = paragraphs.last_mut() {
                        p.runs.push(Run::Break);
                    }
                }
                "fld" => {
                    depth += 1;
                    field = Some(e.plain_attr("type").unwrap_or("field").to_owned());
                }
                _ => depth += 1,
            },
            Event::End(name) => {
                if name == "fld" {
                    field = None;
                }
                depth -= 1;
                if depth == 0 {
                    return Ok(paragraphs);
                }
            }
            Event::Text(_) => {}
            Event::Eof => return Ok(paragraphs),
        }
    }
}

/// The notes page's text: its body placeholder's paragraphs.
fn read_notes(text: &str, part: &str, budget: &mut Budget) -> Result<Vec<Paragraph>, Error> {
    let mut parser = Parser::new(text, part);
    loop {
        match parser.next()? {
            Event::Start(e) if e.name == "spTree" => {
                let tree = read_shape(&mut parser, ShapeKind::Group, budget)?;
                let body = tree
                    .walk()
                    .find(|s| s.placeholder.as_ref().is_some_and(|p| p.kind == "body"))
                    .and_then(|s| s.text.clone());
                return Ok(body.unwrap_or_default());
            }
            Event::Eof => return Ok(Vec::new()),
            _ => {}
        }
    }
}
