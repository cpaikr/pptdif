// Copied from xldif (crates/xldif/src/xml.rs) at 8b272fd; keep the copies in step until a
// shared crate replaces them (PLAN.md, "Relation to xldif").

//! A small pull parser over `quick-xml` for `PresentationML` and `DrawingML` parts.
//!
//! It yields namespace-agnostic local names (generators differ in prefixes:
//! `<p:sp>` and `<sp>` are the same element), resolves entity and character
//! references in text and attributes, and caps nesting depth so hostile
//! input fails instead of exhausting the stack of a recursive consumer.

use quick_xml::escape::resolve_xml_entity;
use quick_xml::events::{BytesStart, Event as QEvent};
use std::fmt::Write as _;

use quick_xml::{Reader, XmlVersion};

use crate::{Error, ErrorCode};

/// Deeper nesting than any presentation part needs (group shapes nest, but
/// not this far).
pub const MAX_DEPTH: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Element {
    /// Local name, without any namespace prefix.
    pub name: String,
    /// Attributes by local name, values unescaped, in document order.
    pub attrs: Vec<(String, String)>,
    /// Which of `attrs` carried a namespace prefix, by index. Not in xldif:
    /// `PresentationML` puts `id` and `r:id` on one element (`<p:sldId>`).
    pub prefixed: Vec<bool>,
}

impl Element {
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    /// The unprefixed attribute `name`, ignoring one in another namespace.
    pub fn plain_attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().zip(&self.prefixed).find(|((k, _), p)| k == name && !**p).map(|((_, v), _)| v.as_str())
    }

    /// The prefixed attribute `name`, such as `r:id`.
    pub fn prefixed_attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().zip(&self.prefixed).find(|((k, _), p)| k == name && **p).map(|((_, v), _)| v.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// An opening tag. A self-closing tag yields `Start` then `End`.
    Start(Element),
    End(String),
    /// Character data with references resolved. Adjacent pieces may arrive
    /// as several events.
    Text(String),
    Eof,
}

pub struct Parser<'a> {
    reader: Reader<&'a [u8]>,
    part: &'a str,
    depth: usize,
    /// The `End` owed for a self-closing tag.
    pending_end: Option<String>,
}

fn local(name: &str) -> String {
    name.rsplit(':').next().unwrap_or(name).to_owned()
}

impl<'a> Parser<'a> {
    /// `part` names the archive member in error messages.
    pub fn new(text: &'a str, part: &'a str) -> Self {
        let mut reader = Reader::from_str(text);
        reader.config_mut().trim_text(false);
        Self { reader, part, depth: 0, pending_end: None }
    }

    fn error(&self, detail: impl std::fmt::Display) -> Error {
        Error::new(ErrorCode::InvalidXml, format!("{} is not well-formed XML: {detail}", self.part))
    }

    fn element(&self, start: &BytesStart<'_>) -> Result<Element, Error> {
        let mut attrs = Vec::new();
        let mut prefixed = Vec::new();
        for attr in start.attributes() {
            let attr = attr.map_err(|e| self.error(e))?;
            let value = attr.normalized_value(XmlVersion::Implicit1_0).map_err(|e| self.error(e))?;
            let key: &str = attr.key.as_ref();
            prefixed.push(key.contains(':'));
            attrs.push((local(key), value.into_owned()));
        }
        Ok(Element { name: local(start.name().as_ref()), attrs, prefixed })
    }

    pub fn next(&mut self) -> Result<Event, Error> {
        if let Some(name) = self.pending_end.take() {
            self.depth -= 1;
            return Ok(Event::End(name));
        }
        loop {
            let event = self.reader.read_event().map_err(|e| self.error(e))?;
            return Ok(match event {
                QEvent::Start(start) => {
                    self.depth += 1;
                    if self.depth > MAX_DEPTH {
                        return Err(Error::new(
                            ErrorCode::InvalidArchive,
                            format!("{} nests deeper than {MAX_DEPTH} elements", self.part),
                        ));
                    }
                    Event::Start(self.element(&start)?)
                }
                QEvent::Empty(start) => {
                    let element = self.element(&start)?;
                    self.depth += 1;
                    self.pending_end = Some(element.name.clone());
                    Event::Start(element)
                }
                QEvent::End(end) => {
                    self.depth = self.depth.saturating_sub(1);
                    Event::End(local(end.name().as_ref()))
                }
                QEvent::Text(text) => Event::Text(text.xml10_content().into_owned()),
                QEvent::CData(data) => Event::Text(data.xml10_content().into_owned()),
                QEvent::GeneralRef(reference) => {
                    if let Some(ch) = reference.resolve_char_ref().map_err(|e| self.error(e))? {
                        Event::Text(ch.to_string())
                    } else {
                        let name: &str = &reference;
                        let resolved =
                            resolve_xml_entity(name).ok_or_else(|| self.error(format!("unknown entity &{name};")))?;
                        Event::Text(resolved.to_owned())
                    }
                }
                // A truncated part must not read as a complete, smaller one.
                QEvent::Eof if self.depth > 0 => return Err(self.error("the part ends inside an open element")),
                QEvent::Eof => Event::Eof,
                // Declarations, comments, processing instructions, doctypes.
                _ => continue,
            });
        }
    }

    /// Text up to the end of the element just started, skipping any child
    /// elements whose local name is in `skip` (with everything inside them)
    /// and collecting text from all others.
    #[cfg_attr(not(test), expect(dead_code, reason = "paragraph text is read in M1"))]
    pub fn text_until_end(&mut self, skip: &[&str]) -> Result<String, Error> {
        let mut out = String::new();
        let mut depth = 1usize;
        let mut skipping: Option<usize> = None;
        loop {
            match self.next()? {
                Event::Start(e) => {
                    depth += 1;
                    if skipping.is_none() && skip.contains(&e.name.as_str()) {
                        skipping = Some(depth);
                    }
                }
                Event::End(_) => {
                    if skipping == Some(depth) {
                        skipping = None;
                    }
                    depth -= 1;
                    if depth == 0 {
                        return Ok(out);
                    }
                }
                Event::Text(t) => {
                    if skipping.is_none() {
                        out.push_str(&t);
                    }
                }
                Event::Eof => return Err(self.error("unexpected end of part")),
            }
        }
    }

    /// The text inside `wanted` elements up to the end of the element just
    /// started, skipping any element named in `skip` with everything inside
    /// it. Text outside `wanted` (indentation between elements) is dropped.
    #[cfg_attr(not(test), expect(dead_code, reason = "paragraph text is read in M1"))]
    pub fn text_of(&mut self, wanted: &str, skip: &[&str]) -> Result<String, Error> {
        let mut out = String::new();
        let mut depth = 1usize;
        let mut skipping: Option<usize> = None;
        let mut inside: Option<usize> = None;
        loop {
            match self.next()? {
                Event::Start(e) => {
                    depth += 1;
                    if skipping.is_none() && skip.contains(&e.name.as_str()) {
                        skipping = Some(depth);
                    } else if skipping.is_none() && inside.is_none() && e.name == wanted {
                        inside = Some(depth);
                    }
                }
                Event::End(_) => {
                    if skipping == Some(depth) {
                        skipping = None;
                    }
                    if inside == Some(depth) {
                        inside = None;
                    }
                    depth -= 1;
                    if depth == 0 {
                        return Ok(out);
                    }
                }
                Event::Text(t) => {
                    if skipping.is_none() && inside.is_some() {
                        out.push_str(&t);
                    }
                }
                Event::Eof => return Err(self.error("unexpected end of part")),
            }
        }
    }

    /// A canonical description of the rest of the element just started:
    /// each child as `name(attr=value,…){…}` with attributes sorted, and
    /// text trimmed. Two elements written the same way, whatever their
    /// attribute order or whitespace, describe the same.
    #[expect(dead_code, reason = "shape properties compare canonically in M2")]
    pub fn describe_rest(&mut self) -> Result<String, Error> {
        let mut out = String::new();
        let mut depth = 1usize;
        loop {
            match self.next()? {
                Event::Start(e) => {
                    depth += 1;
                    let mut attrs: Vec<String> = e.attrs.iter().map(|(k, v)| format!("{k}={v}")).collect();
                    attrs.sort();
                    out.push_str(&e.name);
                    if !attrs.is_empty() {
                        out.push('(');
                        out.push_str(&attrs.join(","));
                        out.push(')');
                    }
                    out.push('{');
                }
                Event::End(_) => {
                    depth -= 1;
                    if depth == 0 {
                        return Ok(out);
                    }
                    out.push('}');
                }
                Event::Text(t) => {
                    let t = t.trim();
                    if !t.is_empty() {
                        let _ = write!(out, "{t:?}");
                    }
                }
                Event::Eof => return Err(self.error("unexpected end of part")),
            }
        }
    }

    /// Skips the rest of the element just started.
    pub fn skip_element(&mut self) -> Result<(), Error> {
        let mut depth = 1usize;
        loop {
            match self.next()? {
                Event::Start(_) => depth += 1,
                Event::End(_) => {
                    depth -= 1;
                    if depth == 0 {
                        return Ok(());
                    }
                }
                Event::Text(_) => {}
                Event::Eof => return Err(self.error("unexpected end of part")),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Event, Parser};

    fn events(xml: &str) -> Vec<Event> {
        let mut p = Parser::new(xml, "test.xml");
        let mut out = Vec::new();
        loop {
            let e = p.next().unwrap();
            if e == Event::Eof {
                return out;
            }
            out.push(e);
        }
    }

    #[test]
    fn prefixes_are_dropped_and_empty_tags_close() {
        let ev = events(r#"<x:c r="A1" x:t="s"/>"#);
        let Event::Start(e) = &ev[0] else { panic!("{ev:?}") };
        assert_eq!(e.name, "c");
        assert_eq!(e.attr("t"), Some("s"));
        assert_eq!(ev[1], Event::End("c".into()));
    }

    #[test]
    fn references_resolve_in_text_and_attributes() {
        let mut p = Parser::new(r#"<t a="&lt;&#x41;">x &amp; y&#66;</t>"#, "test.xml");
        let Event::Start(e) = p.next().unwrap() else { panic!() };
        assert_eq!(e.attr("a"), Some("<A"));
        assert_eq!(p.text_until_end(&[]).unwrap(), "x & yB");
    }

    #[test]
    fn skipped_children_drop_their_text() {
        let mut p = Parser::new("<si><r><t>a</t></r><rPh><t>b</t></rPh><t>c</t></si>", "test.xml");
        p.next().unwrap();
        assert_eq!(p.text_until_end(&["rPh"]).unwrap(), "ac");
    }

    #[test]
    fn text_of_keeps_only_wanted_elements() {
        let xml = "<si>\n  <r>\n    <t>rich </t>\n  </r>\n  <r><t>text</t></r>\n  <rPh><t>yomi</t></rPh>\n</si>";
        let mut p = Parser::new(xml, "test.xml");
        p.next().unwrap();
        assert_eq!(p.text_of("t", &["rPh"]).unwrap(), "rich text");
    }

    #[test]
    fn a_truncated_part_is_an_error() {
        let mut p = Parser::new("<sld><cSld><spTree><sp><txBody>", "cut.xml");
        let err = loop {
            match p.next() {
                Ok(Event::Eof) => panic!("truncation accepted"),
                Ok(_) => {}
                Err(e) => break e,
            }
        };
        assert_eq!(err.code, crate::ErrorCode::InvalidXml);
    }

    #[test]
    fn depth_is_capped() {
        let xml = "<a>".repeat(super::MAX_DEPTH + 1);
        let mut p = Parser::new(&xml, "deep.xml");
        let err = loop {
            match p.next() {
                Ok(Event::Eof) => panic!("no error"),
                Ok(_) => {}
                Err(e) => break e,
            }
        };
        assert_eq!(err.code, crate::ErrorCode::InvalidArchive);
    }
}
