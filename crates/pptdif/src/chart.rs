//! A chart part's cached data (`c:chartSpace`): the plot kinds, the title,
//! and each series' name, categories, and values as PowerPoint last drew
//! them. The embedded workbook is not read; xldif compares that.

use crate::Error;
use crate::model::{Chart, Series};
use crate::reader::Budget;
use crate::xml::{Event, Parser};

/// Which part of a series a cached value belongs to.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Slot {
    Name,
    Category,
    Value,
}

/// The `c:v` texts of the cache element just started, by point index, and
/// the point count it declares (`c:ptCount`), which counts trailing empty
/// points too.
fn read_points(parser: &mut Parser<'_>, budget: &mut Budget) -> Result<(Vec<(usize, String)>, usize), Error> {
    let mut points = Vec::new();
    let mut declared = 0usize;
    let mut index: Option<usize> = None;
    let mut depth = 1usize;
    loop {
        match parser.next()? {
            Event::Start(e) => match e.name.as_str() {
                "ptCount" => {
                    depth += 1;
                    declared = e.plain_attr("val").and_then(|v| v.parse().ok()).unwrap_or(0);
                }
                "pt" => {
                    depth += 1;
                    index = e.plain_attr("idx").and_then(|v| v.parse().ok());
                }
                "v" => {
                    budget.take()?;
                    let text = parser.text_until_end(&[])?;
                    // A name cache's lone value may have no `pt`.
                    points.push((index.unwrap_or(points.len()), text));
                }
                _ => depth += 1,
            },
            Event::End(_) => {
                depth -= 1;
                if depth == 0 {
                    return Ok((points, declared));
                }
            }
            Event::Text(_) => {}
            Event::Eof => return Ok((points, declared)),
        }
    }
}

/// Point indices are written by the file: every slot up to the highest is
/// charged to the record budget, so a lone point at a far index costs what
/// its padding does.
fn place<T: Clone + Default>(points: Vec<(usize, T)>, declared: usize, budget: &mut Budget) -> Result<Vec<T>, Error> {
    let highest = points.iter().map(|p| p.0.checked_add(1)).try_fold(0usize, |m, n| n.map(|n| m.max(n)));
    let len = highest
        .ok_or_else(|| Error::new(crate::ErrorCode::InvalidArchive, "a chart point index is out of range"))?
        .max(declared);
    budget.take_n(len.saturating_sub(points.len()))?;
    let mut out = vec![T::default(); len];
    for (i, v) in points {
        out[i] = v;
    }
    Ok(out)
}

/// Multi-level category labels: each point's levels joined, innermost
/// first, as `inner / outer`.
fn join_levels(points: Vec<(usize, String)>) -> Vec<(usize, String)> {
    let mut by_index: std::collections::BTreeMap<usize, Vec<String>> = std::collections::BTreeMap::new();
    for (i, v) in points {
        by_index.entry(i).or_default().push(v);
    }
    by_index.into_iter().map(|(i, v)| (i, v.join(" / "))).collect()
}

pub(crate) fn read_chart(text: &str, part: &str, budget: &mut Budget) -> Result<Chart, Error> {
    let mut parser = Parser::new(text, part);
    let mut chart = Chart::default();
    let mut in_plot_area = false;
    let mut slot: Option<Slot> = None;
    // The depth of the open `c:ser`: a series' own name, categories, and
    // values sit right below it; a data label's or trendline's `c:tx` is
    // deeper.
    let mut series_depth: Option<usize> = None;
    let mut depth = 0usize;
    loop {
        match parser.next()? {
            Event::Start(e) => {
                depth += 1;
                let in_series = series_depth.is_some_and(|d| depth == d + 1);
                match e.name.as_str() {
                    "plotArea" => in_plot_area = true,
                    // The chart's own title comes before the plot area; axis
                    // titles sit inside it. A title linked to a cell holds
                    // its formula (`c:f`) beside the cached text.
                    "title" if !in_plot_area && chart.title.is_none() => {
                        depth -= 1;
                        let title = parser.text_until_end(&["txPr", "spPr", "layout", "overlay", "f", "ptCount"])?;
                        let title = title.trim();
                        if !title.is_empty() {
                            chart.title = Some(title.to_owned());
                        }
                    }
                    name if in_plot_area && name.ends_with("Chart") => chart.kinds.push(name.to_owned()),
                    "ser" => {
                        budget.take()?;
                        series_depth = Some(depth);
                        chart.series.push(Series { name: None, categories: Vec::new(), values: Vec::new() });
                    }
                    "tx" if in_series => slot = Some(Slot::Name),
                    // A series name may be written as `c:tx/c:v`, with no cache.
                    "v" if slot == Some(Slot::Name) => {
                        depth -= 1;
                        let name = parser.text_until_end(&[])?;
                        if let Some(series) = chart.series.last_mut() {
                            series.name = Some(name);
                        }
                    }
                    "cat" | "xVal" if in_series => slot = Some(Slot::Category),
                    "val" | "yVal" if in_series => slot = Some(Slot::Value),
                    "strCache" | "numCache" | "strLit" | "numLit" | "multiLvlStrCache" if slot.is_some() => {
                        depth -= 1;
                        let multi = e.name == "multiLvlStrCache";
                        let (points, declared) = read_points(&mut parser, budget)?;
                        let points = if multi { join_levels(points) } else { points };
                        let Some(series) = chart.series.last_mut() else { continue };
                        match slot {
                            Some(Slot::Name) => series.name = points.into_iter().next().map(|p| p.1),
                            Some(Slot::Category) => series.categories = place(points, declared, budget)?,
                            Some(Slot::Value) => {
                                let values = points.into_iter().map(|(i, v)| (i, v.trim().parse::<f64>().ok()));
                                series.values = place(values.collect(), declared, budget)?;
                            }
                            None => {}
                        }
                    }
                    _ => {}
                }
            }
            Event::End(name) => {
                match name.as_str() {
                    "plotArea" => in_plot_area = false,
                    "ser" => series_depth = None,
                    "tx" | "cat" | "xVal" | "val" | "yVal" if series_depth.is_some_and(|d| depth == d + 1) => {
                        slot = None;
                    }
                    _ => {}
                }
                depth = depth.saturating_sub(1);
            }
            Event::Text(_) => {}
            Event::Eof => return Ok(chart),
        }
    }
}
