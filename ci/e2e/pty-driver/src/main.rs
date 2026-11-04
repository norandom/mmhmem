//! `mmhmem-e2e-pty <script> <output-prefix>`: runs the script's program in
//! a pseudo-terminal (a pty on Linux and macOS, ConPTY on Windows) and
//! follows the script's steps (`script.rs`), the way a user at a terminal
//! would: it reads the screen and presses keys.
//!
//! Writes `<prefix>.raw` (every byte the program wrote), `<prefix>.screen`
//! (the screen after each step, and on failure), `<prefix>.final` (the last
//! screen) and `<prefix>.exit` (the program's exit code, or `killed`).
//!
//! Exit codes: 0 every step passed; 1 a step failed or timed out (the
//! program is killed); 2 the script or the terminal could not be set up.
//!
//! Terminal answers: a program (or ConPTY itself, which starts with the
//! cursor-position request `ESC [ 6 n`) that asks for the cursor position
//! gets the position of the screen model, as a terminal would answer.

mod script;

use std::fs::File;
use std::io::{Read, Write};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::{Duration, Instant};

use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use script::{ExitWant, Script, Step};

/// How long output is still read after the program ended.
const DRAIN: Duration = Duration::from_millis(700);
/// How long `select` waits for the screen to change after a ↓.
const SELECT_SETTLE: Duration = Duration::from_millis(1500);
/// The most ↓ presses of one `select`.
const SELECT_MAX: usize = 40;

struct Session {
    parser: vt100::Parser,
    output: Receiver<Vec<u8>>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send + Sync>,
    master: Option<Box<dyn MasterPty + Send>>,
    raw: File,
    screens: File,
    exit: Option<i32>,
    ended_at: Option<Instant>,
    closed: bool,
    tail: Vec<u8>,
}

impl Session {
    fn start(script: &Script, prefix: &str) -> Result<Self, String> {
        let pty = native_pty_system()
            .openpty(PtySize {
                rows: script.rows,
                cols: script.cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| format!("cannot open a pseudo-terminal: {e}"))?;
        let mut cmd = CommandBuilder::new(&script.program);
        cmd.args(&script.args);
        if let Ok(dir) = std::env::current_dir() {
            cmd.cwd(dir);
        }
        if std::env::var_os("TERM").is_none() {
            cmd.env("TERM", "xterm-256color");
        }
        for (name, value) in &script.env {
            cmd.env(name, value);
        }
        let child = pty
            .slave
            .spawn_command(cmd)
            .map_err(|e| format!("cannot start {}: {e}", script.program))?;
        drop(pty.slave);
        let mut reader = pty.master.try_clone_reader().map_err(|e| e.to_string())?;
        let writer = pty.master.take_writer().map_err(|e| e.to_string())?;
        let (tx, output) = channel();
        std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            while let Ok(n) = reader.read(&mut buf) {
                if n == 0 || tx.send(buf[..n].to_vec()).is_err() {
                    break;
                }
            }
        });
        let create = |ext: &str| File::create(format!("{prefix}.{ext}")).map_err(|e| e.to_string());
        Ok(Self {
            parser: vt100::Parser::new(nonzero(script.rows), nonzero(script.cols), 0),
            output,
            writer,
            child,
            master: Some(pty.master),
            raw: create("raw")?,
            screens: create("screen")?,
            exit: None,
            ended_at: None,
            closed: false,
            tail: Vec::new(),
        })
    }

    /// Reads output for up to `wait`; notices the end of the program.
    fn pump(&mut self, wait: Duration) {
        match self.output.recv_timeout(wait) {
            Ok(bytes) => self.feed(&bytes),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                self.closed = true;
                std::thread::sleep(wait.min(Duration::from_millis(50)));
            }
        }
        if self.exit.is_none()
            && let Ok(Some(status)) = self.child.try_wait()
        {
            self.exit = Some(status.exit_code() as i32);
            self.ended_at = Some(Instant::now());
        }
        // ConPTY keeps its output open until the pseudo console is closed.
        if self.ended_at.is_some_and(|t| t.elapsed() > DRAIN) && self.master.is_some() {
            self.master = None;
        }
    }

    fn feed(&mut self, bytes: &[u8]) {
        let _ = self.raw.write_all(bytes);
        self.parser.process(bytes);
        // A request may be split across two reads.
        let mut window = std::mem::take(&mut self.tail);
        let old = window.len();
        window.extend_from_slice(bytes);
        let answers = answers(&window, old, self.parser.screen().cursor_position());
        if !answers.is_empty() {
            let _ = self.writer.write_all(&answers);
            let _ = self.writer.flush();
        }
        let keep = window.len().min(3);
        self.tail = window[window.len() - keep..].to_vec();
    }

    fn finished(&self) -> bool {
        self.exit.is_some() && (self.closed || self.master.is_none())
    }

    fn screen(&self) -> String {
        let screen = self.parser.screen();
        let (_, cols) = screen.size();
        let rows: Vec<String> = screen
            .rows(0, cols.get())
            .map(|r| r.trim_end().to_owned())
            .collect();
        let mut text = rows.join("\n");
        text.truncate(text.trim_end().len());
        text
    }

    fn dump(&mut self, title: &str) {
        let text = format!(
            "===== {title} =====\n{}\n=====\n",
            self.screen().replace('\n', "\n| ")
        );
        let _ = self.screens.write_all(text.as_bytes());
        let _ = self.screens.flush();
    }

    /// Waits until `done` holds for the screen, or the time is up.
    fn wait_for(&mut self, limit: Duration, done: impl Fn(&str) -> bool) -> bool {
        let deadline = Instant::now() + limit;
        loop {
            if done(&self.screen()) {
                return true;
            }
            if Instant::now() >= deadline || self.finished() {
                // One last look at everything already received.
                self.pump(Duration::from_millis(10));
                return done(&self.screen());
            }
            self.pump(Duration::from_millis(100));
        }
    }

    fn send(&mut self, bytes: &[u8]) -> Result<(), String> {
        if self.exit.is_some() {
            return Err("the program has already ended".to_owned());
        }
        self.writer
            .write_all(bytes)
            .and_then(|()| self.writer.flush())
            .map_err(|e| format!("cannot type into the terminal: {e}"))
    }

    fn select(&mut self, re: &regex::Regex, limit: Duration) -> Result<(), String> {
        let matches = |s: &str| s.lines().any(|l| re.is_match(l));
        if self.wait_for(limit.min(SELECT_SETTLE), matches) {
            return Ok(());
        }
        for _ in 0..SELECT_MAX {
            let before = self.screen();
            self.send(b"\x1b[B")?;
            self.wait_for(SELECT_SETTLE, |s| s != before);
            if self.wait_for(Duration::from_millis(200), matches) {
                return Ok(());
            }
        }
        Err(format!(
            "no line matched /{re}/ after {SELECT_MAX} presses of ↓"
        ))
    }

    fn wait_exit(&mut self, want: &ExitWant, limit: Duration) -> Result<(), String> {
        let deadline = Instant::now() + limit;
        while !self.finished() {
            if Instant::now() >= deadline {
                return Err(format!(
                    "the program did not end within {}s",
                    limit.as_secs()
                ));
            }
            self.pump(Duration::from_millis(100));
        }
        let code = self.exit.unwrap_or(-1);
        if want.accepts(code) {
            Ok(())
        } else {
            Err(format!(
                "the program ended with exit code {code}, wanted {want:?}"
            ))
        }
    }

    fn run(&mut self, step: &Step, limit: Duration) -> Result<(), String> {
        match step {
            Step::Timeout(_) => Ok(()),
            Step::Expect(re) => self
                .wait_for(limit, |s| re.is_match(s))
                .then_some(())
                .ok_or_else(|| format!("the screen did not show /{re}/")),
            Step::Gone(re) => self
                .wait_for(limit, |s| !re.is_match(s))
                .then_some(())
                .ok_or_else(|| format!("the screen still shows /{re}/")),
            Step::Select(re) => self.select(re, limit),
            Step::Send(bytes) => self.send(bytes),
            Step::Pause(wait) => {
                let until = Instant::now() + *wait;
                while Instant::now() < until {
                    self.pump(Duration::from_millis(50));
                }
                Ok(())
            }
            Step::Exit(want) => self.wait_exit(want, limit),
        }
    }
}

/// What a terminal answers to the requests in `window` that end after its
/// first `old` bytes (those were answered already): the cursor position
/// (`ESC [ 6 n`, 1-based) and the primary device attributes (`ESC [ c`).
fn answers(window: &[u8], old: usize, (row, col): (u16, u16)) -> Vec<u8> {
    let mut out = Vec::new();
    for i in 0..window.len() {
        let rest = &window[i..];
        let ends_new = |len: usize| rest.len() >= len && i + len > old;
        if rest.starts_with(b"[6n") && ends_new(4) {
            out.extend(format!("[{};{}R", row + 1, col + 1).into_bytes());
        } else if (rest.starts_with(b"[c") && ends_new(3))
            || (rest.starts_with(b"[0c") && ends_new(4))
        {
            out.extend_from_slice(b"[?1;2c");
        }
    }
    out
}

fn nonzero(n: u16) -> std::num::NonZeroU16 {
    std::num::NonZeroU16::new(n).unwrap_or(std::num::NonZeroU16::MIN)
}

fn run_script(script: &Script, prefix: &str) -> Result<i32, String> {
    let mut session = Session::start(script, prefix).map_err(|e| format!("setup: {e}"))?;
    let mut limit = Duration::from_secs(120);
    let started = Instant::now();
    let mut failure = None;
    for (line, text, step) in &script.steps {
        if let Step::Timeout(t) = step {
            limit = *t;
        }
        println!(
            "[{:>6.1}s] line {line}: {text}",
            started.elapsed().as_secs_f32()
        );
        if let Err(e) = session.run(step, limit) {
            failure = Some(format!("line {line} ({text}): {e}"));
            break;
        }
        session.dump(&format!("after line {line}: {text}"));
    }
    if failure.is_some() && session.exit.is_none() {
        let _ = session.child.kill();
    }
    // Whatever is still on its way, for the logs.
    session.pump(Duration::from_millis(200));
    session.dump(if failure.is_some() { "FAILED" } else { "final" });
    let exit = session.exit.map_or("killed".to_owned(), |c| c.to_string());
    std::fs::write(format!("{prefix}.exit"), &exit).map_err(|e| e.to_string())?;
    std::fs::write(format!("{prefix}.final"), session.screen()).map_err(|e| e.to_string())?;
    println!("program exit code: {exit}");
    println!(
        "----- screen -----\n{}\n------------------",
        session.screen()
    );
    match failure {
        Some(e) => {
            eprintln!("FAILED: {e}");
            Ok(1)
        }
        None => Ok(0),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [script_file, prefix] = args.as_slice() else {
        eprintln!("usage: mmhmem-e2e-pty <script> <output-prefix>");
        std::process::exit(2);
    };
    let script = std::fs::read_to_string(script_file)
        .map_err(|e| format!("cannot read {script_file}: {e}"))
        .and_then(|text| script::parse(&text));
    let code = match script.and_then(|s| run_script(&s, prefix)) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("{e}");
            2
        }
    };
    std::process::exit(code);
}

#[cfg(test)]
mod tests {
    use super::answers;

    #[test]
    fn answers_each_request_once_even_when_split() {
        assert_eq!(answers(b"ab[6ncd", 0, (2, 4)), b"[3;5R");
        assert_eq!(answers(b"[c[0c", 0, (0, 0)), b"[?1;2c[?1;2c");
        // "[6" arrived before, "n" now: answered now.
        assert_eq!(answers(b"[6n", 3, (0, 0)), b"[1;1R");
        // The whole request was in the earlier bytes: not again.
        assert_eq!(answers(b"[c", 3, (0, 0)), b"");
        assert_eq!(answers(b"plain text", 0, (0, 0)), b"");
    }
}
