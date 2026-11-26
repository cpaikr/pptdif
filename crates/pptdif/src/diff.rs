//! The comparison: slides paired by content, shapes paired within each
//! slide pair, and changes reported where they are.
//!
//! - **Slides** are aligned as a sequence ([`crate::align`]): a slide's
//!   label is its title, its tokens its paragraphs, and its shape its
//!   layout. Slides left unpaired on both sides are then paired by
//!   similarity regardless of order, so a slide that moved and was edited
//!   is still one slide. A pair outside the longest in-order run is moved.
//! - **Shapes** are a set: they pair by placeholder, then by id and name,
//!   then by name, then by content ([`pair_shapes`]).
//! - **Paragraphs** of a paired shape, and of the notes, align as a
//!   sequence, so a new bullet is one change.
//!
//! Every pairing runs in a fixed orientation (by a content hash of the two
//! sides) and is turned round when that is not the order given, so swapping
//! the files mirrors the result. Ties break by index, never by hash order.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write as _;

use serde::Serialize;
use serde_json::Value;

use crate::align::{self, Item, MIN_SIMILARITY, Options, fnv, longest_increasing};
use crate::model::{Paragraph, Presentation, Shape, ShapeKind, Slide};
use crate::textdiff::{Marker, Marks};

mod objects;

/// One vocabulary for `--kind` and the output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    /// A slide added, removed, or moved, or its visibility or layout.
    Slide,
    /// A shape added or removed, or its kind or name.
    Shape,
    /// A paragraph of a shape's text.
    Text,
    /// A paragraph of the speaker notes.
    Notes,
    /// A shape's position, size, rotation, or flip, beyond the tolerance.
    Geometry,
    /// Formatting as written: a run's or paragraph's, or a shape's outline,
    /// fill, and line.
    Format,
    /// A table's rows, columns, or cells.
    Table,
    /// A picture's image or crop.
    Picture,
    /// A chart's kinds, title, or series data.
    Chart,
    /// A slide's comments and their replies.
    Comment,
    /// The deck's sections.
    Section,
}

impl Kind {
    pub const ALL: [Kind; 11] = [
        Kind::Slide,
        Kind::Shape,
        Kind::Text,
        Kind::Notes,
        Kind::Geometry,
        Kind::Format,
        Kind::Table,
        Kind::Picture,
        Kind::Chart,
        Kind::Comment,
        Kind::Section,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Kind::Slide => "slide",
            Kind::Shape => "shape",
            Kind::Text => "text",
            Kind::Notes => "notes",
            Kind::Geometry => "geometry",
            Kind::Format => "format",
            Kind::Table => "table",
            Kind::Picture => "picture",
            Kind::Chart => "chart",
            Kind::Comment => "comment",
            Kind::Section => "section",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Status {
    Same,
    Changed,
    OnlyBase,
    OnlyOther,
    Moved,
}

impl Status {
    const fn mirrored(self) -> Self {
        match self {
            Status::OnlyBase => Status::OnlyOther,
            Status::OnlyOther => Status::OnlyBase,
            s => s,
        }
    }
}

/// Which shape a change is on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ShapeRef {
    /// `cNvPr` id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<u32>,
    pub name: String,
    /// `type` or `type:idx` for a placeholder.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
    /// Position among the slide's shapes (groups' members included,
    /// depth first), 1-based: names and ids may repeat.
    #[serde(skip)]
    pub position: usize,
}

/// One side of a change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Side {
    /// 1-based position of the slide in its deck.
    pub slide: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shape: Option<ShapeRef>,
    /// 1-based paragraph, for text and notes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub paragraph: Option<usize>,
    /// 1-based table row and column, for table changes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub row: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Property {
    pub name: &'static str,
    pub base: Value,
    pub other: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Change {
    /// Stable: built from the change's locations, never from list order.
    pub id: String,
    pub kind: Kind,
    pub status: Status,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base: Option<Side>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub other: Option<Side>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub properties: Vec<Property>,
    /// For a changed paragraph: which parts of each side's text changed, as
    /// `[start, end)` UTF-16 ranges, when the two share enough words.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub marks: Option<Marks>,
}

impl Change {
    fn new(kind: Kind, status: Status, base: Option<Side>, other: Option<Side>) -> Self {
        Self { id: String::new(), kind, status, base, other, properties: Vec::new(), marks: None }
    }

    fn mirrored(mut self) -> Self {
        std::mem::swap(&mut self.base, &mut self.other);
        if let Some(m) = &mut self.marks {
            std::mem::swap(&mut m.base, &mut m.other);
        }
        self.status = self.status.mirrored();
        for p in &mut self.properties {
            std::mem::swap(&mut p.base, &mut p.other);
        }
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SlideRef {
    /// 1-based position in the deck.
    pub index: usize,
    /// `sldId` id.
    pub id: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SlidePair {
    /// `p<n>`: base slides in order, then slides only in the other deck.
    pub id: String,
    pub status: Status,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base: Option<SlideRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub other: Option<SlideRef>,
    pub moved: bool,
    pub changes: Vec<Change>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DeckRef {
    pub file: String,
    pub slides: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AlignMode {
    /// Slides pair by content.
    #[default]
    Content,
    /// Slide n pairs with slide n.
    Position,
}

#[derive(Debug, Clone, Copy)]
pub struct CompareOptions {
    pub align: AlignMode,
    /// Positions and sizes closer than this, in EMU, are equal.
    pub tolerance: i64,
}

/// Half a point: well under a visible nudge, well over unit-conversion
/// noise.
pub const DEFAULT_TOLERANCE: i64 = 6_350;

impl Default for CompareOptions {
    fn default() -> Self {
        Self { align: AlignMode::Content, tolerance: DEFAULT_TOLERANCE }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Comparison {
    pub base: DeckRef,
    pub other: DeckRef,
    pub slides: Vec<SlidePair>,
    /// Changes to the deck itself (sections).
    pub objects: Vec<ObjectChange>,
    /// A slide, shape, or paragraph pairing exceeded its work bounds
    /// somewhere and may have missed the best pairing.
    pub fell_back: bool,
}

/// A change to the deck rather than a slide.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ObjectChange {
    pub id: String,
    pub kind: Kind,
    pub status: Status,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub other: Option<Value>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub properties: Vec<Property>,
}

// ---------------------------------------------------------------------------
// Tokens

fn words(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split_whitespace().map(str::to_lowercase)
}

fn hash(text: &str) -> u64 {
    fnv(&[text.as_bytes()])
}

/// A slide's title: its title or centered-title placeholder's text.
fn title_of(slide: &Slide) -> Option<String> {
    slide
        .all_shapes()
        .find(|s| s.placeholder.as_ref().is_some_and(|p| p.kind == "title" || p.kind == "ctrTitle"))
        .and_then(|s| s.text.as_ref())
        .map(|ps| ps.iter().map(Paragraph::text).collect::<Vec<_>>().join("\n"))
        .filter(|t| !t.trim().is_empty())
}

/// Labels drop whitespace and case, so a title re-spaced or re-cased still
/// pairs.
fn normalize(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).flat_map(char::to_lowercase).collect()
}

/// The shapes compared one by one: every shape except groups, whose
/// members stand for them.
fn leaves(slide: &Slide) -> Vec<&Shape> {
    slide.all_shapes().filter(|s| s.kind != ShapeKind::Group).collect()
}

/// For each of [`leaves`], the names of the groups holding it, outermost
/// first, or `None` at the slide's top level.
fn group_paths(slide: &Slide) -> Vec<Option<String>> {
    fn walk(shapes: &[Shape], path: &mut Vec<String>, out: &mut Vec<Option<String>>) {
        for shape in shapes {
            if shape.kind == ShapeKind::Group {
                path.push(shape.name.clone());
                walk(&shape.children, path, out);
                path.pop();
            } else {
                out.push((!path.is_empty()).then(|| path.join(" / ")));
            }
        }
    }
    let mut out = Vec::new();
    walk(&slide.shapes, &mut Vec::new(), &mut out);
    out
}

/// What a shape holds, as tokens: each paragraph (of its text or its table
/// cells), its image, and its chart's title and series names.
fn content_tokens(shape: &Shape) -> Vec<u64> {
    let mut tokens = Vec::new();
    let mut paragraph = |p: &Paragraph| {
        let text = normalize(&p.compared());
        if !text.is_empty() {
            tokens.push(hash(&text));
        }
    };
    shape.text.iter().flatten().for_each(&mut paragraph);
    for cell in shape.table.iter().flat_map(|t| t.rows.iter().flatten()) {
        cell.text.iter().for_each(&mut paragraph);
    }
    if let Some(h) = shape.image.as_ref().and_then(|i| i.hash) {
        tokens.push(fnv(&[b"image", &h.to_le_bytes()]));
    }
    if let Some(chart) = shape.chart.as_ref().and_then(|c| c.data.as_ref()) {
        for name in chart.title.iter().chain(chart.series.iter().filter_map(|s| s.name.as_ref())) {
            tokens.push(fnv(&[b"chart", normalize(name).as_bytes()]));
        }
    }
    tokens
}

fn slide_item(slide: &Slide) -> Item {
    let mut tokens: Vec<u64> = leaves(slide).into_iter().flat_map(content_tokens).collect();
    tokens.sort_unstable();
    Item { key: title_of(slide).map(|t| normalize(&t)), tokens, shape: slide.layout.as_deref().map_or(0, hash) }
}

fn items_hash(items: &[Item]) -> u64 {
    let mut parts: Vec<Vec<u8>> = Vec::new();
    for item in items {
        parts.push(item.key.clone().unwrap_or_default().into_bytes());
        parts.push(item.tokens.iter().flat_map(|t| t.to_le_bytes()).collect());
        parts.push(item.shape.to_le_bytes().to_vec());
    }
    let refs: Vec<&[u8]> = parts.iter().map(Vec::as_slice).collect();
    fnv(&refs)
}

/// What every pairing in one comparison shares: whether any of them ran
/// out of work and settled for less, and the budget for text marks.
struct Work {
    fell_back: bool,
    marker: Marker,
}

/// The most item pairs an order-free pairing scores in full. Beyond it,
/// each item is scored only against items near its own position, and the
/// comparison says it fell back.
const MAX_PAIRING_WORK: usize = 4_000_000;

// ---------------------------------------------------------------------------
// Slide pairing

/// Paired slides `(base, other)`, in base order, and which of them moved.
struct SlideMatch {
    pairs: Vec<(usize, usize)>,
    moved: HashSet<(usize, usize)>,
    fell_back: bool,
}

fn match_slides(base: &[Item], other: &[Item], mode: AlignMode, work: &mut Work) -> SlideMatch {
    if mode == AlignMode::Position {
        let a = align::Alignment::positional(base.len(), other.len());
        return SlideMatch { pairs: a.pairs, moved: HashSet::new(), fell_back: false };
    }
    // A fixed orientation, so that swapping the decks mirrors the pairing.
    let flip = items_hash(base) > items_hash(other);
    let (a, b) = if flip { (other, base) } else { (base, other) };
    let alignment = align::align(a, b, Options { positional_equal_gaps: false });
    let mut pairs: Vec<(usize, usize)> = alignment.pairs.iter().chain(&alignment.moves).copied().collect();

    // Slides left on both sides pair by similarity, best first, wherever
    // they sit: a slide moved and edited is still one slide.
    let used_a: HashSet<usize> = pairs.iter().map(|p| p.0).collect();
    let used_b: HashSet<usize> = pairs.iter().map(|p| p.1).collect();
    let free_a: Vec<usize> = (0..a.len()).filter(|i| !used_a.contains(i)).collect();
    let free_b: Vec<usize> = (0..b.len()).filter(|j| !used_b.contains(j)).collect();
    pairs.extend(best_first(&free_a, &free_b, |i, j| align::similarity(&a[i], &b[j]), work));

    // Slides with no title and no content (blank, or holding only what is
    // not compared) say nothing to pair on but their layout: those left on
    // both sides pair in order, layout for layout.
    let used_a: HashSet<usize> = pairs.iter().map(|p| p.0).collect();
    let used_b: HashSet<usize> = pairs.iter().map(|p| p.1).collect();
    let empty = |it: &Item| it.key.is_none() && it.tokens.is_empty();
    let blank_b: Vec<usize> = (0..b.len()).filter(|j| !used_b.contains(j) && empty(&b[*j])).collect();
    let mut next = 0;
    for i in (0..a.len()).filter(|i| !used_a.contains(i) && empty(&a[*i])) {
        if let Some(k) = blank_b[next..].iter().position(|&j| b[j].shape == a[i].shape) {
            pairs.push((i, blank_b[next + k]));
            next += k + 1;
        }
    }

    // Pairs outside the longest run in order moved. Which of two crossing
    // pairs that is depends on the orientation, so it is settled before the
    // pairs are turned round.
    pairs.sort_unstable();
    let order: Vec<usize> = pairs.iter().map(|p| p.1).collect();
    let in_order: HashSet<usize> = longest_increasing(&order).into_iter().collect();
    let mut moved: HashSet<(usize, usize)> =
        pairs.iter().enumerate().filter(|(k, _)| !in_order.contains(k)).map(|(_, p)| *p).collect();
    if flip {
        for p in &mut pairs {
            *p = (p.1, p.0);
        }
        moved = moved.into_iter().map(|(i, j)| (j, i)).collect();
        pairs.sort_unstable();
    }
    SlideMatch { pairs, moved, fell_back: alignment.fell_back }
}

/// Pairs items greedily by similarity (at least [`MIN_SIMILARITY`]), the
/// most similar first, ties by index. Within [`MAX_PAIRING_WORK`] every
/// pair is scored; beyond it each item only against those near its own
/// position in the list, and `work` records the fallback.
fn best_first(a: &[usize], b: &[usize], sim: impl Fn(usize, usize) -> f64, work: &mut Work) -> Vec<(usize, usize)> {
    let band = if a.len().saturating_mul(b.len()) > MAX_PAIRING_WORK {
        work.fell_back = true;
        MAX_PAIRING_WORK / a.len().max(b.len()).max(1) / 2
    } else {
        usize::MAX
    };
    let mut candidates: Vec<(f64, usize, usize)> = Vec::new();
    for (x, &i) in a.iter().enumerate() {
        // Positions scale, so lists of different lengths keep their diagonal.
        let centre = x * b.len() / a.len().max(1);
        let (lo, hi) = (centre.saturating_sub(band), centre.saturating_add(band).min(b.len().saturating_sub(1)));
        for &j in b.get(lo..=hi).unwrap_or_default() {
            let s = sim(i, j);
            if s >= MIN_SIMILARITY {
                candidates.push((s, i, j));
            }
        }
    }
    candidates.sort_by(|x, y| y.0.total_cmp(&x.0).then(x.1.cmp(&y.1)).then(x.2.cmp(&y.2)));
    let (mut used_a, mut used_b) = (HashSet::new(), HashSet::new());
    let mut out = Vec::new();
    for (_, i, j) in candidates {
        if !used_a.contains(&i) && !used_b.contains(&j) {
            used_a.insert(i);
            used_b.insert(j);
            out.push((i, j));
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Shape pairing

fn placeholder_key(shape: &Shape) -> Option<String> {
    shape.placeholder.as_ref().map(|p| format!("{}:{}", p.kind, p.index))
}

fn shape_ref(shape: &Shape, position: usize) -> ShapeRef {
    ShapeRef {
        id: shape.id,
        name: shape.name.clone(),
        placeholder: shape
            .placeholder
            .as_ref()
            .map(|p| if p.index == 0 { p.kind.clone() } else { format!("{}:{}", p.kind, p.index) }),
        position,
    }
}

fn shape_item(shape: &Shape) -> Item {
    let mut tokens: Vec<u64> = vec![fnv(&[b"kind", &[shape.kind as u8]])];
    for p in shape.text.iter().flatten() {
        tokens.extend(words(&p.compared()).map(|w| hash(&w)));
    }
    // Tables, pictures, and charts weigh in by what they hold.
    if shape.text.is_none() {
        tokens.extend(content_tokens(shape));
    }
    tokens.sort_unstable();
    Item { key: None, tokens, shape: 0 }
}

/// Pairs shapes in steps, each over what is still unpaired, and each only
/// where the key is unique on both sides: placeholder type and index; id
/// and name; name; then content similarity among shapes of one kind.
fn pair_shapes(a: &[&Shape], b: &[&Shape], work: &mut Work) -> Vec<(usize, usize)> {
    let mut pairs: Vec<(usize, usize)> = Vec::new();
    let (mut used_a, mut used_b) = (HashSet::new(), HashSet::new());
    let by_key = |key: &dyn Fn(&Shape) -> Option<String>,
                  pairs: &mut Vec<(usize, usize)>,
                  used_a: &mut HashSet<usize>,
                  used_b: &mut HashSet<usize>| {
        let index = |side: &[&Shape], used: &HashSet<usize>| {
            let mut map: HashMap<String, Vec<usize>> = HashMap::new();
            for (i, s) in side.iter().enumerate() {
                if !used.contains(&i)
                    && let Some(k) = key(s)
                {
                    map.entry(k).or_default().push(i);
                }
            }
            map
        };
        let (ma, mb) = (index(a, used_a), index(b, used_b));
        let mut found: Vec<(usize, usize)> = ma
            .iter()
            .filter_map(|(k, ia)| match (ia.as_slice(), mb.get(k).map(Vec::as_slice)) {
                ([i], Some([j])) => Some((*i, *j)),
                _ => None,
            })
            .collect();
        found.sort_unstable();
        for (i, j) in found {
            used_a.insert(i);
            used_b.insert(j);
            pairs.push((i, j));
        }
    };
    by_key(&placeholder_key, &mut pairs, &mut used_a, &mut used_b);
    by_key(&|s| s.id.map(|id| format!("{id}\u{0}{}", s.name)), &mut pairs, &mut used_a, &mut used_b);
    by_key(
        &|s| (!s.name.is_empty()).then(|| format!("{:?}\u{0}{}", s.kind, s.name)),
        &mut pairs,
        &mut used_a,
        &mut used_b,
    );

    let free_a: Vec<usize> = (0..a.len()).filter(|i| !used_a.contains(i)).collect();
    let free_b: Vec<usize> = (0..b.len()).filter(|j| !used_b.contains(j)).collect();
    let items_a: Vec<Item> = a.iter().map(|s| shape_item(s)).collect();
    let items_b: Vec<Item> = b.iter().map(|s| shape_item(s)).collect();
    // A shape with nothing in it (only its kind token) has nothing to be
    // recognised by once its identity failed to match: it stays unpaired.
    let telling = |it: &Item| it.tokens.len() > 1;
    pairs.extend(best_first(
        &free_a,
        &free_b,
        |i, j| {
            let (x, y) = (&items_a[i], &items_b[j]);
            if a[i].kind == b[j].kind && telling(x) && telling(y) { align::similarity(x, y) } else { 0.0 }
        },
        work,
    ));
    pairs.sort_unstable();
    pairs
}

// ---------------------------------------------------------------------------
// Paragraphs

/// What two paragraph lists differ by: `(base, other)` 0-based indices,
/// either missing for a paragraph on one side only, with the status:
/// `Same` for a pair whose text did not change.
/// Empty paragraphs are left out: a blank line added or removed is not
/// reported.
fn diff_paragraphs(a: &[Paragraph], b: &[Paragraph], work: &mut Work) -> Vec<(Status, Option<usize>, Option<usize>)> {
    let keep = |ps: &[Paragraph]| -> Vec<usize> {
        ps.iter().enumerate().filter(|(_, p)| !p.compared().trim().is_empty()).map(|(i, _)| i).collect()
    };
    let (ka, kb) = (keep(a), keep(b));
    // The whole text is the label, so an unchanged paragraph anchors as
    // itself; the words let an edited one still pair.
    let item = |p: &Paragraph| {
        let text = p.compared();
        let mut tokens: Vec<u64> = words(&text).map(|w| hash(&w)).collect();
        tokens.sort_unstable();
        Item { key: Some(normalize(&text)), tokens, shape: 0 }
    };
    let ia: Vec<Item> = ka.iter().map(|&i| item(&a[i])).collect();
    let ib: Vec<Item> = kb.iter().map(|&j| item(&b[j])).collect();
    let alignment = align::align(&ia, &ib, Options { positional_equal_gaps: true });
    let mut out = Vec::new();
    let (mut used_a, mut used_b) = (HashSet::new(), HashSet::new());
    for &(i, j) in &alignment.pairs {
        used_a.insert(i);
        used_b.insert(j);
        let status = if a[ka[i]].compared() == b[kb[j]].compared() { Status::Same } else { Status::Changed };
        out.push((status, Some(ka[i]), Some(kb[j])));
    }
    for &(i, j) in &alignment.moves {
        used_a.insert(i);
        used_b.insert(j);
        out.push((Status::Moved, Some(ka[i]), Some(kb[j])));
    }
    // A paragraph that grew or shrank shares too few words to align, but
    // most of the shorter one's words are in the longer: within each gap
    // between aligned paragraphs, those pair as one edit, without crossing.
    let free_a: Vec<usize> = (0..ka.len()).filter(|i| !used_a.contains(i)).collect();
    let free_b: Vec<usize> = (0..kb.len()).filter(|j| !used_b.contains(j)).collect();
    for (i, j) in ordered_pairs(&free_a, &free_b, &alignment.pairs, |i, j| containment(&ia[i], &ib[j]), work) {
        used_a.insert(i);
        used_b.insert(j);
        out.push((Status::Changed, Some(ka[i]), Some(kb[j])));
    }
    out.extend((0..ka.len()).filter(|i| !used_a.contains(i)).map(|i| (Status::OnlyBase, Some(ka[i]), None)));
    out.extend((0..kb.len()).filter(|j| !used_b.contains(j)).map(|j| (Status::OnlyOther, None, Some(kb[j]))));
    out.sort_by_key(|(_, i, j)| (i.unwrap_or(usize::MAX), j.unwrap_or(usize::MAX)));
    out
}

/// Pairs left-over items within each gap between aligned `pairs`, never
/// crossing, for the greatest total score (at least [`MIN_SIMILARITY`]
/// each): a weighted longest common subsequence per gap. A gap beyond
/// [`MAX_PAIRING_WORK`] is left unpaired and `work` records the fallback.
// Dynamic-programming indices (n, m, p, q, s) read best short here.
#[allow(clippy::many_single_char_names)]
fn ordered_pairs(
    free_a: &[usize],
    free_b: &[usize],
    pairs: &[(usize, usize)],
    score: impl Fn(usize, usize) -> f64,
    work: &mut Work,
) -> Vec<(usize, usize)> {
    // An item's gap is how many aligned pairs come before it.
    let gap_a = |i: usize| pairs.iter().filter(|p| p.0 < i).count();
    let gap_b = |j: usize| pairs.iter().filter(|p| p.1 < j).count();
    let mut gaps: HashMap<usize, (Vec<usize>, Vec<usize>)> = HashMap::new();
    for &i in free_a {
        gaps.entry(gap_a(i)).or_default().0.push(i);
    }
    for &j in free_b {
        gaps.entry(gap_b(j)).or_default().1.push(j);
    }
    let mut keys: Vec<usize> = gaps.keys().copied().collect();
    keys.sort_unstable();
    let mut out = Vec::new();
    for key in keys {
        let (xs, ys) = &gaps[&key];
        if xs.is_empty() || ys.is_empty() {
            continue;
        }
        if xs.len().saturating_mul(ys.len()) > MAX_PAIRING_WORK {
            work.fell_back = true;
            continue;
        }
        let (n, m) = (xs.len(), ys.len());
        let mut best = vec![vec![0.0f64; m + 1]; n + 1];
        for p in (0..n).rev() {
            for q in (0..m).rev() {
                let s = score(xs[p], ys[q]);
                let take = if s >= MIN_SIMILARITY { s + best[p + 1][q + 1] } else { f64::NEG_INFINITY };
                best[p][q] = take.max(best[p + 1][q]).max(best[p][q + 1]);
            }
        }
        let (mut p, mut q) = (0, 0);
        while p < n && q < m {
            let s = score(xs[p], ys[q]);
            if s >= MIN_SIMILARITY && (s + best[p + 1][q + 1] - best[p][q]).abs() < 1e-12 {
                out.push((xs[p], ys[q]));
                (p, q) = (p + 1, q + 1);
            } else if (best[p + 1][q] - best[p][q]).abs() < 1e-12 {
                p += 1;
            } else {
                q += 1;
            }
        }
    }
    out
}

/// The share of the smaller item's tokens found in the larger.
fn containment(a: &Item, b: &Item) -> f64 {
    let small = a.tokens.len().min(b.tokens.len());
    if small == 0 {
        return 0.0;
    }
    let (mut i, mut j, mut shared) = (0, 0, 0usize);
    while i < a.tokens.len() && j < b.tokens.len() {
        match a.tokens[i].cmp(&b.tokens[j]) {
            std::cmp::Ordering::Less => i += 1,
            std::cmp::Ordering::Greater => j += 1,
            std::cmp::Ordering::Equal => {
                shared += 1;
                i += 1;
                j += 1;
            }
        }
    }
    shared as f64 / small as f64
}

fn paragraph_changes(
    kind: Kind,
    a: (&[Paragraph], usize, Option<&ShapeRef>),
    b: (&[Paragraph], usize, Option<&ShapeRef>),
    work: &mut Work,
) -> Vec<Change> {
    let side = |(ps, slide, shape): (&[Paragraph], usize, Option<&ShapeRef>), i: usize| Side {
        slide,
        shape: shape.cloned(),
        paragraph: Some(i + 1),
        text: Some(ps[i].text()),
        row: None,
        column: None,
    };
    let mut changes = Vec::new();
    for (status, i, j) in diff_paragraphs(a.0, b.0, work) {
        if status == Status::Same {
            continue;
        }
        let mut c = Change::new(kind, status, i.map(|i| side(a, i)), j.map(|j| side(b, j)));
        if let (Status::Changed, Some(x), Some(y)) = (status, &c.base, &c.other) {
            c.marks = work.marker.marks(x.text.as_deref().unwrap_or_default(), y.text.as_deref().unwrap_or_default());
        }
        changes.push(c);
    }
    changes
}

// ---------------------------------------------------------------------------
// One slide pair

fn shape_text(shape: &Shape) -> Option<String> {
    let text: Vec<String> = shape.text.iter().flatten().map(Paragraph::text).collect();
    (!text.is_empty()).then(|| text.join("\n")).filter(|t| !t.trim().is_empty())
}

/// Everything a slide comparison reads, in order, as a hash: two slides
/// with equal fingerprints compare as equal whichever is the base.
fn fingerprint(slide: &Slide) -> u64 {
    let described = format!("{:?}{:?}{:?}{:?}", slide.hidden, slide.layout, slide.shapes, slide.notes);
    hash(&described)
}

fn compare_slide(a: &Slide, ai: usize, b: &Slide, bi: usize, tolerance: i64, work: &mut Work) -> Vec<Change> {
    // A fixed orientation, as for the slides.
    if fingerprint(a) > fingerprint(b) {
        return compare_slide_oriented(b, bi, a, ai, tolerance, work).into_iter().map(Change::mirrored).collect();
    }
    compare_slide_oriented(a, ai, b, bi, tolerance, work)
}

fn compare_slide_oriented(a: &Slide, ai: usize, b: &Slide, bi: usize, tolerance: i64, work: &mut Work) -> Vec<Change> {
    let mut changes = Vec::new();
    let slide_side = |slide: usize| Side { slide, shape: None, paragraph: None, text: None, row: None, column: None };

    let mut props = Vec::new();
    if a.hidden != b.hidden {
        props.push(Property { name: "hidden", base: Value::Bool(a.hidden), other: Value::Bool(b.hidden) });
    }
    if a.layout != b.layout {
        let v = |l: &Option<std::sync::Arc<str>>| l.as_deref().map_or(Value::Null, Value::from);
        props.push(Property { name: "layout", base: v(&a.layout), other: v(&b.layout) });
    }
    if !props.is_empty() {
        let mut c = Change::new(Kind::Slide, Status::Changed, Some(slide_side(ai)), Some(slide_side(bi)));
        c.properties = props;
        changes.push(c);
    }

    let (la, lb) = (leaves(a), leaves(b));
    let pairs = pair_shapes(&la, &lb, work);
    let (pa, pb): (HashSet<usize>, HashSet<usize>) =
        (pairs.iter().map(|p| p.0).collect(), pairs.iter().map(|p| p.1).collect());
    let shape_side = |slide: usize, shape: &Shape, k: usize| Side {
        slide,
        shape: Some(shape_ref(shape, k + 1)),
        paragraph: None,
        text: shape_text(shape),
        row: None,
        column: None,
    };
    for (i, s) in la.iter().enumerate().filter(|(i, _)| !pa.contains(i)) {
        changes.push(Change::new(Kind::Shape, Status::OnlyBase, Some(shape_side(ai, s, i)), None));
    }
    for (j, s) in lb.iter().enumerate().filter(|(j, _)| !pb.contains(j)) {
        changes.push(Change::new(Kind::Shape, Status::OnlyOther, None, Some(shape_side(bi, s, j))));
    }
    for &(i, j) in &pairs {
        let (sa, sb) = (la[i], lb[j]);
        let (ra, rb) = (shape_ref(sa, i + 1), shape_ref(sb, j + 1));
        let mut props = Vec::new();
        if sa.kind != sb.kind {
            props.push(Property {
                name: "kind",
                base: serde_json::to_value(sa.kind).unwrap_or(Value::Null),
                other: serde_json::to_value(sb.kind).unwrap_or(Value::Null),
            });
        }
        if sa.hidden != sb.hidden {
            props.push(Property { name: "hidden", base: Value::Bool(sa.hidden), other: Value::Bool(sb.hidden) });
        }
        if sa.name != sb.name {
            props.push(Property {
                name: "name",
                base: Value::from(sa.name.as_str()),
                other: Value::from(sb.name.as_str()),
            });
        }
        if !props.is_empty() {
            let side = |slide, r: &ShapeRef| Side {
                slide,
                shape: Some(r.clone()),
                paragraph: None,
                text: None,
                row: None,
                column: None,
            };
            let mut c = Change::new(Kind::Shape, Status::Changed, Some(side(ai, &ra)), Some(side(bi, &rb)));
            c.properties = props;
            changes.push(c);
        }
        let (ta, tb) = (sa.text.as_deref().unwrap_or_default(), sb.text.as_deref().unwrap_or_default());
        changes.extend(paragraph_changes(Kind::Text, (ta, ai, Some(&ra)), (tb, bi, Some(&rb)), work));
        changes.extend(objects::shape_changes(sa, (ai, &ra), sb, (bi, &rb), tolerance, work));
    }
    changes.extend(objects::arrangement_changes(&la, &group_paths(a), ai, &lb, &group_paths(b), bi, &pairs));

    let (na, nb) = (a.notes.as_deref().unwrap_or_default(), b.notes.as_deref().unwrap_or_default());
    changes.extend(paragraph_changes(Kind::Notes, (na, ai, None), (nb, bi, None), work));
    changes.extend(objects::comment_changes(&a.comments, ai, &b.comments, bi, work));
    changes
}

// ---------------------------------------------------------------------------
// The whole comparison

fn slide_ref(slide: &Slide, index: usize) -> SlideRef {
    SlideRef { index, id: slide.id, title: title_of(slide) }
}

fn locator(side: Option<&Side>) -> String {
    let Some(side) = side else { return "-".to_owned() };
    let mut out = String::new();
    if let Some(shape) = &side.shape {
        match shape.id {
            Some(id) => {
                let _ = write!(out, "s{id}");
            }
            None => {
                let _ = write!(out, "n{}", shape.position);
            }
        }
    } else {
        out.push_str("slide");
    }
    if let Some(p) = side.paragraph {
        let _ = write!(out, ".{p}");
    }
    match (side.row, side.column) {
        (Some(r), Some(c)) => {
            let _ = write!(out, ".r{r}c{c}");
        }
        (Some(r), None) => {
            let _ = write!(out, ".r{r}");
        }
        (None, Some(c)) => {
            let _ = write!(out, ".c{c}");
        }
        (None, None) => {}
    }
    out
}

/// Gives every change an id from its locations, unique within the
/// comparison: a shape id repeated on one slide gets a `~n` suffix.
fn assign_ids(pair: &mut SlidePair) {
    let mut seen: HashMap<String, usize> = HashMap::new();
    for c in &mut pair.changes {
        let id = format!("{}:{}:{}:{}", pair.id, c.kind.as_str(), locator(c.base.as_ref()), locator(c.other.as_ref()));
        let n = seen.entry(id.clone()).or_insert(0);
        *n += 1;
        c.id = if *n == 1 { id } else { format!("{id}~{n}") };
    }
}

pub fn compare(
    base_file: &str,
    base: &Presentation,
    other_file: &str,
    other: &Presentation,
    options: CompareOptions,
) -> Comparison {
    let ia: Vec<Item> = base.slides.iter().map(slide_item).collect();
    let ib: Vec<Item> = other.slides.iter().map(slide_item).collect();
    let mut work = Work { fell_back: false, marker: Marker::new() };
    let matched = match_slides(&ia, &ib, options.align, &mut work);
    let by_base: HashMap<usize, usize> = matched.pairs.iter().copied().collect();
    let paired_other: HashSet<usize> = matched.pairs.iter().map(|p| p.1).collect();

    let mut slides = Vec::new();
    for (i, a) in base.slides.iter().enumerate() {
        let n = slides.len() + 1;
        let (status, other_ref, moved, changes) = if let Some(&j) = by_base.get(&i) {
            {
                let b = &other.slides[j];
                let moved = matched.moved.contains(&(i, j));
                let changes = compare_slide(a, i + 1, b, j + 1, options.tolerance, &mut work);
                let status = if moved {
                    Status::Moved
                } else if changes.is_empty() {
                    Status::Same
                } else {
                    Status::Changed
                };
                (status, Some(slide_ref(b, j + 1)), moved, changes)
            }
        } else {
            {
                let side =
                    Side { slide: i + 1, shape: None, paragraph: None, text: title_of(a), row: None, column: None };
                (Status::OnlyBase, None, false, vec![Change::new(Kind::Slide, Status::OnlyBase, Some(side), None)])
            }
        };
        let mut changes = changes;
        if moved {
            let side = |slide| Side { slide, shape: None, paragraph: None, text: None, row: None, column: None };
            let j = by_base[&i];
            changes.insert(0, Change::new(Kind::Slide, Status::Moved, Some(side(i + 1)), Some(side(j + 1))));
        }
        slides.push(SlidePair {
            id: format!("p{n}"),
            status,
            base: Some(slide_ref(a, i + 1)),
            other: other_ref,
            moved,
            changes,
        });
    }
    for (j, b) in other.slides.iter().enumerate().filter(|(j, _)| !paired_other.contains(j)) {
        let n = slides.len() + 1;
        let side = Side { slide: j + 1, shape: None, paragraph: None, text: title_of(b), row: None, column: None };
        slides.push(SlidePair {
            id: format!("p{n}"),
            status: Status::OnlyOther,
            base: None,
            other: Some(slide_ref(b, j + 1)),
            moved: false,
            changes: vec![Change::new(Kind::Slide, Status::OnlyOther, None, Some(side))],
        });
    }
    for pair in &mut slides {
        assign_ids(pair);
    }
    let objects = objects::section_changes(base, other, &matched.pairs);
    Comparison {
        base: DeckRef { file: base_file.to_owned(), slides: base.slides.len() },
        other: DeckRef { file: other_file.to_owned(), slides: other.slides.len() },
        slides,
        objects,
        fell_back: matched.fell_back || work.fell_back,
    }
}

// ---------------------------------------------------------------------------
// The listing: filters, limits, and the summary

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub slides_changed: usize,
    pub slides_only_base: usize,
    pub slides_only_other: usize,
    pub slides_moved: usize,
    /// Changes by kind, every kind listed.
    pub changes: BTreeMap<&'static str, usize>,
}

#[derive(Debug, Clone, Default)]
pub struct Filter {
    /// Only slide pairs with this 1-based slide on either side.
    pub slide: Option<usize>,
    /// Only these kinds; empty means all.
    pub kinds: Vec<Kind>,
    /// The most changes to list; 0 lists every change.
    pub limit: usize,
    /// Also list slide pairs with no changes.
    pub all: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Listing {
    pub base: DeckRef,
    pub other: DeckRef,
    pub summary: Summary,
    /// Changes listed after filters and the limit.
    pub listed: usize,
    /// Changes the limit cut, by slide pair and kind.
    pub omitted: BTreeMap<String, BTreeMap<&'static str, usize>>,
    pub slides: Vec<SlidePair>,
    /// Deck-level changes; `--slide` leaves them out.
    pub objects: Vec<ObjectChange>,
}

pub fn summarize(comparison: &Comparison) -> Summary {
    let mut summary = Summary { changes: Kind::ALL.iter().map(|k| (k.as_str(), 0)).collect(), ..Summary::default() };
    for pair in &comparison.slides {
        match pair.status {
            Status::Changed => summary.slides_changed += 1,
            Status::OnlyBase => summary.slides_only_base += 1,
            Status::OnlyOther => summary.slides_only_other += 1,
            Status::Moved => {
                summary.slides_moved += 1;
                // A moved slide may be edited as well.
                if pair.changes.iter().any(|c| c.status != Status::Moved || c.kind != Kind::Slide) {
                    summary.slides_changed += 1;
                }
            }
            Status::Same => {}
        }
        for c in &pair.changes {
            *summary.changes.entry(c.kind.as_str()).or_default() += 1;
        }
    }
    for o in &comparison.objects {
        *summary.changes.entry(o.kind.as_str()).or_default() += 1;
    }
    summary
}

pub fn listing(comparison: &Comparison, filter: &Filter) -> Listing {
    let mut listed = 0;
    let mut omitted: BTreeMap<String, BTreeMap<&'static str, usize>> = BTreeMap::new();
    let mut slides = Vec::new();
    for pair in &comparison.slides {
        let on = |r: &Option<SlideRef>| r.as_ref().map(|r| r.index);
        if let Some(n) = filter.slide
            && on(&pair.base) != Some(n)
            && on(&pair.other) != Some(n)
        {
            continue;
        }
        let wanted: Vec<&Change> =
            pair.changes.iter().filter(|c| filter.kinds.is_empty() || filter.kinds.contains(&c.kind)).collect();
        // `--all` lists slides with no changes at all, whatever the kinds.
        if wanted.is_empty() && !(filter.all && pair.changes.is_empty()) {
            continue;
        }
        let mut kept = Vec::new();
        for c in wanted {
            if filter.limit == 0 || listed < filter.limit {
                listed += 1;
                kept.push(c.clone());
            } else {
                *omitted.entry(pair.id.clone()).or_default().entry(c.kind.as_str()).or_default() += 1;
            }
        }
        slides.push(SlidePair {
            id: pair.id.clone(),
            status: pair.status,
            base: pair.base.clone(),
            other: pair.other.clone(),
            moved: pair.moved,
            changes: kept,
        });
    }
    let objects: Vec<ObjectChange> = if filter.slide.is_some() {
        Vec::new()
    } else {
        comparison
            .objects
            .iter()
            .filter(|o| filter.kinds.is_empty() || filter.kinds.contains(&o.kind))
            .cloned()
            .collect()
    };
    Listing {
        base: comparison.base.clone(),
        other: comparison.other.clone(),
        summary: summarize(comparison),
        listed: listed + objects.len(),
        omitted,
        slides,
        objects,
    }
}
