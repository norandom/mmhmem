//! Antigravity's read-routing hook (antigravity-native-setup task 3.3,
//! design.md "Hook entry contract"; Requirements 2.1, 2.3 and 2.5).
//!
//! mmhmem owns one whole named hook, [`HOOK_NAME`], in the global hook file
//! `~/.gemini/config/hooks.json`: a `PreToolUse` group matching exactly the
//! `read_file` tool (MCP calls such as `smart_read` itself arrive as
//! `call_mcp_tool` and must never be denied) with one command handler. Other
//! named hooks are never touched; a missing file is created as `{}` with only
//! mmhmem's hook.
//!
//! **The command** (research.md, confirmed 2026-11-09): Antigravity splits it
//! on whitespace and passes quotes literally, and a hook that fails to start
//! blocks every tool call. So [`hook_command`] writes unquoted,
//! whitespace-free tokens only: the binary's path, then `hook antigravity`.
//! On Windows a path with whitespace gets 8.3 short names up to its last
//! component with whitespace, which must exist; the components after it
//! keep their long names and need not exist yet. On a first install the
//! program is copied only when the plan is applied, and the command stays
//! the same after the copy, so the next plan changes nothing. When the
//! result still has whitespace (a volume without 8.3 names) or the OS cannot
//! shorten the part with whitespace (a folder that does not exist), and off
//! Windows, there is no safe command and the read hook is left out with
//! [`AgentProblem::HookCommandUnrepresentable`].

use std::path::Path;
#[cfg(windows)]
use std::path::{Component, PathBuf};

use serde_json::{Value, json};

use super::{Antigravity, SCOPE};
use crate::setup::agents::{AgentId, AgentProblem, AgentRecord, Env, FileChain};
use crate::setup::edit::json as edit;
use crate::setup::plan::{Action, Change};
use crate::setup::record::{KeySegment, Os, Target, key_path};
use crate::setup::ui::text::changes::{self as say, What};

/// mmhmem's top-level hook name in `hooks.json`.
pub const HOOK_NAME: &str = "mmhmem-read-routing";

/// The tool-name regex: Antigravity's file-read tool and nothing else.
pub const HOOK_MATCHER: &str = "^read_file$";

/// Seconds Antigravity waits for the hook.
pub const HOOK_TIMEOUT: u64 = 10;

const HOOK: [&str; 1] = [HOOK_NAME];

/// The arguments after the program (`mmhmem hook antigravity`).
const HOOK_ARGS: &str = "hook antigravity";

impl Antigravity {
    /// The value of mmhmem's named hook: one `PreToolUse` group with one
    /// command handler; the problem when the command cannot be written.
    pub fn hook_entry(&self, os: Os) -> Result<Value, AgentProblem> {
        Ok(json!({"PreToolUse": [{
            "matcher": HOOK_MATCHER,
            "hooks": [{
                "type": "command",
                "command": hook_command(&self.binary, os)?,
                "timeout": HOOK_TIMEOUT,
            }],
        }]}))
    }

    /// Plans the whole named hook `value` in [`Antigravity::hooks_file`];
    /// nothing when the file already holds it and the record lists it.
    pub(super) fn hook_change(
        chain: &mut FileChain,
        env: &Env,
        rec: Option<&AgentRecord>,
        value: &Value,
    ) -> Result<(), AgentProblem> {
        let file = Self::hooks_file(env);
        let target = hook_target(&file);
        let recorded = rec.is_some_and(|r| r.managed.iter().any(|e| e.target == target));
        let before = chain.bytes(&file)?;
        let text = chain.text(&file)?;
        let input = text.as_deref().unwrap_or("{}\n");
        let bad = |error| AgentProblem::Json {
            file: file.clone(),
            error,
        };
        let current = edit::get(input, &HOOK).map_err(bad)?;
        if recorded && current.as_ref() == Some(value) {
            return Ok(());
        }
        let after = edit::set(input, &HOOK, value).map_err(bad)?;
        let label = AgentId::Antigravity.label();
        let (action, summary) = match current {
            Some(_) => (Action::Update, say::update(What::ReadHook, label, &file)),
            None => (Action::Add, say::add(What::ReadHook, label, &file)),
        };
        chain.push(Change {
            scope: SCOPE,
            target,
            file,
            action,
            before,
            after: Some(after.into_bytes()),
            owner_only: false,
            hide_diff: false,
            missing_parents: Some(0),
            summary,
        });
        Ok(())
    }
}

/// `<binary> hook antigravity` as unquoted, whitespace-free tokens (see the
/// module docs); [`AgentProblem::HookCommandUnrepresentable`] when the
/// binary's path has whitespace and no whitespace-free short form, or is not
/// Unicode (it could not be written exactly).
pub fn hook_command(binary: &Path, os: Os) -> Result<String, AgentProblem> {
    let program =
        program_token(binary, os).ok_or_else(|| AgentProblem::HookCommandUnrepresentable {
            file: binary.to_path_buf(),
        })?;
    Ok(format!("{program} {HOOK_ARGS}"))
}

/// The binary's path as one token: as it is when it has no whitespace, else
/// its Windows short form ([`short_path`]) when that has none.
fn program_token(binary: &Path, os: Os) -> Option<String> {
    let path = binary.to_str()?;
    if !has_whitespace(path) {
        return Some(path.to_owned());
    }
    if os != Os::Windows {
        return None;
    }
    short_path(binary).filter(|short| !has_whitespace(short))
}

fn has_whitespace(text: &str) -> bool {
    text.contains(char::is_whitespace)
}

/// mmhmem's whole named hook in `file`.
pub(super) fn hook_target(file: &Path) -> Target {
    Target::JsonEntry {
        file: file.to_path_buf(),
        pointer: key_path(&HOOK).expect("static keys"),
        managed_keys: None,
    }
}

/// Whether a recorded pointer is mmhmem's named hook.
pub(super) fn is_hook(pointer: &[KeySegment]) -> bool {
    pointer.iter().map(KeySegment::as_str).eq(HOOK)
}

/// The text without mmhmem's named hook (a single top-level key: no parent
/// is ever created or pruned).
pub(super) fn without_hook(text: &str) -> Result<String, edit::JsonEditError> {
    edit::remove(text, &HOOK)
}

/// `path` with 8.3 short names up to and including its last component with
/// whitespace, which must exist: the OS shortens that part
/// ([`os_short_path`]); the components after it keep their long names and
/// need not exist (the program is copied there only when the plan is
/// applied). A path without whitespace comes back as it is. `None` when the
/// OS cannot shorten the part with whitespace, e.g. because it does not
/// exist. The result may still have whitespace: a volume without 8.3 names
/// keeps the long names.
#[cfg(windows)]
pub(super) fn short_path(path: &Path) -> Option<String> {
    let parts: Vec<Component<'_>> = path.components().collect();
    let spaced = |part: &Component<'_>| part.as_os_str().to_str().is_some_and(has_whitespace);
    let Some(last) = parts.iter().rposition(spaced) else {
        return path.to_str().map(str::to_owned);
    };
    let existing: PathBuf = parts[..=last].iter().collect();
    let mut short = PathBuf::from(os_short_path(&existing)?);
    short.extend(&parts[last + 1..]);
    short.into_os_string().into_string().ok()
}

/// The OS's 8.3 short form of an existing `path` (`GetShortPathNameW`);
/// `None` when the call fails, e.g. for a path that does not exist. A
/// volume without 8.3 names returns the long path unchanged.
#[cfg(windows)]
pub(super) fn os_short_path(path: &Path) -> Option<String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetShortPathNameW;

    let long: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY: `long` is NUL-terminated; a null buffer of length 0 is the
    // documented size query (the result counts the terminator).
    let needed = unsafe { GetShortPathNameW(long.as_ptr(), std::ptr::null_mut(), 0) };
    if needed == 0 {
        return None;
    }
    let mut short = vec![0u16; needed as usize];
    // SAFETY: `short` is writable for `needed` u16s, as passed.
    let len = unsafe { GetShortPathNameW(long.as_ptr(), short.as_mut_ptr(), needed) };
    // 0 is a failure; `needed` or more means the path changed in between.
    if len == 0 || len >= needed {
        return None;
    }
    short.truncate(len as usize);
    String::from_utf16(&short).ok()
}

/// No short forms off Windows.
#[cfg(not(windows))]
pub(super) fn short_path(_path: &Path) -> Option<String> {
    None
}
