//! mmhmem's delimited section in a Markdown file, shared by every agent
//! (reliable-installers tasks 3.2 and 3.5): found, written and removed
//! exactly as `deploy/manage_routing.ps1` and `deploy/manage_routing.py` do.
//!
//! **Format.** The start marker line, a `prefix=` line, the body in the
//! file's newline style (CRLF when the file has any CRLF, else LF), the end
//! marker line, each ended by that newline. The `prefix` (`none`, `lf`,
//! `crlf`) names the newline mmhmem added before the section when the file
//! did not end with one, so removal gives back the user's bytes exactly. A
//! UTF-8 BOM stays in front. Markers that repeat, are out of order or are
//! not closed are [`AmbiguousMarkers`]; nothing is guessed.

use std::fmt;

use crate::setup::record::SectionId;

/// The id of the routing-instructions section every agent uses.
pub fn routing() -> SectionId {
    SectionId::try_from("routing".to_owned()).expect("static section id")
}

/// The fixed reason for [`AmbiguousMarkers`].
pub const AMBIGUOUS_MARKERS: &str = "mmhmem's section markers are repeated or not closed";

/// mmhmem's section markers are repeated, out of order, not closed, or the
/// recorded prefix does not match the bytes before the section.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AmbiguousMarkers;

impl fmt::Display for AmbiguousMarkers {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(AMBIGUOUS_MARKERS)
    }
}

impl std::error::Error for AmbiguousMarkers {}

/// Where the section is: `from..to` (with the owned newline before it) and
/// that newline.
struct Found {
    from: usize,
    to: usize,
    prefix: &'static str,
}

const PREFIXES: [(&str, &str); 3] = [("none", ""), ("lf", "\n"), ("crlf", "\r\n")];

fn prefix_line(id: &SectionId, name: &str) -> String {
    format!("<!-- mmhmem:{}:prefix={name} -->", id.as_str())
}

fn tag(id: &SectionId) -> String {
    format!("mmhmem:{}:", id.as_str())
}

/// The owned newline a `prefix=` line names, `None` for any other line.
fn prefix_of(id: &SectionId, line: &str) -> Option<&'static str> {
    PREFIXES
        .iter()
        .find(|(name, _)| line == prefix_line(id, name))
        .map(|(_, newline)| *newline)
}

fn prefix_name(newline: &str) -> &'static str {
    PREFIXES
        .iter()
        .find(|(_, n)| *n == newline)
        .map_or("none", |(name, _)| name)
}

/// Finds the section; `Ok(None)` when it is absent.
fn find_section(text: &str, id: &SectionId) -> Result<Option<Found>, AmbiguousMarkers> {
    let (start, end, tag) = (id.start_marker(), id.end_marker(), tag(id));
    let (mut first, mut last, mut prefix, mut meta) = (None, None, "", false);
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let at = offset;
        offset += line.len();
        let bare = line.trim_end_matches(['\r', '\n']);
        if !bare.contains(&tag) {
            continue;
        }
        let open = first.is_some() && last.is_none();
        if bare == start && first.is_none() {
            first = Some(at);
        } else if bare == end && open {
            last = Some(offset);
        } else if let (true, false, Some(p)) = (open, meta, prefix_of(id, bare)) {
            (prefix, meta) = (p, true);
        } else {
            return Err(AmbiguousMarkers);
        }
    }
    let (from, to) = match (first, last) {
        (None, None) => return Ok(None),
        (Some(from), Some(to)) => (from, to),
        _ => return Err(AmbiguousMarkers),
    };
    let head = &text[..from];
    let crlf_clash = prefix == "\n" && head.ends_with("\r\n");
    if !head.ends_with(prefix) || crlf_clash {
        return Err(AmbiguousMarkers);
    }
    let from = from - prefix.len();
    Ok(Some(Found { from, to, prefix }))
}

fn split_bom(text: &str) -> (&str, &str) {
    text.strip_prefix('\u{feff}')
        .map_or(("", text), |rest| ("\u{feff}", rest))
}

/// `text` with section `id` holding `body` added at the end or replaced in
/// place. A `body` holding mmhmem's marker tag is refused.
pub fn install_section(text: &str, id: &SectionId, body: &str) -> Result<String, AmbiguousMarkers> {
    let (bom, text) = split_bom(text);
    let found = find_section(text, id)?;
    let nl = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let body = body
        .trim_end_matches(['\r', '\n'])
        .replace("\r\n", "\n")
        .replace('\n', nl);
    if body.contains(&tag(id)) {
        return Err(AmbiguousMarkers);
    }
    let sep = match &found {
        Some(f) => f.prefix,
        None if !text.is_empty() && !text.ends_with('\n') => nl,
        None => "",
    };
    let block = format!(
        "{sep}{}{nl}{}{nl}{body}{nl}{}{nl}",
        id.start_marker(),
        prefix_line(id, prefix_name(sep)),
        id.end_marker()
    );
    Ok(match found {
        Some(f) => format!("{bom}{}{block}{}", &text[..f.from], &text[f.to..]),
        None => format!("{bom}{text}{block}"),
    })
}

/// `text` without section `id` (unchanged when it is absent).
pub fn remove_section(text: &str, id: &SectionId) -> Result<String, AmbiguousMarkers> {
    let (bom, rest) = split_bom(text);
    Ok(match find_section(rest, id)? {
        Some(f) => format!("{bom}{}{}", &rest[..f.from], &rest[f.to..]),
        None => text.to_owned(),
    })
}

#[cfg(test)]
#[path = "markdown_tests.rs"]
mod tests;
