//! Req 13.3 machine enforcement: no `src/` file over 500 lines, no function in
//! `src/` over 80 lines, no test file over 800 lines. Functions are measured
//! from the signature line to the matching close brace over a "skeleton" in
//! which string/char literals and comments are blanked, so only real code
//! braces drive the depth count (braces inside string literals are ignored;
//! `json!`/`format!` braces are balanced code braces).

use std::fs;
use std::path::{Path, PathBuf};

/// Lexical state of the blanker.
#[derive(Clone, Copy, PartialEq)]
enum St {
    Normal,
    Str,
    Block,
    Raw(u8),
}

/// Blanks every byte that is not code (string contents, comments), preserving
/// newlines, so `{`/`}` in the result are real code braces only.
fn blank_non_code(src: &str) -> String {
    let b = src.as_bytes();
    let mut out = b.to_vec();
    let mut i = 0usize;
    let mut st = St::Normal;
    while i < b.len() {
        st = match st {
            St::Normal => step_normal(b, &mut out, &mut i),
            St::Str => step_str(b, &mut out, &mut i),
            St::Block => step_block(b, &mut out, &mut i),
            St::Raw(n) => step_raw(b, &mut out, &mut i, n),
        };
    }
    String::from_utf8(out).expect("blanking preserves utf8")
}

/// One byte in Normal state: comments, strings, char literals, raw strings.
fn step_normal(b: &[u8], out: &mut [u8], i: &mut usize) -> St {
    let nxt = *i + 1 < b.len();
    match b[*i] {
        b'/' if nxt && b[*i + 1] == b'/' => skip_line_comment(b, out, i),
        b'/' if nxt && b[*i + 1] == b'*' => {
            out[*i] = b' ';
            out[*i + 1] = b' ';
            *i += 2;
            St::Block
        }
        b'"' => {
            out[*i] = b' ';
            *i += 1;
            St::Str
        }
        b'\'' if char_literal_end(b, *i).is_some() => {
            let end = char_literal_end(b, *i).expect("just checked");
            blank_range(out, *i, end);
            *i = end;
            St::Normal
        }
        _ if starts_raw_string(b, *i) => {
            // Enter Raw state PAST the opening quote. Stopping at the quote
            // made `raw_closes` judge the OPENER for 0 hashes, where its
            // hash check is vacuously true: the quote closed itself, the
            // content stayed as code, and the real closing quote opened a
            // Str state that blanks newlines (task 4.1 rider). Blanking
            // through `r`, the hashes and the quote fixes 0-hash `r"..."`
            // and `r#"..."#` openers alike.
            let hashes = raw_hashes(b, *i);
            let after_open = (*i + 1 + hashes as usize + 1).min(b.len());
            blank_range(out, *i, after_open);
            *i = after_open;
            St::Raw(hashes)
        }
        _ => {
            *i += 1;
            St::Normal
        }
    }
}

/// One byte in String state until the unescaped closing quote.
fn step_str(b: &[u8], out: &mut [u8], i: &mut usize) -> St {
    out[*i] = b' ';
    if b[*i] == b'\\' && *i + 1 < b.len() {
        out[*i + 1] = b' ';
        *i += 2;
        return St::Str;
    }
    let closed = b[*i] == b'"';
    *i += 1;
    if closed { St::Normal } else { St::Str }
}

/// One byte in Block-comment state until `*/`.
fn step_block(b: &[u8], out: &mut [u8], i: &mut usize) -> St {
    let closed = b[*i] == b'*' && *i + 1 < b.len() && b[*i + 1] == b'/';
    if b[*i] != b'\n' {
        out[*i] = b' ';
    }
    if closed {
        out[*i + 1] = b' ';
        *i += 2;
        return St::Normal;
    }
    *i += 1;
    St::Block
}

/// One byte in Raw-string state until the quote plus its hashes.
fn step_raw(b: &[u8], out: &mut [u8], i: &mut usize, hashes: u8) -> St {
    if b[*i] == b'"' && raw_closes(b, *i, hashes) {
        let end = (*i + 1 + hashes as usize).min(b.len());
        blank_range(out, *i, end);
        *i = end;
        return St::Normal;
    }
    if b[*i] != b'\n' {
        out[*i] = b' ';
    }
    *i += 1;
    St::Raw(hashes)
}

/// Blanks `out[a..b]` (clamped).
fn blank_range(out: &mut [u8], a: usize, b: usize) {
    for j in a..b.min(out.len()) {
        out[j] = b' ';
    }
}

/// True when the `r` at `b[i]` opens an `r"..."` / `r#"..."#` literal: `r`,
/// zero or more `#`, then the opening quote. The quote is required so raw
/// identifiers (`r#type`) are not mistaken for raw strings.
fn starts_raw_string(b: &[u8], i: usize) -> bool {
    if b[i] != b'r' {
        return false;
    }
    let mut j = i + 1;
    while j < b.len() && b[j] == b'#' {
        j += 1;
    }
    j < b.len() && b[j] == b'"'
}

/// Number of `#` in the raw-string opening whose `r` is at `b[i]`.
fn raw_hashes(b: &[u8], i: usize) -> u8 {
    let mut j = i + 1;
    let mut n = 0u8;
    while j < b.len() && b[j] == b'#' {
        n += 1;
        j += 1;
    }
    n
}

/// True when the quote at `b[i]` is followed by `hashes` `#` bytes. For 0
/// hashes every quote closes (the empty hash check is vacuous), which is
/// correct for a content quote — the OPENER must never be evaluated, so
/// `step_normal` skips past it before entering `St::Raw`.
fn raw_closes(b: &[u8], i: usize, hashes: u8) -> bool {
    let end = i + 1 + hashes as usize;
    end <= b.len() && b[i + 1..end].iter().all(|&c| c == b'#')
}

/// End (exclusive) of the char literal whose tick is at `b[i]`, or None when
/// the tick opens a lifetime instead (`'a` with no closing quote).
fn char_literal_end(b: &[u8], i: usize) -> Option<usize> {
    let escaped = b.get(i + 1) == Some(&b'\\');
    let simple = i + 2 < b.len() && b[i + 2] == b'\'';
    if !escaped && !simple {
        return None;
    }
    let mut j = i + 1;
    while j + 1 < b.len() {
        if b[j] == b'\\' {
            j += 2;
        } else if b[j] == b'\'' {
            return Some(j + 1);
        } else {
            j += 1;
        }
    }
    None
}

/// Skips a `//` comment to end of line, blanking it (the `\n` is kept).
fn skip_line_comment(b: &[u8], out: &mut [u8], i: &mut usize) -> St {
    while *i < b.len() && b[*i] != b'\n' {
        out[*i] = b' ';
        *i += 1;
    }
    St::Normal
}

/// All `fn` extents in the skeleton: (name, signature line, closing-brace
/// line), 1-based inclusive. Bodyless declarations (trait methods) are
/// skipped; nested fns report their own extents too.
fn fn_extents(skeleton: &str) -> Vec<(String, usize, usize)> {
    let lines: Vec<&str> = skeleton.lines().collect();
    let mut out = Vec::new();
    for (idx, line) in lines.iter().enumerate() {
        if is_fn_signature(line)
            && let Some(close) = body_close_line(&lines, idx)
        {
            out.push((fn_name(line), idx + 1, close + 1));
        }
    }
    out
}

/// True when the (blanked) line starts a function definition.
fn is_fn_signature(line: &str) -> bool {
    let mut t = line.trim_start();
    loop {
        let stripped = ["pub", "async", "unsafe", "const", "extern"]
            .iter()
            .find_map(|p| t.strip_prefix(p));
        match stripped {
            Some(rest) => {
                t = rest.trim_start();
                if let Some((_, after)) = t.split_once(')') {
                    t = after.trim_start();
                }
            }
            None => break,
        }
    }
    t.starts_with("fn ") || t == "fn"
}

/// The name of the fn starting on this line (for failure messages).
fn fn_name(line: &str) -> String {
    line.find("fn ")
        .and_then(|p| {
            line[p + 3..]
                .trim_start()
                .split(['(', '<', ' '])
                .next()
                .map(str::to_string)
        })
        .unwrap_or_else(|| "?".to_string())
}

/// 0-based line of the brace closing the body of the fn at `sig`; None when
/// the declaration has no body (`;` before any brace).
fn body_close_line(lines: &[&str], sig: usize) -> Option<usize> {
    let mut depth = 0i64;
    let mut seen_open = false;
    for (idx, line) in lines.iter().enumerate().skip(sig) {
        for ch in line.bytes() {
            match ch {
                b';' if !seen_open => return None,
                b'{' => {
                    depth += 1;
                    seen_open = true;
                }
                b'}' => depth -= 1,
                _ => {}
            }
            if seen_open && depth == 0 {
                return Some(idx);
            }
        }
    }
    None
}

/// Recursively collects `.rs` files under `root`.
fn rs_files(root: &Path, into: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            rs_files(&p, into);
        } else if p.extension().is_some_and(|e| e == "rs") {
            into.push(p);
        }
    }
}

#[test]
fn req_13_3_size_caps_hold() {
    let mut src = Vec::new();
    rs_files(Path::new("src"), &mut src);
    assert!(!src.is_empty(), "src/ not found — run from the repo root");

    for f in &src {
        let content = fs::read_to_string(f).expect("read src file");
        let lines = content.lines().count();
        assert!(
            lines <= 500,
            "Req 13.3: {} is {lines} lines (cap 500) — split by responsibility",
            f.display()
        );
        for (name, sig, close) in fn_extents(&blank_non_code(&content)) {
            let span = close - sig + 1;
            assert!(
                span <= 80,
                "Req 13.3: {}::{name} spans {span} lines (cap 80) — split by responsibility",
                f.display()
            );
        }
    }

    let mut tests = Vec::new();
    rs_files(Path::new("tests"), &mut tests);
    for f in &tests {
        let lines = fs::read_to_string(f)
            .expect("read test file")
            .lines()
            .count();
        assert!(
            lines <= 800,
            "Req 13.3: {} is {lines} lines (cap 800)",
            f.display()
        );
    }
}

/// Task 4.1 rider (2.2 review, 2026-11-07): 0-hash `r"..."` raw strings
/// must not collapse the skeleton. The old lexer consumed only the `r`,
/// entered `St::Raw(0)` AT the opening quote, and `raw_closes` then treated
/// that quote as its own closer (its hash check is vacuous for 0 hashes),
/// so the raw content stayed as code and the raw string's CLOSING quote
/// opened `St::Str`, which blanks newlines — merging lines until the next
/// quote and letting >80-line functions escape the cap.
#[test]
fn zero_hash_raw_strings_do_not_collapse_the_skeleton() {
    // A 100-line function (cap 80): signature, a 0-hash raw string on the
    // first body line, quote-bearing lines at 39 and 60 that the old lexer
    // chained into long Str states, filler, closing brace on line 100.
    let mut src = String::from("fn over_cap_raw_string() {\n");
    src.push_str("    let banner = r\"no quotes inside\";\n"); // line 2
    for line in 3..39 {
        src.push_str(&format!("    let a{line} = {line};\n"));
    }
    src.push_str("    let msg = \"pair one\";\n"); // line 39
    for line in 40..60 {
        src.push_str(&format!("    let b{line} = {line};\n"));
    }
    src.push_str("    let q = '\"';\n"); // line 60
    for line in 61..100 {
        src.push_str(&format!("    let c{line} = {line};\n"));
    }
    src.push_str("}\n"); // line 100
    assert_eq!(src.lines().count(), 100, "fixture sanity: 100 lines");

    let skeleton = blank_non_code(&src);
    assert_eq!(
        skeleton.lines().count(),
        100,
        "blanking must preserve every newline (0-hash raw strings)"
    );
    assert!(
        !skeleton.contains("no quotes inside"),
        "raw-string content must be blanked, not kept as code"
    );
    let (_, sig, close) = fn_extents(&skeleton)
        .into_iter()
        .find(|(name, _, _)| name == "over_cap_raw_string")
        .expect("fn extent");
    assert_eq!(
        close - sig + 1,
        100,
        "the 100-line function must measure 100 skeleton lines (cap 80)"
    );
}
