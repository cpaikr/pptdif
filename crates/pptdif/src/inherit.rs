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

use crate::model::{Format, LayoutDef, Levels, MasterDef, Paragraph, Presentation, Shape, Theme};

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

fn placeholders(shapes: &[Shape]) -> impl Iterator<Item = &Shape> {
    shapes.iter().flat_map(Shape::walk).filter(|s| s.placeholder.is_some())
}

/// The layout placeholder a slide placeholder takes after: the one with
/// its index when it has one, else the one of its type.
fn layout_placeholder<'a>(layout: &'a LayoutDef, shape: &Shape) -> Option<&'a Shape> {
    let ph = shape.placeholder.as_ref()?;
    let found = if ph.index != 0 {
        placeholders(&layout.shapes).find(|s| s.placeholder.as_ref().is_some_and(|l| l.index == ph.index))
    } else {
        None
    };
    found.or_else(|| placeholders(&layout.shapes).find(|s| s.placeholder.as_ref().is_some_and(|l| l.kind == ph.kind)))
}

fn master_placeholder<'a>(master: &'a MasterDef, kind: &str) -> Option<&'a Shape> {
    let wanted = master_kind(kind);
    placeholders(&master.shapes).find(|s| s.placeholder.as_ref().is_some_and(|m| m.kind == wanted))
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
    for key in ["font", "fontEastAsian"] {
        let face = match format.get(key).map(String::as_str) {
            Some("+mj-lt") => theme.major.clone(),
            Some("+mn-lt") => theme.minor.clone(),
            Some("+mj-ea") => theme.major_east_asian.clone(),
            Some("+mn-ea") => theme.minor_east_asian.clone(),
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

fn resolve_shape(shape: &mut Shape, layout: Option<&LayoutDef>, master: Option<&MasterDef>, default: &Levels) {
    for child in &mut shape.children {
        resolve_shape(child, layout, master, default);
    }
    let kind = shape.placeholder.as_ref().map(|p| p.kind.clone());
    let from_layout = layout.and_then(|l| layout_placeholder(l, shape));
    let from_master = match (master, &kind) {
        (Some(m), Some(k)) => master_placeholder(m, k),
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
        if let Some(m) = master {
            sources.push(master_style(m, kind.as_deref()));
        }
    }
    sources.push(default);
    let theme = master.and_then(|m| m.theme.as_ref());
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
    let layouts: HashMap<&str, &LayoutDef> = template.layouts.iter().map(|l| (l.part.as_str(), l)).collect();
    for slide in slides {
        let layout = slide.layout_part.as_deref().and_then(|p| layouts.get(p).copied());
        let master = layout.and_then(|l| l.master).and_then(|m| template.masters.get(m));
        for shape in &mut slide.shapes {
            resolve_shape(shape, layout, master, &template.default_style);
        }
        if let Some(notes) = &mut slide.notes {
            resolve_paragraphs(notes, &[], None);
        }
    }
}
