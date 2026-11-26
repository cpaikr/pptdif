//! The command model. Help text here is the CLI's only command inventory.

use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

const ROOT_AFTER: &str = "\
Typical flow:
  1. pptdif inspect deck.pptx                 slides, layouts, counts (no text)
  2. pptdif compare old.pptx new.pptx         what changed, slides aligned
  3. pptdif compare old.pptx new.pptx --slide 4 --kind text --format text
  4. pptdif text new.pptx --slide 4 --format text

Output:
  Commands print one JSON object to stdout: {ok, command, result, warnings,
  error, metadata}. Exit status 0 = ok, 1 = failure, 2 = invalid invocation.
  Views accept --format text for a compact listing; failures stay JSON.
  --pretty indents JSON. Help and --version are plain text.

pptdif reads .pptx and .pptm files (and their template and show variants)
as saved; it needs no PowerPoint. A not_compared warning names content it
does not compare: SmartArt, embedded objects, media, ink, animations, custom
shows, and macros.";

#[derive(Debug, Parser)]
#[command(
    name = "pptdif",
    bin_name = "pptdif",
    version,
    about = "Compare PowerPoint presentations: slides, text, shapes, and formatting.",
    after_help = ROOT_AFTER
)]
pub struct Cli {
    /// Indent JSON output.
    #[arg(long, global = true)]
    pub pretty: bool,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Format {
    Json,
    Text,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Kind {
    Slide,
    Shape,
    Text,
    Notes,
    Geometry,
    Format,
    Table,
    Picture,
    Chart,
    Comment,
    Section,
    Layout,
    Master,
    Theme,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Align {
    /// Pair slides by their titles and text.
    Content,
    /// Pair slide n with slide n.
    Position,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Presentation structure without slide text: the slide size, each
    /// slide's id, visibility, layout, section, and counts of shapes,
    /// groups, placeholders, pictures, tables, charts, notes, and comments,
    /// plus sections and the masters with their layouts.
    #[command(after_help = "\
Examples:
  pptdif inspect deck.pptx
  pptdif inspect deck.pptx --format text

A slide's id (sldId) is stable while the slide stays in one file; index is
its position in the show. shapes counts every shape except groups, group
members included.")]
    Inspect {
        /// The presentation (.pptx or .pptm).
        deck: PathBuf,
        #[arg(long, value_enum, default_value_t = Format::Json)]
        format: Format,
    },
    /// The text of every shape, slide by slide, with each paragraph's
    /// outline level, and the speaker notes.
    #[command(after_help = "\
Examples:
  pptdif text deck.pptx
  pptdif text deck.pptx --slide 3 --format text

Fields (slide numbers, dates) show the text PowerPoint saved; line breaks
inside a paragraph are \\n.")]
    Text {
        /// The presentation (.pptx or .pptm).
        deck: PathBuf,
        /// Only this slide (1-based position).
        #[arg(long)]
        slide: Option<usize>,
        #[arg(long, value_enum, default_value_t = Format::Json)]
        format: Format,
    },
    /// What changed between two presentations. Slides pair by title and
    /// text, so an inserted or reordered slide is one change; shapes pair
    /// within each slide by placeholder, id, name, and content; paragraphs
    /// align, so a new bullet is one change.
    #[command(after_help = "\
Examples:
  pptdif compare v1.pptx v2.pptx
  pptdif compare v1.pptx v2.pptx --kind text,notes --format text
  pptdif compare v1.pptx v2.pptx --slide 4 --limit 0

Reading the result:
  slides lists slide pairs (id p<n>) with a status: same, changed, onlyBase,
  onlyOther, or moved (a moved slide may also carry edits). Each change has a
  stable id, a kind, a status (changed, onlyBase, onlyOther, moved), and
  base and other sides naming the slide (1-based), the shape (id, name,
  placeholder), the paragraph or table row and column (1-based), and its
  text. Kinds: slide (added, removed, moved, visibility, layout), shape
  (added, removed, kind, name, zOrder, group), text and notes (paragraphs),
  geometry (x, y, width, height in points, rotation, flips; beyond
  --tolerance), format (run and paragraph formatting of unchanged text, a
  shape's outline, fill, and line), table (rows, columns,
  cells), picture (image content, crop), chart (kinds, title, series
  categories and values as cached), comment (text, author, replies), and,
  under objects, section, layout and master (placeholders, text styles),
  and theme (fonts, colours). Changed properties are listed with
  both values. summary counts everything; listed and omitted say what
  --kind, --slide, and --limit left out. Fields compare by kind, not by the
  number or date they showed, so renumbered footers are not changes.
  Formatting and geometry show the values in effect (inherited through the
  layout, master, and theme), and are reported on a slide only where the
  slide itself writes something different; a change to a layout, master,
  or theme is reported once, under objects.")]
    Compare {
        /// The earlier presentation (left side).
        base: PathBuf,
        /// The later presentation (right side).
        other: PathBuf,
        /// Only slide pairs with this slide (1-based) on either side.
        #[arg(long)]
        slide: Option<usize>,
        /// Only these kinds of change.
        #[arg(long, value_enum, value_delimiter = ',')]
        kind: Vec<Kind>,
        /// The most changes to list; 0 lists every change.
        #[arg(long, default_value_t = 500)]
        limit: usize,
        /// Also list slides with no changes.
        #[arg(long)]
        all: bool,
        /// Positions and sizes closer than this, in points, are equal.
        #[arg(long, default_value_t = 0.5)]
        tolerance: f64,
        #[arg(long, value_enum, default_value_t = Align::Content)]
        align: Align,
        #[arg(long, value_enum, default_value_t = Format::Json)]
        format: Format,
    },
}

impl Command {
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Inspect { .. } => "inspect",
            Self::Text { .. } => "text",
            Self::Compare { .. } => "compare",
        }
    }
}
