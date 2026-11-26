//! What a paired shape holds beyond its text (geometry, formatting,
//! pictures, tables, charts), where shapes sit (z-order, groups), and the
//! slide's comments and the deck's sections.
//!
//! Every function here runs inside one slide pair's fixed orientation (see
//! the parent module), so ties broken by index still mirror.

use std::collections::{BTreeSet, HashMap, HashSet};

use serde_json::{Value, json};

use super::{
    Change, Kind, ObjectChange, Property, ShapeRef, Side, Status, Work, best_first, containment, diff_paragraphs, hash,
    normalize, words,
};
use crate::align::{self, Item, Options, longest_increasing};
use crate::model::{Chart, Comment, Format, Geometry, Paragraph, Presentation, RunKind, Series, Shape, Table};

const EMU_PER_POINT: f64 = 12_700.0;

fn side(slide: usize, shape: &ShapeRef) -> Side {
    Side { slide, shape: Some(shape.clone()), paragraph: None, text: None, row: None, column: None }
}

fn changed(kind: Kind, a: Side, b: Side, properties: Vec<Property>) -> Change {
    let mut c = Change::new(kind, Status::Changed, Some(a), Some(b));
    c.properties = properties;
    c
}

fn opt(v: Option<&str>) -> Value {
    v.map_or(Value::Null, Value::from)
}

/// Points, rounded to hundredths.
fn points(emu: i64) -> Value {
    json!((emu as f64 / EMU_PER_POINT * 100.0).round() / 100.0)
}

fn geometry_properties(a: &Geometry, b: &Geometry, tolerance: i64) -> Vec<Property> {
    let mut props = Vec::new();
    for (name, x, y) in [("x", a.x, b.x), ("y", a.y, b.y), ("width", a.cx, b.cx), ("height", a.cy, b.cy)] {
        if (x - y).abs() > tolerance {
            props.push(Property { name, base: points(x), other: points(y) });
        }
    }
    if a.rotation != b.rotation {
        let degrees = |r: i64| json!(r as f64 / 60_000.0);
        props.push(Property { name: "rotation", base: degrees(a.rotation), other: degrees(b.rotation) });
    }
    if a.flip_h != b.flip_h {
        props.push(Property { name: "flipH", base: json!(a.flip_h), other: json!(b.flip_h) });
    }
    if a.flip_v != b.flip_v {
        props.push(Property { name: "flipV", base: json!(a.flip_v), other: json!(b.flip_v) });
    }
    props
}

/// Every change a paired shape holds besides its kind, name, and text.
pub(super) fn shape_changes(
    sa: &Shape,
    (ai, ra): (usize, &ShapeRef),
    sb: &Shape,
    (bi, rb): (usize, &ShapeRef),
    tolerance: i64,
    work: &mut Work,
) -> Vec<Change> {
    let mut changes = Vec::new();
    // A placeholder with no `xfrm` sits where its layout puts it; that is
    // resolved with inheritance (M3), so only written geometry compares.
    if let (Some(ga), Some(gb)) = (&sa.geometry, &sb.geometry) {
        let props = geometry_properties(ga, gb, tolerance);
        if !props.is_empty() {
            changes.push(changed(Kind::Geometry, side(ai, ra), side(bi, rb), props));
        }
    }
    let mut props = Vec::new();
    for (name, x, y) in
        [("outline", &sa.outline, &sb.outline), ("fill", &sa.fill, &sb.fill), ("line", &sa.line, &sb.line)]
    {
        if x != y {
            props.push(Property { name, base: opt(x.as_deref()), other: opt(y.as_deref()) });
        }
    }
    if !props.is_empty() {
        changes.push(changed(Kind::Format, side(ai, ra), side(bi, rb), props));
    }
    let (ta, tb) = (sa.text.as_deref().unwrap_or_default(), sb.text.as_deref().unwrap_or_default());
    changes.extend(text_format_changes(ta, (ai, ra), tb, (bi, rb), work));
    if let (Some(ia), Some(ib)) = (&sa.image, &sb.image) {
        let mut props = Vec::new();
        if ia.hash != ib.hash {
            props.push(Property { name: "image", base: opt(ia.part.as_deref()), other: opt(ib.part.as_deref()) });
        }
        if ia.crop != ib.crop {
            props.push(Property { name: "crop", base: opt(ia.crop.as_deref()), other: opt(ib.crop.as_deref()) });
        }
        if !props.is_empty() {
            changes.push(changed(Kind::Picture, side(ai, ra), side(bi, rb), props));
        }
    }
    if let (Some(ta), Some(tb)) = (&sa.table, &sb.table) {
        changes.extend(table_changes(ta, (ai, ra), tb, (bi, rb), work));
    }
    if let (Some(ca), Some(cb)) =
        (sa.chart.as_ref().and_then(|c| c.data.as_ref()), sb.chart.as_ref().and_then(|c| c.data.as_ref()))
    {
        changes.extend(chart_changes(ca, (ai, ra), cb, (bi, rb)));
    }
    changes
}

// ---------------------------------------------------------------------------
// Formatting of unchanged text

/// A paragraph as compared, one format per character: the spans formats
/// are compared over when the text itself did not change.
fn char_formats(p: &Paragraph) -> Vec<(char, &Format)> {
    static NONE: std::sync::LazyLock<Format> = std::sync::LazyLock::new(Format::new);
    let mut out = Vec::new();
    for run in &p.runs {
        match &run.kind {
            RunKind::Text => out.extend(run.text.chars().map(|c| (c, &run.format))),
            RunKind::Field(kind) => {
                // As `Paragraph::compared` writes a field.
                out.extend(format!("\0{kind}\0").chars().map(|c| (c, &run.format)));
            }
            RunKind::Break => out.push(('\n', &*NONE)),
        }
    }
    out
}

/// The properties two formats differ in.
fn format_differences(a: &Format, b: &Format) -> Vec<Property> {
    let keys: BTreeSet<&&'static str> = a.keys().chain(b.keys()).collect();
    keys.into_iter()
        .filter(|k| a.get(**k) != b.get(**k))
        .map(|k| Property {
            name: k,
            base: opt(a.get(*k).map(String::as_str)),
            other: opt(b.get(*k).map(String::as_str)),
        })
        .collect()
}

/// Formatting changes on paragraphs whose text did not change (moved or
/// not): the
/// paragraph's own (level, alignment, bullet), then each stretch of
/// characters whose run formatting differs in the same way. A paragraph
/// whose text changed reports the text change only.
fn text_format_changes(
    a: &[Paragraph],
    (ai, ra): (usize, &ShapeRef),
    b: &[Paragraph],
    (bi, rb): (usize, &ShapeRef),
    work: &mut Work,
) -> Vec<Change> {
    let mut changes = Vec::new();
    for (status, i, j) in diff_paragraphs(a, b, work) {
        // Unchanged text, in place or moved.
        let (Status::Same | Status::Moved, Some(i), Some(j)) = (status, i, j) else { continue };
        let (pa, pb) = (&a[i], &b[j]);
        let at = |slide, r: &ShapeRef, n: usize, text: String| Side {
            paragraph: Some(n + 1),
            text: Some(text),
            ..side(slide, r)
        };
        let mut props = format_differences(&pa.format, &pb.format);
        if pa.level != pb.level {
            props.insert(0, Property { name: "level", base: json!(pa.level), other: json!(pb.level) });
        }
        if !props.is_empty() {
            changes.push(changed(Kind::Format, at(ai, ra, i, pa.text()), at(bi, rb, j, pb.text()), props));
        }
        // Stretches of characters with one difference between the sides.
        let (ca, cb) = (char_formats(pa), char_formats(pb));
        let mut stretch: Option<(String, Vec<Property>)> = None;
        let flush = |stretch: &mut Option<(String, Vec<Property>)>, changes: &mut Vec<Change>| {
            if let Some((text, props)) = stretch.take() {
                let text = text.trim().to_owned();
                if !text.is_empty() {
                    changes.push(changed(Kind::Format, at(ai, ra, i, text.clone()), at(bi, rb, j, text), props));
                }
            }
        };
        for ((c, fa), (_, fb)) in ca.iter().zip(&cb) {
            let diff = format_differences(fa, fb);
            match &mut stretch {
                Some((text, props)) if *props == diff => text.push(*c),
                _ => {
                    flush(&mut stretch, &mut changes);
                    if !diff.is_empty() {
                        stretch = Some((c.to_string(), diff));
                    }
                }
            }
        }
        flush(&mut stretch, &mut changes);
    }
    changes
}

// ---------------------------------------------------------------------------
// Tables

fn cell_text(table: &Table, r: usize, c: usize) -> String {
    table.rows[r]
        .get(c)
        .map(|cell| cell.text.iter().map(Paragraph::text).collect::<Vec<_>>().join("\n"))
        .unwrap_or_default()
}

fn columns(table: &Table) -> usize {
    table.rows.iter().map(Vec::len).max().unwrap_or(0)
}

/// Columns align first, by their cells' text with the header as label;
/// rows then align by their cells in aligned columns, so an inserted
/// column cannot break row matching.
/// The most grid cells (rows times the widest row) a table comparison
/// reads; a larger table, which a ragged hostile file can make from few
/// records, is not compared cell by cell and the comparison falls back.
const MAX_TABLE_CELLS: usize = 1_000_000;

fn table_changes(
    ta: &Table,
    (ai, ra): (usize, &ShapeRef),
    tb: &Table,
    (bi, rb): (usize, &ShapeRef),
    work: &mut Work,
) -> Vec<Change> {
    let cells = |t: &Table| t.rows.len().saturating_mul(columns(t));
    if cells(ta) > MAX_TABLE_CELLS || cells(tb) > MAX_TABLE_CELLS {
        work.fell_back = true;
        return Vec::new();
    }
    let column_item = |t: &Table, c: usize| {
        let mut tokens: Vec<u64> = (0..t.rows.len()).map(|r| hash(&normalize(&cell_text(t, r, c)))).collect();
        tokens.sort_unstable();
        let header = normalize(&cell_text(t, 0, c));
        Item { key: (!header.is_empty()).then_some(header), tokens, shape: 0 }
    };
    let ca: Vec<Item> = (0..columns(ta)).map(|c| column_item(ta, c)).collect();
    let cb: Vec<Item> = (0..columns(tb)).map(|c| column_item(tb, c)).collect();
    let cols = align::align(&ca, &cb, Options { positional_equal_gaps: true });
    let col_map: HashMap<usize, usize> = cols.pairs.iter().chain(&cols.moves).copied().collect();

    // A row's tokens tie each cell to its column on the other side.
    let row_item = |t: &Table, r: usize, map: &dyn Fn(usize) -> Option<usize>| {
        let mut tokens: Vec<u64> = (0..t.rows[r].len())
            .filter_map(|c| map(c).map(|k| super::fnv(&[&k.to_le_bytes(), normalize(&cell_text(t, r, c)).as_bytes()])))
            .collect();
        tokens.sort_unstable();
        let label = normalize(&cell_text(t, r, 0));
        Item { key: (!label.is_empty()).then_some(label), tokens, shape: 0 }
    };
    let ra_items: Vec<Item> = (0..ta.rows.len()).map(|r| row_item(ta, r, &|c| col_map.get(&c).copied())).collect();
    let rb_items: Vec<Item> = (0..tb.rows.len()).map(|r| row_item(tb, r, &Some)).collect();
    let rows = align::align(&ra_items, &rb_items, Options { positional_equal_gaps: true });

    let mut changes = Vec::new();
    let at = |slide, r: &ShapeRef, row: Option<usize>, column: Option<usize>, text: String| Side {
        row: row.map(|x| x + 1),
        column: column.map(|x| x + 1),
        text: Some(text),
        ..side(slide, r)
    };
    let header = |t: &Table, c: usize| cell_text(t, 0, c);
    let row_text =
        |t: &Table, r: usize| (0..t.rows[r].len()).map(|c| cell_text(t, r, c)).collect::<Vec<_>>().join(" | ");
    let paired_cols: HashSet<usize> = col_map.keys().copied().collect();
    let paired_cols_b: HashSet<usize> = col_map.values().copied().collect();
    for c in (0..ca.len()).filter(|c| !paired_cols.contains(c)) {
        changes.push(Change::new(Kind::Table, Status::OnlyBase, Some(at(ai, ra, None, Some(c), header(ta, c))), None));
    }
    for c in (0..cb.len()).filter(|c| !paired_cols_b.contains(c)) {
        changes.push(Change::new(Kind::Table, Status::OnlyOther, None, Some(at(bi, rb, None, Some(c), header(tb, c)))));
    }
    let row_pairs: Vec<(usize, usize)> = rows.pairs.iter().chain(&rows.moves).copied().collect();
    let paired_rows: HashSet<usize> = row_pairs.iter().map(|p| p.0).collect();
    let paired_rows_b: HashSet<usize> = row_pairs.iter().map(|p| p.1).collect();
    for r in (0..ta.rows.len()).filter(|r| !paired_rows.contains(r)) {
        changes.push(Change::new(
            Kind::Table,
            Status::OnlyBase,
            Some(at(ai, ra, Some(r), None, row_text(ta, r))),
            None,
        ));
    }
    for r in (0..tb.rows.len()).filter(|r| !paired_rows_b.contains(r)) {
        changes.push(Change::new(
            Kind::Table,
            Status::OnlyOther,
            None,
            Some(at(bi, rb, Some(r), None, row_text(tb, r))),
        ));
    }
    let mut cell_pairs: Vec<(usize, usize, usize, usize)> = Vec::new();
    for &(r, s) in &row_pairs {
        for (&c, &d) in &col_map {
            cell_pairs.push((r, c, s, d));
        }
    }
    cell_pairs.sort_unstable();
    for (r, c, s, d) in cell_pairs {
        let (x, y) = (cell_text(ta, r, c), cell_text(tb, s, d));
        if x != y {
            changes.push(changed(
                Kind::Table,
                at(ai, ra, Some(r), Some(c), x),
                at(bi, rb, Some(s), Some(d), y),
                Vec::new(),
            ));
        }
    }
    changes
}

// ---------------------------------------------------------------------------
// Charts

fn values(series: &Series) -> Value {
    json!(series.values)
}

/// How many series carry each name.
fn name_counts(s: &[Series]) -> HashMap<&str, usize> {
    let mut n: HashMap<&str, usize> = HashMap::new();
    for x in s {
        *n.entry(x.name.as_deref().unwrap_or_default()).or_default() += 1;
    }
    n
}

/// The series' name, when no other series on its side shares it.
fn unique<'a>(s: &'a Series, n: &HashMap<&str, usize>) -> Option<&'a str> {
    s.name.as_deref().filter(|x| n.get(x) == Some(&1))
}

/// Series pair by name where it is unique on both sides, the rest by
/// position; each paired series reports its changed categories and values.
fn chart_changes(a: &Chart, (ai, ra): (usize, &ShapeRef), b: &Chart, (bi, rb): (usize, &ShapeRef)) -> Vec<Change> {
    let mut changes = Vec::new();
    let mut props = Vec::new();
    if a.kinds != b.kinds {
        props.push(Property { name: "kinds", base: json!(a.kinds), other: json!(b.kinds) });
    }
    if a.title != b.title {
        props.push(Property { name: "title", base: opt(a.title.as_deref()), other: opt(b.title.as_deref()) });
    }
    if !props.is_empty() {
        changes.push(changed(Kind::Chart, side(ai, ra), side(bi, rb), props));
    }
    let (na, nb) = (name_counts(&a.series), name_counts(&b.series));
    let mut pairs: Vec<(usize, usize)> = Vec::new();
    for (i, s) in a.series.iter().enumerate() {
        if let Some(name) = unique(s, &na)
            && let Some(j) = b.series.iter().position(|t| unique(t, &nb) == Some(name))
        {
            pairs.push((i, j));
        }
    }
    let (used_a, used_b): (HashSet<usize>, HashSet<usize>) =
        (pairs.iter().map(|p| p.0).collect(), pairs.iter().map(|p| p.1).collect());
    let free_a: Vec<usize> = (0..a.series.len()).filter(|i| !used_a.contains(i)).collect();
    let free_b: Vec<usize> = (0..b.series.len()).filter(|j| !used_b.contains(j)).collect();
    pairs.extend(free_a.iter().zip(&free_b).map(|(&i, &j)| (i, j)));
    let (used_a, used_b): (HashSet<usize>, HashSet<usize>) =
        (pairs.iter().map(|p| p.0).collect(), pairs.iter().map(|p| p.1).collect());
    pairs.sort_unstable();
    let named = |slide, r: &ShapeRef, s: &Series| Side { text: s.name.clone(), ..side(slide, r) };
    for (i, s) in a.series.iter().enumerate().filter(|(i, _)| !used_a.contains(i)) {
        changes.push(Change::new(Kind::Chart, Status::OnlyBase, Some(named(ai, ra, s)), None));
        let _ = i;
    }
    for (j, s) in b.series.iter().enumerate().filter(|(j, _)| !used_b.contains(j)) {
        changes.push(Change::new(Kind::Chart, Status::OnlyOther, None, Some(named(bi, rb, s))));
        let _ = j;
    }
    for (i, j) in pairs {
        let (x, y) = (&a.series[i], &b.series[j]);
        let mut props = Vec::new();
        if x.name != y.name {
            props.push(Property { name: "name", base: opt(x.name.as_deref()), other: opt(y.name.as_deref()) });
        }
        if x.categories != y.categories {
            props.push(Property { name: "categories", base: json!(x.categories), other: json!(y.categories) });
        }
        if x.values != y.values {
            props.push(Property { name: "values", base: values(x), other: values(y) });
        }
        if !props.is_empty() {
            changes.push(changed(Kind::Chart, named(ai, ra, x), named(bi, rb, y), props));
        }
    }
    changes
}

// ---------------------------------------------------------------------------
// Z-order and groups

/// Paired shapes outside the longest run that keeps its stacking order
/// moved forward or back; a shape that changed groups names both. Returns
/// the properties by shape pair, for the caller to add to that pair's
/// shape change.
pub(super) fn arrangement_properties(
    ga: &[(Option<String>, Vec<Option<u32>>)],
    gb: &[(Option<String>, Vec<Option<u32>>)],
    pairs: &[(usize, usize)],
) -> Vec<((usize, usize), Vec<Property>)> {
    let mut out = Vec::new();
    let order: Vec<usize> = pairs.iter().map(|p| p.1).collect();
    let in_order: HashSet<usize> = longest_increasing(&order).into_iter().collect();
    for (k, &(i, j)) in pairs.iter().enumerate() {
        let mut props = Vec::new();
        if !in_order.contains(&k) {
            props.push(Property { name: "zOrder", base: json!(i + 1), other: json!(j + 1) });
        }
        // Groups compare by their ids where written, so moving between two
        // groups of one name still shows; the names are what is reported.
        if ga[i] != gb[j] {
            props.push(Property { name: "group", base: opt(ga[i].0.as_deref()), other: opt(gb[j].0.as_deref()) });
        }
        if !props.is_empty() {
            out.push(((i, j), props));
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Comments

fn comment_text(c: &Comment) -> String {
    match &c.author {
        Some(a) => format!("{a}: {}", c.text),
        None => c.text.clone(),
    }
}

/// Comments pair by their text (exactly, then by shared words); a paired
/// comment reports a changed author or replies.
// Two sides (a, b), their indices (i, j), and their comments (x, y).
#[allow(clippy::many_single_char_names)]
pub(super) fn comment_changes(a: &[Comment], ai: usize, b: &[Comment], bi: usize, work: &mut Work) -> Vec<Change> {
    let item = |c: &Comment| {
        let mut tokens: Vec<u64> = words(&c.text).map(|w| hash(&w)).collect();
        tokens.sort_unstable();
        Item { key: None, tokens, shape: 0 }
    };
    let (ia, ib): (Vec<Item>, Vec<Item>) = (a.iter().map(item).collect(), b.iter().map(item).collect());
    let mut pairs: Vec<(usize, usize)> = Vec::new();
    let mut used_b = HashSet::new();
    for (i, x) in a.iter().enumerate() {
        if let Some(j) = (0..b.len()).find(|j| !used_b.contains(j) && b[*j].text == x.text) {
            used_b.insert(j);
            pairs.push((i, j));
        }
    }
    let used_a: HashSet<usize> = pairs.iter().map(|p| p.0).collect();
    let free_a: Vec<usize> = (0..a.len()).filter(|i| !used_a.contains(i)).collect();
    let free_b: Vec<usize> = (0..b.len()).filter(|j| !used_b.contains(j)).collect();
    pairs.extend(best_first(
        &free_a,
        &free_b,
        |i, j| containment(&ia[i], &ib[j]).min(align::similarity(&ia[i], &ib[j]) * 2.0),
        work,
    ));
    pairs.sort_unstable();
    let (used_a, used_b): (HashSet<usize>, HashSet<usize>) =
        (pairs.iter().map(|p| p.0).collect(), pairs.iter().map(|p| p.1).collect());
    let at = |slide, c: &Comment| Side {
        slide,
        shape: None,
        paragraph: None,
        text: Some(comment_text(c)),
        row: None,
        column: None,
    };
    let mut changes = Vec::new();
    for (i, c) in a.iter().enumerate().filter(|(i, _)| !used_a.contains(i)) {
        changes.push(Change::new(Kind::Comment, Status::OnlyBase, Some(at(ai, c)), None));
        let _ = i;
    }
    for (j, c) in b.iter().enumerate().filter(|(j, _)| !used_b.contains(j)) {
        changes.push(Change::new(Kind::Comment, Status::OnlyOther, None, Some(at(bi, c))));
        let _ = j;
    }
    for (i, j) in pairs {
        let (x, y) = (&a[i], &b[j]);
        let mut props = Vec::new();
        if x.author != y.author {
            props.push(Property { name: "author", base: opt(x.author.as_deref()), other: opt(y.author.as_deref()) });
        }
        if x.replies != y.replies {
            props.push(Property { name: "replies", base: json!(x.replies), other: json!(y.replies) });
        }
        if x.text != y.text || !props.is_empty() {
            let mut c = Change::new(Kind::Comment, Status::Changed, Some(at(ai, x)), Some(at(bi, y)));
            c.properties = props;
            changes.push(c);
        }
    }
    changes
}

// ---------------------------------------------------------------------------
// Sections

/// Sections pair by name, the k-th of a name with the k-th; a paired
/// section whose slides are not the same slides (through the slide
/// pairing) lists both. Ids carry the occurrence (`name#k`), so sections
/// sharing a name keep distinct ids.
pub(super) fn section_changes(
    base: &Presentation,
    other: &Presentation,
    slide_pairs: &[(usize, usize)],
) -> Vec<ObjectChange> {
    let position = |deck: &Presentation| -> HashMap<u32, usize> {
        deck.slides.iter().enumerate().map(|(i, s)| (s.id, i)).collect()
    };
    let (pa, pb) = (position(base), position(other));
    let to_other: HashMap<usize, usize> = slide_pairs.iter().copied().collect();
    let slides_of = |ids: &[u32], pos: &HashMap<u32, usize>| -> Vec<usize> {
        ids.iter().filter_map(|id| pos.get(id).copied()).collect()
    };
    let mut changes = Vec::new();
    let occurrences = |deck: &Presentation| -> Vec<String> {
        let mut seen: HashMap<&str, usize> = HashMap::new();
        deck.sections
            .iter()
            .map(|s| {
                let k = seen.entry(s.name.as_str()).or_default();
                *k += 1;
                format!("{}#{k}", s.name)
            })
            .collect()
    };
    let (oa, ob) = (occurrences(base), occurrences(other));
    let named_b: HashMap<&str, usize> = ob.iter().enumerate().map(|(j, n)| (n.as_str(), j)).collect();
    let mut used_b = HashSet::new();
    for (s, key) in base.sections.iter().zip(&oa) {
        let Some(&j) = named_b.get(key.as_str()) else {
            changes.push(ObjectChange {
                id: format!("section:{key}:-"),
                kind: Kind::Section,
                status: Status::OnlyBase,
                base: Some(json!(s.name)),
                other: None,
                properties: Vec::new(),
            });
            continue;
        };
        used_b.insert(j);
        let t = &other.sections[j];
        let mine: Vec<usize> = slides_of(&s.slide_ids, &pa);
        let theirs: Vec<usize> = slides_of(&t.slide_ids, &pb);
        let mapped: Vec<Option<usize>> = mine.iter().map(|i| to_other.get(i).copied()).collect();
        if mapped != theirs.iter().map(|j| Some(*j)).collect::<Vec<_>>() {
            let one_based = |v: &[usize]| json!(v.iter().map(|x| x + 1).collect::<Vec<_>>());
            changes.push(ObjectChange {
                id: format!("section:{key}:{key}"),
                kind: Kind::Section,
                status: Status::Changed,
                base: Some(json!(s.name)),
                other: Some(json!(t.name)),
                properties: vec![Property { name: "slides", base: one_based(&mine), other: one_based(&theirs) }],
            });
        }
    }
    for (j, t) in other.sections.iter().enumerate().filter(|(j, _)| !used_b.contains(j)) {
        changes.push(ObjectChange {
            id: format!("section:-:{}", ob[j]),
            kind: Kind::Section,
            status: Status::OnlyOther,
            base: None,
            other: Some(json!(t.name)),
            properties: Vec::new(),
        });
    }
    changes
}
