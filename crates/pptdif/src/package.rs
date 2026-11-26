// Copied from xldif (crates/xldif/src/package.rs) at 8b272fd; keep the copies in step until a
// shared crate replaces them (PLAN.md, "Relation to xldif").

//! The OPC package: a bounded ZIP archive, member lookup, and relationships.
//!
//! Every read is capped so a hostile or corrupt file fails with
//! `invalid_archive` instead of exhausting memory: archive size, member
//! count, each member's decompressed size (measured while reading, since
//! declared sizes can lie), and the total decompressed across one presentation.

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek};
use std::path::Path;

use zip::ZipArchive;

use crate::xml::{Event, Parser};
use crate::{Error, ErrorCode};

/// Bounds on what one presentation may make the reader do. The defaults sit
/// far above real decks; tests lower them to exercise each bound.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub max_file_bytes: u64,
    pub max_members: usize,
    /// One member's decompressed size, measured while reading.
    pub max_member_bytes: u64,
    /// All members read from one presentation, decompressed.
    pub max_total_bytes: u64,
    /// Slides, shapes, paragraphs, and runs together (about 100 bytes of
    /// memory each).
    pub max_records: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_file_bytes: 1 << 30,
            max_members: 20_000,
            max_member_bytes: 1 << 30,
            max_total_bytes: 4 << 30,
            max_records: 20_000_000,
        }
    }
}

const ZIP_MAGIC: &[u8] = b"PK\x03\x04";
/// OLE compound files: legacy `.ppt`, and password-protected OOXML files.
const OLE_MAGIC: &[u8] = &[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];

fn invalid(message: impl Into<String>) -> Error {
    Error::new(ErrorCode::InvalidArchive, message)
}

/// One relationship from a `.rels` part, its target resolved to a member name.
#[derive(Debug, Clone)]
pub struct Relationship {
    pub id: String,
    /// The relationship type's last path segment, such as `slide`. The
    /// transitional and strict namespaces differ only before it.
    pub kind: String,
    /// A member name for an internal target; the address as written for an
    /// external one.
    pub target: String,
    /// Points outside the package (a web address, a linked file).
    pub external: bool,
}

pub struct Package<R> {
    archive: ZipArchive<R>,
    /// Lower-cased member name → actual name; some generators vary case.
    names: HashMap<String, String>,
    read_total: u64,
    limits: Limits,
}

/// Classifies the first bytes of a file before treating it as a ZIP.
fn check_magic(head: &[u8]) -> Result<(), Error> {
    if head.starts_with(ZIP_MAGIC) {
        Ok(())
    } else if head.starts_with(OLE_MAGIC) {
        Err(Error::new(
            ErrorCode::UnsupportedFormat,
            "this is a legacy .ppt presentation or a password-protected file, which pptdif cannot read",
        )
        .with_hint("save it from PowerPoint as .pptx without a password to open, then compare that copy"))
    } else {
        Err(invalid("the file is not a ZIP-based PowerPoint presentation"))
    }
}

impl Package<File> {
    pub fn open(path: &Path, limits: &Limits) -> Result<Self, Error> {
        let meta = std::fs::metadata(path).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => {
                Error::new(ErrorCode::FileNotFound, format!("{} does not exist", path.display()))
            }
            _ => Error::new(ErrorCode::IoError, format!("reading {}: {e}", path.display())),
        })?;
        if !meta.is_file() {
            return Err(Error::new(ErrorCode::FileNotFound, format!("{} is not a file", path.display())));
        }
        if meta.len() > limits.max_file_bytes {
            return Err(invalid(format!("the file is larger than {} bytes", limits.max_file_bytes)));
        }
        let io = |e: std::io::Error| Error::new(ErrorCode::IoError, format!("reading {}: {e}", path.display()));
        let mut file = File::open(path).map_err(io)?;
        let mut head = [0u8; 8];
        let n = file.read(&mut head).map_err(io)?;
        check_magic(&head[..n])?;
        file.rewind().map_err(io)?;
        Self::new(file, limits)
    }
}

impl Package<std::io::Cursor<Vec<u8>>> {
    /// A presentation held in memory, checked to start like a ZIP.
    pub fn from_bytes(bytes: Vec<u8>, limits: &Limits) -> Result<Self, Error> {
        if bytes.len() as u64 > limits.max_file_bytes {
            return Err(invalid(format!("the file is larger than {} bytes", limits.max_file_bytes)));
        }
        check_magic(&bytes[..bytes.len().min(8)])?;
        Self::new(std::io::Cursor::new(bytes), limits)
    }
}

impl<R: Read + Seek> Package<R> {
    pub fn new(reader: R, limits: &Limits) -> Result<Self, Error> {
        let archive = ZipArchive::new(reader).map_err(|e| invalid(format!("the ZIP container is unreadable: {e}")))?;
        if archive.len() > limits.max_members {
            return Err(invalid(format!("the archive has more than {} members", limits.max_members)));
        }
        let mut names = HashMap::new();
        for name in archive.file_names() {
            let name = name.map_err(|e| invalid(format!("a member name is unreadable: {e}")))?;
            names.insert(name.to_ascii_lowercase(), name.into_owned());
        }
        Ok(Self { archive, names, read_total: 0, limits: *limits })
    }

    /// The member as UTF-8 text (a leading BOM dropped), or `None` when absent.
    pub fn read_text(&mut self, name: &str) -> Result<Option<String>, Error> {
        let Some(actual) = self.names.get(&name.to_ascii_lowercase()).cloned() else {
            return Ok(None);
        };
        let Limits { max_member_bytes, max_total_bytes, .. } = self.limits;
        let room = max_member_bytes.min(max_total_bytes.saturating_sub(self.read_total));
        let mut member = self.archive.by_name(&actual).map_err(|e| invalid(format!("{actual}: {e}")))?;
        let mut bytes = Vec::new();
        (&mut member)
            .take(room.saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(|e| invalid(format!("{actual} cannot be decompressed: {e}")))?;
        drop(member);
        let len = bytes.len() as u64;
        if len > max_member_bytes {
            return Err(invalid(format!("{actual} decompresses to more than {max_member_bytes} bytes")));
        }
        if len > room {
            return Err(invalid(format!("the presentation decompresses to more than {max_total_bytes} bytes")));
        }
        self.read_total += len;
        decode(bytes)
            .map(Some)
            .ok_or_else(|| Error::new(ErrorCode::InvalidXml, format!("{actual} is not UTF-8 or UTF-16 text")))
    }

    /// The relationships of `part` (from `<dir>/_rels/<file>.rels`), with
    /// internal targets resolved against the part's directory and external
    /// ones (hyperlinks, linked files) kept as written.
    pub fn relationships(&mut self, part: &str) -> Result<Vec<Relationship>, Error> {
        let (dir, file) = part.rsplit_once('/').map_or(("", part), |(d, f)| (d, f));
        let rels_name = if dir.is_empty() { format!("_rels/{file}.rels") } else { format!("{dir}/_rels/{file}.rels") };
        let Some(text) = self.read_text(&rels_name)? else {
            return Ok(Vec::new());
        };
        let mut parser = Parser::new(&text, &rels_name);
        let mut out = Vec::new();
        loop {
            match parser.next()? {
                Event::Start(e) if e.name == "Relationship" => {
                    let external = e.attr("TargetMode").is_some_and(|m| m.eq_ignore_ascii_case("External"));
                    let (Some(id), Some(kind), Some(target)) = (e.attr("Id"), e.attr("Type"), e.attr("Target")) else {
                        continue;
                    };
                    out.push(Relationship {
                        id: id.to_owned(),
                        kind: kind.rsplit('/').next().unwrap_or(kind).to_owned(),
                        target: if external { target.to_owned() } else { resolve(dir, target) },
                        external,
                    });
                }
                Event::Eof => return Ok(out),
                _ => {}
            }
        }
    }

    /// The content type declared for `part` in `[Content_Types].xml`.
    pub fn content_type(&mut self, part: &str) -> Result<Option<String>, Error> {
        let Some(text) = self.read_text("[Content_Types].xml")? else {
            return Ok(None);
        };
        let wanted = format!("/{part}");
        let ext = part.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase());
        let mut parser = Parser::new(&text, "[Content_Types].xml");
        let mut by_default = None;
        let mut overridden = None;
        // Read to the end, so a truncated part fails rather than answering
        // from its first half.
        loop {
            match parser.next()? {
                Event::Start(e) if e.name == "Override" => {
                    if overridden.is_none() && e.attr("PartName").is_some_and(|p| p.eq_ignore_ascii_case(&wanted)) {
                        overridden = Some(e.attr("ContentType").map(str::to_owned));
                    }
                }
                Event::Start(e) if e.name == "Default" => {
                    if e.attr("Extension").map(str::to_ascii_lowercase) == ext {
                        by_default = e.attr("ContentType").map(str::to_owned);
                    }
                }
                Event::Eof => return Ok(overridden.unwrap_or(by_default)),
                _ => {}
            }
        }
    }
}

/// A part's text: UTF-8 (a leading BOM dropped), or UTF-16 when a BOM says
/// so, which XML allows and some writers use. `None` for malformed text.
fn decode(bytes: Vec<u8>) -> Option<String> {
    let utf16 = |rest: &[u8], unit: fn([u8; 2]) -> u16| {
        if !rest.len().is_multiple_of(2) {
            return None;
        }
        let units: Vec<u16> = rest.chunks_exact(2).map(|c| unit([c[0], c[1]])).collect();
        String::from_utf16(&units).ok()
    };
    match bytes.as_slice() {
        [0xFF, 0xFE, rest @ ..] => utf16(rest, u16::from_le_bytes),
        [0xFE, 0xFF, rest @ ..] => utf16(rest, u16::from_be_bytes),
        _ => {
            let mut text = String::from_utf8(bytes).ok()?;
            if text.starts_with('\u{feff}') {
                text.drain(..'\u{feff}'.len_utf8());
            }
            Some(text)
        }
    }
}

/// Resolves a relationship target against the source part's directory.
fn resolve(dir: &str, target: &str) -> String {
    let joined = if let Some(abs) = target.strip_prefix('/') {
        abs.to_owned()
    } else if dir.is_empty() {
        target.to_owned()
    } else {
        format!("{dir}/{target}")
    };
    let mut parts: Vec<&str> = Vec::new();
    for seg in joined.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    parts.join("/")
}

#[cfg(test)]
mod tests {
    use super::{Limits, Package, resolve};
    use crate::ErrorCode;

    #[test]
    fn targets_resolve_relative_and_absolute() {
        assert_eq!(resolve("ppt", "slides/slide1.xml"), "ppt/slides/slide1.xml");
        assert_eq!(resolve("ppt/slides", "../slideLayouts/slideLayout2.xml"), "ppt/slideLayouts/slideLayout2.xml");
        assert_eq!(resolve("ppt", "/ppt/theme/theme1.xml"), "ppt/theme/theme1.xml");
        assert_eq!(resolve("", "ppt/presentation.xml"), "ppt/presentation.xml");
    }

    #[test]
    fn parts_may_be_utf16() {
        let le: Vec<u8> =
            [0xFF, 0xFE].into_iter().chain("<a>é</a>".encode_utf16().flat_map(u16::to_le_bytes)).collect();
        let be: Vec<u8> =
            [0xFE, 0xFF].into_iter().chain("<a>é</a>".encode_utf16().flat_map(u16::to_be_bytes)).collect();
        assert_eq!(super::decode(le).as_deref(), Some("<a>é</a>"));
        assert_eq!(super::decode(be).as_deref(), Some("<a>é</a>"));
        assert_eq!(super::decode(vec![0xFF, 0xFE, 0x41]), None);
        assert_eq!(super::decode(b"\xEF\xBB\xBF<a/>".to_vec()).as_deref(), Some("<a/>"));
        assert_eq!(super::decode(vec![0xC3]), None);
    }

    #[test]
    fn non_zip_input_is_classified() {
        let ole = [0xD0u8, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1, 0, 0];
        let err = Package::from_bytes(ole.to_vec(), &Limits::default()).err().unwrap();
        assert_eq!(err.code, ErrorCode::UnsupportedFormat);
        let err = Package::from_bytes(b"hello".to_vec(), &Limits::default()).err().unwrap();
        assert_eq!(err.code, ErrorCode::InvalidArchive);
    }
}
