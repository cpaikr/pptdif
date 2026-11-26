//! Synthetic presentations for pptdif tests, assembled from hand-written
//! XML parts. A deck has one master and the layouts its slides name; each
//! slide's shape tree is given as XML, so tests control exactly what the
//! reader sees, including shapes writer libraries never produce.
//!
//! The parts are the minimum the reader needs, not what PowerPoint needs to
//! open the file: PowerPoint-saved fixtures live in `fixtures/powerpoint/`.

// Fixture XML is built for clarity, not speed.
#![allow(clippy::format_collect)]

use std::fmt::Write as _;
use std::io::{Cursor, Write as _};

use zip::write::SimpleFileOptions;

const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
const P14: &str = "http://schemas.microsoft.com/office/powerpoint/2010/main";
const CT: &str = "application/vnd.openxmlformats-officedocument.presentationml";
pub const PRESENTATION_TYPE: &str =
    "application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml";
pub const MACRO_PRESENTATION_TYPE: &str = "application/vnd.ms-powerpoint.presentation.macroEnabled.main+xml";

/// The namespace declarations a slide's shape XML may use (`p:`, `a:`, `r:`).
pub const NAMESPACES: &str = concat!(
    r#"xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" "#,
    r#"xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" "#,
    r#"xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships""#
);

#[derive(Debug, Clone, Default)]
pub struct RawSlide {
    layout: usize,
    shapes: String,
    hidden: bool,
    notes: Option<String>,
    comments: usize,
    replies: usize,
    timing: bool,
    /// Extra parts this slide refers to: relationship id, kind, part name
    /// under `ppt/`, and bytes.
    parts: Vec<(String, &'static str, String, Vec<u8>)>,
}

impl RawSlide {
    /// A slide on layout `layout` (an index into [`RawDeck::layouts`])
    /// whose `spTree` holds `shapes_xml`.
    pub fn new(layout: usize, shapes_xml: &str) -> Self {
        Self { layout, shapes: shapes_xml.to_owned(), ..Self::default() }
    }

    #[must_use]
    pub fn hidden(mut self) -> Self {
        self.hidden = true;
        self
    }

    #[must_use]
    pub fn notes(mut self, text: &str) -> Self {
        self.notes = Some(text.to_owned());
        self
    }

    /// Legacy comments, each with invented text.
    #[must_use]
    pub fn comments(mut self, n: usize) -> Self {
        self.comments = n;
        self
    }

    /// Legacy replies to the first comment: `cm` entries of their own whose
    /// `p15:threadingInfo` names it as `parentCm`.
    #[must_use]
    pub fn replies(mut self, n: usize) -> Self {
        self.replies = n;
        self
    }

    /// A part this slide refers to by `rid`, such as an image
    /// (`"image"`, `"media/image1.png"`) or a chart (`"chart"`,
    /// `"charts/chart1.xml"`); `name` is under `ppt/`.
    #[must_use]
    pub fn part(mut self, rid: &str, kind: &'static str, name: &str, bytes: &[u8]) -> Self {
        self.parts.push((rid.to_owned(), kind, name.to_owned(), bytes.to_vec()));
        self
    }

    /// An animation timing tree.
    #[must_use]
    pub fn animated(mut self) -> Self {
        self.timing = true;
        self
    }
}

/// A presentation built part by part.
#[derive(Debug, Clone)]
pub struct RawDeck {
    layouts: Vec<String>,
    slides: Vec<RawSlide>,
    /// Section names and the 0-based indices of their slides.
    sections: Vec<(String, Vec<usize>)>,
    main_type: &'static str,
    presentation_extra: String,
    /// Extra `sldIdLst` entries and presentation relationships.
    slide_entries: String,
    presentation_rels: Vec<(&'static str, String)>,
    /// Extra text parts, by member name.
    extra_parts: Vec<(String, String)>,
}

impl Default for RawDeck {
    fn default() -> Self {
        Self::new()
    }
}

impl RawDeck {
    /// A deck with the layouts "Title Slide" and "Title and Content".
    pub fn new() -> Self {
        Self {
            layouts: vec!["Title Slide".to_owned(), "Title and Content".to_owned()],
            slides: Vec::new(),
            sections: Vec::new(),
            main_type: PRESENTATION_TYPE,
            presentation_extra: String::new(),
            slide_entries: String::new(),
            presentation_rels: Vec::new(),
            extra_parts: Vec::new(),
        }
    }

    #[must_use]
    pub fn slide(mut self, slide: RawSlide) -> Self {
        self.slides.push(slide);
        self
    }

    /// A section holding the slides at the given 0-based indices.
    #[must_use]
    pub fn section(mut self, name: &str, slides: &[usize]) -> Self {
        self.sections.push((name.to_owned(), slides.to_vec()));
        self
    }

    /// The content type of the main part.
    #[must_use]
    pub fn main_type(mut self, content_type: &'static str) -> Self {
        self.main_type = content_type;
        self
    }

    /// Raw XML placed after the slide size in `presentation.xml`.
    #[must_use]
    pub fn presentation_xml(mut self, xml: &str) -> Self {
        self.presentation_extra.push_str(xml);
        self
    }

    /// Raw XML appended to `sldIdLst`, for entries the builder would not
    /// write (a part listed twice, an entry naming another kind of part).
    #[must_use]
    pub fn slide_entry_xml(mut self, xml: &str) -> Self {
        self.slide_entries.push_str(xml);
        self
    }

    /// A relationship from `presentation.xml`, numbered `rIdP1`, `rIdP2`, ...
    #[must_use]
    pub fn presentation_rel(mut self, kind: &'static str, target: &str) -> Self {
        self.presentation_rels.push((kind, target.to_owned()));
        self
    }

    /// A text part at `name` (a member name), such as
    /// `ppt/commentAuthors.xml`; pair it with [`Self::presentation_rel`].
    #[must_use]
    pub fn text_part(mut self, name: &str, xml: &str) -> Self {
        self.extra_parts.push((name.to_owned(), xml.to_owned()));
        self
    }

    /// The `sldId` id of the slide at a 0-based index.
    pub fn slide_id(index: usize) -> usize {
        256 + index
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut parts: Vec<(String, String)> = self.extra_parts.clone();
        let mut binary: Vec<(String, Vec<u8>)> = Vec::new();
        let mut overrides = format!(r#"<Override PartName="/ppt/presentation.xml" ContentType="{}"/>"#, self.main_type);
        let mut add_override = |part: &str, kind: &str| {
            let _ = write!(overrides, r#"<Override PartName="/{part}" ContentType="{CT}.{kind}+xml"/>"#);
        };

        parts.push(("_rels/.rels".to_owned(), rels(&[("rId1", "officeDocument", "ppt/presentation.xml")])));

        // presentation.xml and its relationships.
        let mut pres_rels = vec![("rId1".to_owned(), "slideMaster", "slideMasters/slideMaster1.xml".to_owned())];
        let mut slide_ids = String::new();
        for (i, _) in self.slides.iter().enumerate() {
            let rid = format!("rId{}", i + 2);
            let _ = write!(slide_ids, r#"<p:sldId id="{}" r:id="{rid}"/>"#, Self::slide_id(i));
            pres_rels.push((rid, "slide", format!("slides/slide{}.xml", i + 1)));
        }
        slide_ids.push_str(&self.slide_entries);
        for (k, (kind, target)) in self.presentation_rels.iter().enumerate() {
            pres_rels.push((format!("rIdP{}", k + 1), kind, target.clone()));
        }
        let sections = if self.sections.is_empty() {
            String::new()
        } else {
            let mut xml = format!(
                r#"<p:extLst><p:ext uri="{{521415D9-36F7-43E2-AB2F-B90AF26B5E84}}"><p14:sectionLst xmlns:p14="{P14}">"#
            );
            for (n, (name, slides)) in self.sections.iter().enumerate() {
                let _ = write!(
                    xml,
                    r#"<p14:section name="{}" id="{{00000000-0000-0000-0000-{n:012}}}"><p14:sldIdLst>"#,
                    escape(name)
                );
                for &i in slides {
                    let _ = write!(xml, r#"<p14:sldId id="{}"/>"#, Self::slide_id(i));
                }
                xml.push_str("</p14:sldIdLst></p14:section>");
            }
            xml.push_str("</p14:sectionLst></p:ext></p:extLst>");
            xml
        };
        parts.push((
            "ppt/presentation.xml".to_owned(),
            format!(
                r#"<p:presentation {NAMESPACES}><p:sldMasterIdLst><p:sldMasterId id="2147483648" r:id="rId1"/></p:sldMasterIdLst><p:sldIdLst>{slide_ids}</p:sldIdLst><p:sldSz cx="12192000" cy="6858000"/><p:notesSz cx="6858000" cy="9144000"/>{}{sections}</p:presentation>"#,
                self.presentation_extra
            ),
        ));
        parts.push(("ppt/_rels/presentation.xml.rels".to_owned(), owned_rels(&pres_rels)));

        // The master, its theme, and its layouts.
        let mut layout_ids = String::new();
        let mut master_rels = vec![("rId1".to_owned(), "theme", "../theme/theme1.xml".to_owned())];
        for (i, name) in self.layouts.iter().enumerate() {
            let rid = format!("rId{}", i + 2);
            let _ = write!(layout_ids, r#"<p:sldLayoutId id="{}" r:id="{rid}"/>"#, 2_147_483_649 + i);
            master_rels.push((rid, "slideLayout", format!("../slideLayouts/slideLayout{}.xml", i + 1)));
            let part = format!("ppt/slideLayouts/slideLayout{}.xml", i + 1);
            parts.push((
                part.clone(),
                format!(
                    r#"<p:sldLayout {NAMESPACES}><p:cSld name="{}"><p:spTree/></p:cSld></p:sldLayout>"#,
                    escape(name)
                ),
            ));
            parts.push((
                format!("ppt/slideLayouts/_rels/slideLayout{}.xml.rels", i + 1),
                rels(&[("rId1", "slideMaster", "../slideMasters/slideMaster1.xml")]),
            ));
            add_override(&part, "slideLayout");
        }
        parts.push((
            "ppt/slideMasters/slideMaster1.xml".to_owned(),
            format!("<p:sldMaster {NAMESPACES}><p:cSld><p:spTree/></p:cSld><p:sldLayoutIdLst>{layout_ids}</p:sldLayoutIdLst></p:sldMaster>"),
        ));
        parts.push(("ppt/slideMasters/_rels/slideMaster1.xml.rels".to_owned(), owned_rels(&master_rels)));
        add_override("ppt/slideMasters/slideMaster1.xml", "slideMaster");
        parts.push((
            "ppt/theme/theme1.xml".to_owned(),
            format!(r#"<a:theme xmlns:a="{A}" name="Synthetic"><a:themeElements/></a:theme>"#),
        ));

        // Slides, with their notes and comments.
        for (i, slide) in self.slides.iter().enumerate() {
            let n = i + 1;
            let part = format!("ppt/slides/slide{n}.xml");
            let show = if slide.hidden { r#" show="0""# } else { "" };
            let timing = if slide.timing { "<p:timing><p:tnLst><p:par/></p:tnLst></p:timing>" } else { "" };
            parts.push((
                part.clone(),
                format!(
                    r#"<p:sld {NAMESPACES}{show}><p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/>{}</p:spTree></p:cSld>{timing}</p:sld>"#,
                    slide.shapes
                ),
            ));
            add_override(&part, "slide");
            let mut slide_rels = vec![(
                "rId1".to_owned(),
                "slideLayout",
                format!("../slideLayouts/slideLayout{}.xml", slide.layout + 1),
            )];
            if let Some(text) = &slide.notes {
                let notes = format!("ppt/notesSlides/notesSlide{n}.xml");
                parts.push((
                    notes.clone(),
                    format!(
                        r#"<p:notes {NAMESPACES}><p:cSld><p:spTree><p:sp><p:nvSpPr><p:cNvPr id="2" name="Notes Placeholder 1"/><p:cNvSpPr/><p:nvPr><p:ph type="body" idx="1"/></p:nvPr></p:nvSpPr><p:spPr/><p:txBody><a:bodyPr/><a:p><a:r><a:t>{}</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld></p:notes>"#,
                        escape(text)
                    ),
                ));
                add_override(&notes, "notesSlide");
                slide_rels.push(("rId2".to_owned(), "notesSlide", format!("../notesSlides/notesSlide{n}.xml")));
            }
            if slide.comments + slide.replies > 0 {
                let comments = format!("ppt/comments/comment{n}.xml");
                let mut xml = format!("<p:cmLst {NAMESPACES}>");
                for k in 0..slide.comments {
                    let _ = write!(
                        xml,
                        r#"<p:cm authorId="0" idx="{}"><p:pos x="10" y="10"/><p:text>Comment {k}</p:text></p:cm>"#,
                        k + 1
                    );
                }
                for k in 0..slide.replies {
                    let _ = write!(
                        xml,
                        r#"<p:cm authorId="0" idx="{}"><p:pos x="10" y="10"/><p:text>Reply {k}</p:text><p:extLst><p:ext uri="{{C676402C-5697-4E1C-873F-D02D1690AC5C}}"><p15:threadingInfo xmlns:p15="http://schemas.microsoft.com/office/powerpoint/2012/main" timeZoneBias="0"><p15:parentCm authorId="0" idx="1"/></p15:threadingInfo></p:ext></p:extLst></p:cm>"#,
                        slide.comments + k + 1
                    );
                }
                xml.push_str("</p:cmLst>");
                parts.push((comments.clone(), xml));
                add_override(&comments, "comments");
                slide_rels.push(("rId3".to_owned(), "comments", format!("../comments/comment{n}.xml")));
            }
            for (rid, kind, name, bytes) in &slide.parts {
                slide_rels.push((rid.clone(), kind, format!("../{name}")));
                binary.push((format!("ppt/{name}"), bytes.clone()));
            }
            parts.push((format!("ppt/slides/_rels/slide{n}.xml.rels"), owned_rels(&slide_rels)));
        }

        let content_types = format!(
            r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/>{overrides}<Override PartName="/ppt/theme/theme1.xml" ContentType="application/vnd.openxmlformats-officedocument.theme+xml"/></Types>"#
        );
        parts.insert(0, ("[Content_Types].xml".to_owned(), content_types));
        zip_parts(&parts, &binary)
    }
}

fn rels(entries: &[(&str, &str, &str)]) -> String {
    let owned: Vec<(String, &str, String)> =
        entries.iter().map(|(id, kind, target)| ((*id).to_owned(), *kind, (*target).to_owned())).collect();
    owned_rels(&owned)
}

fn owned_rels(entries: &[(String, &str, String)]) -> String {
    let mut xml = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#.to_owned();
    for (id, kind, target) in entries {
        let _ = write!(xml, r#"<Relationship Id="{id}" Type="{REL}/{kind}" Target="{target}"/>"#);
    }
    xml.push_str("</Relationships>");
    xml
}

fn zip_parts(parts: &[(String, String)], binary: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let members = parts.iter().map(|(n, t)| (n, t.as_bytes())).chain(binary.iter().map(|(n, b)| (n, b.as_slice())));
    for (name, bytes) in members {
        zip.start_file(name.as_str(), options).expect("writing to memory cannot fail");
        zip.write_all(bytes).expect("writing to memory cannot fail");
    }
    zip.finish().expect("writing to memory cannot fail").into_inner()
}

/// Escapes text for an XML attribute or element.
pub fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// A shape with text, optionally a placeholder (`type`, `idx`).
pub fn text_shape(id: u32, name: &str, placeholder: Option<(&str, u32)>, text: &str) -> String {
    let ph = placeholder.map_or(String::new(), |(kind, idx)| format!(r#"<p:ph type="{kind}" idx="{idx}"/>"#));
    // A placeholder takes its geometry from the layout; a plain box writes
    // its own, as PowerPoint does.
    let geometry = if placeholder.is_some() { "" } else { r#"<a:prstGeom prst="rect"><a:avLst/></a:prstGeom>"# };
    format!(
        r#"<p:sp><p:nvSpPr><p:cNvPr id="{id}" name="{}"/><p:cNvSpPr/><p:nvPr>{ph}</p:nvPr></p:nvSpPr><p:spPr>{geometry}</p:spPr><p:txBody><a:bodyPr/><a:p><a:r><a:t>{}</a:t></a:r></a:p></p:txBody></p:sp>"#,
        escape(name),
        escape(text)
    )
}

/// A picture shape whose image is relationship `rIdImg` (add the image
/// with [`RawSlide::part`] when it matters).
pub fn picture(id: u32, name: &str) -> String {
    picture_of(id, name, "rIdImg")
}

/// A picture shape whose image is relationship `rid`.
pub fn picture_of(id: u32, name: &str, rid: &str) -> String {
    format!(
        r#"<p:pic><p:nvPicPr><p:cNvPr id="{id}" name="{}"/><p:cNvPicPr/><p:nvPr/></p:nvPicPr><p:blipFill><a:blip r:embed="{rid}"/></p:blipFill><p:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="914400" cy="914400"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></p:spPr></p:pic>"#,
        escape(name)
    )
}

/// A text box at `(x, y)` sized `(cx, cy)` in EMU, with `spPr_extra`
/// (fill, line) after its geometry.
#[allow(clippy::similar_names)]
pub fn placed_box(id: u32, name: &str, text: &str, (x, y, cx, cy): (i64, i64, i64, i64), sp_pr_extra: &str) -> String {
    format!(
        r#"<p:sp><p:nvSpPr><p:cNvPr id="{id}" name="{}"/><p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x="{x}" y="{y}"/><a:ext cx="{cx}" cy="{cy}"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom>{sp_pr_extra}</p:spPr><p:txBody><a:bodyPr/><a:p><a:r><a:t>{}</a:t></a:r></a:p></p:txBody></p:sp>"#,
        escape(name),
        escape(text)
    )
}

/// A table frame whose rows are given cell by cell.
pub fn table(id: u32, name: &str, rows: &[&[&str]]) -> String {
    let columns = rows.iter().map(|r| r.len()).max().unwrap_or(0);
    let grid: String = (0..columns).map(|_| r#"<a:gridCol w="914400"/>"#).collect();
    let body: String = rows
        .iter()
        .map(|r| {
            let cells: String = r
                .iter()
                .map(|c| {
                    format!(
                        "<a:tc><a:txBody><a:bodyPr/><a:p><a:r><a:t>{}</a:t></a:r></a:p></a:txBody><a:tcPr/></a:tc>",
                        escape(c)
                    )
                })
                .collect();
            format!(r#"<a:tr h="370840">{cells}</a:tr>"#)
        })
        .collect();
    frame(id, name, uri::TABLE, &format!("<a:tbl><a:tblPr/><a:tblGrid>{grid}</a:tblGrid>{body}</a:tbl>"))
}

/// A chart frame whose chart part is relationship `rid`.
pub fn chart_frame(id: u32, name: &str, rid: &str) -> String {
    frame(
        id,
        name,
        uri::CHART,
        &format!(r#"<c:chart xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" r:id="{rid}"/>"#),
    )
}

/// A chart part: one bar chart with these series of `(name, values)` over
/// `categories`.
pub fn chart_part(categories: &[&str], series: &[(&str, &[f64])]) -> String {
    let points = |values: &mut dyn Iterator<Item = String>| -> String {
        values.enumerate().map(|(i, v)| format!(r#"<c:pt idx="{i}"><c:v>{v}</c:v></c:pt>"#)).collect()
    };
    let cats = points(&mut categories.iter().map(|c| escape(c)));
    let body: String = series
        .iter()
        .enumerate()
        .map(|(k, (name, values))| {
            let vals = points(&mut values.iter().map(f64::to_string));
            format!(
                r#"<c:ser><c:idx val="{k}"/><c:tx><c:strRef><c:strCache><c:pt idx="0"><c:v>{}</c:v></c:pt></c:strCache></c:strRef></c:tx><c:cat><c:strRef><c:strCache>{cats}</c:strCache></c:strRef></c:cat><c:val><c:numRef><c:numCache>{vals}</c:numCache></c:numRef></c:val></c:ser>"#,
                escape(name)
            )
        })
        .collect();
    format!(
        r#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart><c:plotArea><c:barChart>{body}</c:barChart></c:plotArea></c:chart></c:chartSpace>"#
    )
}

/// A graphic frame whose `graphicData` has the given URI.
pub fn frame(id: u32, name: &str, uri: &str, inner: &str) -> String {
    format!(
        r#"<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="{id}" name="{}"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr><p:xfrm/><a:graphic><a:graphicData uri="{uri}">{inner}</a:graphicData></a:graphic></p:graphicFrame>"#,
        escape(name)
    )
}

/// A group shape holding `members_xml`.
pub fn group(id: u32, name: &str, members_xml: &str) -> String {
    format!(
        r#"<p:grpSp><p:nvGrpSpPr><p:cNvPr id="{id}" name="{}"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/>{members_xml}</p:grpSp>"#,
        escape(name)
    )
}

/// A text shape with one paragraph per entry. An entry starting with `<`
/// is raw run XML (a field, a line break); any other is one plain run.
pub fn paragraphs_shape(id: u32, name: &str, placeholder: Option<(&str, u32)>, paragraphs: &[&str]) -> String {
    let ph = placeholder.map_or(String::new(), |(kind, idx)| format!(r#"<p:ph type="{kind}" idx="{idx}"/>"#));
    let geometry = if placeholder.is_some() { "" } else { r#"<a:prstGeom prst="rect"><a:avLst/></a:prstGeom>"# };
    let body: String = paragraphs
        .iter()
        .map(|p| {
            if p.starts_with('<') {
                format!("<a:p>{p}</a:p>")
            } else {
                format!("<a:p><a:r><a:t>{}</a:t></a:r></a:p>", escape(p))
            }
        })
        .collect();
    format!(
        r#"<p:sp><p:nvSpPr><p:cNvPr id="{id}" name="{}"/><p:cNvSpPr/><p:nvPr>{ph}</p:nvPr></p:nvSpPr><p:spPr>{geometry}</p:spPr><p:txBody><a:bodyPr/>{body}</p:txBody></p:sp>"#,
        escape(name)
    )
}

/// A content slide: a title and a body placeholder with these bullets.
pub fn content_slide(title: &str, bullets: &[&str]) -> RawSlide {
    RawSlide::new(
        1,
        &[
            text_shape(2, "Title 1", Some(("title", 0)), title),
            paragraphs_shape(3, "Content Placeholder 2", Some(("body", 1)), bullets),
        ]
        .concat(),
    )
}

/// The URIs PowerPoint writes in `graphicData`.
pub mod uri {
    pub const TABLE: &str = "http://schemas.openxmlformats.org/drawingml/2006/table";
    pub const CHART: &str = "http://schemas.openxmlformats.org/drawingml/2006/chart";
    pub const DIAGRAM: &str = "http://schemas.openxmlformats.org/drawingml/2006/diagram";
    pub const OLE: &str = "http://schemas.openxmlformats.org/presentationml/2006/ole";
}

/// A two-slide deck: a title slide and a content slide.
pub fn basic() -> RawDeck {
    RawDeck::new().slide(RawSlide::new(0, &text_shape(2, "Title 1", Some(("ctrTitle", 0)), "Quarterly review"))).slide(
        RawSlide::new(
            1,
            &[
                text_shape(2, "Title 1", Some(("title", 0)), "Results"),
                text_shape(3, "Content Placeholder 2", Some(("body", 1)), "Revenue grew"),
                picture(4, "Picture 3"),
            ]
            .concat(),
        ),
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn decks_are_zip_archives() {
        let bytes = super::basic().to_bytes();
        assert!(bytes.starts_with(b"PK\x03\x04"));
    }
}
