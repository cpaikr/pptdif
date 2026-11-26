//! A chart part's cached data (`c:chartSpace`): the plot kinds, the title,
//! and each series' name, categories, and values as PowerPoint last drew
//! them. The embedded workbook is not read; xldif compares that.

use crate::model::{Chart, Series};
use crate::reader::Budget;
use crate::xml::{Event, Parser};
use crate::{Error, ErrorCode};

/// Which part of a series a cached value belongs to.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Slot {
    Name,
    Category,
    Value,
}

/// The `c:v` texts of the cache element just started, by point index.
fn read_points(parser: &mut Parser<'_>, budget: &mut Budget) -> Result<Vec<(usize, String)>, Error> {
    let mut points = Vec::new();
    let mut index: Option<usize> = None;
    let mut depth = 1usize;
    loop {
        match parser.next()? {
            Event::Start(e) => match e.name.as_str() {
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
                    return Ok(points);
                }
            }
            Event::Text(_) => {}
            Event::Eof => return Ok(points),
        }
    }
}

/// Point indices are written by the file; one far beyond the others would
/// make the vectors huge, so indices are bounded by the points read.
fn place<T: Clone + Default>(points: Vec<(usize, T)>) -> Result<Vec<T>, Error> {
    let len = points.iter().map(|p| p.0 + 1).max().unwrap_or(0);
    if len > points.len().saturating_mul(16).max(1024) {
        return Err(Error::new(ErrorCode::InvalidXml, "a chart point index is far beyond the points it holds"));
    }
    let mut out = vec![T::default(); len];
    for (i, v) in points {
        out[i] = v;
    }
    Ok(out)
}

pub(crate) fn read_chart(text: &str, part: &str, budget: &mut Budget) -> Result<Chart, Error> {
    let mut parser = Parser::new(text, part);
    let mut chart = Chart::default();
    let mut in_plot_area = false;
    let mut slot: Option<Slot> = None;
    loop {
        match parser.next()? {
            Event::Start(e) => match e.name.as_str() {
                "plotArea" => in_plot_area = true,
                // The chart's own title comes before the plot area; axis
                // titles sit inside it.
                "title" if !in_plot_area && chart.title.is_none() => {
                    let title = parser.text_until_end(&["txPr", "spPr", "layout", "overlay"])?;
                    let title = title.trim();
                    if !title.is_empty() {
                        chart.title = Some(title.to_owned());
                    }
                }
                name if in_plot_area && name.ends_with("Chart") => chart.kinds.push(name.to_owned()),
                "ser" => {
                    budget.take()?;
                    chart.series.push(Series { name: None, categories: Vec::new(), values: Vec::new() });
                }
                "tx" if !chart.series.is_empty() => slot = Some(Slot::Name),
                "cat" | "xVal" => slot = Some(Slot::Category),
                "val" | "yVal" => slot = Some(Slot::Value),
                "strCache" | "numCache" | "strLit" | "numLit" => {
                    let points = read_points(&mut parser, budget)?;
                    let Some(series) = chart.series.last_mut() else { continue };
                    match slot {
                        Some(Slot::Name) => series.name = points.into_iter().next().map(|p| p.1),
                        Some(Slot::Category) => series.categories = place(points)?,
                        Some(Slot::Value) => {
                            let values = points.into_iter().map(|(i, v)| (i, v.trim().parse::<f64>().ok()));
                            series.values = place(values.collect())?;
                        }
                        None => {}
                    }
                }
                _ => {}
            },
            Event::End(name) => match name.as_str() {
                "plotArea" => in_plot_area = false,
                "tx" | "cat" | "xVal" | "val" | "yVal" => slot = None,
                _ => {}
            },
            Event::Text(_) => {}
            Event::Eof => return Ok(chart),
        }
    }
}
