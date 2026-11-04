//! Element-level array edits for JSON/JSONC documents (task 2.2, design.md
//! §5: Claude Code hook groups under `hooks.PreToolUse` share the array with
//! the user's own groups, so only our element may change).
//!
//! The same guarantees as the parent module apply: other elements stay
//! byte-identical, the result is parsed again (strictly when the input was
//! strict JSON), and errors never quote document text.

use jsonc_parser::cst::{CstArray, CstRootNode};
use serde_json::Value;

use super::{
    JsonEditError, find_prop, finish, is_strict, parse_root, prune_empty_parents, remove_node, set,
    split_bom, to_input, value_from_text,
};

/// Appends `value` to the array at `path`, creating the array and any missing
/// parent objects. Existing elements are untouched; the new element follows
/// the indentation, newline and trailing-comma style of the document.
pub fn array_append(input: &str, path: &[&str], value: &Value) -> Result<String, JsonEditError> {
    let (bom, body) = split_bom(input);
    let root = parse_root(body)?;
    let Some(array) = find_array(&root, path)? else {
        return set(input, path, &Value::Array(vec![value.clone()]));
    };
    let strict = is_strict(input)?;
    array.append(to_input(value));
    finish(bom, &root, strict)
}

/// Returns the indices of the elements of the array at `path` for which
/// `predicate` is true, in document order. An absent array gives no indices.
pub fn array_find(
    input: &str,
    path: &[&str],
    predicate: impl Fn(&Value) -> bool,
) -> Result<Vec<usize>, JsonEditError> {
    let (_, body) = split_bom(input);
    let root = parse_root(body)?;
    let Some(array) = find_array(&root, path)? else {
        return Ok(Vec::new());
    };
    let mut found = Vec::new();
    for (index, element) in array.elements().iter().enumerate() {
        if predicate(&value_from_text(&element.to_string())?) {
            found.push(index);
        }
    }
    Ok(found)
}

/// Removes element `index` of the array at `path` with its dangling comma.
/// An absent array or an index out of range returns the input unchanged.
/// The array is kept even when it becomes empty.
pub fn array_remove(input: &str, path: &[&str], index: usize) -> Result<String, JsonEditError> {
    array_remove_and_prune(input, path, index, 0)
}

/// Like [`array_remove`], then removes up to `max_levels` containers along
/// `path`, deepest first (the array itself, then its parent objects), while
/// each is left without entries or comments. Passing the [`missing_keys`]
/// count taken before [`array_append`] undoes exactly what it created.
pub fn array_remove_and_prune(
    input: &str,
    path: &[&str],
    index: usize,
    max_levels: usize,
) -> Result<String, JsonEditError> {
    let (bom, body) = split_bom(input);
    let root = parse_root(body)?;
    let Some(array) = find_array(&root, path)? else {
        return Ok(input.to_owned());
    };
    let Some(element) = array.elements().into_iter().nth(index) else {
        return Ok(input.to_owned());
    };
    let strict = is_strict(input)?;
    remove_node(element);
    if max_levels > 0 && is_bare_empty_array(&array) {
        if let Some(prop) = find_prop(&root, path) {
            prop.remove();
        }
        prune_empty_parents(&root, path, max_levels - 1);
    }
    finish(bom, &root, strict)
}

/// Counts how many keys of `path` are absent, the last key included: the
/// number of containers [`array_append`] (or a `set`) would create.
pub fn missing_keys(input: &str, path: &[&str]) -> Result<usize, JsonEditError> {
    let (_, body) = split_bom(input);
    let root = parse_root(body)?;
    let Some(mut current) = root.object_value() else {
        return Ok(path.len());
    };
    for (index, key) in path.iter().enumerate() {
        let Some(prop) = current.get(key) else {
            return Ok(path.len() - index);
        };
        match prop.object_value() {
            Some(object) => current = object,
            None => return Ok(0),
        }
    }
    Ok(0)
}

/// The array at `path`, `None` when absent, or an error when the value there
/// is not an array.
fn find_array(root: &CstRootNode, path: &[&str]) -> Result<Option<CstArray>, JsonEditError> {
    if path.is_empty() {
        return Err(JsonEditError::EmptyPath);
    }
    match find_prop(root, path) {
        None => Ok(None),
        Some(prop) => prop
            .array_value()
            .map(Some)
            .ok_or(JsonEditError::NotAnArray),
    }
}

fn is_bare_empty_array(array: &CstArray) -> bool {
    array.elements().is_empty() && !array.children().iter().any(|child| child.is_comment())
}
