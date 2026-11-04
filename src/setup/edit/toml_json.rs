//! Read-only JSON view of one TOML item (task 2.4 follow-up, design.md §3
//! "Entry-level hashes"): the install record hashes mmhmem's whole table,
//! including keys the user added, so it needs every key, not the lossy
//! [`super::McpServerEntry`].
//!
//! Conversion rule: tables, inline tables and dotted keys become objects;
//! arrays of tables become arrays of objects; strings, integers and booleans
//! map directly; floats become `{"$float":"<Rust Debug repr>"}` and
//! date-times `{"$datetime":"<TOML text>"}`, so no TOML type is confused
//! with a JSON string or loses precision. Layout, comments and key order are
//! not part of the result (callers sort keys when they need a canonical form).

use serde_json::Value as Json;
use toml_edit::{Item, Value};

use super::{TomlEditError, item_at, parse};

/// The item at `path` as JSON (see the module rule), or `None` when it is
/// absent. Errors carry positions only.
pub fn get_item_json(input: &str, path: &[&str]) -> Result<Option<Json>, TomlEditError> {
    if path.is_empty() {
        return Err(TomlEditError::EmptyPath);
    }
    let doc = parse(input)?;
    Ok(item_at(doc.as_item(), path)
        .filter(|item| !item.is_none())
        .map(item_json))
}

fn item_json(item: &Item) -> Json {
    match item {
        Item::None => Json::Null,
        Item::Value(v) => value_json(v),
        Item::Table(t) => Json::Object(
            t.iter()
                .map(|(k, v)| (k.to_owned(), item_json(v)))
                .collect(),
        ),
        Item::ArrayOfTables(a) => Json::Array(
            a.iter()
                .map(|t| {
                    Json::Object(
                        t.iter()
                            .map(|(k, v)| (k.to_owned(), item_json(v)))
                            .collect(),
                    )
                })
                .collect(),
        ),
    }
}

fn value_json(value: &Value) -> Json {
    let tagged = |tag: &str, text: String| {
        Json::Object([(tag.to_owned(), Json::String(text))].into_iter().collect())
    };
    match value {
        Value::String(s) => Json::String(s.value().clone()),
        Value::Integer(i) => Json::from(*i.value()),
        Value::Float(f) => tagged("$float", format!("{:?}", f.value())),
        Value::Boolean(b) => Json::Bool(*b.value()),
        Value::Datetime(d) => tagged("$datetime", d.value().to_string()),
        Value::Array(a) => Json::Array(a.iter().map(value_json).collect()),
        Value::InlineTable(t) => Json::Object(
            t.iter()
                .map(|(k, v)| (k.to_owned(), value_json(v)))
                .collect(),
        ),
    }
}
