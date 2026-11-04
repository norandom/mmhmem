//! [`Registration`]: how an agent will start mmhmem, read back from the bytes
//! of the agent's own config file (task 4.1, design.md §6 step 1).
//!
//! The constructors read what the agent will read, not the installer's plan,
//! so the functional check starts exactly the command the agent starts.
//! Environment values are kept private: [`Registration`]'s `Debug` lists the
//! variable names only, and [`RegistrationError`] never repeats file text.

use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;

use serde_json::{Map, Value};

use crate::setup::edit::json as jedit;
use crate::setup::edit::toml as tedit;

/// The program, arguments, environment overrides and folder an agent uses
/// to start mmhmem.
#[derive(Clone, PartialEq, Eq)]
pub struct Registration {
    /// The program, as written in the config (absolute path or bare name).
    pub program: PathBuf,
    /// Its arguments.
    pub args: Vec<String>,
    /// The folder it starts in, when the config names one.
    pub cwd: Option<PathBuf>,
    env: BTreeMap<String, String>,
}

impl fmt::Debug for Registration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Registration")
            .field("program", &self.program)
            .field("args", &self.args)
            .field("cwd", &self.cwd)
            .field("env_keys", &self.env.keys().collect::<Vec<_>>())
            .finish()
    }
}

/// Why no registration could be read. Messages never contain file text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistrationError {
    /// The file is not valid text, JSON or TOML.
    Unreadable,
    /// The file has no entry under that name.
    Missing,
    /// The entry names no program.
    NoCommand,
    /// The entry starts a remote server, not a local program.
    NotLocal,
    /// The entry is switched off (`enabled = false`).
    Disabled,
    /// A field of the entry has the wrong form.
    BadField(&'static str),
}

impl fmt::Display for RegistrationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&super::text::registration_reason(self))
    }
}

impl std::error::Error for RegistrationError {}

impl Registration {
    /// `program` with `args`, no environment overrides and no folder.
    pub fn new(program: impl Into<PathBuf>, args: &[&str]) -> Self {
        Self {
            program: program.into(),
            args: args.iter().map(|a| (*a).to_owned()).collect(),
            cwd: None,
            env: BTreeMap::new(),
        }
    }

    /// The same registration with one more environment override.
    pub fn with_env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.insert(key.into(), value.into());
        self
    }

    /// The same registration started in `cwd`.
    pub fn with_cwd(mut self, cwd: impl Into<PathBuf>) -> Self {
        self.cwd = Some(cwd.into());
        self
    }

    /// The names of the environment overrides (never their values).
    pub fn env_keys(&self) -> impl Iterator<Item = &str> {
        self.env.keys().map(String::as_str)
    }

    pub(super) fn env(&self) -> &BTreeMap<String, String> {
        &self.env
    }

    /// The entry `mcpServers.<name>` of a Claude Code (`~/.claude.json`),
    /// Claude Desktop (`claude_desktop_config.json`) or Antigravity config:
    /// `command`, optional `args`, `env` and `type` (`stdio` when present).
    pub fn from_claude_json(bytes: &[u8], name: &str) -> Result<Self, RegistrationError> {
        let entry = json_entry(bytes, &["mcpServers", name])?;
        match entry.get("type") {
            None | Some(Value::Null) => {}
            Some(Value::String(kind)) if kind == "stdio" => {}
            Some(_) => return Err(RegistrationError::NotLocal),
        }
        let program = command_string(&entry)?;
        let args = optional_strings(&entry, "args")?;
        let env = optional_map(&entry, "env")?;
        Ok(Self::build(program, args, env, optional_cwd(&entry)?))
    }

    /// The table `[mcp_servers.<name>]` of Codex's `config.toml`: `command`,
    /// optional `args`, `env`, `cwd` and `enabled`.
    pub fn from_codex_toml(bytes: &[u8], name: &str) -> Result<Self, RegistrationError> {
        let text = std::str::from_utf8(bytes).map_err(|_| RegistrationError::Unreadable)?;
        let entry = tedit::get_item_json(text, &["mcp_servers", name])
            .map_err(|_| RegistrationError::Unreadable)?
            .ok_or(RegistrationError::Missing)?;
        let entry = as_object(entry)?;
        check_enabled(&entry)?;
        let program = command_string(&entry)?;
        let args = optional_strings(&entry, "args")?;
        let env = optional_map(&entry, "env")?;
        Ok(Self::build(program, args, env, optional_cwd(&entry)?))
    }

    /// The entry `mcp.servers.<name>` of an OpenCode `opencode.json(c)`:
    /// `command` as a list (program first), optional `type` (`local`),
    /// `environment`, `cwd` and `enabled`.
    pub fn from_opencode_json(bytes: &[u8], name: &str) -> Result<Self, RegistrationError> {
        let entry = json_entry(bytes, &["mcp", "servers", name])?;
        match entry.get("type") {
            None | Some(Value::Null) => {}
            Some(Value::String(kind)) if kind == "local" => {}
            Some(_) => return Err(RegistrationError::NotLocal),
        }
        check_enabled(&entry)?;
        let mut command = match entry.get("command") {
            None | Some(Value::Null) => return Err(RegistrationError::NoCommand),
            Some(value) => strings(value, "command")?,
        };
        if command.first().is_none_or(String::is_empty) {
            return Err(RegistrationError::NoCommand);
        }
        let program = command.remove(0);
        let env = optional_map(&entry, "environment")?;
        Ok(Self::build(program, command, env, optional_cwd(&entry)?))
    }

    fn build(
        program: String,
        args: Vec<String>,
        env: BTreeMap<String, String>,
        cwd: Option<PathBuf>,
    ) -> Self {
        Self {
            program: PathBuf::from(program),
            args,
            cwd,
            env,
        }
    }
}

fn json_entry(bytes: &[u8], path: &[&str]) -> Result<Map<String, Value>, RegistrationError> {
    let text = std::str::from_utf8(bytes).map_err(|_| RegistrationError::Unreadable)?;
    let entry = jedit::get(text, path)
        .map_err(|_| RegistrationError::Unreadable)?
        .ok_or(RegistrationError::Missing)?;
    as_object(entry)
}

fn as_object(value: Value) -> Result<Map<String, Value>, RegistrationError> {
    match value {
        Value::Object(map) => Ok(map),
        _ => Err(RegistrationError::BadField("entry")),
    }
}

fn check_enabled(entry: &Map<String, Value>) -> Result<(), RegistrationError> {
    match entry.get("enabled") {
        None | Some(Value::Bool(true)) => Ok(()),
        Some(Value::Bool(false)) => Err(RegistrationError::Disabled),
        Some(_) => Err(RegistrationError::BadField("enabled")),
    }
}

fn command_string(entry: &Map<String, Value>) -> Result<String, RegistrationError> {
    match entry.get("command") {
        None | Some(Value::Null) => Err(RegistrationError::NoCommand),
        Some(Value::String(s)) if s.is_empty() => Err(RegistrationError::NoCommand),
        Some(Value::String(s)) => Ok(s.clone()),
        Some(_) => Err(RegistrationError::BadField("command")),
    }
}

fn strings(value: &Value, field: &'static str) -> Result<Vec<String>, RegistrationError> {
    value
        .as_array()
        .ok_or(RegistrationError::BadField(field))?
        .iter()
        .map(|v| {
            v.as_str()
                .map(str::to_owned)
                .ok_or(RegistrationError::BadField(field))
        })
        .collect()
}

fn optional_strings(
    entry: &Map<String, Value>,
    field: &'static str,
) -> Result<Vec<String>, RegistrationError> {
    match entry.get(field) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(value) => strings(value, field),
    }
}

fn optional_map(
    entry: &Map<String, Value>,
    field: &'static str,
) -> Result<BTreeMap<String, String>, RegistrationError> {
    let map = match entry.get(field) {
        None | Some(Value::Null) => return Ok(BTreeMap::new()),
        Some(Value::Object(map)) => map,
        Some(_) => return Err(RegistrationError::BadField(field)),
    };
    map.iter()
        .map(|(k, v)| match v {
            Value::String(s) => Ok((k.clone(), s.clone())),
            _ => Err(RegistrationError::BadField(field)),
        })
        .collect()
}

fn optional_cwd(entry: &Map<String, Value>) -> Result<Option<PathBuf>, RegistrationError> {
    match entry.get("cwd") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) if s.is_empty() => Ok(None),
        Some(Value::String(s)) => Ok(Some(PathBuf::from(s))),
        Some(_) => Err(RegistrationError::BadField("cwd")),
    }
}
