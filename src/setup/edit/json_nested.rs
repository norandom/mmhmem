//! Edits inside one object element of a shared JSON array (follow-up G1,
//! `.kiro/specs/reliable-installers/research.md` "2026-11-06 G1/G3"): a
//! Claude Code or Codex hook group in `hooks.PreToolUse` can hold the user's
//! handlers next to mmhmem's, so setup edits the handler, never the group.
//!
//! An element is named by its index in the array at `path` (`outer`) and,
//! for a handler, by the key of the array inside it and the index there
//! (`inner`, e.g. `Some(("hooks", 1))`). The parent module's guarantees hold:
//! every other byte is kept, the result is parsed again (strictly for strict
//! input), and errors never quote document text.

use jsonc_parser::cst::{CstObject, CstRootNode};
use serde_json::Value;

use super::array::find_array;
use super::{
    JsonEditError, finish, is_strict, parse_root, remove_node, split_bom, to_input, value_from_text,
};

/// Where an object inside the array at a path is: element `outer`, then
/// optionally element `.1` of the array under key `.0` inside it.
pub type Inner<'a> = Option<(&'a str, usize)>;

/// The object at `outer`/`inner`, `None` when any step is absent or not the
/// expected kind.
fn locate(
    root: &CstRootNode,
    path: &[&str],
    outer: usize,
    inner: Inner<'_>,
) -> Result<Option<CstObject>, JsonEditError> {
    let Some(array) = find_array(root, path)? else {
        return Ok(None);
    };
    let Some(element) = array.elements().get(outer).and_then(|e| e.as_object()) else {
        return Ok(None);
    };
    let Some((key, index)) = inner else {
        return Ok(Some(element));
    };
    let items = element.get(key).and_then(|p| p.array_value());
    Ok(items.and_then(|a| a.elements().get(index).and_then(|e| e.as_object())))
}

/// Sets `key` to `value` in the object at `outer`/`inner`: an existing value
/// is replaced in place (its key, comma and comments stay), a missing key is
/// appended. An absent object or an equal value returns the input unchanged.
pub fn nested_set(
    input: &str,
    path: &[&str],
    outer: usize,
    inner: Inner<'_>,
    key: &str,
    value: &Value,
) -> Result<String, JsonEditError> {
    let (bom, body) = split_bom(input);
    let root = parse_root(body)?;
    let Some(object) = locate(&root, path, outer, inner)? else {
        return Ok(input.to_owned());
    };
    match object.get(key) {
        Some(prop) => {
            let current = prop.value().map(|v| v.to_string());
            if current.map(|t| value_from_text(&t)).transpose()?.as_ref() == Some(value) {
                return Ok(input.to_owned());
            }
            prop.set_value(to_input(value));
        }
        None => {
            object.append(key, to_input(value));
        }
    }
    finish(bom, &root, is_strict(input)?)
}

/// Removes `key` from the object at `outer`/`inner` with its dangling comma.
/// An absent object or key returns the input unchanged.
pub fn nested_remove_key(
    input: &str,
    path: &[&str],
    outer: usize,
    inner: Inner<'_>,
    key: &str,
) -> Result<String, JsonEditError> {
    let (bom, body) = split_bom(input);
    let root = parse_root(body)?;
    let Some(prop) = locate(&root, path, outer, inner)?.and_then(|o| o.get(key)) else {
        return Ok(input.to_owned());
    };
    let strict = is_strict(input)?;
    remove_node(prop.into());
    finish(bom, &root, strict)
}

/// Removes element `index` of the array under `key` in element `outer` of
/// the array at `path`, with its dangling comma; the inner array is kept even
/// when it becomes empty. Anything absent returns the input unchanged.
pub fn nested_remove(
    input: &str,
    path: &[&str],
    outer: usize,
    key: &str,
    index: usize,
) -> Result<String, JsonEditError> {
    let (bom, body) = split_bom(input);
    let root = parse_root(body)?;
    let Some(object) = locate(&root, path, outer, None)? else {
        return Ok(input.to_owned());
    };
    let items = object.get(key).and_then(|p| p.array_value());
    let Some(element) = items.and_then(|a| a.elements().into_iter().nth(index)) else {
        return Ok(input.to_owned());
    };
    let strict = is_strict(input)?;
    remove_node(element);
    finish(bom, &root, strict)
}
