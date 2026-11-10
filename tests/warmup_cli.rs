//! `mmhmem warmup` end to end (shunt-cache-efficiency tasks 6.1/6.2; Req
//! 6.1-6.6): the real binary over a temp git repository with an in-test
//! worker stub (the tests/warmup_prune.rs pattern). The first run summarises
//! the three large files, the second only hits, the dry run writes nothing,
//! and without a worker the command exits 1 without creating a store.

use std::path::Path;
use std::process::Output;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::process::Command;

use mmhmem::config::DEFAULT_SHUNT_MIN_LINES;
use mmhmem::services::shunt::ShuntError;

/// A canned chat-completions response with real usage numbers.
const SUMMARY_CHAT: &str = r#"{"choices":[{"message":{"content":"- foo [L1-L9]\n- bar [L10-L20]"}}],"usage":{"prompt_tokens":500,"completion_tokens":30}}"#;

/// Above the shipped threshold, so warm-up summarises it.
const BIG: usize = DEFAULT_SHUNT_MIN_LINES as usize + 50;

/// The three files a warm-up must summarise.
const LARGE: [&str; 3] = ["one.rs", "src/three.rs", "src/two.rs"];

fn lines(n: usize, tag: &str) -> String {
    (0..n).map(|i| format!("{tag}: line {i}\n")).collect()
}

fn write(root: &Path, rel: &str, body: &str) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, body).unwrap();
}

fn git(dir: &Path, args: &[&str]) -> bool {
    std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .is_ok_and(|o| o.status.success())
}

/// A repository with three large files (one tracked, two untracked), an
/// ignored large file, a short file and a security-critical large file.
fn repository() -> Option<tempfile::TempDir> {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    if !git(root, &["init", "-q"]) {
        eprintln!("git is not available; skipping");
        return None;
    }
    write(root, ".gitignore", "build/\n");
    for rel in LARGE {
        write(root, rel, &lines(BIG, rel));
    }
    write(root, "build/generated.rs", &lines(BIG, "generated"));
    write(root, "notes.md", &lines(10, "notes"));
    write(root, "src/session_token.rs", &lines(BIG, "token"));
    assert!(git(root, &["add", "one.rs"]), "git add");
    Some(dir)
}

/// Runs `mmhmem warmup <args>` with the store in `data` and the working
/// folder there too (the repository is always passed explicitly).
async fn warmup(data: &Path, worker: Option<&str>, args: &[&str]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_mmhmem"));
    cmd.arg("warmup")
        .args(args)
        .current_dir(data)
        .env("AGENT_MEM_DATA_DIR", data)
        .env_remove("AGENT_MEM_LOG_FILE")
        .env_remove("AGENT_MEM_ALLOWED_ROOTS")
        .env_remove("SHUNT_MIN_LINES")
        .env_remove("OTEL_EXPORTER_OTLP_ENDPOINT")
        // Blank — not removed: removed worker vars fall back to the
        // machine's global .mmhmem.json (armed on dev hosts).
        .env("WORKER_API_KEY", "");
    match worker {
        Some(url) => cmd.env("WORKER_API_URL", url).env("WORKER_MODEL", "stub"),
        None => cmd.env("WORKER_API_URL", "").env("WORKER_MODEL", ""),
    };
    cmd.output().await.expect("run mmhmem warmup")
}

fn text(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

fn find_subslice(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// HTTP stub: serves `SUMMARY_CHAT` to up to `max` connections, counting
/// requests. Returns (url, request_count).
async fn spawn_stub(max: usize) -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let count = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&count);
    tokio::spawn(async move {
        for _ in 0..max {
            let Ok((mut sock, _)) = listener.accept().await else {
                break;
            };
            seen.fetch_add(1, Ordering::SeqCst);
            let mut buf: Vec<u8> = Vec::new();
            let mut tmp = [0u8; 8192];
            let header_end = loop {
                let r = sock.read(&mut tmp).await.unwrap_or(0);
                if r == 0 {
                    break buf.len();
                }
                buf.extend_from_slice(&tmp[..r]);
                if let Some(pos) = find_subslice(&buf, b"\r\n\r\n") {
                    break pos + 4;
                }
            };
            let head = String::from_utf8_lossy(&buf[..header_end]).to_lowercase();
            let need: usize = head
                .lines()
                .find_map(|l| l.strip_prefix("content-length:"))
                .and_then(|v| v.trim().parse().ok())
                .unwrap_or(0);
            let mut body = buf[header_end..].to_vec();
            while body.len() < need {
                let r = sock.read(&mut tmp).await.unwrap_or(0);
                if r == 0 {
                    break;
                }
                body.extend_from_slice(&tmp[..r]);
            }
            let resp = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{SUMMARY_CHAT}",
                SUMMARY_CHAT.len()
            );
            let _ = sock.write_all(resp.as_bytes()).await;
        }
    });
    (format!("http://{addr}/v1/chat/completions"), count)
}

#[tokio::test]
async fn the_first_run_summarises_three_files_and_the_second_only_hits() {
    let Some(repo) = repository() else { return };
    let data = tempfile::tempdir().unwrap();
    let (url, calls) = spawn_stub(10).await;
    let root = repo.path().to_str().unwrap();

    let first = warmup(data.path(), Some(&url), &[root]).await;
    let out = text(&first);
    assert!(first.status.success(), "{out}");
    assert_eq!(
        calls.load(Ordering::SeqCst),
        3,
        "one worker call per file: {out}"
    );
    assert!(out.contains("3 summaries produced, 0 hits"), "{out}");
    assert!(out.contains("6 files examined"), "{out}");
    // .gitignore and notes.md.
    assert!(out.contains("2 below threshold"), "{out}");
    assert!(out.contains("1 security-critical"), "{out}");
    assert!(
        out.contains("Worker tokens: 1500 prompt + 90 completion."),
        "{out}"
    );
    assert!(!out.contains("second run: hits only"), "{out}");
    assert!(
        !out.contains("generated.rs"),
        "ignored files never appear: {out}"
    );

    // The same root through --root: hits only, no worker call.
    let second = warmup(data.path(), Some(&url), &["--root", root]).await;
    let out = text(&second);
    assert!(second.status.success(), "{out}");
    assert_eq!(calls.load(Ordering::SeqCst), 3, "no worker call: {out}");
    assert!(out.contains("0 summaries produced, 3 hits"), "{out}");
    assert!(
        out.contains("Worker tokens: 0 prompt + 0 completion."),
        "{out}"
    );
    assert!(out.contains("second run: hits only"), "{out}");

    // The statistics stay honest: three delegations, three hits.
    let audit = std::fs::read_to_string(data.path().join("audit.jsonl")).unwrap();
    let count = |kind: &str| audit.lines().filter(|l| l.contains(kind)).count();
    assert_eq!(count("\"IO_SHUNT_DELEGATED\""), 3, "{audit}");
    assert_eq!(count("\"IO_SHUNT_CACHE_HIT\""), 3, "{audit}");
}

#[tokio::test]
async fn the_dry_run_lists_the_candidates_and_writes_nothing() {
    let Some(repo) = repository() else { return };
    let data = tempfile::tempdir().unwrap();
    let (url, calls) = spawn_stub(10).await;
    let root = repo.path().to_str().unwrap();

    let dry = warmup(data.path(), Some(&url), &[root, "--dry-run"]).await;
    let out = text(&dry);
    assert!(dry.status.success(), "{out}");
    for rel in LARGE {
        assert!(out.contains(&format!("- {rel}\n")), "{rel}: {out}");
    }
    assert!(!out.contains("session_token.rs"), "{out}");
    assert!(
        out.contains("would summarise 3 files (worker calls); 0 already cached"),
        "{out}"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0, "{out}");
    let left: Vec<_> = std::fs::read_dir(data.path()).unwrap().collect();
    assert!(
        left.is_empty(),
        "a dry run creates no store and no log: {left:?}"
    );

    // After a real run the dry run sees the cache and leaves the store as is.
    assert!(
        warmup(data.path(), Some(&url), &[root])
            .await
            .status
            .success()
    );
    let store = data.path().join("memory.db");
    let before = std::fs::read(&store).unwrap();
    let dry = warmup(data.path(), Some(&url), &["--dry-run", "--root", root]).await;
    let out = text(&dry);
    assert!(
        out.contains("would summarise 0 files (worker calls); 3 already cached"),
        "{out}"
    );
    assert_eq!(std::fs::read(&store).unwrap(), before, "untouched store");
    assert_eq!(calls.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn without_a_worker_it_says_so_exits_1_and_changes_nothing() {
    let Some(repo) = repository() else { return };
    let data = tempfile::tempdir().unwrap();
    let root = repo.path().to_str().unwrap();
    let out = warmup(data.path(), None, &[root]).await;
    let all = text(&out);
    assert_eq!(out.status.code(), Some(1), "{all}");
    assert!(
        all.contains(&ShuntError::WorkerNotConfigured.to_string()),
        "{all}"
    );
    assert!(all.contains("nothing was changed"), "{all}");
    let left: Vec<_> = std::fs::read_dir(data.path()).unwrap().collect();
    assert!(left.is_empty(), "no store, no rows, no log: {left:?}");
}

#[tokio::test]
async fn the_cap_stops_the_run_with_a_clear_message() {
    let Some(repo) = repository() else { return };
    let data = tempfile::tempdir().unwrap();
    let root = repo.path().to_str().unwrap();
    let out = warmup(data.path(), None, &[root, "--dry-run", "--max-files", "2"]).await;
    let all = text(&out);
    assert!(out.status.success(), "{all}");
    assert!(
        all.contains("stopped at 2 files; raise --max-files to continue"),
        "{all}"
    );
    assert!(all.contains("would summarise 2 files"), "{all}");
}

#[tokio::test]
async fn bad_arguments_print_the_usage_and_exit_2() {
    let data = tempfile::tempdir().unwrap();
    for args in [
        &["--bogus"][..],
        &["--max-files"][..],
        &["--max-files", "0"][..],
        &["a", "b"][..],
        &["a", "--root", "b"][..],
    ] {
        let out = warmup(data.path(), None, args).await;
        let all = text(&out);
        assert_eq!(out.status.code(), Some(2), "{args:?}: {all}");
        assert!(all.contains("usage: mmhmem warmup"), "{args:?}: {all}");
    }
    let missing = data.path().join("no-such-folder");
    let out = warmup(data.path(), None, &[missing.to_str().unwrap()]).await;
    assert_eq!(out.status.code(), Some(1), "{}", text(&out));
    assert!(text(&out).contains("not a folder"), "{}", text(&out));
}
