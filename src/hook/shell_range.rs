//! Partial shell readers whose lines can be sized (shunt-cache-efficiency
//! task 3.1, design.md "LineRange and span rule (`hook`)"): `sed -n`,
//! `head` and `tail` on one file. Recognition stays conservative: a count
//! that is not a plain non-negative integer, any other option, a second
//! file or a script other than one print command is `None`, so the call is
//! not a read and proceeds unchanged (Req 3.5).

use super::LineRange;
use super::shell::Token;

/// `(file, range)` for `sed -n A,Bp FILE`, `sed -n Ap FILE`, `head [-n N |
/// -nN | -N] FILE` (lines 1 to N) and `tail [-n N | -nN | -N] FILE` (the
/// last N lines) or `tail -n +N FILE` (line N to the end); `head` and
/// `tail` without a count print 10 lines. Program names are exact
/// (lower-case); everything else is `None`.
pub(super) fn partial<'a>(name: &str, args: &'a [Token]) -> Option<(&'a str, LineRange)> {
    match name {
        "sed" => sed(args),
        "head" => match counted(args)? {
            (file, Count::Lines(n)) => Some((file, LineRange::new(1, Some(n)))),
            (_, Count::From(_)) => None,
        },
        "tail" => match counted(args)? {
            (file, Count::Lines(n)) => Some((file, LineRange::from_tail(n))),
            (file, Count::From(n)) => Some((file, LineRange::new(n, None))),
        },
        _ => None,
    }
}

/// Lines `head` and `tail` print without a count.
const DEFAULT_COUNT: u64 = 10;

/// A `head`/`tail` count: `N` lines, or `+N` (from line N to the end).
enum Count {
    Lines(u64),
    From(u64),
}

/// `sed -n A,Bp FILE` or `sed -n Ap FILE` with `A >= 1`. Only the script
/// may hold an unquoted comma (the tokenizer lets commas through for
/// `sed`), so a file like `a,b` must be quoted.
fn sed(args: &[Token]) -> Option<(&str, LineRange)> {
    let [flag, script, file] = args else {
        return None;
    };
    let plain_file = !file.is_option() && (file.quoted || !file.text.contains(','));
    if flag.quoted || flag.text != "-n" || !plain_file {
        return None;
    }
    let body = script.text.strip_suffix('p')?;
    let (first, last) = match body.split_once(',') {
        Some((a, b)) => (digits(a)?, digits(b)?),
        None => (digits(body)?, digits(body)?),
    };
    (first > 0).then(|| (file.text.as_str(), LineRange::new(first, Some(last))))
}

/// The file and count of `head`/`tail`: `FILE`, `-n N FILE`, `-nN FILE` or
/// `-N FILE` (`+N` only after `-n`).
fn counted(args: &[Token]) -> Option<(&str, Count)> {
    let (file, options) = args.split_last()?;
    if file.is_option() {
        return None;
    }
    let count = match options {
        [] => Count::Lines(DEFAULT_COUNT),
        [flag, value] if flag.is_option() && flag.text == "-n" => count(&value.text)?,
        [flag] if flag.is_option() => match flag.text.strip_prefix("-n") {
            Some(value) => count(value)?,
            None => Count::Lines(digits(&flag.text[1..])?),
        },
        _ => return None,
    };
    Some((&file.text, count))
}

/// `N` or `+N`.
fn count(text: &str) -> Option<Count> {
    match text.strip_prefix('+') {
        Some(from) => digits(from).map(Count::From),
        None => digits(text).map(Count::Lines),
    }
}

/// A plain non-negative decimal integer that fits in `u64`.
fn digits(text: &str) -> Option<u64> {
    let plain = !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit());
    plain.then(|| text.parse().ok()).flatten()
}

#[cfg(test)]
mod tests {
    use crate::hook::LineRange;
    use crate::hook::shell::recognise;
    use std::path::{Path, PathBuf};

    fn cwd() -> PathBuf {
        PathBuf::from("/work/proj")
    }

    fn ranged(command: &str) -> Option<(PathBuf, Option<LineRange>)> {
        recognise(command, &cwd()).map(|r| (r.path, r.range))
    }

    #[test]
    fn sized_partial_readers_become_ranged_reads() {
        let lines = |a, b| Some(LineRange::new(a, Some(b)));
        let cases: &[(&str, &str, Option<LineRange>)] = &[
            ("sed -n 1,400p big.rs", "big.rs", lines(1, 400)),
            ("sed -n '10,20p' big.rs", "big.rs", lines(10, 20)),
            ("sed -n \"5p\" a.rs", "a.rs", lines(5, 5)),
            ("sed -n 1,5p 'a,b.rs'", "a,b.rs", lines(1, 5)),
            ("head a.rs", "a.rs", lines(1, 10)),
            ("head -n 50 a.rs", "a.rs", lines(1, 50)),
            ("head -n50 a.rs", "a.rs", lines(1, 50)),
            ("head -50 a.rs", "a.rs", lines(1, 50)),
            ("head -n '7' a.rs", "a.rs", lines(1, 7)),
            ("tail a.rs", "a.rs", Some(LineRange::from_tail(10))),
            ("tail -n 50 a.rs", "a.rs", Some(LineRange::from_tail(50))),
            ("tail -n50 a.rs", "a.rs", Some(LineRange::from_tail(50))),
            ("tail -50 a.rs", "a.rs", Some(LineRange::from_tail(50))),
            ("tail -n +2 a.rs", "a.rs", Some(LineRange::new(2, None))),
            ("tail -n+2 a.rs", "a.rs", Some(LineRange::new(2, None))),
            ("tail -n +0 a.rs", "a.rs", Some(LineRange::new(1, None))),
        ];
        for (command, file, range) in cases {
            let want = (cwd().join(file), *range);
            assert_eq!(ranged(command), Some(want), "{command:?}");
        }
        let absolute = ranged("sed -n 1,2p /etc/hosts").expect("read");
        assert_eq!(absolute.0, Path::new("/etc/hosts"));
    }

    #[test]
    fn partial_readers_that_cannot_be_sized_are_not_reads() {
        for command in [
            "sed 1,5p a.rs",
            "sed -n 1,5 a.rs",
            "sed -n 0,5p a.rs",
            "sed -n 1,5p",
            "sed -n 1,5p a.rs b.rs",
            "sed -n '1,5p;9q' a.rs",
            "sed -n 1,$p a.rs",
            "sed -n x,5p a.rs",
            "sed -n 1,5p a,b.rs",
            "sed -i 1,5p a.rs",
            "sed -n -e 1,5p a.rs",
            "sed '-n' 1,5p a.rs",
            "SED -n 1,5p a.rs",
            "head -n x a.rs",
            "head -n -5 a.rs",
            "head -n 1.5 a.rs",
            "head -n +5 a.rs",
            "head -c 5 a.rs",
            "head -n 5",
            "head -n 5 a.rs b.rs",
            "head -n 5 -q a.rs",
            "head a.rs,b.rs",
            "head -n 5 a.rs | cat",
            "tail -f a.rs",
            "tail -n 99999999999999999999999 a.rs",
            "tail -+5 a.rs",
            "tail -n 5 *.rs",
            "Head -n 5 a.rs",
        ] {
            assert_eq!(ranged(command), None, "{command:?}");
        }
    }
}
