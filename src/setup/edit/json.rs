//! JSON and JSONC entry edits on the `jsonc-parser` CST (task 2.2,
//! `.kiro/specs/reliable-installers/design.md` §4 and §5).
//!
//! Every function works on the document text in memory and never touches the
//! file system. Edits go through the concrete syntax tree
//! ([`jsonc_parser::cst::CstRootNode`]), so every byte outside the edited
//! entry (comments, whitespace, key order, trailing commas, CRLF line endings
//! and a UTF-8 byte order mark) is kept. Each result is parsed again before it
//! is returned, and a strict JSON input must still be strict JSON afterwards.
//! Errors carry a line, a column and a short fixed message, never document
//! text, because config files can hold secrets (research.md §2).

use std::fmt;

use jsonc_parser::cst::{
    CstContainerNode, CstInputValue, CstNode, CstObject, CstObjectProp, CstRootNode,
};
use jsonc_parser::errors::{ParseError, ParseErrorKind};
use jsonc_parser::{JsonValue, ParseOptions, parse_to_value};
use serde_json::Value;

const BOM: char = '\u{feff}';

/// Whether a document is plain JSON or uses JSONC extensions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    /// Strict JSON (RFC 8259): no comments, trailing commas or other extensions.
    Json,
    /// JSON with comments, trailing commas or other JSONC/JSON5 extensions.
    Jsonc,
}

/// Why a JSON edit was refused. Messages never contain document text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JsonEditError {
    /// The input document does not parse.
    Parse {
        /// 1-based line of the problem.
        line: usize,
        /// 1-based column of the problem.
        column: usize,
        /// Short, fixed description of the problem.
        message: String,
    },
    /// The edit produced text that does not parse (or that is no longer strict
    /// JSON when the input was); nothing should be written.
    InvalidOutput {
        /// Short, fixed description of the problem.
        message: String,
    },
    /// The path was empty.
    EmptyPath,
    /// The document's top-level value is not an object.
    RootNotObject,
    /// The key at this 0-based path index holds a value that is not an object,
    /// so the path cannot continue through it.
    NotAnObject {
        /// Index into the path of the offending key.
        index: usize,
    },
    /// The value at the path exists but is not an array.
    NotAnArray,
    /// A number in the document cannot be represented as a JSON number
    /// (for example a hexadecimal JSONC literal).
    UnsupportedNumber,
}

impl fmt::Display for JsonEditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse {
                line,
                column,
                message,
            } => write!(
                f,
                "not valid JSON (line {line}, column {column}: {message})"
            ),
            Self::InvalidOutput { message } => {
                write!(f, "the edited JSON would not be valid: {message}")
            }
            Self::EmptyPath => write!(f, "no key was given to edit"),
            Self::RootNotObject => write!(f, "the top-level JSON value is not an object"),
            Self::NotAnObject { index } => {
                write!(f, "key number {} in the path is not an object", index + 1)
            }
            Self::NotAnArray => write!(f, "the value at the path is not a list"),
            Self::UnsupportedNumber => write!(f, "a number in the file is not plain JSON"),
        }
    }
}

impl std::error::Error for JsonEditError {}

/// Reports whether `input` is strict JSON or needs the JSONC parser.
///
/// Callers use this to avoid introducing JSONC features into a `.json` file.
pub fn dialect(input: &str) -> Result<Dialect, JsonEditError> {
    let (_, body) = split_bom(input);
    if parse_to_value(body, &strict_options()).is_ok() {
        return Ok(Dialect::Json);
    }
    parse_to_value(body, &jsonc_options()).map_err(parse_error)?;
    Ok(Dialect::Jsonc)
}

/// Returns the value at `path` (a list of object keys), or `None` when any key
/// along the path is absent or a parent is not an object. An empty path
/// returns the whole document value.
pub fn get(input: &str, path: &[&str]) -> Result<Option<Value>, JsonEditError> {
    let (_, body) = split_bom(input);
    let root = parse_root(body)?;
    let text = if path.is_empty() {
        root.value().map(|node| node.to_string())
    } else {
        find_prop(&root, path).and_then(|prop| prop.value().map(|node| node.to_string()))
    };
    match text {
        Some(text) => value_from_text(&text).map(Some),
        None => Ok(None),
    }
}

/// Inserts or replaces the value at `path`, creating missing parent objects.
///
/// Only the edited entry changes: a replaced value keeps its key, comma and
/// surrounding comments, and a new entry is appended to its parent object
/// using the document's indentation, newline style and trailing-comma style.
/// Setting a value equal to the current one returns the input unchanged.
pub fn set(input: &str, path: &[&str], value: &Value) -> Result<String, JsonEditError> {
    let (bom, body) = split_bom(input);
    let root = parse_root(body)?;
    let Some((last, parents)) = path.split_last() else {
        return Err(JsonEditError::EmptyPath);
    };
    if get(input, path)?.as_ref() == Some(value) {
        return Ok(input.to_owned());
    }
    let strict = is_strict(input)?;
    let mut current = root
        .object_value_or_create()
        .ok_or(JsonEditError::RootNotObject)?;
    for (index, key) in parents.iter().enumerate() {
        match current.get(key) {
            Some(prop) => {
                current = prop
                    .object_value()
                    .ok_or(JsonEditError::NotAnObject { index })?;
            }
            None => {
                current.append(key, nested_input(&path[index + 1..], value));
                return finish(bom, &root, strict);
            }
        }
    }
    match current.get(last) {
        Some(prop) => prop.set_value(to_input(value)),
        None => {
            current.append(last, to_input(value));
        }
    }
    finish(bom, &root, strict)
}

/// Removes the entry at `path` together with its now-dangling comma and the
/// rest of its line. An absent entry (or a non-object parent) leaves the input
/// unchanged. Parent objects are kept even when they become empty.
pub fn remove(input: &str, path: &[&str]) -> Result<String, JsonEditError> {
    remove_inner(input, path, 0)
}

/// Like [`remove`], but afterwards also removes up to `max_parents` parent
/// objects along `path`, deepest first, while each is left without properties
/// or comments. Passing the [`missing_parents`] count taken before a [`set`]
/// undoes the parents that `set` created and nothing else. The top-level
/// object itself is always kept.
pub fn remove_and_prune(
    input: &str,
    path: &[&str],
    max_parents: usize,
) -> Result<String, JsonEditError> {
    remove_inner(input, path, max_parents)
}

/// Counts the parent objects of `path` that are absent, i.e. how many objects
/// [`set`] would create. Returns 0 when the parents exist or `path` is empty.
pub fn missing_parents(input: &str, path: &[&str]) -> Result<usize, JsonEditError> {
    let (_, body) = split_bom(input);
    let root = parse_root(body)?;
    let parents = path.len().saturating_sub(1);
    let Some(mut current) = root.object_value() else {
        return Ok(parents);
    };
    for (index, key) in path.iter().take(parents).enumerate() {
        match current.get(key).and_then(|prop| prop.object_value()) {
            Some(object) => current = object,
            None => return Ok(parents - index),
        }
    }
    Ok(0)
}

fn remove_inner(input: &str, path: &[&str], max_parents: usize) -> Result<String, JsonEditError> {
    let (bom, body) = split_bom(input);
    let root = parse_root(body)?;
    if path.is_empty() {
        return Err(JsonEditError::EmptyPath);
    }
    let Some(prop) = find_prop(&root, path) else {
        return Ok(input.to_owned());
    };
    let strict = is_strict(input)?;
    remove_node(prop.into());
    prune_empty_parents(&root, path, max_parents);
    finish(bom, &root, strict)
}

/// Removes up to `max_parents` parent objects along `path` (deepest first)
/// that hold nothing.
fn prune_empty_parents(root: &CstRootNode, path: &[&str], max_parents: usize) {
    for depth in (1..path.len()).rev().take(max_parents) {
        let Some(prop) = find_prop(root, &path[..depth]) else {
            return;
        };
        let Some(object) = prop.object_value() else {
            return;
        };
        if !is_bare_empty(&object) {
            return;
        }
        prop.remove();
    }
}

/// True when the object has no properties and no comments inside.
fn is_bare_empty(object: &CstObject) -> bool {
    object.properties().is_empty() && !object.children().iter().any(|child| child.is_comment())
}

/// Finds the property at `path`, or `None` when any step is missing.
fn find_prop(root: &CstRootNode, path: &[&str]) -> Option<CstObjectProp> {
    let (last, parents) = path.split_last()?;
    let mut current = root.object_value()?;
    for key in parents {
        current = current.get(key)?.object_value()?;
    }
    current.get(last)
}

/// Serializes the edited tree, re-parses it and restores the BOM.
fn finish(bom: &str, root: &CstRootNode, strict: bool) -> Result<String, JsonEditError> {
    let body = root.to_string();
    let options = if strict {
        strict_options()
    } else {
        jsonc_options()
    };
    if let Err(err) = parse_to_value(&body, &options) {
        return Err(JsonEditError::InvalidOutput {
            message: safe_message(err.kind()),
        });
    }
    Ok(format!("{bom}{body}"))
}

/// True when the input is strict JSON, so the output must stay strict.
fn is_strict(input: &str) -> Result<bool, JsonEditError> {
    Ok(dialect(input)? == Dialect::Json)
}

fn parse_root(body: &str) -> Result<CstRootNode, JsonEditError> {
    CstRootNode::parse(body, &jsonc_options()).map_err(parse_error)
}

/// Splits a leading UTF-8 BOM off; the parser does not accept one.
fn split_bom(input: &str) -> (&str, &str) {
    match input.strip_prefix(BOM) {
        Some(body) => (&input[..BOM.len_utf8()], body),
        None => ("", input),
    }
}

fn strict_options() -> ParseOptions {
    ParseOptions {
        allow_comments: false,
        allow_loose_object_property_names: false,
        allow_trailing_commas: false,
        allow_missing_commas: false,
        allow_single_quoted_strings: false,
        allow_hexadecimal_numbers: false,
        allow_unary_plus_numbers: false,
        allow_bare_decimal_point_numbers: false,
        allow_non_finite_numbers: false,
        allow_extended_string_escapes: false,
    }
}

/// JSONC as the agents read it: strict JSON plus comments and trailing
/// commas. The parser's wider defaults (missing commas, JSON5 literals) would
/// accept files the agents themselves reject.
fn jsonc_options() -> ParseOptions {
    ParseOptions {
        allow_comments: true,
        allow_trailing_commas: true,
        ..strict_options()
    }
}

/// After removing the first entry of a one-line container, drops the space
/// it leaves behind the opening bracket (`[ 2, 3]` becomes `[2, 3]`).
fn trim_after_open(container: &CstContainerNode) {
    let children = container.children();
    if let (Some(space), Some(next)) = (children.get(1), children.get(2))
        && space.is_whitespace()
        && !next.is_newline()
        && !next.is_comment()
    {
        space.clone().remove();
    }
}

/// Removes an object property or array element with its dangling comma. When
/// it sat directly after the opening bracket, the space the library leaves
/// there is removed too.
fn remove_node(node: CstNode) {
    let tight = node
        .previous_sibling()
        .is_some_and(|prev| matches!(prev.token_char(), Some('[' | '{')));
    let parent = node.parent();
    node.remove();
    if let Some(parent) = parent.filter(|_| tight) {
        trim_after_open(&parent);
    }
}

fn parse_error(err: ParseError) -> JsonEditError {
    JsonEditError::Parse {
        line: err.line_display(),
        column: err.column_display(),
        message: safe_message(err.kind()),
    }
}

/// Parser messages are fixed text except string errors, which can quote input.
fn safe_message(kind: &ParseErrorKind) -> String {
    match kind {
        ParseErrorKind::String(_) => "Invalid string literal".to_owned(),
        other => other.to_string(),
    }
}

/// Builds `{ "a": { "b": value } }` for the remaining path keys.
fn nested_input(keys: &[&str], value: &Value) -> CstInputValue {
    keys.iter().rev().fold(to_input(value), |inner, key| {
        CstInputValue::Object(vec![((*key).to_owned(), inner)])
    })
}

fn to_input(value: &Value) -> CstInputValue {
    match value {
        Value::Null => CstInputValue::Null,
        Value::Bool(b) => CstInputValue::Bool(*b),
        Value::Number(n) => CstInputValue::Number(n.to_string()),
        Value::String(s) => CstInputValue::String(s.clone()),
        Value::Array(items) => CstInputValue::Array(items.iter().map(to_input).collect()),
        Value::Object(map) => CstInputValue::Object(
            map.iter()
                .map(|(key, item)| (key.clone(), to_input(item)))
                .collect(),
        ),
    }
}

fn value_from_text(text: &str) -> Result<Value, JsonEditError> {
    match parse_to_value(text, &jsonc_options()).map_err(parse_error)? {
        Some(value) => from_jsonc(value),
        None => Ok(Value::Null),
    }
}

fn from_jsonc(value: JsonValue<'_>) -> Result<Value, JsonEditError> {
    Ok(match value {
        JsonValue::Null => Value::Null,
        JsonValue::Boolean(b) => Value::Bool(b),
        JsonValue::String(s) => Value::String(s.into_owned()),
        JsonValue::Number(raw) => Value::Number(
            raw.parse::<serde_json::Number>()
                .map_err(|_| JsonEditError::UnsupportedNumber)?,
        ),
        JsonValue::Array(items) => Value::Array(
            items
                .into_iter()
                .map(from_jsonc)
                .collect::<Result<_, _>>()?,
        ),
        JsonValue::Object(object) => Value::Object(
            object
                .into_iter()
                .map(|(key, item)| Ok((key.into_owned(), from_jsonc(item)?)))
                .collect::<Result<_, JsonEditError>>()?,
        ),
    })
}

#[path = "json_array.rs"]
mod array;
pub use array::{array_append, array_find, array_remove, array_remove_and_prune, missing_keys};

#[path = "json_nested.rs"]
mod nested;
pub use nested::{Inner, nested_remove, nested_remove_key, nested_set};

#[cfg(test)]
#[path = "json_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "json_array_tests.rs"]
mod array_tests;
