//! The `WorkerClient` component (task 3.1; design.md "services layer" /
//! "Components and Interfaces" WorkerClient row; requirements.md Req 6.1-6.4,
//! 3.5, 4.1, 9.4, 10.2).
//!
//! A thin typed client for a configurable OpenAI-compatible chat-completions
//! endpoint (`WORKER_API_URL`, `WORKER_MODEL`, `WORKER_API_KEY` — Req 6.1).
//! It is the ONLY egress of the daemon (design.md "Boundary Commitments").
//! Requests always use temperature 0.0 for deterministic output (Req 6.2);
//! every failure is an explicit [`WorkerError`] and never stale or empty
//! content masquerading as a fresh result (Req 6.3). Token usage from the
//! response `usage` field is returned as [`WorkerUsage`] for the stats
//! collector (Req 10.2).
//!
//! The client stays pure (deps: reqwest + config only, per design.md): the
//! WORKER_ERROR audit event is written by the calling service (shunt /
//! boilerplate tasks) via `storage::audit::AuditWriter` (Req 6.3), which
//! keeps the dependency direction `services -> storage` clean.

use std::fmt;
use std::time::Duration;

use serde::Deserialize;

use crate::config::Config;

/// Timeout for shunted reads (design.md WorkerClient: 30 s).
pub const SUMMARIZE_TIMEOUT: Duration = Duration::from_secs(30);
/// Timeout for boilerplate generation (design.md WorkerClient: 45 s).
pub const GENERATE_TIMEOUT: Duration = Duration::from_secs(45);

/// Bulk-reader system prompt (Req 3.5): only concise bullets, each beginning
/// with an exact symbol name and a `[L<start>-L<end>]` line range.
pub const BULK_READER_SYSTEM: &str = "You are a deterministic code analyst. Answer the question using ONLY the \
file content provided. Return ONLY concise bullet points; every bullet MUST begin with an exact symbol/method name \
and a line range [L<start>-L<end>]. No conversational prose.";

/// Code-writer system prompt (Req 4.1): follow the reference file's
/// conventions, output only raw source code (callers strip fences, Req 4.2).
pub const CODE_WRITER_SYSTEM: &str = "You are a deterministic code generator. Follow the reference file's conventions \
exactly. Output ONLY raw source code: no markdown fences, no explanation.";

/// Token usage reported by the worker API `usage` field (Req 10.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkerUsage {
    /// Prompt tokens consumed by the worker call.
    pub prompt_tokens: u64,
    /// Completion tokens produced by the worker call.
    pub completion_tokens: u64,
}

/// Explicit worker-call failure (Req 6.3). Never converted into stale or
/// empty "success" content.
#[derive(Debug)]
pub enum WorkerError {
    /// `WORKER_MODEL` is not configured; the daemon runs without a worker
    /// on cache-hit / direct-read / session paths (Req 9.4).
    NotConfigured,
    /// Endpoint unreachable / DNS / connect error.
    Unreachable(String),
    /// Per-call timeout elapsed.
    Timeout,
    /// Non-2xx response; body snippet included for the audit trail.
    HttpStatus(u16, String),
    /// 2xx but the body is not a well-formed chat-completions response.
    Malformed(String),
}

impl fmt::Display for WorkerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WorkerError::NotConfigured => {
                write!(f, "worker model not configured (WORKER_MODEL is unset)")
            }
            WorkerError::Unreachable(e) => write!(f, "worker endpoint unreachable: {e}"),
            WorkerError::Timeout => write!(f, "worker call timed out"),
            WorkerError::HttpStatus(code, body) => {
                write!(f, "worker endpoint returned HTTP {code}: {body}")
            }
            WorkerError::Malformed(e) => write!(f, "worker response malformed: {e}"),
        }
    }
}

impl std::error::Error for WorkerError {}

/// Typed client for the OpenAI-compatible worker endpoint (Req 6.1).
#[derive(Clone)]
pub struct WorkerClient {
    client: reqwest::Client,
    url: String,
    model: String,
    api_key: Option<String>,
    summarize_timeout: Duration,
    generate_timeout: Duration,
}

impl WorkerClient {
    /// Builds a client from the env-only config. Fails with
    /// [`WorkerError::NotConfigured`] when `WORKER_MODEL` is unset (Req 9.4:
    /// the daemon operates fully without a worker when it is not configured).
    pub fn from_config(config: &Config) -> Result<Self, WorkerError> {
        let model = config
            .worker_model
            .clone()
            .ok_or(WorkerError::NotConfigured)?;
        let client = reqwest::Client::builder()
            .build()
            .map_err(|e| WorkerError::Unreachable(e.to_string()))?;
        Ok(Self {
            client,
            url: config.worker_api_url.clone(),
            model,
            api_key: config.worker_api_key.clone(),
            summarize_timeout: SUMMARIZE_TIMEOUT,
            generate_timeout: GENERATE_TIMEOUT,
        })
    }

    /// Overrides the per-call timeouts. Intended for tests (a 30 s timeout
    /// cannot be exercised in a unit test).
    pub fn with_timeouts(mut self, summarize: Duration, generate: Duration) -> Self {
        self.summarize_timeout = summarize;
        self.generate_timeout = generate;
        self
    }

    /// The configured worker model name (recorded in the audit trail, Req 6.4).
    pub fn model(&self) -> &str {
        &self.model
    }

    /// Shunted read: line-numbered file + question -> structural summary
    /// (Req 3.5). 30 s timeout.
    pub async fn summarize_file(
        &self,
        file_xml: &str,
        question: &str,
    ) -> Result<(String, WorkerUsage), WorkerError> {
        let user = format!("File:\n{file_xml}\n\nQuestion: {question}");
        self.call(self.summarize_timeout, BULK_READER_SYSTEM, &user)
            .await
    }

    /// Boilerplate generation: reference file + spec -> raw source code
    /// (Req 4.1). 45 s timeout.
    pub async fn generate_code(
        &self,
        reference_xml: &str,
        spec: &str,
    ) -> Result<(String, WorkerUsage), WorkerError> {
        let user = format!("Reference file:\n{reference_xml}\n\nSpecification:\n{spec}");
        self.call(self.generate_timeout, CODE_WRITER_SYSTEM, &user)
            .await
    }

    /// One POST to the chat-completions endpoint with temperature 0.0
    /// (Req 6.2); maps every failure mode to an explicit [`WorkerError`]
    /// (Req 6.3).
    async fn call(
        &self,
        timeout: Duration,
        system: &str,
        user: &str,
    ) -> Result<(String, WorkerUsage), WorkerError> {
        let body = serde_json::json!({
            "model": self.model,
            "messages": [
                {"role": "system", "content": system},
                {"role": "user", "content": user},
            ],
            "temperature": 0.0,
        });
        let mut req = self.client.post(&self.url).timeout(timeout).json(&body);
        if let Some(key) = &self.api_key {
            req = req.bearer_auth(key);
        }
        let resp = req.send().await.map_err(|e| {
            if e.is_timeout() {
                WorkerError::Timeout
            } else {
                WorkerError::Unreachable(e.to_string())
            }
        })?;
        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| WorkerError::Unreachable(e.to_string()))?;
        if !status.is_success() {
            let snippet: String = text.chars().take(200).collect();
            return Err(WorkerError::HttpStatus(status.as_u16(), snippet));
        }
        parse_chat_response(&text)
    }
}

/// Well-formed chat-completions response shape (only the fields we use).
#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
    usage: Option<ChatUsage>,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: ChatMessage,
}

#[derive(Deserialize)]
struct ChatMessage {
    content: String,
}

#[derive(Deserialize)]
struct ChatUsage {
    prompt_tokens: u64,
    completion_tokens: u64,
}

/// Parses a 2xx body into (content, usage); malformed -> explicit error.
fn parse_chat_response(text: &str) -> Result<(String, WorkerUsage), WorkerError> {
    let parsed: ChatResponse =
        serde_json::from_str(text).map_err(|e| WorkerError::Malformed(e.to_string()))?;
    let content = parsed
        .choices
        .into_iter()
        .next()
        .map(|c| c.message.content)
        .unwrap_or_default();
    let usage = parsed.usage.map_or(
        WorkerUsage {
            prompt_tokens: 0,
            completion_tokens: 0,
        },
        |u| WorkerUsage {
            prompt_tokens: u.prompt_tokens,
            completion_tokens: u.completion_tokens,
        },
    );
    Ok((content, usage))
}
