//! TOML table edits on `toml_edit` (task 2.3).
//!
//! Pure in-memory edits over `&str` for Codex `config.toml`
//! (`[mcp_servers.mmhmem]`, design.md section 4). The parser is used to find
//! exact table spans; edits then splice text, so every byte outside the
//! edited table (comments, blank lines, CRLF, order) stays unchanged. Every
//! result is re-parsed before it is returned. Errors carry positions and
//! fixed messages only; they never echo file contents (which may hold tokens).

use std::fmt;
use std::ops::Range;

use toml_edit::{Array, Document, DocumentMut, Item, Key, Table, Value};

/// Old installer marker line (PowerShell variant).
pub const MARKER_BEGIN_PS1: &str = "# BEGIN mmhmem (managed by install_local.ps1 -- do not edit)";
/// Old installer marker line (shell variant).
pub const MARKER_BEGIN_SH: &str = "# BEGIN mmhmem (managed by install_local.sh -- do not edit)";
/// Old installer closing marker line.
pub const MARKER_END: &str = "# END mmhmem";

/// Why a marker block could not be converted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkerProblem {
    /// A BEGIN without END, or an END without BEGIN (1-based line).
    Unmatched(usize),
    /// A BEGIN inside an open block (1-based line).
    Nested(usize),
    /// The block does not contain the target table.
    NoTable,
}

/// Errors from the TOML editor. Messages never contain input text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TomlEditError {
    /// Invalid TOML at a 1-based line and column.
    Syntax { line: usize, column: usize },
    /// A table or key is defined twice; `is_mmhmem` when it names
    /// `mcp_servers` and `mmhmem`.
    DuplicateTable {
        line: usize,
        column: usize,
        is_mmhmem: bool,
    },
    /// The table exists as dotted keys, an inline table or an array of tables.
    UnsupportedLayout,
    /// A known field has the wrong type.
    UnexpectedType { field: &'static str },
    /// The marker block is malformed.
    MarkerBlock(MarkerProblem),
    /// The edit would have produced invalid TOML; nothing was changed.
    InvalidOutput,
    /// An empty key path was given.
    EmptyPath,
    /// The file mixes CRLF and LF; an in-place update would rewrite them.
    MixedLineEndings,
}

impl fmt::Display for TomlEditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Syntax { line, column } => {
                write!(f, "invalid TOML at line {line}, column {column}")
            }
            Self::DuplicateTable { line, column, .. } => {
                write!(
                    f,
                    "duplicate TOML table or key at line {line}, column {column}"
                )
            }
            Self::UnsupportedLayout => f.write_str("unsupported table layout"),
            Self::UnexpectedType { field } => write!(f, "field `{field}` has an unexpected type"),
            Self::MarkerBlock(MarkerProblem::Unmatched(l)) => {
                write!(f, "unmatched marker at line {l}")
            }
            Self::MarkerBlock(MarkerProblem::Nested(l)) => {
                write!(f, "nested marker at line {l}")
            }
            Self::MarkerBlock(MarkerProblem::NoTable) => f.write_str("marker block has no table"),
            Self::InvalidOutput => f.write_str("edit would make invalid TOML"),
            Self::EmptyPath => f.write_str("empty table path"),
            Self::MixedLineEndings => f.write_str("mixed line endings"),
        }
    }
}

impl std::error::Error for TomlEditError {}

/// The MCP server entry mmhmem manages (confirmed Codex shape, research 8).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct McpServerEntry {
    /// Executable to launch.
    pub command: String,
    /// Arguments.
    pub args: Vec<String>,
    /// Environment variables, in order.
    pub env: Vec<(String, String)>,
    /// Working directory.
    pub cwd: Option<String>,
    /// Startup timeout in seconds.
    pub startup_timeout_sec: Option<i64>,
    /// Whether the server is enabled.
    pub enabled: Option<bool>,
}

fn line_col(input: &str, offset: usize) -> (usize, usize) {
    let end = offset.min(input.len());
    let end = (0..=end)
        .rev()
        .find(|i| input.is_char_boundary(*i))
        .unwrap_or(0);
    let before = &input[..end];
    let line = before.matches('\n').count() + 1;
    let start = before.rfind('\n').map_or(0, |i| i + 1);
    (line, before[start..].chars().count() + 1)
}

fn parse(input: &str) -> Result<Document<&str>, TomlEditError> {
    Document::parse(input).map_err(|e| {
        let offset = e.span().map_or(0, |s| s.start);
        let (line, column) = line_col(input, offset);
        if e.message().contains("duplicate") {
            let text = input.lines().nth(line - 1).unwrap_or("");
            let flat: String = text.split_whitespace().collect();
            let is_mmhmem = flat.contains("mcp_servers") && flat.contains("mmhmem");
            TomlEditError::DuplicateTable {
                line,
                column,
                is_mmhmem,
            }
        } else {
            TomlEditError::Syntax { line, column }
        }
    })
}

/// Check that `input` is valid TOML (positions only in the error).
pub fn validate(input: &str) -> Result<(), TomlEditError> {
    parse(input).map(|_| ())
}

fn checked(output: String) -> Result<String, TomlEditError> {
    match Document::parse(output.as_str()) {
        Ok(_) => Ok(output),
        Err(_) => Err(TomlEditError::InvalidOutput),
    }
}

fn eol_of(input: &str) -> &'static str {
    if input.contains("\r\n") { "\r\n" } else { "\n" }
}

fn str_field(
    t: &dyn toml_edit::TableLike,
    key: &'static str,
) -> Result<Option<String>, TomlEditError> {
    match t.get(key) {
        None => Ok(None),
        Some(i) => i
            .as_str()
            .map(|s| Some(s.to_owned()))
            .ok_or(TomlEditError::UnexpectedType { field: key }),
    }
}

fn entry_from(t: &dyn toml_edit::TableLike) -> Result<McpServerEntry, TomlEditError> {
    let bad = |field| TomlEditError::UnexpectedType { field };
    let mut e = McpServerEntry {
        command: str_field(t, "command")?.unwrap_or_default(),
        cwd: str_field(t, "cwd")?,
        ..McpServerEntry::default()
    };
    if let Some(a) = t.get("args") {
        let arr = a.as_array().ok_or_else(|| bad("args"))?;
        for v in arr {
            e.args
                .push(v.as_str().ok_or_else(|| bad("args"))?.to_owned());
        }
    }
    if let Some(env) = t.get("env") {
        let tbl = env.as_table_like().ok_or_else(|| bad("env"))?;
        for (k, v) in tbl.iter() {
            let s = v.as_str().ok_or_else(|| bad("env"))?;
            e.env.push((k.to_owned(), s.to_owned()));
        }
    }
    if let Some(v) = t.get("startup_timeout_sec") {
        e.startup_timeout_sec = Some(v.as_integer().ok_or_else(|| bad("startup_timeout_sec"))?);
    }
    if let Some(v) = t.get("enabled") {
        e.enabled = Some(v.as_bool().ok_or_else(|| bad("enabled"))?);
    }
    Ok(e)
}

fn item_at<'a>(root: &'a Item, path: &[&str]) -> Option<&'a Item> {
    path.iter().try_fold(root, |item, seg| item.get(*seg))
}

/// Read the table at `path` (for example `["mcp_servers", "mmhmem"]`).
/// Returns `None` when it is absent. Unknown keys are ignored.
pub fn get_table(input: &str, path: &[&str]) -> Result<Option<McpServerEntry>, TomlEditError> {
    if path.is_empty() {
        return Err(TomlEditError::EmptyPath);
    }
    let doc = parse(input)?;
    let Some(item) = item_at(doc.as_item(), path) else {
        return Ok(None);
    };
    match item.as_table_like() {
        Some(t) => entry_from(t).map(Some),
        None => Err(TomlEditError::UnsupportedLayout),
    }
}

/// End offset of the key/value lines directly under a header table
/// (sub-tables are separate regions; trailing comments stay outside).
fn body_end(table: &Table) -> usize {
    let mut end = 0;
    for (_, item) in table {
        let e = match item {
            Item::Value(v) => v.span().map_or(0, |s| s.end),
            Item::Table(t) if t.is_dotted() => body_end(t),
            _ => 0,
        };
        end = end.max(e);
    }
    end
}

/// True when `prefix` plus `key` is exactly `target`.
fn is_target(prefix: &[String], key: &str, target: &[&str]) -> bool {
    prefix.len() + 1 == target.len()
        && prefix.iter().zip(target).all(|(a, b)| a == b)
        && target.last() == Some(&key)
}

fn collect(table: &Table, prefix: &mut Vec<String>, target: &[&str], out: &mut Vec<Range<usize>>) {
    for (k, item) in table {
        match item {
            Item::Table(t) => {
                prefix.push(k.to_owned());
                let under =
                    prefix.len() >= target.len() && prefix.iter().zip(target).all(|(a, b)| a == b);
                if under
                    && !t.is_dotted()
                    && !t.is_implicit()
                    && let Some(head) = t.span()
                {
                    out.push(head.start..body_end(t).max(head.end));
                }
                collect(t, prefix, target, out);
                prefix.pop();
            }
            Item::ArrayOfTables(_) if is_target(prefix, k, target) => {
                out.push(0..0);
            }
            _ => {}
        }
    }
}

/// Line-aligned byte ranges of the table at `path` and its sub-tables.
fn regions(input: &str, path: &[&str]) -> Result<Vec<Range<usize>>, TomlEditError> {
    if path.is_empty() {
        return Err(TomlEditError::EmptyPath);
    }
    let doc = parse(input)?;
    let mut spans = Vec::new();
    collect(doc.as_table(), &mut Vec::new(), path, &mut spans);
    if spans.iter().any(|s| s.is_empty()) {
        return Err(TomlEditError::UnsupportedLayout);
    }
    if item_at(doc.as_item(), path).is_some() && spans.is_empty() {
        return Err(TomlEditError::UnsupportedLayout);
    }
    spans.sort_by_key(|s| s.start);
    Ok(spans
        .into_iter()
        .map(|s| {
            let start = input[..s.start].rfind('\n').map_or(0, |i| i + 1);
            let tail = s.end.saturating_sub(1).max(start);
            let end = input[tail..]
                .find('\n')
                .map_or(input.len(), |i| tail + i + 1);
            start..end
        })
        .collect())
}

fn is_blank(line: &str) -> bool {
    line.trim().is_empty()
}

/// Delete `range` and, to keep spacing tidy, one adjacent blank line.
fn delete_region(text: &mut String, range: Range<usize>) {
    let (s, e) = (range.start, range.end);
    text.replace_range(s..e, "");
    let prev_start = text[..s.saturating_sub(1)].rfind('\n').map_or(0, |i| i + 1);
    let prev_blank = s > 0 && is_blank(&text[prev_start..s]);
    let next_end = text[s..].find('\n').map_or(text.len(), |i| s + i + 1);
    let next_blank = s == text.len() || is_blank(&text[s..next_end]);
    if prev_blank && next_blank {
        text.replace_range(prev_start..s, "");
    } else if s == 0 && next_blank && next_end > s {
        text.replace_range(s..next_end, "");
    }
}

/// Set `key` to `new`, keeping the old value's surrounding whitespace and
/// inline comment. Missing keys are appended to the table.
fn put(t: &mut dyn toml_edit::TableLike, key: &str, mut new: Value) {
    if let Some(Item::Value(old)) = t.get(key) {
        *new.decor_mut() = old.decor().clone();
    }
    t.insert(key, Item::Value(new));
}

/// Update only the managed fields of an existing table in place. Keys,
/// comments and blank lines mmhmem does not manage stay byte-identical;
/// fields that are `None` or an empty `env` keep what the user has.
fn update_existing(
    input: &str,
    path: &[&str],
    entry: &McpServerEntry,
) -> Result<String, TomlEditError> {
    let crlf = input.contains("\r\n");
    if crlf && input.replace("\r\n", "").contains('\n') {
        return Err(TomlEditError::MixedLineEndings);
    }
    let lf = input.replace("\r\n", "\n");
    let mut doc: DocumentMut = lf.parse().map_err(|_| TomlEditError::InvalidOutput)?;
    let mut table = doc.as_table_mut();
    for seg in path {
        table = table
            .get_mut(seg)
            .and_then(Item::as_table_mut)
            .ok_or(TomlEditError::UnsupportedLayout)?;
    }
    let cur = entry_from(&*table)?;
    let has = |t: &Table, k: &str| t.contains_key(k);
    if cur.command != entry.command || !has(table, "command") {
        put(table, "command", Value::from(entry.command.as_str()));
    }
    if cur.args != entry.args || !has(table, "args") {
        let args: Array = entry.args.iter().map(String::as_str).collect();
        put(table, "args", Value::Array(args));
    }
    if let Some(cwd) = entry.cwd.as_ref().filter(|c| cur.cwd.as_ref() != Some(*c)) {
        put(table, "cwd", Value::from(cwd.as_str()));
    }
    if let Some(n) = entry
        .startup_timeout_sec
        .filter(|n| cur.startup_timeout_sec != Some(*n))
    {
        put(table, "startup_timeout_sec", Value::from(n));
    }
    if let Some(b) = entry.enabled.filter(|b| cur.enabled != Some(*b)) {
        put(table, "enabled", Value::from(b));
    }
    update_env(table, &cur, entry);
    let text = doc.to_string();
    checked(if crlf {
        text.replace('\n', "\r\n")
    } else {
        text
    })
}

fn update_env(table: &mut Table, cur: &McpServerEntry, entry: &McpServerEntry) {
    if entry.env.is_empty() {
        return;
    }
    if !table.contains_key("env") {
        table.insert("env", Item::Table(Table::new()));
    }
    let Some(env) = table.get_mut("env").and_then(Item::as_table_like_mut) else {
        return;
    };
    for (k, v) in &entry.env {
        let same = cur.env.iter().any(|(ck, cv)| ck == k && cv == v);
        if !same {
            put(env, k, Value::from(v.as_str()));
        }
    }
}

/// Insert or update the table at `path`. A new table is appended after a
/// blank line. An existing table is updated in place: only the managed
/// fields change (`command`, `args`; `cwd`, `startup_timeout_sec`, `enabled`
/// only when `Some`; `env` entries only when non-empty, merged by key), and
/// every other key, comment and blank line stays byte-identical. Setting
/// identical values returns the input unchanged.
pub fn set_table(
    input: &str,
    path: &[&str],
    entry: &McpServerEntry,
) -> Result<String, TomlEditError> {
    let spans = regions(input, path)?;
    if !spans.is_empty() {
        return update_existing(input, path, entry);
    }
    let eol = eol_of(input);
    let mut out = input.to_owned();
    if !out.is_empty() {
        if !out.ends_with('\n') {
            out.push_str(eol);
        }
        out.push_str(eol);
    }
    let keys: Vec<String> = path
        .iter()
        .map(|k| Key::new(*k).display_repr().into_owned())
        .collect();
    out.push_str(&format!("[{}]{eol}", keys.join(".")));
    update_existing(&out, path, entry)
}

/// Remove the table at `path` with its sub-tables. Absent: input unchanged.
pub fn remove_table(input: &str, path: &[&str]) -> Result<String, TomlEditError> {
    let spans = regions(input, path)?;
    let mut out = input.to_owned();
    for span in spans.into_iter().rev() {
        delete_region(&mut out, span);
    }
    checked(out)
}

fn is_begin(line: &str) -> bool {
    let t = line.trim_end();
    t == MARKER_BEGIN_PS1 || t == MARKER_BEGIN_SH
}

/// True when the old installer's BEGIN or END marker line is present.
pub fn has_marker_block(input: &str) -> bool {
    input
        .lines()
        .any(|l| is_begin(l) || l.trim_end() == MARKER_END)
}

/// Convert the old `# BEGIN mmhmem` block into a plain table: the marker
/// lines are removed, the table and all other bytes stay. Without markers
/// the input is returned unchanged; malformed markers return an error.
pub fn convert_marker_block(input: &str, path: &[&str]) -> Result<String, TomlEditError> {
    let mut drop: Vec<Range<usize>> = Vec::new();
    let mut open: Option<usize> = None;
    let mut offset = 0;
    for (idx, raw) in input.split_inclusive('\n').enumerate() {
        let line = raw.trim_end_matches(['\r', '\n']);
        let range = offset..offset + raw.len();
        offset += raw.len();
        if is_begin(line) {
            if open.is_some() {
                return Err(TomlEditError::MarkerBlock(MarkerProblem::Nested(idx + 1)));
            }
            open = Some(idx + 1);
            drop.push(range);
        } else if line.trim_end() == MARKER_END {
            if open.take().is_none() {
                return Err(TomlEditError::MarkerBlock(MarkerProblem::Unmatched(
                    idx + 1,
                )));
            }
            drop.push(range);
        }
    }
    if let Some(line) = open {
        return Err(TomlEditError::MarkerBlock(MarkerProblem::Unmatched(line)));
    }
    if drop.is_empty() {
        return Ok(input.to_owned());
    }
    let mut out = input.to_owned();
    for r in drop.into_iter().rev() {
        out.replace_range(r, "");
    }
    if get_table(&out, path)?.is_none() {
        return Err(TomlEditError::MarkerBlock(MarkerProblem::NoTable));
    }
    checked(out)
}

#[path = "toml_json.rs"]
mod json_view;
pub use json_view::get_item_json;

#[cfg(test)]
#[path = "toml_tests.rs"]
mod tests;
