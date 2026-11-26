//! What each slide shape inherits: the geometry and text formatting in
//! effect once the layout, master, and presentation defaults fill in what
//! the slide leaves out.
//!
//! For a placeholder, properties come first from what the slide writes,
//! then from the layout placeholder it matches, then from the master
//! placeholder of its type, then from the master's text style for that
//! type (`titleStyle` for titles, `bodyStyle` for bodies, `otherStyle` for
//! the rest), then from the presentation's `defaultTextStyle`. A shape
//! that is not a placeholder (a text box) takes the default style after
//! its own list styles. Theme fonts (`+mj-lt`, `+mn-ea`, ...) are named
//! from the master's theme. Checked against PowerPoint's own resolution by
//! `scripts/format_parity.py`.

use std::collections::HashMap;

use crate::model::{Format, Levels, MasterDef, Paragraph, Presentation, Shape, Theme};

/// The master's text style a placeholder of `kind` draws on.
fn master_style<'a>(master: &'a MasterDef, kind: Option<&str>) -> &'a Levels {
    match kind {
        Some("title" | "ctrTitle") => &master.title_style,
        Some("dt" | "ftr" | "sldNum" | "hdr") | None => &master.other_style,
        Some(_) => &master.body_style,
    }
}

/// The master placeholder type a slide or layout placeholder falls back
/// to: titles to the title, dates, footers, numbers, and headers to their
/// own, everything else to the body.
fn master_kind(kind: &str) -> &str {
    match kind {
        "title" | "ctrTitle" => "title",
        "dt" | "ftr" | "sldNum" | "hdr" => kind,
        _ => "body",
    }
}

/// A layout's or master's placeholders, indexed once by index and by type
/// (the first of each), so matching costs the same however many
/// placeholders a hostile part holds.
#[derive(Default)]
struct Placeholders<'a> {
    by_index: HashMap<u32, &'a Shape>,
    by_kind: HashMap<&'a str, &'a Shape>,
}

impl<'a> Placeholders<'a> {
    fn of(shapes: &'a [Shape]) -> Self {
        let mut out = Self::default();
        for s in shapes.iter().flat_map(Shape::walk) {
            if let Some(ph) = &s.placeholder {
                out.by_index.entry(ph.index).or_insert(s);
                out.by_kind.entry(ph.kind.as_str()).or_insert(s);
            }
        }
        out
    }

    /// The placeholder a slide placeholder takes after: the one with its
    /// index when it has one, else the one of its type.
    fn matching(&self, shape: &Shape) -> Option<&'a Shape> {
        let ph = shape.placeholder.as_ref()?;
        let by_index = if ph.index == 0 && !ph.explicit_index { None } else { self.by_index.get(&ph.index).copied() };
        by_index.or_else(|| self.by_kind.get(ph.kind.as_str()).copied())
    }

    fn of_kind(&self, kind: &str) -> Option<&'a Shape> {
        self.by_kind.get(master_kind(kind)).copied()
    }
}

/// A layout with its placeholders indexed, and its master's.
struct Inherited<'a> {
    layout: Placeholders<'a>,
    master: Option<(&'a MasterDef, Placeholders<'a>)>,
}

/// Fills `into` with what `from` has that `into` lacks.
fn inherit(into: &mut Format, from: &Format) {
    for (k, v) in from {
        into.entry(k).or_insert_with(|| v.clone());
    }
}

/// Theme font tokens named: `+mj-lt` is the major Latin typeface, `+mn-ea`
/// the minor East Asian one.
fn name_fonts(format: &mut Format, theme: Option<&Theme>) {
    let Some(theme) = theme else { return };
    for key in ["font", "fontEastAsian", "fontComplex"] {
        let face = match format.get(key).map(String::as_str) {
            Some("+mj-lt") => theme.major.clone(),
            Some("+mn-lt") => theme.minor.clone(),
            Some("+mj-ea") => theme.major_east_asian.clone(),
            Some("+mn-ea") => theme.minor_east_asian.clone(),
            Some("+mj-cs") => theme.major_complex.clone(),
            Some("+mn-cs") => theme.minor_complex.clone(),
            _ => continue,
        };
        match face {
            Some(face) => format.insert(key, face),
            None => format.remove(key),
        };
    }
}

fn resolve_paragraphs(paragraphs: &mut [Paragraph], sources: &[&Levels], theme: Option<&Theme>) {
    for p in paragraphs {
        let level = usize::from(p.level).min(8);
        p.effective = p.format.clone();
        for levels in sources {
            if let Some(l) = levels.get(level) {
                inherit(&mut p.effective, &l.paragraph);
            }
        }
        for run in &mut p.runs {
            run.effective = run.format.clone();
            for levels in sources {
                if let Some(l) = levels.get(level) {
                    inherit(&mut run.effective, &l.run);
                }
            }
            name_fonts(&mut run.effective, theme);
        }
    }
}

fn resolve_shape(shape: &mut Shape, from: Option<&Inherited<'_>>, default: &Levels) {
    for child in &mut shape.children {
        resolve_shape(child, from, default);
    }
    let from_layout = from.and_then(|f| f.layout.matching(shape));
    // A placeholder that names no type (`<p:ph idx="11"/>` reads as `obj`)
    // is whatever its layout placeholder is: a footer, say.
    let kind = shape.placeholder.as_ref().map(|p| {
        let layout_kind = from_layout.and_then(|l| l.placeholder.as_ref()).map(|l| l.kind.clone());
        if p.kind == "obj" { layout_kind.unwrap_or_else(|| p.kind.clone()) } else { p.kind.clone() }
    });
    let master = from.and_then(|f| f.master.as_ref());
    let from_master = match (master, &kind) {
        (Some((_, m)), Some(k)) => m.of_kind(k),
        _ => None,
    };
    shape.effective_geometry = shape
        .geometry
        .or_else(|| from_layout.and_then(|s| s.geometry))
        .or_else(|| from_master.and_then(|s| s.geometry));

    let mut sources: Vec<&Levels> = Vec::new();
    if let Some(own) = &shape.list_style {
        sources.push(own);
    }
    if kind.is_some() {
        sources.extend(from_layout.and_then(|s| s.list_style.as_ref()));
        sources.extend(from_master.and_then(|s| s.list_style.as_ref()));
        if let Some((m, _)) = master {
            sources.push(master_style(m, kind.as_deref()));
        }
    }
    sources.push(default);
    let theme = master.and_then(|(m, _)| m.theme.as_ref());
    if let Some(text) = &mut shape.text {
        resolve_paragraphs(text, &sources, theme);
    }
    // Table cells take their formatting from the table style, which is not
    // read: their effective formatting is what they write.
    if let Some(table) = &mut shape.table {
        for cell in table.rows.iter_mut().flatten() {
            resolve_paragraphs(&mut cell.text, &[], theme);
        }
    }
}

/// Fills in every slide shape's effective geometry and text formatting.
pub(crate) fn resolve(presentation: &mut Presentation) {
    let Presentation { slides, template, .. } = presentation;
    let indexed: HashMap<&str, Inherited<'_>> = template
        .layouts
        .iter()
        .map(|l| {
            let master = l.master.and_then(|m| template.masters.get(m)).map(|m| (m, Placeholders::of(&m.shapes)));
            (l.part.as_str(), Inherited { layout: Placeholders::of(&l.shapes), master })
        })
        .collect();
    for slide in slides {
        let from = slide.layout_part.as_deref().and_then(|p| indexed.get(p));
        for shape in &mut slide.shapes {
            resolve_shape(shape, from, &template.default_style);
        }
        if let Some(notes) = &mut slide.notes {
            resolve_paragraphs(notes, &[], None);
        }
    }
}
