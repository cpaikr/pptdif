//! The immutable presentation model: slides in show order, each with its
//! shape tree, and the masters and layouts they use.
//!
//! As built so far, the model holds structure only (shape kinds, names,
//! placeholders, notes and comment presence). Text, geometry, and
//! formatting arrive with M1 and M2 (PLAN.md, "Model").

use std::sync::Arc;

use serde::Serialize;

/// Which kind of package the presentation part declares.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DeckFormat {
    Pptx,
    /// Macro-enabled; the macros are not compared.
    Pptm,
    Potx,
    Potm,
    Ppsx,
    Ppsm,
}

/// A size in EMU (914,400 per inch, 12,700 per point).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Size {
    pub cx: i64,
    pub cy: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ShapeKind {
    /// `p:sp`: an autoshape or text box, placeholders included.
    Shape,
    /// `p:grpSp`, whose members are its children.
    Group,
    /// `p:pic`, including video and audio, which also count as media.
    Picture,
    /// `p:cxnSp`.
    Connector,
    /// A graphic frame holding a table.
    Table,
    /// A graphic frame holding a chart (classic or `chartex`).
    Chart,
    /// A graphic frame holding SmartArt.
    Diagram,
    /// A graphic frame holding an embedded or linked OLE object.
    OleObject,
    /// A graphic frame holding anything else.
    OtherFrame,
    /// `p:contentPart`: ink.
    Ink,
}

/// A placeholder's identity on its layout (`<p:ph>`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Placeholder {
    /// `type`, defaulting to `obj` as the schema does.
    #[serde(rename = "type")]
    pub kind: String,
    /// `idx`, defaulting to 0.
    pub index: u32,
}

/// One piece of a paragraph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Run {
    Text(String),
    /// `a:br`: a line break inside the paragraph.
    Break,
    /// `a:fld`: text PowerPoint fills in (slide number, date). `text` is
    /// what it showed when saved; `kind` is its `type`.
    Field {
        kind: String,
        text: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paragraph {
    /// Outline level, 0 for the top (`a:pPr` `lvl`).
    pub level: u8,
    pub runs: Vec<Run>,
}

impl Paragraph {
    /// The text as shown: field text as saved, line breaks as `\n`.
    pub fn text(&self) -> String {
        let mut out = String::new();
        for run in &self.runs {
            match run {
                Run::Text(t) | Run::Field { text: t, .. } => out.push_str(t),
                Run::Break => out.push('\n'),
            }
        }
        out
    }

    /// The text as compared: a field by its kind, since PowerPoint rewrites
    /// its text (renumbering every footer after an inserted slide). The
    /// kind sits between NUL characters, which XML text cannot hold, so no
    /// typed text compares equal to a field.
    pub fn compared(&self) -> String {
        let mut out = String::new();
        for run in &self.runs {
            match run {
                Run::Text(t) => out.push_str(t),
                Run::Field { kind, .. } => {
                    out.push('\0');
                    out.push_str(kind);
                    out.push('\0');
                }
                Run::Break => out.push('\n'),
            }
        }
        out
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shape {
    pub kind: ShapeKind,
    /// `cNvPr` `id`, unique within one slide as written.
    pub id: Option<u32>,
    pub name: String,
    pub hidden: bool,
    pub placeholder: Option<Placeholder>,
    /// A picture that plays video or audio.
    pub media: bool,
    /// The text body of an autoshape or text box, when it has one.
    pub text: Option<Vec<Paragraph>>,
    /// A group's members, in z-order.
    pub children: Vec<Shape>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Slide {
    /// The slide part's member name.
    pub part: String,
    /// `sldId` `id`: stable while the slide stays in this file.
    pub id: u32,
    pub hidden: bool,
    /// The layout's name (`cSld` `name`), when it has one; shared with the
    /// master's list.
    pub layout: Option<Arc<str>>,
    /// Shapes in z-order (back to front).
    pub shapes: Vec<Shape>,
    /// The speaker notes' text (the notes page's body placeholder), when the
    /// slide has a notes page.
    pub notes: Option<Vec<Paragraph>>,
    /// Comments, without their replies.
    pub comments: usize,
    /// The slide carries a `<p:timing>` tree (animations).
    pub animated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Section {
    pub name: String,
    /// The `sldId` ids of its slides, in order.
    pub slide_ids: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Master {
    pub name: Option<Arc<str>>,
    /// Its layouts' names in the master's order (`None` for a nameless one).
    pub layouts: Vec<Option<Arc<str>>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Warning {
    pub code: &'static str,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Presentation {
    pub format: DeckFormat,
    pub slide_size: Option<Size>,
    /// In show order (`sldIdLst`).
    pub slides: Vec<Slide>,
    pub sections: Vec<Section>,
    pub masters: Vec<Master>,
    pub warnings: Vec<Warning>,
}

impl Shape {
    /// This shape and every shape inside it, depth first.
    pub fn walk(&self) -> impl Iterator<Item = &Shape> {
        let mut stack = vec![self];
        std::iter::from_fn(move || {
            let shape = stack.pop()?;
            stack.extend(shape.children.iter().rev());
            Some(shape)
        })
    }
}

impl Slide {
    /// Every shape on the slide, group members included, depth first.
    pub fn all_shapes(&self) -> impl Iterator<Item = &Shape> {
        self.shapes.iter().flat_map(Shape::walk)
    }
}
