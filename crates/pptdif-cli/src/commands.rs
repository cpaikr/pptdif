//! Runs one parsed command against the library.

use std::fmt::Write as _;
use std::path::Path;

use pptdif::diff::{self, AlignMode, Change, CompareOptions, Filter, Listing, Side, Status};
use pptdif::model::Presentation;
use pptdif::views::{self, Inspection, TextListing};
use pptdif::{Error, ErrorCode};
use serde_json::Value;

use crate::cli::{Align, Command, Format, Kind};
use crate::envelope::{Failure, Warning};

pub enum Output {
    Text(String),
    Json { result: Value, warnings: Vec<Warning> },
}

fn json<T: serde::Serialize>(value: &T) -> Value {
    serde_json::to_value(value).expect("view types always serialize")
}

fn file_label(path: &Path) -> String {
    path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned())
}

pub fn run(command: Command) -> Result<Output, Failure> {
    match command {
        Command::Inspect { deck, format } => {
            let presentation = pptdif::read_path(&deck)?;
            let inspection = views::inspect(&file_label(&deck), &presentation);
            let warnings: Vec<Warning> = presentation.warnings.into_iter().map(Warning::from).collect();
            Ok(match format {
                Format::Json => Output::Json { result: json(&inspection), warnings },
                Format::Text => Output::Text(inspect_text(&inspection, &warnings)),
            })
        }
        Command::Text { deck, slide, format } => {
            let presentation = pptdif::read_path(&deck)?;
            check_slide(slide, &[&presentation])?;
            let listing = views::text(&file_label(&deck), &presentation, slide);
            let warnings: Vec<Warning> = presentation.warnings.into_iter().map(Warning::from).collect();
            Ok(match format {
                Format::Json => Output::Json { result: json(&listing), warnings },
                Format::Text => Output::Text(text_text(&listing, &warnings)),
            })
        }
        Command::Compare { base, other, slide, kind, limit, all, align, format } => {
            let (a, b) = (pptdif::read_path(&base)?, pptdif::read_path(&other)?);
            check_slide(slide, &[&a, &b])?;
            let (fa, fb) = (file_label(&base), file_label(&other));
            let options = CompareOptions {
                align: match align {
                    Align::Content => AlignMode::Content,
                    Align::Position => AlignMode::Position,
                },
            };
            let comparison = diff::compare(&fa, &a, &fb, &b, options);
            let filter = Filter { slide, kinds: kind.into_iter().map(kind_of).collect(), limit, all };
            let listing = diff::listing(&comparison, &filter);
            let mut warnings = Vec::new();
            for (label, deck) in [(&fa, a), (&fb, b)] {
                warnings.extend(
                    deck.warnings
                        .into_iter()
                        .map(|w| Warning { code: w.code.to_owned(), message: format!("{label}: {}", w.message) }),
                );
            }
            if comparison.fell_back {
                warnings.push(Warning {
                    code: "alignment_fallback".to_owned(),
                    message: "a run of slides was too large to align fully and was paired near its diagonal; \
                              the best pairing may have been missed"
                        .to_owned(),
                });
            }
            Ok(match format {
                Format::Json => Output::Json { result: json(&listing), warnings },
                Format::Text => Output::Text(compare_text(&listing, &warnings)),
            })
        }
    }
}

const fn kind_of(kind: Kind) -> diff::Kind {
    match kind {
        Kind::Slide => diff::Kind::Slide,
        Kind::Shape => diff::Kind::Shape,
        Kind::Text => diff::Kind::Text,
        Kind::Notes => diff::Kind::Notes,
    }
}

/// A `--slide` past the end of every deck is a mistake, not an empty result.
fn check_slide(slide: Option<usize>, decks: &[&Presentation]) -> Result<(), Failure> {
    let most = decks.iter().map(|d| d.slides.len()).max().unwrap_or(0);
    match slide {
        Some(n) if n == 0 || n > most => {
            Err(Error::new(ErrorCode::InvalidArgument, format!("--slide {n} is outside the deck (slides 1 to {most})"))
                .into())
        }
        _ => Ok(()),
    }
}

fn inspect_text(i: &Inspection, warnings: &[Warning]) -> String {
    let mut out = String::new();
    let size = i
        .slide_size
        .as_ref()
        .map_or_else(|| "no slide size".to_owned(), |s| format!("{} x {} pt", s.width_pt, s.height_pt));
    let _ = writeln!(out, "{} ({:?}, {size}, {} slides)", escape(&i.file), i.format, i.slides.len());
    for s in &i.slides {
        let mut counts = format!("shapes {}", s.shapes);
        for (n, what) in [
            (s.groups, "groups"),
            (s.pictures, "pictures"),
            (s.tables, "tables"),
            (s.charts, "charts"),
            (s.comments, "comments"),
        ] {
            if n > 0 {
                let _ = write!(counts, " {what} {n}");
            }
        }
        if s.notes {
            counts.push_str(" notes");
        }
        let _ = writeln!(
            out,
            "{}\tid {}\t{}\t{}\t{counts}",
            s.index,
            s.id,
            if s.hidden { "hidden" } else { "shown" },
            escape(s.layout.as_deref().unwrap_or("-")),
        );
    }
    for s in &i.sections {
        let _ = writeln!(out, "section\t{}\t{} slides", escape(&s.name), s.slides);
    }
    for (n, m) in (1..).zip(&i.masters) {
        let layouts: Vec<String> = m.layouts.iter().map(|l| escape(l.as_deref().unwrap_or("-"))).collect();
        let _ = writeln!(out, "master\t{n}\t{}\t{}", escape(m.name.as_deref().unwrap_or("-")), layouts.join(", "));
    }
    write_warnings(&mut out, warnings);
    out
}

fn text_text(listing: &TextListing, warnings: &[Warning]) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "{}", escape(&listing.file));
    for slide in &listing.slides {
        let _ = writeln!(out, "slide {}\tid {}", slide.index, slide.id);
        for shape in &slide.shapes {
            let label = match &shape.placeholder {
                Some(p) => format!("{} [{p}]", escape(&shape.name)),
                None => escape(&shape.name),
            };
            for p in &shape.paragraphs {
                let _ = writeln!(out, "  {label}\t{}{}", "  ".repeat(usize::from(p.level)), escape(&p.text));
            }
        }
        for p in slide.notes.iter().flatten() {
            let _ = writeln!(out, "  notes\t{}", escape(&p.text));
        }
    }
    write_warnings(&mut out, warnings);
    out
}

fn side_label(side: &Side) -> String {
    let mut out = format!("slide {}", side.slide);
    if let Some(shape) = &side.shape {
        let _ = write!(out, " {}", escape(&shape.name));
    }
    if let Some(p) = side.paragraph {
        let _ = write!(out, " para {p}");
    }
    out
}

fn change_line(c: &Change) -> String {
    let status = match c.status {
        Status::Changed => "changed",
        Status::OnlyBase => "removed",
        Status::OnlyOther => "added",
        Status::Moved => "moved",
        Status::Same => "same",
    };
    let at = match (&c.base, &c.other) {
        (Some(a), Some(b)) => format!("{} -> {}", side_label(a), side_label(b)),
        (Some(a), None) => side_label(a),
        (None, Some(b)) => side_label(b),
        (None, None) => String::new(),
    };
    let mut line = format!("{}\t{}\t{status}\t{at}", c.id, c.kind.as_str());
    match (c.base.as_ref().and_then(|s| s.text.as_ref()), c.other.as_ref().and_then(|s| s.text.as_ref())) {
        (Some(a), Some(b)) if a != b => {
            let _ = write!(line, "\t{} => {}", escape(a), escape(b));
        }
        (Some(t), None) | (None, Some(t)) => {
            let _ = write!(line, "\t{}", escape(t));
        }
        _ => {}
    }
    for p in &c.properties {
        let _ = write!(line, "\t{}: {} => {}", p.name, escape(&p.base.to_string()), escape(&p.other.to_string()));
    }
    line
}

fn compare_text(listing: &Listing, warnings: &[Warning]) -> String {
    let mut out = String::new();
    let s = &listing.summary;
    let _ = writeln!(
        out,
        "{} ({} slides) -> {} ({} slides): {} changed, {} removed, {} added, {} moved",
        escape(&listing.base.file),
        listing.base.slides,
        escape(&listing.other.file),
        listing.other.slides,
        s.slides_changed,
        s.slides_only_base,
        s.slides_only_other,
        s.slides_moved
    );
    for pair in &listing.slides {
        for c in &pair.changes {
            let _ = writeln!(out, "{}", change_line(c));
        }
    }
    let omitted: usize = listing.omitted.values().flat_map(|m| m.values()).sum();
    if omitted > 0 {
        let _ = writeln!(out, "omitted\t{omitted} changes (raise --limit or narrow with --slide and --kind)");
    }
    write_warnings(&mut out, warnings);
    out
}

fn write_warnings(out: &mut String, warnings: &[Warning]) {
    for w in warnings {
        let _ = writeln!(out, "warning\t{}\t{}", w.code, escape(&w.message));
    }
}

/// Keeps one record per line: tabs, newlines, and control characters in
/// names are escaped.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c if c.is_control() => {
                let _ = write!(out, "\\u{{{:x}}}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out
}
