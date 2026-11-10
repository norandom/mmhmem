//! Read recognition for shell commands (read-routing-hooks design.md "shell
//! (`src/hook/shell.rs`)", Req 2.6): whole-file readers here, the partial
//! readers whose lines can be sized (`sed -n`, `head`, `tail`) in
//! [`super::shell_range`] (shunt-cache-efficiency task 3.1).
//!
//! Pure string analysis: nothing is executed, expanded or looked up. The
//! recogniser is conservative by design: anything it is not sure about is
//! `None`, so the read proceeds unchanged.

use std::path::{Path, PathBuf};

use super::LineRange;

/// Commands longer than this are never analysed (bounded untrusted input).
pub const MAX_COMMAND_BYTES: usize = 4096;

/// Characters rejected anywhere, quoted or not: expansion (`$`, `%`, `!`,
/// backtick), globs (`*`, `?`, `[`; PowerShell `-Path` globs even quoted)
/// and line breaks (multiline scripts, here-strings).
const UNSAFE_ANYWHERE: &[char] = &['$', '`', '%', '!', '*', '?', '[', '\n', '\r'];

/// Characters rejected outside quotes: pipes, redirects, separators,
/// subshells, PowerShell arrays and splatting, comments, home and brace
/// expansion.
const UNSAFE_UNQUOTED: &[char] = &[
    '|', '>', '<', ';', '&', '(', ')', ',', '@', '#', '~', '{', '}', ']',
];

/// PowerShell startup switches allowed before `-Command`.
const PWSH_STARTUP_FLAGS: &[&str] = &["-noprofile", "-nologo", "-noninteractive"];

/// One shell word after quote removal.
pub(super) struct Token {
    pub(super) text: String,
    pub(super) quoted: bool,
}

impl Token {
    /// An unquoted word starting with `-` is an option (`'-n'` is a file).
    pub(super) fn is_option(&self) -> bool {
        !self.quoted && self.text.starts_with('-')
    }
}

/// One recognised shell read of one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellRead {
    /// The file, resolved against the working folder.
    pub path: PathBuf,
    /// The lines a partial reader prints; `None` for a whole-file reader.
    pub range: Option<LineRange>,
}

/// `Some` only for one simple command that reads one file.
///
/// Accepted whole-file readers: `cat`, `type`, `less`, `more`, `gc` and
/// `Get-Content` (case-insensitive) with exactly one file argument. The
/// PowerShell readers (`Get-Content` and its aliases `gc`, `cat`, `type`)
/// also take `-Path`, `-LiteralPath` and `-Raw`; `-Raw` still reads the whole
/// file. Partial readers with a range are listed in [`super::shell_range`].
/// Every other option, several files, and any compound, expanding or
/// globbing command is `None`. The path is resolved against `cwd` unless it
/// is absolute (Unix, `C:\`, `C:/` or UNC); a drive-relative `C:x` is `None`.
pub fn recognise(command: &str, cwd: &Path) -> Option<ShellRead> {
    if command.len() > MAX_COMMAND_BYTES {
        return None;
    }
    // PowerShell splits on more whitespace than space and tab (NBSP, U+2003,
    // \v, \f, U+0085): such a character could hide an option inside a "path".
    if command.chars().any(|c| c.is_whitespace() && !is_blank(c)) {
        return None;
    }
    // `sed -n A,Bp` needs its comma; native programs get it literally in
    // PowerShell too (only cmdlets build arrays from it).
    let sed = command.split(is_blank).find(|w| !w.is_empty()) == Some("sed");
    let tokens = tokenize(command, sed)?;
    let (name, args) = tokens.split_first()?;
    if name.quoted {
        return None;
    }
    if let Some((file, range)) = super::shell_range::partial(&name.text, args) {
        let path = resolve(file, cwd)?;
        return Some(ShellRead {
            path,
            range: Some(range),
        });
    }
    let powershell = reader_kind(&name.text)?;
    let path = resolve(file_argument(args, powershell)?, cwd)?;
    Some(ShellRead { path, range: None })
}

/// The script of a shell-invocation argv such as `["bash", "-lc", "cat x"]`
/// or `["pwsh", "-NoProfile", "-Command", "gc x"]`; `None` for any other
/// shape, so the caller treats the call as not a read.
pub fn unwrap_invocation(argv: &[String]) -> Option<String> {
    let (program, rest) = argv.split_first()?;
    let script = match program_name(program).as_str() {
        "bash" | "sh" | "zsh" => posix_script(rest),
        "pwsh" | "powershell" => pwsh_script(rest),
        _ => None,
    }?;
    Some(script.clone())
}

/// `Some(true)` for the PowerShell-style readers that take `-Path` options,
/// `Some(false)` for pagers that take no options, `None` for anything else.
fn reader_kind(name: &str) -> Option<bool> {
    match name.to_ascii_lowercase().as_str() {
        "cat" | "type" | "gc" | "get-content" => Some(true),
        "less" | "more" => Some(false),
        _ => None,
    }
}

/// The single file argument, or `None` for zero or several files or any
/// option that is not a whole-file `Get-Content` parameter.
fn file_argument(args: &[Token], powershell: bool) -> Option<&str> {
    let mut file = None;
    let mut iter = args.iter();
    while let Some(token) = iter.next() {
        let value = if token.is_option() {
            if !powershell {
                return None;
            }
            match token.text.to_ascii_lowercase().as_str() {
                "-raw" => continue,
                "-path" | "-literalpath" => iter.next().filter(|t| !t.is_option())?,
                _ => return None,
            }
        } else {
            token
        };
        if file.replace(value.text.as_str()).is_some() {
            return None;
        }
    }
    file
}

/// Resolves `raw` against `cwd` unless it is already absolute.
pub(super) fn resolve(raw: &str, cwd: &Path) -> Option<PathBuf> {
    let bytes = raw.as_bytes();
    match bytes {
        [] => None,
        [b'/' | b'\\', ..] => Some(PathBuf::from(raw)),
        [drive, b':', rest @ ..] if drive.is_ascii_alphabetic() => {
            matches!(rest, [b'/' | b'\\', ..]).then(|| PathBuf::from(raw))
        }
        _ => Some(cwd.join(raw)),
    }
}

fn is_blank(c: char) -> bool {
    c == ' ' || c == '\t'
}

/// Splits a command into words; `None` for anything outside the simple
/// subset (unsafe characters, unterminated or concatenated quotes). With
/// `comma`, an unquoted `,` is kept in a word (for `sed -n A,Bp`).
fn tokenize(command: &str, comma: bool) -> Option<Vec<Token>> {
    let mut tokens = Vec::new();
    let mut chars = command.chars().peekable();
    loop {
        while chars.next_if(|c| is_blank(*c)).is_some() {}
        let token = match chars.peek() {
            None => return Some(tokens),
            Some('\'' | '"') => quoted_word(&mut chars)?,
            Some(_) => bare_word(&mut chars, comma)?,
        };
        tokens.push(token);
    }
}

type Chars<'a> = std::iter::Peekable<std::str::Chars<'a>>;

/// A whole word in single or double quotes, ending at a blank or the end.
fn quoted_word(chars: &mut Chars<'_>) -> Option<Token> {
    let quote = chars.next()?;
    let mut text = String::new();
    loop {
        match chars.next()? {
            c if c == quote => break,
            c if UNSAFE_ANYWHERE.contains(&c) => return None,
            c => text.push(c),
        }
    }
    if chars.peek().is_some_and(|c| !is_blank(*c)) {
        return None;
    }
    Some(Token { text, quoted: true })
}

/// An unquoted word; a quote inside it (concatenation) is `None`.
fn bare_word(chars: &mut Chars<'_>, comma: bool) -> Option<Token> {
    let mut text = String::new();
    while let Some(c) = chars.next_if(|c| !is_blank(*c)) {
        let unquoted = UNSAFE_UNQUOTED.contains(&c) && !(comma && c == ',');
        let unsafe_char = UNSAFE_ANYWHERE.contains(&c) || unquoted;
        if unsafe_char || c == '\'' || c == '"' {
            return None;
        }
        text.push(c);
    }
    Some(Token {
        text,
        quoted: false,
    })
}

/// Lower-case program basename without directory or `.exe`.
fn program_name(program: &str) -> String {
    let base = program
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(program)
        .to_ascii_lowercase();
    match base.strip_suffix(".exe") {
        Some(stem) => stem.to_owned(),
        None => base,
    }
}

/// `[-c|-lc, script]` for POSIX shells.
fn posix_script(rest: &[String]) -> Option<&String> {
    match rest {
        [flag, script] if flag == "-c" || flag == "-lc" => Some(script),
        _ => None,
    }
}

/// `[startup flags..., -Command|-c, script]` for PowerShell.
fn pwsh_script(rest: &[String]) -> Option<&String> {
    let (script, flags) = rest.split_last()?;
    let (command, startup) = flags.split_last()?;
    let is_command = command.eq_ignore_ascii_case("-command") || command.eq_ignore_ascii_case("-c");
    let startup_ok = startup
        .iter()
        .all(|f| PWSH_STARTUP_FLAGS.iter().any(|k| f.eq_ignore_ascii_case(k)));
    (is_command && startup_ok).then_some(script)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cwd() -> PathBuf {
        PathBuf::from("/work/proj")
    }

    fn rel(p: &str) -> Option<PathBuf> {
        Some(cwd().join(p))
    }

    fn abs(p: &str) -> Option<PathBuf> {
        Some(PathBuf::from(p))
    }

    /// Whole-file reads only: the expected path and no range.
    fn check_accepts(cases: &[(&str, Option<PathBuf>)]) {
        for (cmd, want) in cases {
            let got = recognise(cmd, &cwd());
            assert_eq!(got.as_ref().map(|r| &r.path), want.as_ref(), "{cmd:?}");
            assert_eq!(got.and_then(|r| r.range), None, "command: {cmd:?}");
        }
    }

    fn check_rejects(cases: &[&str]) {
        for cmd in cases {
            assert_eq!(recognise(cmd, &cwd()), None, "command: {cmd:?}");
        }
    }

    #[test]
    fn accepts_bash_whole_file_reads() {
        check_accepts(&[
            ("cat src/main.rs", rel("src/main.rs")),
            ("  cat   README.md  ", rel("README.md")),
            ("cat\tREADME.md", rel("README.md")),
            ("less notes.txt", rel("notes.txt")),
            ("more notes.txt", rel("notes.txt")),
            ("cat 'my file.txt'", rel("my file.txt")),
            ("cat \"dir with space/a.rs\"", rel("dir with space/a.rs")),
            ("cat /etc/hosts", abs("/etc/hosts")),
            ("cat ./a.rs", rel("./a.rs")),
            ("cat ../up/b.rs", rel("../up/b.rs")),
            ("cat '-n'", rel("-n")),
        ]);
    }

    #[test]
    fn accepts_powershell_and_cmd_whole_file_reads() {
        check_accepts(&[
            ("Get-Content src\\lib.rs", rel("src\\lib.rs")),
            ("get-content a.txt", rel("a.txt")),
            ("GET-CONTENT a.txt", rel("a.txt")),
            ("Get-Content -Path a.txt", rel("a.txt")),
            ("Get-Content -path a.txt", rel("a.txt")),
            ("Get-Content -LiteralPath 'a b.txt'", rel("a b.txt")),
            ("Get-Content -Raw a.txt", rel("a.txt")),
            ("Get-Content -Path a.txt -Raw", rel("a.txt")),
            ("gc a.txt", rel("a.txt")),
            ("GC -Path a.txt", rel("a.txt")),
            ("cat -Path a.txt", rel("a.txt")),
            ("type a.txt", rel("a.txt")),
            ("TYPE a.txt", rel("a.txt")),
            ("type C:\\src\\a.rs", abs("C:\\src\\a.rs")),
            ("Get-Content C:/src/a.rs", abs("C:/src/a.rs")),
            (
                "Get-Content \"C:\\Program Files\\x\\a.txt\"",
                abs("C:\\Program Files\\x\\a.txt"),
            ),
            ("gc \\\\srv\\share\\a.txt", abs("\\\\srv\\share\\a.txt")),
        ]);
    }

    #[test]
    fn rejects_compound_and_expanding_commands() {
        check_rejects(&[
            "cat a.txt | head",
            "cat a.txt > b.txt",
            "cat < a.txt",
            "cat a.txt 2> err",
            "cat a.txt; ls",
            "cat a.txt && ls",
            "cat a.txt || ls",
            "cat a.txt &",
            "(cat a.txt)",
            "cat $(echo a.txt)",
            "cat `echo a.txt`",
            "cat \"$(echo a.txt)\"",
            "cat *.rs",
            "cat a?.rs",
            "cat [ab].rs",
            "cat \"*.rs\"",
            "cat $HOME/a.txt",
            "cat ${HOME}/a.txt",
            "cat \"$HOME/a.txt\"",
            "Get-Content $env:TEMP\\a.txt",
            "type %USERPROFILE%\\a.txt",
            "cat ~/a.txt",
            "cat {a,b}.txt",
            "cat a.txt # note",
            "cat a.txt\nls",
            "cat a.txt\r\nls",
            "Get-Content @'\na.txt\n'@",
            "Get-Content @\"a.txt\"@",
            "& cat a.txt",
            "cat 'unterminated",
            "cat \"unterminated",
            "cat 'a'b",
            "cat a'b'",
            "cat a.txt,b.txt",
            "cat a\\ b.txt",
        ]);
    }

    #[test]
    fn rejects_partial_readers_options_and_several_files() {
        check_rejects(&[
            "",
            "   ",
            "cat",
            "cat a.txt b.txt",
            "cat -n a.txt",
            "cat -- a.txt",
            "cat -",
            "less +F a.txt",
            "head -c 5 a.txt",
            "tail -f a.txt",
            "sed 1,5p a.txt",
            "cat 1,5p a.txt",
            "grep x a.txt",
            "awk 1 a.txt",
            "bat a.txt",
            "ls a.txt",
            "/bin/cat a.txt",
            "'cat' a.txt",
            "Get-Content -TotalCount 5 a.txt",
            "Get-Content a.txt -Head 5",
            "Get-Content a.txt -Tail 5",
            "Get-Content a.txt -ReadCount 10",
            "Get-Content a.txt -Wait",
            "Get-Content -Path",
            "Get-Content -Raw",
            "Get-Content -Path a.txt b.txt",
            "Get-Content -Path a.txt -LiteralPath b.txt",
            "Get-Content -Path:a.txt",
            "Get-Content -Encoding utf8 a.txt",
            "Get-Content -Pa a.txt",
            "gc -First 3 a.txt",
            "type C:a.txt",
            "less -Raw a.txt",
            "more -Path a.txt",
        ]);
    }

    #[test]
    fn ignores_oversized_commands() {
        for odd in [
            "gc a.txt\u{a0}-Tail\u{a0}5",
            "cat a\u{2003}b",
            "type a\u{b}b",
            "cat a\u{85}",
        ] {
            assert_eq!(recognise(odd, &cwd()), None, "{odd:?}");
        }
        let long = format!("cat {}", "a".repeat(MAX_COMMAND_BYTES));
        assert_eq!(recognise(&long, &cwd()), None);
        let fits = format!("cat {}", "a".repeat(MAX_COMMAND_BYTES - 8));
        assert!(recognise(&fits, &cwd()).is_some());
    }

    fn argv(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn unwraps_shell_invocations() {
        let cases: &[(&[&str], Option<&str>)] = &[
            (&["bash", "-lc", "cat a.txt"], Some("cat a.txt")),
            (&["bash", "-c", "cat a.txt"], Some("cat a.txt")),
            (&["sh", "-c", "cat a.txt"], Some("cat a.txt")),
            (&["zsh", "-lc", "cat a.txt"], Some("cat a.txt")),
            (&["/bin/bash", "-lc", "cat a.txt"], Some("cat a.txt")),
            (&["bash.exe", "-lc", "cat a.txt"], Some("cat a.txt")),
            (&["pwsh", "-Command", "gc a.txt"], Some("gc a.txt")),
            (&["pwsh.exe", "-command", "gc a.txt"], Some("gc a.txt")),
            (&["powershell", "-c", "gc a.txt"], Some("gc a.txt")),
            (
                &["powershell.exe", "-NoProfile", "-Command", "gc a.txt"],
                Some("gc a.txt"),
            ),
            (
                &["C:\\Windows\\powershell.exe", "-Command", "gc a"],
                Some("gc a"),
            ),
            (&[], None),
            (&["bash"], None),
            (&["bash", "-lc"], None),
            (&["bash", "script.sh"], None),
            (&["bash", "-x", "cat a.txt"], None),
            (&["bash", "-Command", "cat a.txt"], None),
            (&["bash", "-lc", "cat a.txt", "extra"], None),
            (&["bash", "-NoProfile", "-lc", "cat a.txt"], None),
            (&["python", "-c", "print(1)"], None),
            (&["cat", "a.txt"], None),
            (&["pwsh", "-File", "a.ps1"], None),
            (&["pwsh", "-lc", "gc a"], None),
            (&["pwsh", "-Command", "gc a", "b"], None),
        ];
        for (parts, want) in cases {
            let got = unwrap_invocation(&argv(parts));
            assert_eq!(got.as_deref(), *want, "argv: {parts:?}");
        }
    }

    #[test]
    fn unwrapped_script_is_recognised() {
        let script = unwrap_invocation(&argv(&["bash", "-lc", "cat 'a b.txt'"])).expect("unwrap");
        let read = recognise(&script, &cwd()).expect("read");
        assert_eq!((Some(read.path), read.range), (rel("a b.txt"), None));
        let piped = unwrap_invocation(&argv(&["bash", "-lc", "cat a | wc -l"])).expect("unwrap");
        assert_eq!(recognise(&piped, &cwd()), None);
    }
}
