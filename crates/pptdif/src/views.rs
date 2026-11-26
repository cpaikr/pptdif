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
        notes: slide.has_notes,
        comments: slide.comments,
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
