//! The command model. Help text here is the CLI's only command inventory.

use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

const ROOT_AFTER: &str = "\
Typical flow:
  1. pptdif inspect deck.pptx                 slides, layouts, counts (no text)

Comparison (`compare`) and text listings (`text`) are being built; see
PLAN.md.

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
}

impl Command {
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Inspect { .. } => "inspect",
        }
    }
}
