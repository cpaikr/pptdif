//! The self-contained HTML report: each slide pair side by side, shapes
//! drawn where they sit and outlined by how they changed, with the changes
//! listed under each pair and the deck-level changes at the end.
//!
//! The report draws what the comparison paired (`Comparison::slides`); it
//! never pairs anything again, so it cannot disagree with the JSON. Slides
//! are schematic (shape outlines with their text) unless the caller hands
//! in slide images, which PowerPoint renders; outlines are then drawn over
//! them. Everything is inline: no external URL is fetched.

use std::collections::HashMap;
use std::fmt::Write as _;

use crate::diff::{Change, Comparison, ObjectChange, SlidePair, Status};
use crate::model::{Geometry, Presentation, Shape, ShapeKind, Size, Slide};
use crate::textdiff::Marks;

/// Slide images by 1-based slide position, as `data:` URIs.
#[derive(Debug, Default, Clone)]
pub struct Images {
    pub base: HashMap<usize, String>,
    pub other: HashMap<usize, String>,
}

/// What the report shows besides the comparison.
#[derive(Debug, Default, Clone)]
pub struct Report<'a> {
    /// Warnings to show under the header, as `(code, message)`.
    pub warnings: &'a [(String, String)],
    pub images: Option<&'a Images>,
}

/// Text longer than this is cut in the schematic (the details keep all).
const SHAPE_TEXT: usize = 160;

fn esc(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            // Field markers (NUL) and other controls never reach the page.
            c if c.is_control() && c != '\n' && c != '\t' => {}
            c => out.push(c),
        }
    }
    out
}

/// `text` with the marked UTF-16 ranges wrapped in `<mark>`.
fn marked(text: &str, ranges: &[(u32, u32)]) -> String {
    let mut out = String::new();
    let mut unit = 0u32;
    let mut open = false;
    for c in text.chars() {
        let inside = ranges.iter().any(|&(s, e)| unit >= s && unit < e);
        if inside && !open {
            out.push_str("<mark>");
            open = true;
        } else if !inside && open {
            out.push_str("</mark>");
            open = false;
        }
        out.push_str(&esc(&c.to_string()));
        unit += c.len_utf16() as u32;
    }
    if open {
        out.push_str("</mark>");
    }
    out
}

const fn status_word(status: Status) -> &'static str {
    match status {
        Status::Same => "same",
        Status::Changed => "changed",
        Status::OnlyBase => "removed",
        Status::OnlyOther => "added",
        Status::Moved => "moved",
    }
}

fn shape_text(shape: &Shape) -> String {
    let mut text: Vec<String> = shape.text.iter().flatten().map(crate::model::Paragraph::text).collect();
    if let Some(table) = &shape.table {
        for row in &table.rows {
            let cells: Vec<String> = row
                .iter()
                .map(|c| c.text.iter().map(crate::model::Paragraph::text).collect::<Vec<_>>().join(" "))
                .collect();
            text.push(cells.join(" | "));
        }
    }
    let text = text.join("\n");
    if text.chars().count() > SHAPE_TEXT {
        format!("{}…", text.chars().take(SHAPE_TEXT).collect::<String>())
    } else {
        text
    }
}

/// A shape as drawn: its rectangle on the slide (EMU) and rotation, once
/// every group around it has mapped its child frame onto the slide.
#[derive(Clone, Copy)]
struct Placed {
    x: f64,
    y: f64,
    cx: f64,
    cy: f64,
    degrees: f64,
}

/// Maps a group's child coordinates onto the slide.
#[derive(Clone, Copy)]
struct Frame {
    ox: f64,
    oy: f64,
    sx: f64,
    sy: f64,
    chx: f64,
    chy: f64,
}

const IDENTITY: Frame = Frame { ox: 0.0, oy: 0.0, sx: 1.0, sy: 1.0, chx: 0.0, chy: 0.0 };

fn place(g: &Geometry, f: Frame) -> Placed {
    Placed {
        x: f.ox + (g.x as f64 - f.chx) * f.sx,
        y: f.oy + (g.y as f64 - f.chy) * f.sy,
        cx: g.cx as f64 * f.sx,
        cy: g.cy as f64 * f.sy,
        degrees: g.rotation as f64 / 60_000.0,
    }
}

/// Every shape in drawing order (depth first, groups before their
/// members), each with where it lands; members are counted as the
/// comparison counts leaves.
fn placed(slide: &Slide) -> Vec<(&Shape, Option<Placed>)> {
    fn walk<'a>(shapes: &'a [Shape], frame: Frame, out: &mut Vec<(&'a Shape, Option<Placed>)>) {
        for shape in shapes {
            let at = shape.effective_geometry.or(shape.geometry).map(|g| place(&g, frame));
            out.push((shape, at));
            if shape.kind == ShapeKind::Group {
                let inner = match (shape.effective_geometry.or(shape.geometry), at) {
                    (Some(g), Some(p)) => {
                        let (chx, chy, chcx, chcy) = g.child.unwrap_or((g.x, g.y, g.cx, g.cy));
                        let scale = |to: f64, from: i64| if from > 0 { to / from as f64 } else { 1.0 };
                        Frame {
                            ox: p.x,
                            oy: p.y,
                            sx: scale(p.cx, chcx),
                            sy: scale(p.cy, chcy),
                            chx: chx as f64,
                            chy: chy as f64,
                        }
                    }
                    _ => frame,
                };
                walk(&shape.children, inner, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(&slide.shapes, IDENTITY, &mut out);
    out
}

/// One side's slide: shapes placed by their effective geometry through
/// their groups, outlined by the status of the change on them.
fn slide_box(
    out: &mut String,
    slide: Option<(&Slide, usize)>,
    size: Option<Size>,
    marks: &ShapeMarks,
    image: Option<&String>,
) {
    let size = size.unwrap_or(Size { cx: 12_192_000, cy: 6_858_000 });
    let ratio = format!("{} / {}", size.cx.max(1), size.cy.max(1));
    let Some((slide, _)) = slide else {
        let _ = write!(out, r#"<div class="slide none" style="aspect-ratio:{ratio}"><span>no slide</span></div>"#);
        return;
    };
    let class = match (slide.hidden, image.is_some()) {
        (true, true) => "slide hidden imaged",
        (true, false) => "slide hidden",
        (false, true) => "slide imaged",
        (false, false) => "slide",
    };
    let background = image.map_or(String::new(), |uri| format!(";background-image:url('{uri}')"));
    let _ = write!(out, r#"<div class="{class}" style="aspect-ratio:{ratio}{background}">"#);
    let (w, h) = (size.cx.max(1) as f64, size.cy.max(1) as f64);
    // Shapes with no position written or inherited are listed under the
    // slide instead, so a change on them still shows.
    let mut unplaced = String::new();
    let mut leaf = 0usize;
    for (shape, at) in placed(slide) {
        let key = if shape.kind == ShapeKind::Group {
            Mark::Group(shape.id)
        } else {
            leaf += 1;
            Mark::Leaf(leaf)
        };
        let (status, id) = marks.get(&key).map_or(("", ""), |(s, id)| (status_word(*s), id.as_str()));
        let target = if id.is_empty() {
            String::new()
        } else {
            format!(r#" data-change="{}" tabindex="0" role="button""#, esc(id))
        };
        let Some(p) = at else {
            if shape.kind != ShapeKind::Group {
                let _ = write!(
                    unplaced,
                    r#"<span class="shape unplaced {status}" title="{}"{target}>{}</span>"#,
                    esc(&shape.name),
                    esc(&shape_text(shape))
                );
            }
            continue;
        };
        let mut style = format!(
            "left:{:.3}%;top:{:.3}%;width:{:.3}%;height:{:.3}%",
            p.x * 100.0 / w,
            p.y * 100.0 / h,
            p.cx * 100.0 / w,
            p.cy * 100.0 / h
        );
        if p.degrees != 0.0 {
            let _ = write!(style, ";transform:rotate({:.2}deg)", p.degrees);
        }
        let group = shape.kind == ShapeKind::Group;
        let text = if image.is_some() || group { String::new() } else { esc(&shape_text(shape)) };
        let _ = write!(
            out,
            r#"<div class="shape {status} k-{:?}{}" style="{style}" title="{}"{target}>{text}</div>"#,
            shape.kind,
            if group { " group" } else { "" },
            esc(&shape.name)
        );
    }
    out.push_str("</div>");
    if !unplaced.is_empty() {
        let _ =
            write!(out, r#"<div class="unplaced-list" title="shapes with no position of their own">{unplaced}</div>"#);
    }
}

/// Which drawn shape a change sits on: a leaf by its position (1-based, as
/// the comparison numbers them), a group by its id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Mark {
    Leaf(usize),
    Group(Option<u32>),
}

/// Drawn shapes with the status and id of the first change on them.
type ShapeMarks = HashMap<Mark, (Status, String)>;

/// The shape positions each side's changes sit on, with the first change's
/// status and id.
fn shape_marks(pair: &SlidePair) -> (ShapeMarks, ShapeMarks) {
    let (mut a, mut b) = (HashMap::new(), HashMap::new());
    for c in &pair.changes {
        for (side, map) in [(&c.base, &mut a), (&c.other, &mut b)] {
            // Group changes carry position 0 and the group's id.
            if let Some(shape) = side.as_ref().and_then(|s| s.shape.as_ref()) {
                let key = if shape.position == 0 { Mark::Group(shape.id) } else { Mark::Leaf(shape.position) };
                map.entry(key).or_insert((c.status, c.id.clone()));
            }
        }
    }
    (a, b)
}

fn side_text(side: Option<&crate::diff::Side>, marks: Option<&[(u32, u32)]>) -> String {
    let Some(side) = side else { return "<span class=\"absent\">—</span>".to_owned() };
    let mut out = String::new();
    let mut at = Vec::new();
    if let Some(shape) = &side.shape {
        at.push(esc(&shape.name));
    }
    if let Some(p) = side.paragraph {
        at.push(format!("paragraph {p}"));
    }
    match (side.row, side.column) {
        (Some(r), Some(c)) => at.push(format!("cell {r},{c}")),
        (Some(r), None) => at.push(format!("row {r}")),
        (None, Some(c)) => at.push(format!("column {c}")),
        (None, None) => {}
    }
    let _ = write!(
        out,
        r#"<div class="at">slide {}{}</div>"#,
        side.slide,
        at.iter().fold(String::new(), |mut s, a| {
            let _ = write!(s, " · {a}");
            s
        })
    );
    if let Some(text) = &side.text {
        let body = match marks {
            Some(m) if !m.is_empty() => marked(text, m),
            _ => esc(text),
        };
        let _ = write!(out, r#"<div class="text">{body}</div>"#);
    }
    out
}

fn properties(out: &mut String, props: &[crate::diff::Property]) {
    if props.is_empty() {
        return;
    }
    out.push_str("<table class=\"props\"><tr><th>property</th><th>base</th><th>other</th></tr>");
    for p in props {
        let show = |v: &serde_json::Value| match v {
            serde_json::Value::Null => "—".to_owned(),
            serde_json::Value::String(s) => esc(s),
            v => esc(&v.to_string()),
        };
        let _ = write!(out, "<tr><td>{}</td><td>{}</td><td>{}</td></tr>", esc(p.name), show(&p.base), show(&p.other));
    }
    out.push_str("</table>");
}

fn change_row(out: &mut String, c: &Change) {
    let marks: Option<&Marks> = c.marks.as_ref();
    let _ = write!(
        out,
        r#"<div class="change {}" id="{}"><div class="head"><span class="kind">{}</span><span class="status">{}</span></div><div class="sides"><div>{}</div><div>{}</div></div>"#,
        status_word(c.status),
        esc(&c.id),
        c.kind.as_str(),
        status_word(c.status),
        side_text(c.base.as_ref(), marks.map(|m| m.base.as_slice())),
        side_text(c.other.as_ref(), marks.map(|m| m.other.as_slice())),
    );
    properties(out, &c.properties);
    out.push_str("</div>");
}

fn object_row(out: &mut String, o: &ObjectChange) {
    let name = |v: &Option<serde_json::Value>| {
        v.as_ref().map_or("—".to_owned(), |v| esc(v.as_str().unwrap_or(&v.to_string())))
    };
    let _ = write!(
        out,
        r#"<div class="change {}" id="{}"><div class="head"><span class="kind">{}</span><span class="status">{}</span></div><div class="sides"><div>{}</div><div>{}</div></div>"#,
        status_word(o.status),
        esc(&o.id),
        o.kind.as_str(),
        status_word(o.status),
        name(&o.base),
        name(&o.other),
    );
    properties(out, &o.properties);
    out.push_str("</div>");
}

fn pair_title(pair: &SlidePair) -> String {
    let at = |r: &Option<crate::diff::SlideRef>| r.as_ref().map_or("–".to_owned(), |r| r.index.to_string());
    let title = pair.other.as_ref().or(pair.base.as_ref()).and_then(|r| r.title.clone()).unwrap_or_default();
    format!("{} → {} {}", at(&pair.base), at(&pair.other), title)
}

const STYLE: &str = r"
:root{--bg:#fff;--fg:#1d2330;--muted:#5d6676;--line:#d7dce4;--panel:#f5f7fa;--changed:#c77700;--added:#1d7f3a;--removed:#c0262d;--moved:#5b4ccf;--mark:#ffe08a}
@media (prefers-color-scheme:dark){:root{--bg:#14171c;--fg:#e3e7ee;--muted:#98a2b3;--line:#323a46;--panel:#1c2027;--mark:#7a5d00}}
*{box-sizing:border-box}html,body{overflow-x:clip}body{margin:0;font:14px/1.45 system-ui,sans-serif;background:var(--bg);color:var(--fg)}
header{position:sticky;top:0;z-index:5;background:var(--bg);border-bottom:1px solid var(--line);padding:10px 16px}
h1{font-size:16px;margin:0 0 6px}.chips span{display:inline-block;margin:0 6px 4px 0;padding:1px 8px;border:1px solid var(--line);border-radius:10px;color:var(--muted)}
.warn{color:var(--changed);font-size:13px;margin:4px 0 0}.tools{margin-top:6px}.tools button,.tools label{margin-right:10px;font:inherit}
main{display:flex}nav{width:240px;flex:none;border-right:1px solid var(--line);padding:8px;position:sticky;top:var(--head,96px);max-height:calc(100vh - var(--head,96px));overflow:auto}
nav a{display:block;padding:2px 6px;color:var(--fg);text-decoration:none;border-left:3px solid transparent;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
nav a.changed{border-color:var(--changed)}nav a.added{border-color:var(--added)}nav a.removed{border-color:var(--removed)}nav a.moved{border-color:var(--moved)}nav a .n{color:var(--muted)}
#pairs{flex:1;min-width:0;padding:12px 16px}section.pair{margin-bottom:28px}section.pair h2{font-size:14px;margin:0 0 6px}
.panes{display:grid;grid-template-columns:minmax(0,1fr) minmax(0,1fr);gap:12px}.panes>div{min-width:0}.pane-label{color:var(--muted);font-size:12px;margin-bottom:2px}
.slide{position:relative;border:1px solid var(--line);background:#fff no-repeat center/100% 100%;overflow:hidden;width:100%}
.slide.hidden{opacity:.55}.slide.none{display:flex;align-items:center;justify-content:center;background:repeating-linear-gradient(45deg,var(--panel),var(--panel) 8px,var(--bg) 8px,var(--bg) 16px)}
.slide.none span{color:var(--muted)}
.shape{position:absolute;border:1px dashed #b8c0cc;color:#333;font-size:clamp(6px,1.1vw,12px);line-height:1.2;overflow:hidden;padding:2px;white-space:pre-wrap;cursor:default}
.imaged .shape:not(.changed):not(.added):not(.removed):not(.moved){border-color:transparent}
.shape.changed{border:2px solid var(--changed);background:rgba(199,119,0,.08);cursor:pointer}.shape.added{border:2px solid var(--added);background:rgba(29,127,58,.08);cursor:pointer}
.shape.removed{border:2px solid var(--removed);background:rgba(192,38,45,.08);cursor:pointer}.shape.moved{border:2px solid var(--moved);cursor:pointer}
.unplaced-list{display:flex;flex-wrap:wrap;gap:4px;margin-top:4px}.shape.unplaced{position:static;display:inline-block;max-width:100%;font-size:12px;color:var(--fg);border-color:var(--line)}
.shape.group{border:1px dotted transparent;background:none;pointer-events:none}.shape.group.changed,.shape.group.moved{pointer-events:auto}
section.pair,.change{scroll-margin-top:var(--head,140px)}body.only-changes nav a.same{display:none}
.changes{margin-top:8px}.change{border:1px solid var(--line);border-left:4px solid var(--line);padding:6px 8px;margin:6px 0;background:var(--panel)}
.change.changed{border-left-color:var(--changed)}.change.added{border-left-color:var(--added)}.change.removed{border-left-color:var(--removed)}.change.moved{border-left-color:var(--moved)}
.change.current{outline:2px solid var(--fg)}.head .kind{font-weight:600;margin-right:8px}.head .status{color:var(--muted)}
.sides{display:grid;grid-template-columns:minmax(0,1fr) minmax(0,1fr);gap:12px}.at{color:var(--muted);font-size:12px}.text{white-space:pre-wrap;overflow-wrap:anywhere}
mark{background:var(--mark);color:inherit}.absent{color:var(--muted)}table.props{border-collapse:collapse;margin-top:4px;font-size:12px;width:100%}
table.props td,table.props th{border:1px solid var(--line);padding:2px 6px;text-align:left;vertical-align:top;overflow-wrap:anywhere}
body.only-changes section.pair.same{display:none}
@media (max-width:760px){main{display:block}nav{position:static;width:auto;max-height:none;border-right:0;border-bottom:1px solid var(--line)}.panes,.sides{grid-template-columns:minmax(0,1fr)}#pairs{padding:12px}header{padding:10px 12px}}
";

const SCRIPT: &str = r"
(function(){
var changes=[].slice.call(document.querySelectorAll('.change')),at=-1;
function head(){document.documentElement.style.setProperty('--head',document.querySelector('header').offsetHeight+'px');}
head();window.addEventListener('resize',head);
function go(d){var shown=changes.filter(function(c){return c.offsetParent!==null});if(!shown.length)return;
var cur=shown.indexOf(changes[at]);at=cur<0?(d>0?0:shown.length-1):(cur+d+shown.length)%shown.length;at=changes.indexOf(shown[at]);changes.forEach(function(c){c.classList.remove('current')});
var c=changes[at];c.classList.add('current');c.scrollIntoView({block:'center'});}
document.getElementById('next').onclick=function(){go(1)};document.getElementById('prev').onclick=function(){go(-1)};
document.getElementById('only').onchange=function(e){document.body.classList.toggle('only-changes',e.target.checked)};
document.addEventListener('keydown',function(e){if(e.target.tagName==='INPUT'||e.ctrlKey||e.metaKey||e.altKey)return;
if(e.key==='n'||e.key==='j')go(1);if(e.key==='p'||e.key==='k')go(-1);});
document.querySelectorAll('.shape[data-change]').forEach(function(s){
s.onkeydown=function(e){if(e.key==='Enter'||e.key===' '){e.preventDefault();s.onclick();}};
s.onclick=function(){
var c=document.getElementById(s.getAttribute('data-change'));if(c){changes.forEach(function(x){x.classList.remove('current')});
c.classList.add('current');at=changes.indexOf(c);c.scrollIntoView({block:'center'});}}});
})();
";

/// The report as one HTML document.
pub fn render(comparison: &Comparison, base: &Presentation, other: &Presentation, report: &Report<'_>) -> String {
    let mut out = String::new();
    let title = format!("{} → {}", comparison.base.file, comparison.other.file);
    let _ = write!(
        out,
        r#"<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>{}</title><style>{STYLE}</style></head><body>"#,
        esc(&title)
    );
    let summary = crate::diff::summarize(comparison);
    let _ = write!(out, "<header><h1>{}</h1><div class=\"chips\">", esc(&title));
    for (n, what) in [
        (summary.slides_changed, "slides changed"),
        (summary.slides_only_other, "added"),
        (summary.slides_only_base, "removed"),
        (summary.slides_moved, "moved"),
    ] {
        let _ = write!(out, "<span>{n} {what}</span>");
    }
    for (kind, n) in summary.changes.iter().filter(|(_, n)| **n > 0) {
        let _ = write!(out, "<span>{kind} {n}</span>");
    }
    out.push_str("</div>");
    for (code, message) in report.warnings {
        let _ = write!(out, r#"<div class="warn">{}: {}</div>"#, esc(code), esc(message));
    }
    out.push_str(r#"<div class="tools"><button id="prev">◀ previous (P)</button><button id="next">next (N) ▶</button><label><input type="checkbox" id="only"> changes only</label></div></header><main><nav>"#);
    for pair in &comparison.slides {
        let _ = write!(
            out,
            r##"<a href="#{}" class="{}">{} <span class="n">{}</span></a>"##,
            esc(&pair.id),
            status_word(pair.status),
            esc(&pair_title(pair)),
            if pair.changes.is_empty() { String::new() } else { format!("({})", pair.changes.len()) }
        );
    }
    if !comparison.objects.is_empty() {
        let _ = write!(
            out,
            r##"<a href="#objects" class="changed">deck <span class="n">({})</span></a>"##,
            comparison.objects.len()
        );
    }
    out.push_str(r#"</nav><div id="pairs">"#);
    for pair in &comparison.slides {
        let (ma, mb) = shape_marks(pair);
        let _ = write!(
            out,
            r#"<section class="pair {}" id="{}"><h2>{} · {}</h2><div class="panes">"#,
            status_word(pair.status),
            esc(&pair.id),
            esc(&pair_title(pair)),
            status_word(pair.status)
        );
        for (label, deck, r, marks, images) in [
            (&comparison.base.file, base, &pair.base, &ma, report.images.map(|i| &i.base)),
            (&comparison.other.file, other, &pair.other, &mb, report.images.map(|i| &i.other)),
        ] {
            let slide = r.as_ref().and_then(|r| deck.slides.get(r.index.wrapping_sub(1)).map(|s| (s, r.index)));
            let image = slide.and_then(|(_, n)| images.and_then(|m| m.get(&n)));
            let _ = write!(
                out,
                r#"<div><div class="pane-label">{}{}</div>"#,
                esc(label),
                slide.map_or(String::new(), |(_, n)| format!(" · slide {n}"))
            );
            slide_box(&mut out, slide, deck.slide_size, marks, image);
            out.push_str("</div>");
        }
        out.push_str("</div><div class=\"changes\">");
        for c in &pair.changes {
            change_row(&mut out, c);
        }
        out.push_str("</div></section>");
    }
    if !comparison.objects.is_empty() {
        out.push_str(r#"<section class="pair changed" id="objects"><h2>Deck · sections, layouts, masters, themes</h2><div class="changes">"#);
        for o in &comparison.objects {
            object_row(&mut out, o);
        }
        out.push_str("</div></section>");
    }
    let _ = writeln!(out, "</div></main><script>{SCRIPT}</script></body></html>");
    out
}

/// Standard base64, for embedding slide images as `data:` URIs.
pub fn base64(bytes: &[u8]) -> String {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        out.push(ABC[(n >> 18) as usize & 63] as char);
        out.push(ABC[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { ABC[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { ABC[n as usize & 63] as char } else { '=' });
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn base64_matches_the_standard() {
        assert_eq!(super::base64(b""), "");
        assert_eq!(super::base64(b"f"), "Zg==");
        assert_eq!(super::base64(b"fo"), "Zm8=");
        assert_eq!(super::base64(b"foobar"), "Zm9vYmFy");
    }

    /// `overflow-x: hidden` (or `auto`/`scroll`) on `html` or `body` makes
    /// `body` a scroll container, so the sticky header and sidebar stick to
    /// a box that never scrolls and leave the screen. `clip` hides the
    /// overflow without creating a scroll container.
    #[test]
    fn page_overflow_keeps_header_and_nav_sticky() {
        let rule = super::STYLE.split('}').find(|r| r.trim_start().starts_with("html,body{")).expect("html,body rule");
        assert!(rule.contains("overflow-x:clip"), "{rule}");
        assert!(!["hidden", "auto", "scroll"].iter().any(|v| rule.contains(&format!("overflow-x:{v}"))), "{rule}");
        assert!(super::STYLE.contains("header{position:sticky"));
        assert!(super::STYLE.contains("position:sticky;top:var(--head"));
    }

    #[test]
    fn marks_wrap_utf16_ranges() {
        assert_eq!(super::marked("a 한글 b", &[(2, 4)]), "a <mark>한글</mark> b");
        assert_eq!(super::marked("<x>", &[]), "&lt;x&gt;");
    }
}
