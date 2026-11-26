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

use crate::align::fnv;
use crate::model::{
    Cell, ChartRef, Comment, DeckFormat, Format, Geometry, Image, Master, Paragraph, Placeholder, Presentation, Run,
    RunKind, Section, Shape, ShapeKind, Size, Slide, Table, Warning,
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
pub(crate) struct Budget {
    left: usize,
    limit: usize,
}

impl Budget {
    pub(crate) fn take(&mut self) -> Result<(), Error> {
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

/// The main part's content type. Only `.pptx` has been observed in a
/// PowerPoint-saved file; the macro-enabled, template, and show types are
/// taken from ECMA-376 and Office's registrations (unverified; TODO.md).
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

    let authors = read_authors(pkg, &rels, &mut budget)?;
    let mut ctx = Context { budget, layouts: LayoutNames::default(), images: HashMap::new(), authors };
    let mut masters = Vec::new();
    // A master listed twice is read once.
    let mut seen_masters = HashSet::new();
    for rid in listing.master_rids.iter().filter(|rid| seen_masters.insert(rid.as_str())) {
        let Some(rel) = rels.get(rid).filter(|r| r.kind == "slideMaster" && !r.external) else { continue };
        masters.push(read_master(pkg, &rel.target, &mut ctx.layouts, &mut ctx.budget)?);
    }

    let mut slides = Vec::new();
    // PowerPoint lists each slide part once; a part listed again would be
    // read again, so it is refused.
    let mut seen = HashSet::new();
    for (id, rid) in &listing.slides {
        let rel = rels.get(rid).filter(|r| r.kind == "slide" && !r.external).ok_or_else(|| {
            Error::new(ErrorCode::InvalidXml, format!("{main}: slide {id} names no slide part ({rid})"))
        })?;
        if !seen.insert(rel.target.as_str()) {
            return Err(Error::new(
                ErrorCode::InvalidXml,
                format!("{main}: {} is listed as more than one slide", rel.target),
            ));
        }
        let slide = read_slide(pkg, &rel.target, *id, &mut ctx)?;
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
                        budget.take()?;
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
                    budget.take()?;
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

/// What reading every slide shares: the record budget, layout names, image
/// hashes, and comment authors.
struct Context {
    budget: Budget,
    layouts: LayoutNames,
    /// Content hashes by image part, so a picture used on many slides is
    /// read once.
    images: HashMap<String, Option<u64>>,
    /// Comment authors: `m:<guid>` for modern comments, `l:<id>` for legacy.
    authors: HashMap<String, String>,
}

/// Comment authors from `ppt/authors.xml` (modern) and
/// `ppt/commentAuthors.xml` (legacy), named by the presentation's
/// relationships.
fn read_authors<R: Read + Seek>(
    pkg: &mut Package<R>,
    rels: &HashMap<String, Relationship>,
    budget: &mut Budget,
) -> Result<HashMap<String, String>, Error> {
    let mut authors = HashMap::new();
    for rel in rels.values().filter(|r| !r.external) {
        let prefix = match rel.kind.as_str() {
            "authors" => "m",
            "commentAuthors" => "l",
            _ => continue,
        };
        let Some(text) = pkg.read_text(&rel.target)? else { continue };
        let mut parser = Parser::new(&text, &rel.target);
        loop {
            match parser.next()? {
                Event::Start(e) if e.name == "author" || e.name == "cmAuthor" => {
                    if let (Some(id), Some(name)) = (e.plain_attr("id"), e.plain_attr("name")) {
                        budget.take()?;
                        authors.insert(format!("{prefix}:{id}"), name.to_owned());
                    }
                }
                Event::Eof => break,
                _ => {}
            }
        }
    }
    Ok(authors)
}

fn read_slide<R: Read + Seek>(pkg: &mut Package<R>, part: &str, id: u32, ctx: &mut Context) -> Result<Slide, Error> {
    let text = pkg.read_text(part)?.ok_or_else(|| missing(part))?;
    let rels = pkg.relationships(part)?;
    let mut slide = Slide {
        part: part.to_owned(),
        id,
        hidden: false,
        layout: None,
        shapes: Vec::new(),
        notes: None,
        comments: Vec::new(),
        animated: false,
    };
    let mut parser = Parser::new(&text, part);
    loop {
        match parser.next()? {
            Event::Start(e) => match e.name.as_str() {
                // Hidden slides carry `show="0"` on the root.
                "sld" => slide.hidden = matches!(e.plain_attr("show"), Some("0" | "false")),
                "spTree" => slide.shapes = read_shape(&mut parser, ShapeKind::Group, &mut ctx.budget)?.children,
                "timing" => slide.animated = true,
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
    }
    let targets: HashMap<&str, &Relationship> = rels.iter().map(|r| (r.id.as_str(), r)).collect();
    resolve_parts(pkg, &mut slide.shapes, &targets, ctx)?;
    for rel in rels.iter().filter(|r| !r.external) {
        match rel.kind.as_str() {
            "slideLayout" => slide.layout = ctx.layouts.get(pkg, &rel.target)?,
            "notesSlide" => {
                let notes = match pkg.read_text(&rel.target)? {
                    Some(text) => read_notes(&text, &rel.target, &mut ctx.budget)?,
                    None => Vec::new(),
                };
                slide.notes = Some(notes);
            }
            // Legacy comment lists and modern comments both end their
            // relationship type in `comments` (modern observed, legacy
            // unverified; TODO.md).
            "comments" => {
                if let Some(text) = pkg.read_text(&rel.target)? {
                    slide.comments.extend(read_comments(&text, &rel.target, &ctx.authors, &mut ctx.budget)?);
                }
            }
            _ => {}
        }
    }
    Ok(slide)
}

/// Fills in what shapes name by relationship: each picture's image part
/// and content hash, and each chart's cached data.
fn resolve_parts<R: Read + Seek>(
    pkg: &mut Package<R>,
    shapes: &mut [Shape],
    rels: &HashMap<&str, &Relationship>,
    ctx: &mut Context,
) -> Result<(), Error> {
    for shape in shapes {
        resolve_parts(pkg, &mut shape.children, rels, ctx)?;
        if let Some(image) = &mut shape.image
            && let Some(rel) = rels.get(image.rid.as_str()).filter(|r| !r.external)
        {
            let hash = if let Some(hash) = ctx.images.get(&rel.target) {
                *hash
            } else {
                let hash = pkg.read_bytes(&rel.target)?.map(|bytes| fnv(&[&bytes]));
                ctx.images.insert(rel.target.clone(), hash);
                hash
            };
            image.part = Some(rel.target.clone());
            image.hash = hash;
        }
        if let Some(chart) = &mut shape.chart
            && let Some(rel) = rels.get(chart.rid.as_str()).filter(|r| r.kind == "chart" && !r.external)
            && let Some(text) = pkg.read_text(&rel.target)?
        {
            chart.data = Some(crate::chart::read_chart(&text, &rel.target, &mut ctx.budget)?);
        }
    }
    Ok(())
}

/// A slide's comments, each with its replies. A modern comment keeps its
/// replies in `p188:replyLst`; a legacy reply is a `cm` of its own whose
/// `p15:threadingInfo` names a `parentCm` (unverified; TODO.md).
fn read_comments(
    text: &str,
    part: &str,
    authors: &HashMap<String, String>,
    budget: &mut Budget,
) -> Result<Vec<Comment>, Error> {
    struct Open {
        author_id: Option<String>,
        legacy_index: Option<String>,
        parent: Option<(String, String)>,
        text: String,
        replies: Vec<String>,
    }
    // Modern author ids are GUIDs and legacy ones numbers, so one lookup
    // in each table cannot confuse them.
    let author = |id: Option<&str>| -> Option<String> {
        let id = id?;
        authors.get(&format!("m:{id}")).or_else(|| authors.get(&format!("l:{id}"))).cloned()
    };
    let mut parser = Parser::new(text, part);
    let mut comments: Vec<Comment> = Vec::new();
    // Legacy comments by `(authorId, idx)`, for replies to find them.
    let mut legacy: HashMap<(String, String), usize> = HashMap::new();
    let mut open: Option<Open> = None;
    let mut reply_author: Option<String> = None;
    loop {
        match parser.next()? {
            Event::Start(e) => match e.name.as_str() {
                "cm" => {
                    budget.take()?;
                    open = Some(Open {
                        author_id: e.plain_attr("authorId").map(str::to_owned),
                        legacy_index: e.plain_attr("idx").map(str::to_owned),
                        parent: None,
                        text: String::new(),
                        replies: Vec::new(),
                    });
                }
                "parentCm" => {
                    if let (Some(o), Some(a), Some(i)) = (open.as_mut(), e.plain_attr("authorId"), e.plain_attr("idx"))
                    {
                        o.parent = Some((a.to_owned(), i.to_owned()));
                    }
                }
                "text" => {
                    let body = parser.text_until_end(&[])?;
                    if let Some(o) = open.as_mut() {
                        o.text = body;
                    }
                }
                "reply" => {
                    budget.take()?;
                    reply_author = Some(e.plain_attr("authorId").unwrap_or_default().to_owned());
                }
                "txBody" => {
                    let body: Vec<String> = read_text_body(&mut parser, budget)?.iter().map(Paragraph::text).collect();
                    let body = body.join("\n");
                    let Some(o) = open.as_mut() else { continue };
                    match &reply_author {
                        Some(id) => o.replies.push(match author(Some(id)) {
                            Some(name) => format!("{name}: {body}"),
                            None => body,
                        }),
                        None => o.text = body,
                    }
                }
                _ => {}
            },
            Event::End(name) if name == "reply" => reply_author = None,
            Event::End(name) if name == "cm" => {
                let Some(o) = open.take() else { continue };
                let name = author(o.author_id.as_deref());
                if let Some(parent) = &o.parent
                    && let Some(&k) = legacy.get(parent)
                {
                    let reply = match name {
                        Some(name) => format!("{name}: {}", o.text),
                        None => o.text,
                    };
                    comments[k].replies.push(reply);
                    continue;
                }
                if let (Some(a), Some(i)) = (o.author_id.clone(), o.legacy_index.clone()) {
                    legacy.insert((a, i), comments.len());
                }
                comments.push(Comment { author: name, text: o.text, replies: o.replies });
            }
            Event::Eof => return Ok(comments),
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

/// Fill elements of a shape's properties.
const FILLS: &[&str] = &["noFill", "solidFill", "gradFill", "blipFill", "pattFill", "grpFill"];

/// An element's attributes and content, described canonically: attribute
/// order and whitespace do not change it.
fn describe(parser: &mut Parser<'_>, e: &Element) -> Result<String, Error> {
    let mut attrs: Vec<String> = e.attrs.iter().map(|(k, v)| format!("{k}={v}")).collect();
    attrs.sort();
    let rest = parser.describe_rest()?;
    Ok(if attrs.is_empty() {
        format!("{}{{{rest}}}", e.name)
    } else {
        format!("{}({}){{{rest}}}", e.name, attrs.join(","))
    })
}

/// `xfrm` (`a:xfrm` in shape properties, `p:xfrm` on a graphic frame),
/// its start tag just read, through its end tag.
fn read_xfrm(parser: &mut Parser<'_>, e: &Element) -> Result<Geometry, Error> {
    let flag = |name: &str| matches!(e.plain_attr(name), Some("1" | "true"));
    let mut g = Geometry {
        rotation: number(e, "rot").unwrap_or(0),
        flip_h: flag("flipH"),
        flip_v: flag("flipV"),
        ..Geometry::default()
    };
    let mut depth = 1usize;
    loop {
        match parser.next()? {
            Event::Start(c) => {
                depth += 1;
                match c.name.as_str() {
                    "off" if depth == 2 => (g.x, g.y) = (number(&c, "x").unwrap_or(0), number(&c, "y").unwrap_or(0)),
                    "ext" if depth == 2 => {
                        (g.cx, g.cy) = (number(&c, "cx").unwrap_or(0), number(&c, "cy").unwrap_or(0));
                    }
                    _ => {}
                }
            }
            Event::End(_) => {
                depth -= 1;
                if depth == 0 {
                    return Ok(g);
                }
            }
            Event::Text(_) => {}
            Event::Eof => return Ok(g),
        }
    }
}

/// A shape's own properties (`spPr`, `grpSpPr`), its start tag just read:
/// geometry, outline, fill, and line.
fn read_shape_properties(parser: &mut Parser<'_>, shape: &mut Shape) -> Result<(), Error> {
    loop {
        match parser.next()? {
            Event::Start(e) => match e.name.as_str() {
                "xfrm" => shape.geometry = Some(read_xfrm(parser, &e)?),
                "prstGeom" => {
                    shape.outline = e.plain_attr("prst").map(str::to_owned);
                    parser.skip_element()?;
                }
                "custGeom" => {
                    shape.outline = Some("custom".to_owned());
                    parser.skip_element()?;
                }
                "ln" => shape.line = Some(describe(parser, &e)?),
                name if FILLS.contains(&name) => shape.fill = Some(describe(parser, &e)?),
                _ => parser.skip_element()?,
            },
            Event::End(_) | Event::Eof => return Ok(()),
            Event::Text(_) => {}
        }
    }
}

/// A picture's `blipFill`, its start tag just read: the image's
/// relationship and any crop.
fn read_blip_fill(parser: &mut Parser<'_>) -> Result<Option<Image>, Error> {
    let mut image: Option<Image> = None;
    let mut crop = None;
    let mut depth = 1usize;
    loop {
        match parser.next()? {
            Event::Start(e) => {
                depth += 1;
                match e.name.as_str() {
                    "blip" => {
                        let rid = e.prefixed_attr("embed").or_else(|| e.prefixed_attr("link"));
                        image = rid.map(|rid| Image { rid: rid.to_owned(), part: None, hash: None, crop: None });
                    }
                    "srcRect" if !e.attrs.is_empty() => {
                        let mut parts: Vec<String> = e.attrs.iter().map(|(k, v)| format!("{k}={v}")).collect();
                        parts.sort();
                        crop = Some(parts.join(","));
                    }
                    _ => {}
                }
            }
            Event::End(_) => {
                depth -= 1;
                if depth == 0 {
                    return Ok(image.map(|i| Image { crop, ..i }));
                }
            }
            Event::Text(_) => {}
            Event::Eof => return Ok(image),
        }
    }
}

/// A table (`a:tbl`), its start tag just read: rows of cells with text.
fn read_table(parser: &mut Parser<'_>, budget: &mut Budget) -> Result<Table, Error> {
    let mut table = Table::default();
    let mut cell: Option<Cell> = None;
    let mut depth = 1usize;
    loop {
        match parser.next()? {
            Event::Start(e) => match e.name.as_str() {
                "tr" => {
                    budget.take()?;
                    depth += 1;
                    table.rows.push(Vec::new());
                }
                "tc" => {
                    budget.take()?;
                    depth += 1;
                    let flag = |name: &str| matches!(e.plain_attr(name), Some("1" | "true"));
                    cell = Some(Cell { text: Vec::new(), merged: flag("hMerge") || flag("vMerge") });
                }
                "txBody" => {
                    let text = read_text_body(parser, budget)?;
                    if let Some(c) = cell.as_mut() {
                        c.text = text;
                    }
                }
                _ => depth += 1,
            },
            Event::End(name) => {
                if name == "tc"
                    && let (Some(c), Some(row)) = (cell.take(), table.rows.last_mut())
                {
                    row.push(c);
                }
                depth -= 1;
                if depth == 0 {
                    return Ok(table);
                }
            }
            Event::Text(_) => {}
            Event::Eof => return Ok(table),
        }
    }
}

/// One shape, its start tag just read, through its end tag. A group's
/// members are read into `children`; the slide's `spTree` is read as a
/// group whose members are the slide's shapes. A markup compatibility
/// block contributes its first choice, whatever it `Requires`: later
/// choices and the fallback repeat the content for other readers
/// (unverified on a PowerPoint-saved file; TODO.md).
fn read_shape(parser: &mut Parser<'_>, mut kind: ShapeKind, budget: &mut Budget) -> Result<Shape, Error> {
    let mut shape = Shape::new(kind);
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
                // The shape's own parts; the same names deeper down (an OLE
                // preview's) belong to something else.
                match e.name.as_str() {
                    "spPr" | "grpSpPr" if depth == 1 => {
                        read_shape_properties(parser, &mut shape)?;
                        continue;
                    }
                    "xfrm" if depth == 1 && shape.geometry.is_none() => {
                        shape.geometry = Some(read_xfrm(parser, &e)?);
                        continue;
                    }
                    "txBody" if depth == 1 && kind == ShapeKind::Shape && shape.text.is_none() => {
                        shape.text = Some(read_text_body(parser, budget)?);
                        continue;
                    }
                    "blipFill" if depth == 1 && kind == ShapeKind::Picture && shape.image.is_none() => {
                        shape.image = read_blip_fill(parser)?;
                        continue;
                    }
                    "tbl" if kind == ShapeKind::Table && shape.table.is_none() => {
                        shape.table = Some(read_table(parser, budget)?);
                        continue;
                    }
                    _ => {}
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
                    "chart" if kind == ShapeKind::Chart && shape.chart.is_none() => {
                        if let Some(rid) = e.prefixed_attr("id") {
                            shape.chart = Some(ChartRef { rid: rid.to_owned(), data: None });
                        }
                    }
                    // Media hangs off a shape's `nvPr`; PowerPoint uses
                    // pictures, but the schema allows it on any shape.
                    "videoFile" | "audioFile" | "wavAudioFile" | "audioCd" | "quickTimeFile" | "media" => {
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

/// Run properties (`a:rPr`, `a:defRPr`), its start tag just read, through
/// its end tag: what is written, by name.
fn read_run_format(parser: &mut Parser<'_>, e: &Element) -> Result<Format, Error> {
    let mut format = Format::new();
    for (attr, key) in [
        ("b", "bold"),
        ("i", "italic"),
        ("u", "underline"),
        ("strike", "strike"),
        ("baseline", "baseline"),
        ("cap", "caps"),
        ("spc", "spacing"),
    ] {
        if let Some(v) = e.plain_attr(attr) {
            format.insert(key, v.to_owned());
        }
    }
    // `sz` is in hundredths of a point.
    if let Some(sz) = number::<u32>(e, "sz") {
        format.insert("size", format!("{}pt", f64::from(sz) / 100.0));
    }
    loop {
        match parser.next()? {
            Event::Start(c) => match c.name.as_str() {
                "solidFill" | "gradFill" | "noFill" | "pattFill" => {
                    let d = describe(parser, &c)?;
                    format.insert("color", d);
                }
                "highlight" => {
                    let d = describe(parser, &c)?;
                    format.insert("highlight", d);
                }
                "latin" | "ea" | "cs" => {
                    if let Some(face) = c.plain_attr("typeface") {
                        let key = match c.name.as_str() {
                            "latin" => "font",
                            "ea" => "fontEastAsian",
                            _ => "fontComplex",
                        };
                        format.insert(key, face.to_owned());
                    }
                    parser.skip_element()?;
                }
                _ => parser.skip_element()?,
            },
            Event::End(_) | Event::Eof => return Ok(format),
            Event::Text(_) => {}
        }
    }
}

/// The paragraphs of the text body just started, through its end tag.
/// Runs inside markup-compatibility blocks (equations) are skipped like
/// any other unknown element: their text is not read. The body's own list
/// styles (`a:lstStyle`) are not read here: they are inherited formatting
/// (M3).
fn read_text_body(parser: &mut Parser<'_>, budget: &mut Budget) -> Result<Vec<Paragraph>, Error> {
    let mut paragraphs: Vec<Paragraph> = Vec::new();
    // A field's kind while inside its `a:fld`.
    let mut field: Option<String> = None;
    // The current run's properties.
    let mut format = Format::new();
    let mut depth = 1usize;
    loop {
        match parser.next()? {
            Event::Start(e) => match e.name.as_str() {
                "AlternateContent" | "lstStyle" | "bodyPr" | "endParaRPr" | "defRPr" => parser.skip_element()?,
                "p" => {
                    budget.take()?;
                    depth += 1;
                    paragraphs.push(Paragraph { level: 0, format: Format::new(), runs: Vec::new() });
                }
                "pPr" => {
                    depth += 1;
                    if let Some(p) = paragraphs.last_mut() {
                        p.level = number(&e, "lvl").unwrap_or(0);
                        if let Some(a) = e.plain_attr("algn") {
                            p.format.insert("align", a.to_owned());
                        }
                    }
                }
                "buNone" | "buChar" | "buAutoNum" | "buBlip" => {
                    depth += 1;
                    let bullet = match e.name.as_str() {
                        "buNone" => "none".to_owned(),
                        "buChar" => format!("char:{}", e.plain_attr("char").unwrap_or_default()),
                        "buAutoNum" => format!("number:{}", e.plain_attr("type").unwrap_or_default()),
                        _ => "picture".to_owned(),
                    };
                    if let Some(p) = paragraphs.last_mut() {
                        p.format.insert("bullet", bullet);
                    }
                }
                "rPr" => format = read_run_format(parser, &e)?,
                "r" => {
                    depth += 1;
                    format = Format::new();
                }
                "t" => {
                    budget.take()?;
                    let text = parser.text_until_end(&[])?;
                    let Some(p) = paragraphs.last_mut() else { continue };
                    let kind = field.clone().map_or(RunKind::Text, RunKind::Field);
                    p.runs.push(Run { kind, text, format: format.clone() });
                }
                "br" => {
                    depth += 1;
                    if let Some(p) = paragraphs.last_mut() {
                        p.runs.push(Run { kind: RunKind::Break, text: String::new(), format: Format::new() });
                    }
                }
                "fld" => {
                    depth += 1;
                    format = Format::new();
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
