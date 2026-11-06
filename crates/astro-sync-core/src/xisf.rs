//! The first header of an XISF frame (PixInsight's format), read without a
//! dependency and without a pixel.
//!
//! An XISF file starts with the signature `XISF0100`, the length of its XML
//! header (32-bit little-endian), four reserved bytes, then the header
//! itself. Capture and processing software copy the FITS keywords of the
//! frame there, one `<FITSKeyword name="…" value="…" comment="…"/>` element
//! each, values written the FITS way (`'LIGHT'`). Some files carry only
//! XISF properties (`<Property id="Instrument:Filter:Name" … value="Ha"/>`):
//! the few the detection needs are read as their FITS equivalents when the
//! keyword itself is absent. The result is a [`FitsHeader`], so the rest of
//! Astro Sync reads both formats the same way.

use std::io::Read;
use std::path::Path;

use crate::analysis::fits::{FitsCard, FitsHeader};

/// `XISF0100`: the signature of the monolithic XISF 1.0 format.
pub const SIGNATURE: &[u8; 8] = b"XISF0100";
/// A header longer than this is refused (real ones are a few kilobytes).
const MAX_HEADER_BYTES: u32 = 16 * 1024 * 1024;

/// The XISF properties read when the FITS keyword is absent, with the
/// keyword they stand for.
const PROPERTY_KEYWORDS: &[(&str, &str)] = &[
    ("Observation:Object:Name", "OBJECT"),
    ("Instrument:Filter:Name", "FILTER"),
    ("Instrument:ExposureTime", "EXPTIME"),
    ("Instrument:Camera:Gain", "GAIN"),
    ("Observation:Time:Start", "DATE-OBS"),
];

#[derive(Debug, PartialEq, Eq)]
pub enum XisfError {
    /// The file does not start with `XISF0100`.
    NotXisf,
    /// The header is cut short, too long, or not UTF-8.
    BadHeader,
}

impl std::fmt::Display for XisfError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            XisfError::NotXisf => f.write_str("not an XISF file (no XISF0100 signature)"),
            XisfError::BadHeader => f.write_str("unreadable XISF header"),
        }
    }
}

impl std::error::Error for XisfError {}

/// Reads the XML header of the XISF file at `path` (only the header's own
/// bytes) and returns its keywords.
pub fn read_header_file(path: &Path) -> std::io::Result<FitsHeader> {
    let invalid = |error: XisfError| std::io::Error::new(std::io::ErrorKind::InvalidData, error);
    let mut file = std::fs::File::open(path)?;
    let mut prefix = [0_u8; 16];
    file.read_exact(&mut prefix)
        .map_err(|_| invalid(XisfError::NotXisf))?;
    if &prefix[..8] != SIGNATURE {
        return Err(invalid(XisfError::NotXisf));
    }
    let length = u32::from_le_bytes([prefix[8], prefix[9], prefix[10], prefix[11]]);
    if length == 0 || length > MAX_HEADER_BYTES {
        return Err(invalid(XisfError::BadHeader));
    }
    let mut xml = vec![0_u8; length as usize];
    file.read_exact(&mut xml)
        .map_err(|_| invalid(XisfError::BadHeader))?;
    let text = std::str::from_utf8(&xml).map_err(|_| invalid(XisfError::BadHeader))?;
    Ok(parse_header_xml(text))
}

/// Parses an XISF XML header into FITS-style cards: every `FITSKeyword`,
/// then the properties of [`PROPERTY_KEYWORDS`] whose keyword is absent.
#[must_use]
pub fn parse_header_xml(xml: &str) -> FitsHeader {
    let mut cards = Vec::new();
    for attributes in elements(xml, "FITSKeyword") {
        let Some(name) = attribute(&attributes, "name") else {
            continue;
        };
        let value = attribute(&attributes, "value").map(|raw| unquote_fits(&raw));
        let comment = attribute(&attributes, "comment").filter(|c| !c.is_empty());
        cards.push(FitsCard {
            keyword: name.trim().to_ascii_uppercase(),
            value,
            comment,
        });
    }
    for (attributes, body) in elements_with_body(xml, "Property") {
        let Some(id) = attribute(&attributes, "id") else {
            continue;
        };
        let Some((_, keyword)) = PROPERTY_KEYWORDS.iter().find(|(p, _)| *p == id) else {
            continue;
        };
        if cards.iter().any(|card| card.keyword == *keyword) {
            continue;
        }
        let value = attribute(&attributes, "value").or_else(|| body.map(|b| decode(b.trim())));
        if let Some(value) = value {
            cards.push(FitsCard {
                keyword: (*keyword).to_string(),
                value: Some(value),
                comment: None,
            });
        }
    }
    FitsHeader { cards }
}

/// A FITS value as XISF writes it: `'LIGHT   '` → `LIGHT` (doubled quotes
/// inside a string are one quote); a number or a logical stays as written.
fn unquote_fits(raw: &str) -> String {
    let trimmed = raw.trim();
    if let Some(inner) = trimmed
        .strip_prefix('\'')
        .and_then(|s| s.strip_suffix('\''))
    {
        return inner.replace("''", "'").trim_end().to_string();
    }
    trimmed.to_string()
}

/// The attribute text of every `<name …>` element of `xml`.
fn elements(xml: &str, name: &str) -> Vec<String> {
    elements_with_body(xml, name)
        .into_iter()
        .map(|(attributes, _)| attributes)
        .collect()
}

/// Every `<name …/>` or `<name …>body</name>` element: its attribute text
/// and its body when it has one.
fn elements_with_body<'a>(xml: &'a str, name: &str) -> Vec<(String, Option<&'a str>)> {
    let open = format!("<{name}");
    let close = format!("</{name}>");
    let mut found = Vec::new();
    let mut rest = xml;
    while let Some(start) = rest.find(&open) {
        let after = &rest[start + open.len()..];
        // `<FITSKeywordX` is another element.
        if !after.starts_with(|c: char| c.is_whitespace() || c == '/' || c == '>') {
            rest = after;
            continue;
        }
        let Some(end) = tag_end(after) else {
            break;
        };
        let tag = &after[..end];
        let self_closing = tag.trim_end().ends_with('/');
        let attributes = tag.trim_end().trim_end_matches('/').to_string();
        rest = &after[end + 1..];
        if self_closing {
            found.push((attributes, None));
        } else if let Some(body_end) = rest.find(&close) {
            found.push((attributes, Some(&rest[..body_end])));
            rest = &rest[body_end + close.len()..];
        } else {
            found.push((attributes, None));
        }
    }
    found
}

/// The index of the `>` closing a tag, quotes honoured.
fn tag_end(text: &str) -> Option<usize> {
    let mut quote: Option<char> = None;
    for (index, c) in text.char_indices() {
        match (quote, c) {
            (None, '"' | '\'') => quote = Some(c),
            (Some(q), _) if c == q => quote = None,
            (None, '>') => return Some(index),
            _ => {}
        }
    }
    None
}

/// The decoded value of attribute `name` in an element's attribute text.
fn attribute(attributes: &str, name: &str) -> Option<String> {
    let mut rest = attributes;
    loop {
        rest = rest.trim_start();
        if rest.is_empty() {
            return None;
        }
        let equals = rest.find('=')?;
        let key = rest[..equals].trim();
        let after = rest[equals + 1..].trim_start();
        let quote = after.chars().next()?;
        if quote != '"' && quote != '\'' {
            return None;
        }
        let body = &after[1..];
        let end = body.find(quote)?;
        if key == name {
            return Some(decode(&body[..end]));
        }
        rest = &body[end + 1..];
    }
}

/// The five XML entities and numeric references.
fn decode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let after = &rest[amp + 1..];
        let Some(semi) = after.find(';') else {
            out.push_str(&rest[amp..]);
            return out;
        };
        let entity = &after[..semi];
        let decoded = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => entity
                .strip_prefix("#x")
                .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                .or_else(|| entity.strip_prefix('#').and_then(|d| d.parse().ok()))
                .and_then(char::from_u32),
        };
        match decoded {
            Some(c) => {
                out.push(c);
                rest = &after[semi + 1..];
            }
            None => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Builds a minimal XISF file around an XML header (tests and fixtures).
#[cfg(test)]
pub(crate) fn build_xisf_bytes(xml: &str) -> Vec<u8> {
    let mut out = SIGNATURE.to_vec();
    out.extend_from_slice(&(xml.len() as u32).to_le_bytes());
    out.extend_from_slice(&[0_u8; 4]);
    out.extend_from_slice(xml.as_bytes());
    out.extend_from_slice(&[0_u8; 64]); // stands for the pixels
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;

    const HEADER: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<xisf version="1.0" xmlns="http://www.pixinsight.com/xisf">
<Image geometry="16:16:1" sampleFormat="UInt16" location="attachment:4096:512">
<FITSKeyword name="IMAGETYP" value="'Light Frame'" comment="Type of exposure"/>
<FITSKeyword name="OBJECT" value="'NGC 7000'" comment=""/>
<FITSKeyword name="EXPTIME" value="120." comment="Exposure time &amp; more"/>
<FITSKeyword name="GAIN" value="100" comment=""/>
<FITSKeyword name="DATE-OBS" value="'2026-10-07T21:30:00.000'" comment=""/>
<Property id="Instrument:Filter:Name" type="String">OIII</Property>
<Property id="Observation:Object:Name" type="String" value="Ignored, the keyword wins"/>
</Image>
</xisf>"#;

    #[test]
    fn reads_the_keywords_and_the_missing_properties_of_an_xisf_header() {
        let dir = TempDir::new("xisf-header");
        let path = dir.path().join("light-0001.xisf");
        std::fs::write(&path, build_xisf_bytes(HEADER)).unwrap();
        let header = read_header_file(&path).unwrap();
        assert_eq!(header.get("IMAGETYP"), Some("Light Frame"));
        assert_eq!(header.get("OBJECT"), Some("NGC 7000"));
        assert_eq!(header.get_f64("EXPTIME"), Some(120.0));
        assert_eq!(header.get_i64("GAIN"), Some(100));
        assert_eq!(header.get("DATE-OBS"), Some("2026-10-07T21:30:00.000"));
        assert_eq!(header.get("FILTER"), Some("OIII"));
        let exptime = header
            .cards
            .iter()
            .find(|c| c.keyword == "EXPTIME")
            .unwrap();
        assert_eq!(exptime.comment.as_deref(), Some("Exposure time & more"));
    }

    #[test]
    fn refuses_a_file_without_the_signature_or_with_a_short_header() {
        let dir = TempDir::new("xisf-refuse");
        let not_xisf = dir.path().join("a.xisf");
        std::fs::write(&not_xisf, b"SIMPLE  =                    T").unwrap();
        assert!(read_header_file(&not_xisf).is_err());
        let short = dir.path().join("b.xisf");
        let mut bytes = build_xisf_bytes(HEADER);
        bytes.truncate(40);
        std::fs::write(&short, bytes).unwrap();
        assert!(read_header_file(&short).is_err());
    }
}
