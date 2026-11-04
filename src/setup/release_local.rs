//! `mmhmem setup --release-dir <dir>` (maintainers and CI only; not in the
//! user docs): release assets in a local folder, served to the normal
//! release code on a loopback port (reliable-installers task 7.1).
//!
//! **Why a loopback server and not a separate file reader.** The update and
//! lookup path is `self_update`'s GitHub backend plus this crate's checks
//! (signed `SHA256SUMS`, the archive's embedded signature, the
//! pre-extraction gate, the in-place replace). Reading the folder directly
//! would need a second copy of that security-relevant flow. Serving the
//! folder as a minimal GitHub-compatible API on `127.0.0.1` lets
//! [`ReleaseSource::with_base_urls`] run exactly the production checks, with
//! the same trusted keys: with no release key built in, an update from a
//! folder is refused just like one from GitHub.
//!
//! The folder holds what a release publishes: `mmhmem-<tag>-<target>.<ext>`
//! archives, `SHA256SUMS` and `SHA256SUMS.sig`. Every file directly in the
//! folder is listed as an asset of every tag found in the archive names;
//! `latest` is the highest tag. Only `GET` of those routes is answered,
//! only plain file names are served, and the server stops when the
//! [`LocalRelease`] is dropped.

use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;

use super::{GITHUB_OWNER, GITHUB_REPO, ReleaseSource, Target};
use crate::setup::record::ReleaseTag;

/// A release folder served on a loopback port.
#[derive(Debug)]
pub struct LocalRelease {
    source: ReleaseSource,
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl LocalRelease {
    /// Serves `dir` until the value is dropped.
    pub fn serve(dir: &Path) -> std::io::Result<Self> {
        if !dir.is_dir() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "the release folder does not exist",
            ));
        }
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let addr = listener.local_addr()?;
        let base = format!("http://{addr}");
        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        let folder = dir.to_path_buf();
        let served = base.clone();
        let thread = std::thread::spawn(move || {
            for stream in listener.incoming() {
                if flag.load(Ordering::SeqCst) {
                    break;
                }
                if let Ok(stream) = stream {
                    let _ = answer(stream, &folder, &served);
                }
            }
        });
        Ok(Self {
            source: ReleaseSource::with_base_urls(GITHUB_OWNER, GITHUB_REPO, &base, &base),
            addr,
            stop,
            thread: Some(thread),
        })
    }

    /// The source that reads this folder.
    pub fn source(&self) -> &ReleaseSource {
        &self.source
    }
}

impl Drop for LocalRelease {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Wake the accept loop so it sees the flag.
        let _ = TcpStream::connect(self.addr);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// One HTTP response.
struct Reply {
    status: &'static str,
    kind: &'static str,
    body: Vec<u8>,
}

impl Reply {
    fn not_found() -> Self {
        Self {
            status: "404 Not Found",
            kind: "text/plain",
            body: b"not found".to_vec(),
        }
    }
}

fn answer(stream: TcpStream, dir: &Path, base: &str) -> std::io::Result<()> {
    stream.set_read_timeout(Some(std::time::Duration::from_secs(10)))?;
    let mut reader = BufReader::new(&stream);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header)? == 0 || header.trim().is_empty() {
            break;
        }
    }
    let mut parts = line.split_whitespace();
    let reply = match (parts.next(), parts.next()) {
        (Some("GET"), Some(target)) => route(dir, base, target.split('?').next().unwrap_or("")),
        _ => Reply::not_found(),
    };
    let mut out = &stream;
    write!(
        out,
        "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        reply.status,
        reply.kind,
        reply.body.len()
    )?;
    out.write_all(&reply.body)?;
    out.flush()
}

fn route(dir: &Path, base: &str, path: &str) -> Reply {
    if let Some(name) = path.strip_prefix("/assets/") {
        return asset(dir, name);
    }
    let prefix = format!("/repos/{GITHUB_OWNER}/{GITHUB_REPO}/releases/");
    let Some(rest) = path.strip_prefix(&prefix) else {
        return Reply::not_found();
    };
    let tags = tags(dir);
    let tag = match rest.strip_prefix("tags/") {
        Some(wanted) => tags.into_iter().find(|t| t.as_str() == wanted),
        None if rest == "latest" => tags.into_iter().max_by_key(version),
        None => None,
    };
    match tag {
        Some(tag) => Reply {
            status: "200 OK",
            kind: "application/json",
            body: release_json(dir, base, &tag).to_string().into_bytes(),
        },
        None => Reply::not_found(),
    }
}

/// A plain file directly in `dir`, by name.
fn asset(dir: &Path, name: &str) -> Reply {
    let plain = !name.is_empty() && !name.contains(['/', '\\', ':']) && name != "." && name != "..";
    match plain.then(|| std::fs::read(dir.join(name))) {
        Some(Ok(body)) => Reply {
            status: "200 OK",
            kind: "application/octet-stream",
            body,
        },
        _ => Reply::not_found(),
    }
}

fn files(dir: &Path) -> Vec<String> {
    let Ok(read) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = read
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    names.sort();
    names
}

/// The tags named by the archives in `dir`.
fn tags(dir: &Path) -> Vec<ReleaseTag> {
    let mut tags: Vec<ReleaseTag> = Vec::new();
    for name in files(dir) {
        let found = Target::ALL.iter().find_map(|t| {
            let tail = format!("-{}.{}", t.triple(), t.extension());
            name.strip_prefix("mmhmem-")?
                .strip_suffix(&tail)
                .map(str::to_owned)
        });
        if let Some(tag) = found.and_then(|t| ReleaseTag::try_from(t).ok())
            && !tags.contains(&tag)
        {
            tags.push(tag);
        }
    }
    tags
}

fn version(tag: &ReleaseTag) -> (u64, u64, u64) {
    let mut parts = tag
        .as_str()
        .trim_start_matches('v')
        .split('.')
        .map(|p| p.parse::<u64>().unwrap_or(0));
    let mut next = || parts.next().unwrap_or(0);
    (next(), next(), next())
}

fn release_json(dir: &Path, base: &str, tag: &ReleaseTag) -> serde_json::Value {
    let assets: Vec<serde_json::Value> = files(dir)
        .into_iter()
        .map(|name| serde_json::json!({"name": name, "url": format!("{base}/assets/{name}")}))
        .collect();
    serde_json::json!({
        "tag_name": tag.as_str(), "name": tag.as_str(),
        "created_at": "2026-01-01T00:00:00Z", "body": "", "assets": assets
    })
}
