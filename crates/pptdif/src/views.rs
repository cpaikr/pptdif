//! Read views over a [`Presentation`]: serializable results the CLI prints.

use std::collections::HashMap;
use std::sync::Arc;

use serde::Serialize;

use crate::model::{DeckFormat, Master, Presentation, ShapeKind, Slide};

/// Points per EMU.
const EMU_PER_POINT: f64 = 12_700.0;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SlideSize {
    pub width_pt: f64,
    pub height_pt: f64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SlideSummary {
    /// 1-based position in the show.
    pub index: usize,
    /// `sldId` id: stable while the slide stays in this file.
    pub id: u32,
    pub hidden: bool,
    pub layout: Option<Arc<str>>,
    pub section: Option<String>,
    /// Shapes other than groups, group members included.
    pub shapes: usize,
    pub groups: usize,
    pub placeholders: usize,
    pub pictures: usize,
    pub tables: usize,
    pub charts: usize,
    pub notes: bool,
    pub comments: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SectionSummary {
    pub name: String,
    pub slides: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Inspection {
    pub file: String,
    pub format: DeckFormat,
    pub slide_size: Option<SlideSize>,
    pub slides: Vec<SlideSummary>,
    pub sections: Vec<SectionSummary>,
    pub masters: Vec<Master>,
}

fn summarize(index: usize, slide: &Slide, section: Option<&str>) -> SlideSummary {
    let mut summary = SlideSummary {
        index,
        id: slide.id,
        hidden: slide.hidden,
        layout: slide.layout.clone(),
        section: section.map(str::to_owned),
        shapes: 0,
        groups: 0,
        placeholders: 0,
        pictures: 0,
        tables: 0,
        charts: 0,
        notes: slide.notes.is_some(),
        comments: slide.comments.len(),
    };
    for shape in slide.all_shapes() {
        match shape.kind {
            ShapeKind::Group => {
                summary.groups += 1;
                continue;
            }
            ShapeKind::Picture => summary.pictures += 1,
            ShapeKind::Table => summary.tables += 1,
            ShapeKind::Chart => summary.charts += 1,
            _ => {}
        }
        summary.shapes += 1;
        if shape.placeholder.is_some() {
            summary.placeholders += 1;
        }
    }
    summary
}

/// The `inspect` result: structure and counts, no slide text.
pub fn inspect(file: &str, deck: &Presentation) -> Inspection {
    // The first section to list a slide owns it.
    let mut section_of: HashMap<u32, &str> = HashMap::new();
    for section in &deck.sections {
        for id in &section.slide_ids {
            section_of.entry(*id).or_insert(section.name.as_str());
        }
    }
    Inspection {
        file: file.to_owned(),
        format: deck.format,
        slide_size: deck
            .slide_size
            .map(|s| SlideSize { width_pt: s.cx as f64 / EMU_PER_POINT, height_pt: s.cy as f64 / EMU_PER_POINT }),
        slides: deck
            .slides
            .iter()
            .enumerate()
            .map(|(i, s)| summarize(i + 1, s, section_of.get(&s.id).copied()))
            .collect(),
        sections: deck
            .sections
            .iter()
            .map(|s| SectionSummary { name: s.name.clone(), slides: s.slide_ids.len() })
            .collect(),
        masters: deck.masters.clone(),
    }
}

#[derive(Debug, Serialize)]
pub struct ParagraphView {
    pub level: u8,
    pub text: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShapeText {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<u32>,
    pub kind: crate::model::ShapeKind,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
    pub paragraphs: Vec<ParagraphView>,
}

#[derive(Debug, Serialize)]
pub struct SlideText {
    pub index: usize,
    pub id: u32,
    pub shapes: Vec<ShapeText>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<Vec<ParagraphView>>,
}

#[derive(Debug, Serialize)]
pub struct TextListing {
    pub file: String,
    pub slides: Vec<SlideText>,
}

fn paragraphs(ps: &[crate::model::Paragraph]) -> Vec<ParagraphView> {
    ps.iter().map(|p| ParagraphView { level: p.level, text: p.text() }).collect()
}

/// A shape's text as lines: its paragraphs; a table's rows as `cell | cell`;
/// a chart's title and each series as `name: category value, ...` (as the
/// chart last drew them).
fn shape_lines(shape: &crate::model::Shape) -> Vec<ParagraphView> {
    let line = |text: String| ParagraphView { level: 0, text };
    if let Some(table) = &shape.table {
        return table
            .rows
            .iter()
            .map(|row| {
                let cells: Vec<String> = row
                    .iter()
                    .map(|c| c.text.iter().map(crate::model::Paragraph::text).collect::<Vec<_>>().join(" "))
                    .collect();
                line(cells.join(" | "))
            })
            .collect();
    }
    if let Some(chart) = shape.chart.as_ref().and_then(|c| c.data.as_ref()) {
        let mut out: Vec<ParagraphView> = chart.title.iter().map(|t| line(t.clone())).collect();
        for s in &chart.series {
            let points: Vec<String> = s
                .values
                .iter()
                .enumerate()
                .map(|(i, v)| {
                    let category = s.categories.get(i).map_or_else(|| (i + 1).to_string(), Clone::clone);
                    format!("{category} {}", v.map_or_else(|| "-".to_owned(), |v| v.to_string()))
                })
                .collect();
            out.push(line(format!("{}: {}", s.name.as_deref().unwrap_or("series"), points.join(", "))));
        }
        return out;
    }
    shape.text.as_deref().map(paragraphs).unwrap_or_default()
}

/// The `text` result: every shape with text, slide by slide, and the notes.
/// `slide` keeps only that 1-based slide.
pub fn text(file: &str, deck: &Presentation, slide: Option<usize>) -> TextListing {
    let slides = deck
        .slides
        .iter()
        .enumerate()
        .filter(|(i, _)| slide.is_none_or(|n| n == i + 1))
        .map(|(i, s)| SlideText {
            index: i + 1,
            id: s.id,
            shapes: s
                .all_shapes()
                .filter_map(|shape| {
                    let lines = shape_lines(shape);
                    if lines.iter().all(|l| l.text.is_empty()) {
                        return None;
                    }
                    Some(ShapeText {
                        id: shape.id,
                        kind: shape.kind,
                        name: shape.name.clone(),
                        placeholder: shape
                            .placeholder
                            .as_ref()
                            .map(|p| if p.index == 0 { p.kind.clone() } else { format!("{}:{}", p.kind, p.index) }),
                        paragraphs: lines,
                    })
                })
                .collect(),
            notes: s.notes.as_deref().map(paragraphs),
        })
        .collect();
    TextListing { file: file.to_owned(), slides }
}
