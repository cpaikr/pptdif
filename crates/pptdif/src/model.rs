//! The immutable presentation model: slides in show order, each with its
//! shape tree, and the masters and layouts they use.
//!
//! As built so far, the model holds structure only (shape kinds, names,
//! placeholders, notes and comment presence). Text, geometry, and
//! formatting arrive with M1 and M2 (PLAN.md, "Model").

use std::collections::BTreeMap;
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

/// Formatting as written, by property name (`bold`, `size`, `color`, ...),
/// values as text. Only what the file states is here: an absent property
/// is inherited from the shape, layout, or master (resolved in M3).
pub type Format = BTreeMap<&'static str, String>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunKind {
    Text,
    /// `a:br`: a line break inside the paragraph.
    Break,
    /// `a:fld`: text PowerPoint fills in (slide number, date), by its
    /// `type`; the run's text is what it showed when saved.
    Field(String),
}

/// One piece of a paragraph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    pub kind: RunKind,
    /// Empty for a break.
    pub text: String,
    /// `a:rPr` as written.
    pub format: Format,
    /// The formatting in effect: `format`, then what the shape, its layout
    /// and master placeholders, the master's text styles, and the
    /// presentation's default style give, with theme fonts named.
    pub effective: Format,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paragraph {
    /// Outline level, 0 for the top (`a:pPr` `lvl`).
    pub level: u8,
    /// `a:pPr` as written (alignment, bullet), apart from the level.
    pub format: Format,
    /// The paragraph formatting in effect, inherited as for runs.
    pub effective: Format,
    pub runs: Vec<Run>,
}

impl Paragraph {
    /// The text as shown: field text as saved, line breaks as `\n`.
    pub fn text(&self) -> String {
        let mut out = String::new();
        for run in &self.runs {
            match run.kind {
                RunKind::Break => out.push('\n'),
                _ => out.push_str(&run.text),
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
            match &run.kind {
                RunKind::Text => out.push_str(&run.text),
                RunKind::Field(kind) => {
                    out.push('\0');
                    out.push_str(kind);
                    out.push('\0');
                }
                RunKind::Break => out.push('\n'),
            }
        }
        out
    }
}

/// A shape's position and size in EMU, as written in its `xfrm`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Geometry {
    pub x: i64,
    pub y: i64,
    pub cx: i64,
    pub cy: i64,
    /// Clockwise, in 60,000ths of a degree.
    pub rotation: i64,
    pub flip_h: bool,
    pub flip_v: bool,
}

/// A picture's image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    /// The `blip`'s relationship id.
    pub rid: String,
    /// The image part, when the picture embeds one (a linked picture has
    /// none).
    pub part: Option<String>,
    /// A content hash of the image part's bytes: the same picture under
    /// another part name compares equal.
    pub hash: Option<u64>,
    /// `a:srcRect` as written, when the picture is cropped.
    pub crop: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cell {
    pub text: Vec<Paragraph>,
    /// Covered by a merged neighbour (`hMerge` or `vMerge`).
    pub merged: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Table {
    /// Rows of cells, one cell per grid column.
    pub rows: Vec<Vec<Cell>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Series {
    pub name: Option<String>,
    /// Category labels (or x values) as cached in the chart part.
    pub categories: Vec<String>,
    /// Values as cached; `None` for a point with no value.
    pub values: Vec<Option<f64>>,
}

/// A chart's cached data: what PowerPoint last drew, not its workbook.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Chart {
    /// The plot kinds, such as `barChart` or `lineChart`.
    pub kinds: Vec<String>,
    pub title: Option<String>,
    pub series: Vec<Series>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChartRef {
    /// The `c:chart` relationship id.
    pub rid: String,
    /// The chart part's data, when it could be read (`chartex` charts are
    /// not).
    pub data: Option<Chart>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Shape {
    pub kind: ShapeKind,
    /// `cNvPr` `id`, unique within one slide as written.
    pub id: Option<u32>,
    pub name: String,
    pub hidden: bool,
    pub placeholder: Option<Placeholder>,
    /// A shape that plays video or audio.
    pub media: bool,
    /// The `xfrm` as written; `None` for a placeholder that takes its
    /// geometry from the layout.
    pub geometry: Option<Geometry>,
    /// The geometry in effect: `geometry`, else the layout placeholder's,
    /// else the master placeholder's.
    pub effective_geometry: Option<Geometry>,
    /// The text body's own list styles (`a:lstStyle`), by level.
    pub list_style: Option<Levels>,
    /// The preset outline (`prstGeom` `prst`), or `custom`.
    pub outline: Option<String>,
    /// The shape's own fill and line, described canonically.
    pub fill: Option<String>,
    pub line: Option<String>,
    /// The text body of an autoshape or text box, when it has one.
    pub text: Option<Vec<Paragraph>>,
    pub image: Option<Image>,
    pub table: Option<Table>,
    pub chart: Option<ChartRef>,
    /// A group's members, in z-order.
    pub children: Vec<Shape>,
}

impl Shape {
    pub(crate) fn new(kind: ShapeKind) -> Self {
        Self {
            kind,
            id: None,
            name: String::new(),
            hidden: false,
            placeholder: None,
            media: false,
            geometry: None,
            effective_geometry: None,
            list_style: None,
            outline: None,
            fill: None,
            line: None,
            text: None,
            image: None,
            table: None,
            chart: None,
            children: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comment {
    pub author: Option<String>,
    pub text: String,
    /// Replies in order, as `author: text` when the author is known.
    pub replies: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Slide {
    /// The slide part's member name.
    pub part: String,
    /// The layout part's member name.
    pub layout_part: Option<String>,
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
    /// Comments in file order, without their replies.
    pub comments: Vec<Comment>,
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

/// One outline level's defaults in a list style (`a:lvlNpPr`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LevelStyle {
    /// The level's paragraph properties (alignment, bullet).
    pub paragraph: Format,
    /// Its default run properties (`a:defRPr`).
    pub run: Format,
}

/// A list style: levels 1 to 9 at indices 0 to 8.
pub type Levels = Vec<LevelStyle>;

/// The colour slots of a theme's colour scheme.
pub const THEME_COLORS: [&str; 12] =
    ["dk1", "lt1", "dk2", "lt2", "accent1", "accent2", "accent3", "accent4", "accent5", "accent6", "hlink", "folHlink"];

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Theme {
    pub name: Option<String>,
    /// Latin and East Asian typefaces of the major (headings) and minor
    /// (body) fonts.
    pub major: Option<String>,
    pub minor: Option<String>,
    pub major_east_asian: Option<String>,
    pub minor_east_asian: Option<String>,
    /// Colours by slot, described canonically.
    pub colors: BTreeMap<&'static str, String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MasterDef {
    pub part: String,
    pub name: Option<Arc<str>>,
    /// The master's shapes, placeholders among them.
    pub shapes: Vec<Shape>,
    /// `p:txStyles`: for titles, for bodies, and for everything else.
    pub title_style: Levels,
    pub body_style: Levels,
    pub other_style: Levels,
    pub theme: Option<Theme>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LayoutDef {
    pub part: String,
    pub name: Option<Arc<str>>,
    /// Index into [`Template::masters`].
    pub master: Option<usize>,
    pub shapes: Vec<Shape>,
}

/// What slides inherit from.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Template {
    pub masters: Vec<MasterDef>,
    pub layouts: Vec<LayoutDef>,
    /// `p:defaultTextStyle` in `presentation.xml`.
    pub default_style: Levels,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Presentation {
    pub format: DeckFormat,
    pub slide_size: Option<Size>,
    /// In show order (`sldIdLst`).
    pub slides: Vec<Slide>,
    pub sections: Vec<Section>,
    pub masters: Vec<Master>,
    pub template: Template,
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
