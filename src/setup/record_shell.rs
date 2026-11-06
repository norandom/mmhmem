//! The Codex hook command string: quoting the binary path for the shell
//! Codex runs it with, and reading the path back (follow-up G3,
//! `.kiro/specs/reliable-installers/research.md` "Note 2026-11-06").
//!
//! Codex runs a hook `command` through the user's shell: PowerShell on
//! Windows (verified with codex-cli 0.166.0, `manage_hooks.ps1:284`), the
//! POSIX shell elsewhere (`manage_hooks.py` `shlex.quote`). PowerShell ends a
//! single-quoted string at `'` and also at the typographic quotes `‘ ’ ‚ ‛`;
//! inside one, a doubled quote character stands for that character, so
//! doubling each of them quotes every path. In a POSIX single-quoted string
//! only `'` is special (`'"'"'`). A path with a control character (a line
//! break could end the command) or that is not Unicode is refused.
//!
//! Reading back is a small tokenizer for each shell that accepts plain
//! words, single- and double-quoted strings and (POSIX) backslash escapes,
//! and rejects anything that would expand or run more (`$`, backticks,
//! globs, `;`, `|`, ...). A command is mmhmem's when either shell's reading
//! gives exactly `<mmhmem binary> hook <harness>`, so a hand-edited but
//! equivalent quoting is recognised too.

use std::path::Path;

use super::Os;

/// PowerShell's single quotes: `'` and `‘ ’ ‚ ‛`.
const PS_SINGLE: [char; 5] = ['\'', '\u{2018}', '\u{2019}', '\u{201A}', '\u{201B}'];
/// PowerShell's double quotes: `"` and `“ ” „`.
const PS_DOUBLE: [char; 4] = ['"', '\u{201C}', '\u{201D}', '\u{201E}'];

/// Why a binary path cannot go into a shell command safely.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnquotablePath {
    /// The path holds a control character such as a line break.
    ControlCharacter,
    /// The path is not valid Unicode.
    NotUnicode,
}

impl UnquotablePath {
    /// Plain words for the reason.
    pub fn reason(self) -> &'static str {
        match self {
            Self::ControlCharacter => {
                "the path contains a control character (such as a line break)"
            }
            Self::NotUnicode => "the path is not valid Unicode text",
        }
    }
}

/// `<binary> hook codex`, with the path quoted for `os`'s shell.
pub fn codex_command(binary: &Path, os: Os) -> Result<String, UnquotablePath> {
    let bin = binary.to_str().ok_or(UnquotablePath::NotUnicode)?;
    if bin.chars().any(char::is_control) {
        return Err(UnquotablePath::ControlCharacter);
    }
    if os == Os::Windows {
        let mut quoted = String::with_capacity(bin.len() + 2);
        for c in bin.chars() {
            quoted.push(c);
            if PS_SINGLE.contains(&c) {
                quoted.push(c);
            }
        }
        return Ok(format!("& '{quoted}' hook codex"));
    }
    let plain = |c: char| c.is_ascii_alphanumeric() || "_@%+=:,./-".contains(c);
    if !bin.is_empty() && bin.chars().all(plain) {
        Ok(format!("{bin} hook codex"))
    } else {
        Ok(format!("'{}' hook codex", bin.replace('\'', "'\"'\"'")))
    }
}

/// The words of `command` under PowerShell, then under POSIX shell rules:
/// every reading that is plain words only. Used to recognise mmhmem's entry
/// whichever OS wrote it.
pub fn readings(command: &str) -> Vec<Vec<String>> {
    [powershell_words(command), posix_words(command)]
        .into_iter()
        .flatten()
        .collect()
}

/// The words of `command` as the shell Codex uses on `os` reads them.
pub fn words_for(command: &str, os: Os) -> Option<Vec<String>> {
    match os {
        Os::Windows => powershell_words(command),
        _ => posix_words(command),
    }
}

/// PowerShell: `[&] word word ...`, each word bare or quoted. A quoted
/// first word needs the call operator `&` (else it is a string, not a
/// command).
fn powershell_words(command: &str) -> Option<Vec<String>> {
    let mut chars = command.trim().chars().peekable();
    let call = chars.next_if_eq(&'&').is_some();
    let mut words = Vec::new();
    loop {
        while chars.next_if(|c| *c == ' ' || *c == '\t').is_some() {}
        let Some(&first) = chars.peek() else {
            return Some(words);
        };
        let quoted = PS_SINGLE.contains(&first) || PS_DOUBLE.contains(&first);
        if quoted && words.is_empty() && !call {
            return None;
        }
        let word = if PS_SINGLE.contains(&first) {
            chars.next();
            ps_single(&mut chars)?
        } else if PS_DOUBLE.contains(&first) {
            chars.next();
            ps_double(&mut chars)?
        } else {
            ps_bare(&mut chars)?
        };
        if chars.peek().is_some_and(|c| *c != ' ' && *c != '\t') {
            return None;
        }
        words.push(word);
    }
}

type Chars<'a> = std::iter::Peekable<std::str::Chars<'a>>;

/// The rest of a single-quoted string: a doubled quote stands for the
/// second one (PowerShell's tokenizer).
fn ps_single(chars: &mut Chars<'_>) -> Option<String> {
    let mut word = String::new();
    loop {
        let c = chars.next()?;
        if PS_SINGLE.contains(&c) {
            match chars.next_if(|n| PS_SINGLE.contains(n)) {
                Some(n) => word.push(n),
                None => return Some(word),
            }
        } else {
            word.push(c);
        }
    }
}

/// The rest of a double-quoted string without expansions: `$` is refused,
/// a backtick escapes a quote, `$` or a backtick, a doubled quote stands for
/// one.
fn ps_double(chars: &mut Chars<'_>) -> Option<String> {
    let mut word = String::new();
    loop {
        match chars.next()? {
            '$' => return None,
            '`' => {
                let n = chars.next()?;
                if !(n == '`' || n == '$' || PS_SINGLE.contains(&n) || PS_DOUBLE.contains(&n)) {
                    return None;
                }
                word.push(n);
            }
            c if PS_DOUBLE.contains(&c) => match chars.next_if(|n| PS_DOUBLE.contains(n)) {
                Some(n) => word.push(n),
                None => return Some(word),
            },
            c => word.push(c),
        }
    }
}

/// A bare word without characters PowerShell would treat specially.
fn ps_bare(chars: &mut Chars<'_>) -> Option<String> {
    let mut word = String::new();
    while let Some(c) = chars.next_if(|c| *c != ' ' && *c != '\t') {
        let special = "$`;|&(){}<>,@\"'".contains(c)
            || PS_SINGLE.contains(&c)
            || PS_DOUBLE.contains(&c)
            || (word.is_empty() && c == '#');
        if special {
            return None;
        }
        word.push(c);
    }
    Some(word)
}

/// POSIX shell: words of plain characters, `'...'`, `"..."` (no expansion)
/// and `\x`, joined where they touch.
fn posix_words(command: &str) -> Option<Vec<String>> {
    let mut chars = command.trim().chars().peekable();
    let mut words = Vec::new();
    let mut word: Option<String> = None;
    while let Some(c) = chars.next() {
        match c {
            ' ' | '\t' => words.extend(word.take()),
            '\'' => {
                let w = word.get_or_insert_with(String::new);
                loop {
                    match chars.next()? {
                        '\'' => break,
                        c => w.push(c),
                    }
                }
            }
            '"' => posix_double(&mut chars, word.get_or_insert_with(String::new))?,
            '\\' => word.get_or_insert_with(String::new).push(chars.next()?),
            '#' | '~' if word.is_none() => return None,
            c if "|&;<>()$`*?[{}\n\r".contains(c) => return None,
            c => word.get_or_insert_with(String::new).push(c),
        }
    }
    words.extend(word);
    Some(words)
}

/// The rest of a POSIX double-quoted string, appended to `word`; `$` and
/// backticks (expansions) are refused.
fn posix_double(chars: &mut Chars<'_>, word: &mut String) -> Option<()> {
    loop {
        match chars.next()? {
            '"' => return Some(()),
            '$' | '`' => return None,
            '\\' => match chars.next()? {
                n @ ('$' | '`' | '"' | '\\') => word.push(n),
                n => {
                    word.push('\\');
                    word.push(n);
                }
            },
            c => word.push(c),
        }
    }
}

#[cfg(test)]
#[path = "record_shell_tests.rs"]
mod tests;
