//! The Codex hook command (follow-up G3): every quotable path round-trips
//! through each shell's reading, and commands that would expand or run
//! more are not read as plain words.

use std::path::Path;

use super::*;

const PATHS: [&str; 12] = [
    "C:\\Users\\O'Brien\\AppData\\Local\\Programs\\mmhmem\\mmhmem.exe",
    "C:\\it's\\mmhmem.exe",
    "C:\\a\u{2019}b\\mmhmem.exe",
    "C:\\q\u{2018}\u{201A}\u{201B}''\u{2019}\\mmhmem.exe",
    "C:\\Program Files\\$env:x\\`b&c;d|e(f)\\mmhmem.exe",
    "C:\\C#\\\"d\u{201C}\u{201D}\\mmhmem.exe",
    "/home/o'brien/.local/bin/mmhmem",
    "/opt/my tools/$HOME/`x`&y/mmhmem",
    "/opt/a\u{2019}b\\c\"d/*?[x]{y}~#/mmhmem",
    "/usr/bin/mmhmem",
    "relative/mmhmem",
    "'",
];

#[test]
fn every_quoted_path_reads_back_as_itself() {
    for path in PATHS {
        for os in [Os::Windows, Os::Linux, Os::MacOs] {
            let command = codex_command(Path::new(path), os).unwrap();
            let words = words_for(&command, os);
            assert_eq!(
                words,
                Some(vec![path.to_owned(), "hook".to_owned(), "codex".to_owned()]),
                "{os:?}: {command}"
            );
            assert!(readings(&command).contains(&words.unwrap()));
        }
    }
}

#[test]
fn powershell_quotes_every_single_quote_character() {
    let path = Path::new("C:\\a'\u{2018}\u{2019}\u{201A}\u{201B}b\\mmhmem.exe");
    assert_eq!(
        codex_command(path, Os::Windows).unwrap(),
        "& 'C:\\a''\u{2018}\u{2018}\u{2019}\u{2019}\u{201A}\u{201A}\u{201B}\u{201B}b\\mmhmem.exe' hook codex"
    );
    let unix = codex_command(Path::new("/o'b/mmhmem"), Os::Linux).unwrap();
    assert_eq!(unix, "'/o'\"'\"'b/mmhmem' hook codex");
}

#[test]
fn control_characters_are_refused() {
    for path in [
        "/a\nb/mmhmem",
        "C:\\a\rb\\mmhmem.exe",
        "/a\tb/mmhmem",
        "/a\u{7f}/mmhmem",
    ] {
        for os in [Os::Windows, Os::Linux] {
            assert_eq!(
                codex_command(Path::new(path), os),
                Err(UnquotablePath::ControlCharacter)
            );
        }
    }
}

#[cfg(unix)]
#[test]
fn a_path_that_is_not_unicode_is_refused() {
    use std::os::unix::ffi::OsStrExt;
    let path = Path::new(std::ffi::OsStr::from_bytes(b"/a\xff/mmhmem"));
    assert_eq!(
        codex_command(path, Os::Linux),
        Err(UnquotablePath::NotUnicode)
    );
}

#[test]
fn equivalent_hand_quoting_reads_the_same() {
    let want = ["/opt/my tools/mmhmem", "hook", "codex"].map(str::to_owned);
    for command in [
        "'/opt/my tools/mmhmem' hook codex",
        "\"/opt/my tools/mmhmem\" hook codex",
        "/opt/my\\ tools/mmhmem   hook\tcodex",
        "/opt/'my tools'/mmhmem hook codex",
    ] {
        assert_eq!(
            words_for(command, Os::Linux).as_deref(),
            Some(&want[..]),
            "{command}"
        );
    }
    let want = ["C:\\O'B\\mmhmem.exe", "hook", "codex"].map(str::to_owned);
    for command in [
        "& 'C:\\O''B\\mmhmem.exe' hook codex",
        "&'C:\\O\u{2019}'B\\mmhmem.exe' hook codex",
        "& \"C:\\O'B\\mmhmem.exe\" hook codex",
        "& \u{201C}C:\\O'B\\mmhmem.exe\u{201D} hook codex",
    ] {
        assert_eq!(
            words_for(command, Os::Windows).as_deref(),
            Some(&want[..]),
            "{command}"
        );
    }
}

#[test]
fn commands_that_expand_or_chain_are_not_plain_words() {
    for command in [
        "/usr/bin/mmhmem hook codex; rm -rf ~",
        "/usr/bin/mmhmem hook codex | tee log",
        "$(which mmhmem) hook codex",
        "`which mmhmem` hook codex",
        "\"$HOME/mmhmem\" hook codex",
        "~/bin/mmhmem hook codex",
        "/opt/*/mmhmem hook codex",
        "/usr/bin/mmhmem hook codex &",
        "'/usr/bin/mmhmem hook codex",
        "/usr/bin/mmhmem hook codex\nevil",
        "# /usr/bin/mmhmem hook codex",
    ] {
        assert_eq!(words_for(command, Os::Linux), None, "{command}");
    }
    for command in [
        "'C:\\mmhmem.exe' hook codex",
        "& \"$env:LOCALAPPDATA\\mmhmem.exe\" hook codex",
        "& 'C:\\mmhmem.exe' hook codex; Remove-Item x",
        "& 'C:\\mmhmem.exe'x hook codex",
        "& 'C:\\mmhmem.exe hook codex",
        "C:\\mmhmem.exe hook codex | Out-Null",
        "& (Get-Command mmhmem) hook codex",
        "& \"C:\\a`nb\\mmhmem.exe\" hook codex",
    ] {
        assert_eq!(words_for(command, Os::Windows), None, "{command}");
    }
}
