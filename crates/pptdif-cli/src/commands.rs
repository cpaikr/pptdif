//! Runs one parsed command against the library.

use std::fmt::Write as _;
use std::path::Path;

use pptdif::views::{self, Inspection};
use serde_json::Value;

use crate::cli::{Command, Format};
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
    for w in warnings {
        let _ = writeln!(out, "warning\t{}\t{}", w.code, escape(&w.message));
    }
    out
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
