//! Runtime configuration (task 1.3; design.md "File Structure Plan"
//! config.rs entry and "Components and Interfaces" Config row).
//!
//! The serve profile is env-only; the local profile additionally reads
//! worker settings from `.mmhmem.json` (global + project, corpus-warmup-prune
//! Req 7) — no other config files are parsed at runtime.
//! Validation is fail-fast: `AGENT_MEM_TOKEN` and `AGENT_MEM_ALLOWED_ROOTS`
//! are required, and any malformed value aborts startup with a clear error
//! before any socket or database work (design.md "Error Handling").

use std::fmt;
use std::path::PathBuf;

/// Per-OS per-user data-dir resolution for the local profile (Req 3.1).
pub mod local_dirs;
/// Worker settings from `.mmhmem.json` for the local profile (Req 7).
pub mod mmhmem_file;
/// Profile dimension and local-profile configuration loading (Req 3.2).
pub mod profile;

/// `AGENT_MEM_DATA_DIR` — durable state directory.
pub const ENV_DATA_DIR: &str = "AGENT_MEM_DATA_DIR";
/// `AGENT_MEM_LOG_FILE` — audit JSONL path.
pub const ENV_LOG_FILE: &str = "AGENT_MEM_LOG_FILE";
/// `AGENT_MEM_ALLOWED_ROOTS` — comma-separated confinement roots (required).
pub const ENV_ALLOWED_ROOTS: &str = "AGENT_MEM_ALLOWED_ROOTS";
/// `AGENT_MEM_TOKEN` — bearer token for the MCP endpoint (required, never logged).
pub const ENV_TOKEN: &str = "AGENT_MEM_TOKEN";
/// `AGENT_MEM_HOST` — bind host.
pub const ENV_HOST: &str = "AGENT_MEM_HOST";
/// `AGENT_MEM_PORT` — bind port (u16).
pub const ENV_PORT: &str = "AGENT_MEM_PORT";
/// `AGENT_MEM_MCP_PATH` — MCP Streamable HTTP path.
pub const ENV_MCP_PATH: &str = "AGENT_MEM_MCP_PATH";
/// `SHUNT_MIN_LINES` — smart_read shunt threshold in lines (u32 > 0, Req 3.9).
pub const ENV_SHUNT_MIN_LINES: &str = "SHUNT_MIN_LINES";
/// `SHUNT_CONTENT_CACHE_MAX_ROWS` — content cache cap in rows (u32 > 0).
pub const ENV_SHUNT_CONTENT_CACHE_MAX_ROWS: &str = "SHUNT_CONTENT_CACHE_MAX_ROWS";
/// `AGENT_MEM_BUSY_TIMEOUT_MS` — store busy-lock wait in milliseconds (Req 6.2).
pub const ENV_BUSY_TIMEOUT_MS: &str = "AGENT_MEM_BUSY_TIMEOUT_MS";
/// `AGENT_MEM_AUDIT_MAX_BYTES` — audit rotation threshold in bytes; 0 disables
/// rotation (Req 7.3; serve default 0 keeps the host-logrotate contract).
pub const ENV_AUDIT_MAX_BYTES: &str = "AGENT_MEM_AUDIT_MAX_BYTES";
/// `AGENT_MEM_AUDIT_ROTATE_KEEP` — rotated audit files to retain (u32 ≥ 1).
pub const ENV_AUDIT_ROTATE_KEEP: &str = "AGENT_MEM_AUDIT_ROTATE_KEEP";
/// `WORKER_API_URL` — OpenAI-compatible chat completions endpoint (Req 6.1).
pub const ENV_WORKER_API_URL: &str = "WORKER_API_URL";
/// `WORKER_MODEL` — worker model name (optional).
pub const ENV_WORKER_MODEL: &str = "WORKER_MODEL";
/// `WORKER_API_KEY` — worker bearer key (optional, never logged).
pub const ENV_WORKER_API_KEY: &str = "WORKER_API_KEY";
/// `OTEL_EXPORTER_OTLP_ENDPOINT` — optional OTLP endpoint (Req 11.1).
pub const ENV_OTEL_ENDPOINT: &str = "OTEL_EXPORTER_OTLP_ENDPOINT";
/// `OTEL_EXPORTER_OTLP_HEADERS` — optional OTLP headers (Req 11.1, redacted in logs).
pub const ENV_OTEL_HEADERS: &str = "OTEL_EXPORTER_OTLP_HEADERS";
/// `OTEL_SERVICE_NAME` — OTLP service name (Req 11.1).
pub const ENV_OTEL_SERVICE_NAME: &str = "OTEL_SERVICE_NAME";

/// Default for [`ENV_DATA_DIR`] on the Linux deployment host (Req 7.5):
/// `/var/lib/agent-memory`.
#[cfg(not(windows))]
pub const DEFAULT_DATA_DIR: &str = "/var/lib/agent-memory";
/// Default for [`ENV_DATA_DIR`] on a Windows dev host. The Linux production
/// default is `/var/lib/agent-memory` (Req 7.5), kept documented above.
#[cfg(windows)]
pub const DEFAULT_DATA_DIR: &str = r"C:\ProgramData\agent-memory";

/// Default for [`ENV_LOG_FILE`] (Req 7.3/7.5): `/var/log/agent-memory/audit.jsonl`.
pub const DEFAULT_LOG_FILE: &str = "/var/log/agent-memory/audit.jsonl";
/// Default bind host.
pub const DEFAULT_HOST: &str = "0.0.0.0";
/// Default bind port (Req 1.1).
pub const DEFAULT_PORT: u16 = 8443;
/// Default MCP Streamable HTTP path (Req 1.1).
pub const DEFAULT_MCP_PATH: &str = "/mcp";
/// Default shunt threshold in lines (Req 3.9; 300 since shunt-cache-efficiency
/// Req 4.1, where summaries pay off).
pub const DEFAULT_SHUNT_MIN_LINES: u32 = 300;
/// Default content cache cap in rows (shunt-content-tool Req 4.3).
pub const DEFAULT_SHUNT_CONTENT_CACHE_MAX_ROWS: u32 = 5000;
/// Default store busy-lock wait (Req 6.2: 5 seconds).
pub const DEFAULT_BUSY_TIMEOUT_MS: u64 = 5_000;
/// Serve profile: audit rotation disabled — host logrotate owns rotation
/// (Req 9.5 unchanged deployment contract).
pub const DEFAULT_AUDIT_MAX_BYTES: u64 = 0;
/// Local profile default rotation threshold (Req 7.3: 10 MiB).
pub const DEFAULT_AUDIT_MAX_BYTES_LOCAL: u64 = 10 * 1024 * 1024;
/// Default rotated audit files retained (Req 7.3: keep 5).
pub const DEFAULT_AUDIT_ROTATE_KEEP: u32 = 5;
/// Default worker endpoint: local Ollama-compatible chat completions (Req 6.1).
pub const DEFAULT_WORKER_API_URL: &str = "http://127.0.0.1:11434/v1/chat/completions";
/// Default OTLP service name (Req 11.1).
pub const DEFAULT_OTEL_SERVICE_NAME: &str = "mmhmem";

/// Marker printed in place of secret values in debug output.
pub const REDACTED: &str = "[REDACTED]";

/// Effective runtime configuration, loaded once at startup from the env.
#[derive(Clone)]
pub struct Config {
    /// Durable state directory (`AGENT_MEM_DATA_DIR`).
    pub data_dir: PathBuf,
    /// Audit JSONL file (`AGENT_MEM_LOG_FILE`).
    pub log_file: PathBuf,
    /// Path-confinement roots (`AGENT_MEM_ALLOWED_ROOTS`, required).
    pub allowed_roots: Vec<PathBuf>,
    /// Bearer token for the MCP endpoint (`AGENT_MEM_TOKEN`, required, never logged).
    pub token: String,
    /// Bind host (`AGENT_MEM_HOST`).
    pub host: String,
    /// Bind port (`AGENT_MEM_PORT`).
    pub port: u16,
    /// MCP Streamable HTTP path (`AGENT_MEM_MCP_PATH`).
    pub mcp_path: String,
    /// smart_read shunt threshold in lines (`SHUNT_MIN_LINES`).
    pub shunt_min_lines: u32,
    /// Content cache cap in rows (`SHUNT_CONTENT_CACHE_MAX_ROWS`).
    pub shunt_content_cache_max_rows: u32,
    /// Worker chat-completions URL (`WORKER_API_URL`).
    pub worker_api_url: String,
    /// Worker model name (`WORKER_MODEL`, optional).
    pub worker_model: Option<String>,
    /// Worker API key (`WORKER_API_KEY`, optional, never logged).
    pub worker_api_key: Option<String>,
    /// OTLP endpoint (`OTEL_EXPORTER_OTLP_ENDPOINT`, optional).
    pub otel_endpoint: Option<String>,
    /// OTLP headers (`OTEL_EXPORTER_OTLP_HEADERS`, optional, redacted in logs).
    pub otel_headers: Option<String>,
    /// OTLP service name (`OTEL_SERVICE_NAME`).
    pub otel_service_name: String,
    /// Store busy-lock wait in ms (`AGENT_MEM_BUSY_TIMEOUT_MS`, Req 6.2).
    pub busy_timeout_ms: u64,
    /// Audit rotation threshold in bytes; 0 disables rotation (Req 7.3).
    pub audit_max_bytes: u64,
    /// Rotated audit files retained (Req 7.3).
    pub audit_rotate_keep: u32,
    /// Operating profile: serve (daemon) vs local (spawned stdio child).
    pub profile: profile::Profile,
}

/// Fail-fast configuration error (design.md "Error Handling").
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigError {
    /// `AGENT_MEM_TOKEN` missing or blank.
    MissingToken,
    /// `AGENT_MEM_ALLOWED_ROOTS` missing, blank, or with no non-empty entry.
    MissingAllowedRoots,
    /// `AGENT_MEM_PORT` is not a valid u16.
    InvalidPort(String),
    /// `SHUNT_MIN_LINES` is not a u32 > 0.
    InvalidShuntMinLines(String),
    /// `SHUNT_CONTENT_CACHE_MAX_ROWS` is not a u32 > 0.
    InvalidShuntContentCacheMaxRows(String),
    /// The per-user location variable (`HOME`/`APPDATA`) is unset or blank,
    /// so the local profile cannot resolve its default data dir (Req 3.1).
    MissingUserDirEnv(&'static str),
    /// `AGENT_MEM_BUSY_TIMEOUT_MS` is not a u64.
    InvalidBusyTimeoutMs(String),
    /// `AGENT_MEM_AUDIT_MAX_BYTES` is not a u64.
    InvalidAuditMaxBytes(String),
    /// `AGENT_MEM_AUDIT_ROTATE_KEEP` is not a u32 ≥ 1.
    InvalidAuditRotateKeep(String),
    /// A `.mmhmem.json` exists but cannot be read or parsed as JSON; the
    /// payload names the offending file (Req 7.3).
    InvalidMmhmemFile(PathBuf),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigError::MissingToken => write!(
                f,
                "required env var {ENV_TOKEN} is not set or empty (bearer token for the MCP endpoint)"
            ),
            ConfigError::MissingAllowedRoots => write!(
                f,
                "required env var {ENV_ALLOWED_ROOTS} is not set, empty, or has no non-blank entry \
                 (comma-separated workspace roots)"
            ),
            ConfigError::InvalidPort(raw) => {
                write!(f, "{ENV_PORT}={raw:?} is not a valid port (u16, 0-65535)")
            }
            ConfigError::InvalidShuntMinLines(raw) => {
                write!(
                    f,
                    "{ENV_SHUNT_MIN_LINES}={raw:?} is not a valid shunt threshold (u32 > 0)"
                )
            }
            ConfigError::InvalidShuntContentCacheMaxRows(raw) => {
                write!(
                    f,
                    "{ENV_SHUNT_CONTENT_CACHE_MAX_ROWS}={raw:?} is not a valid content cache \
                     cap (u32 > 0)"
                )
            }
            ConfigError::MissingUserDirEnv(var) => write!(
                f,
                "required per-user environment variable {var} is not set or empty \
                 (the local profile resolves its default data dir from it)"
            ),
            ConfigError::InvalidBusyTimeoutMs(raw) => {
                write!(
                    f,
                    "{ENV_BUSY_TIMEOUT_MS}={raw:?} is not a valid busy timeout (u64 ms)"
                )
            }
            ConfigError::InvalidAuditMaxBytes(raw) => {
                write!(
                    f,
                    "{ENV_AUDIT_MAX_BYTES}={raw:?} is not a valid rotation threshold (u64 bytes)"
                )
            }
            ConfigError::InvalidAuditRotateKeep(raw) => {
                write!(
                    f,
                    "{ENV_AUDIT_ROTATE_KEEP}={raw:?} is not a valid retention count (u32 >= 1)"
                )
            }
            ConfigError::InvalidMmhmemFile(path) => write!(
                f,
                "config file {} exists but is not valid JSON (fix or remove it; the local \
                 profile reads worker settings from .mmhmem.json)",
                path.display()
            ),
        }
    }
}

impl std::error::Error for ConfigError {}

impl Config {
    /// Loads the effective config from the process environment.
    pub fn load() -> Result<Self, ConfigError> {
        Self::from_env(&|key| std::env::var(key).ok())
    }

    /// Pure env parser. Tests inject a lookup closure instead of touching the
    /// real process env, so tests stay hermetic and parallel-safe.
    pub fn from_env(get: &dyn Fn(&str) -> Option<String>) -> Result<Self, ConfigError> {
        let token = present(get, ENV_TOKEN).ok_or(ConfigError::MissingToken)?;
        let allowed_roots = parse_allowed_roots(get)?;
        let port = profile::parse_port(get)?;
        let shunt_min_lines = profile::parse_shunt_min_lines(get)?;
        Ok(Self {
            data_dir: PathBuf::from(optional(get, ENV_DATA_DIR, DEFAULT_DATA_DIR)),
            log_file: PathBuf::from(optional(get, ENV_LOG_FILE, DEFAULT_LOG_FILE)),
            allowed_roots,
            token,
            host: optional(get, ENV_HOST, DEFAULT_HOST),
            port,
            mcp_path: optional(get, ENV_MCP_PATH, DEFAULT_MCP_PATH),
            shunt_min_lines,
            worker_api_url: optional(get, ENV_WORKER_API_URL, DEFAULT_WORKER_API_URL),
            worker_model: present(get, ENV_WORKER_MODEL),
            worker_api_key: present(get, ENV_WORKER_API_KEY),
            otel_endpoint: present(get, ENV_OTEL_ENDPOINT),
            otel_headers: present(get, ENV_OTEL_HEADERS),
            otel_service_name: optional(get, ENV_OTEL_SERVICE_NAME, DEFAULT_OTEL_SERVICE_NAME),
            busy_timeout_ms: profile::parse_busy_timeout_ms(get)?,
            audit_max_bytes: profile::parse_audit_max_bytes(get, DEFAULT_AUDIT_MAX_BYTES)?,
            audit_rotate_keep: profile::parse_audit_rotate_keep(get)?,
            shunt_content_cache_max_rows: profile::parse_shunt_content_cache_max_rows(get)?,
            profile: profile::Profile::Serve,
        })
    }

    /// All fields rendered with secrets (`token`, `worker_api_key`,
    /// `otel_headers`) masked as `[REDACTED]` — safe to log at startup.
    pub fn redacted_debug(&self) -> String {
        format!("{self:?}")
    }
}

/// Manual `Debug` so the bearer token and API keys can never leak into logs
/// via `{:?}` (design.md "Security Considerations": token supplied via env,
/// never logged).
impl fmt::Debug for Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Config")
            .field("data_dir", &self.data_dir)
            .field("log_file", &self.log_file)
            .field("allowed_roots", &self.allowed_roots)
            .field("token", &REDACTED)
            .field("host", &self.host)
            .field("port", &self.port)
            .field("mcp_path", &self.mcp_path)
            .field("shunt_min_lines", &self.shunt_min_lines)
            .field("worker_api_url", &self.worker_api_url)
            .field("worker_model", &self.worker_model)
            .field(
                "worker_api_key",
                &self.worker_api_key.as_ref().map(|_| REDACTED),
            )
            .field("otel_endpoint", &self.otel_endpoint)
            .field(
                "otel_headers",
                &self.otel_headers.as_ref().map(|_| REDACTED),
            )
            .field("otel_service_name", &self.otel_service_name)
            .field("busy_timeout_ms", &self.busy_timeout_ms)
            .field("audit_max_bytes", &self.audit_max_bytes)
            .field("audit_rotate_keep", &self.audit_rotate_keep)
            .field(
                "shunt_content_cache_max_rows",
                &self.shunt_content_cache_max_rows,
            )
            .field("profile", &self.profile)
            .finish()
    }
}

/// Non-empty trimmed value for `key`, or `None` when unset/blank.
fn present(get: &dyn Fn(&str) -> Option<String>, key: &str) -> Option<String> {
    get(key)
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// Value for `key`, falling back to `default` when unset or blank.
fn optional(get: &dyn Fn(&str) -> Option<String>, key: &str, default: &str) -> String {
    present(get, key).unwrap_or_else(|| default.to_string())
}

/// Comma-separated `AGENT_MEM_ALLOWED_ROOTS`, split and trimmed; blank-only
/// input is an error (fail-fast, design.md "Error Handling").
fn parse_allowed_roots(get: &dyn Fn(&str) -> Option<String>) -> Result<Vec<PathBuf>, ConfigError> {
    let raw = present(get, ENV_ALLOWED_ROOTS).ok_or(ConfigError::MissingAllowedRoots)?;
    let roots: Vec<PathBuf> = raw
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .collect();
    if roots.is_empty() {
        return Err(ConfigError::MissingAllowedRoots);
    }
    Ok(roots)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// Builds an owned lookup map; tests never touch the real process env.
    fn lookup(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    /// Minimal valid env: required token + roots, plus overrides.
    fn base(overrides: &[(&str, &str)]) -> HashMap<String, String> {
        let mut map = lookup(&[(ENV_TOKEN, "test-token"), (ENV_ALLOWED_ROOTS, "/workspace")]);
        for (k, v) in overrides {
            map.insert(k.to_string(), v.to_string());
        }
        map
    }

    fn parse(map: &HashMap<String, String>) -> Result<Config, ConfigError> {
        Config::from_env(&|k| map.get(k).cloned())
    }

    #[test]
    fn defaults_applied_when_only_required_vars_set() {
        let cfg = parse(&base(&[])).unwrap();
        assert_eq!(cfg.data_dir, PathBuf::from(DEFAULT_DATA_DIR));
        assert_eq!(cfg.log_file, PathBuf::from(DEFAULT_LOG_FILE));
        assert_eq!(cfg.allowed_roots, vec![PathBuf::from("/workspace")]);
        assert_eq!(cfg.token, "test-token");
        assert_eq!(cfg.host, DEFAULT_HOST);
        assert_eq!(cfg.port, DEFAULT_PORT);
        assert_eq!(cfg.mcp_path, DEFAULT_MCP_PATH);
        assert_eq!(cfg.shunt_min_lines, DEFAULT_SHUNT_MIN_LINES);
        assert_eq!(cfg.worker_api_url, DEFAULT_WORKER_API_URL);
        assert!(cfg.worker_model.is_none());
        assert!(cfg.worker_api_key.is_none());
        assert!(cfg.otel_endpoint.is_none());
        assert!(cfg.otel_headers.is_none());
        assert_eq!(cfg.otel_service_name, DEFAULT_OTEL_SERVICE_NAME);
    }

    #[test]
    fn missing_token_is_an_error() {
        let map = lookup(&[(ENV_ALLOWED_ROOTS, "/workspace")]);
        assert_eq!(parse(&map).unwrap_err(), ConfigError::MissingToken);
    }

    #[test]
    fn blank_token_is_an_error() {
        let map = base(&[(ENV_TOKEN, "   ")]);
        assert_eq!(parse(&map).unwrap_err(), ConfigError::MissingToken);
    }

    #[test]
    fn missing_allowed_roots_is_an_error() {
        let map = lookup(&[(ENV_TOKEN, "t")]);
        assert_eq!(parse(&map).unwrap_err(), ConfigError::MissingAllowedRoots);
    }

    #[test]
    fn blank_allowed_roots_is_an_error() {
        for raw in ["", "   ", " , , "] {
            let map = base(&[(ENV_ALLOWED_ROOTS, raw)]);
            assert_eq!(
                parse(&map).unwrap_err(),
                ConfigError::MissingAllowedRoots,
                "raw={raw:?}"
            );
        }
    }

    #[test]
    fn allowed_roots_are_split_trimmed_and_deduped_of_blanks() {
        let map = base(&[(ENV_ALLOWED_ROOTS, " /a , C:\\b ,, /c ")]);
        let cfg = parse(&map).unwrap();
        assert_eq!(
            cfg.allowed_roots,
            vec![
                PathBuf::from("/a"),
                PathBuf::from(r"C:\b"),
                PathBuf::from("/c")
            ]
        );
    }

    #[test]
    fn invalid_port_is_an_error() {
        for raw in ["abc", "70000", "-1", "8443x"] {
            let map = base(&[(ENV_PORT, raw)]);
            assert!(
                matches!(parse(&map), Err(ConfigError::InvalidPort(_))),
                "raw={raw:?}"
            );
        }
    }

    #[test]
    fn custom_port_parses_as_u16() {
        let map = base(&[(ENV_PORT, "9000")]);
        assert_eq!(parse(&map).unwrap().port, 9000);
    }

    #[test]
    fn shunt_min_lines_must_be_a_positive_u32() {
        for raw in ["0", "-1", "abc", "1.5"] {
            let map = base(&[(ENV_SHUNT_MIN_LINES, raw)]);
            assert!(
                matches!(parse(&map), Err(ConfigError::InvalidShuntMinLines(_))),
                "raw={raw:?}"
            );
        }
        let map = base(&[(ENV_SHUNT_MIN_LINES, "250")]);
        assert_eq!(parse(&map).unwrap().shunt_min_lines, 250);
    }

    #[test]
    fn optional_worker_and_otel_vars_parse() {
        let map = base(&[
            (ENV_WORKER_API_URL, "http://worker:9/v1/chat/completions"),
            (ENV_WORKER_MODEL, "qwen3"),
            (ENV_WORKER_API_KEY, "wk-secret"),
            (ENV_OTEL_ENDPOINT, "http://otel:4318"),
            (ENV_OTEL_HEADERS, "Authorization=Bearer otlp-secret"),
            (ENV_OTEL_SERVICE_NAME, "mmhmem-prod"),
        ]);
        let cfg = parse(&map).unwrap();
        assert_eq!(cfg.worker_api_url, "http://worker:9/v1/chat/completions");
        assert_eq!(cfg.worker_model.as_deref(), Some("qwen3"));
        assert_eq!(cfg.worker_api_key.as_deref(), Some("wk-secret"));
        assert_eq!(cfg.otel_endpoint.as_deref(), Some("http://otel:4318"));
        assert_eq!(cfg.otel_service_name, "mmhmem-prod");
    }

    #[test]
    fn redacted_debug_masks_token_worker_key_and_otel_headers() {
        let map = base(&[
            (ENV_TOKEN, "super-secret-token"),
            (ENV_WORKER_API_KEY, "wk-secret"),
            (ENV_OTEL_HEADERS, "Authorization=Bearer otlp-secret"),
        ]);
        let cfg = parse(&map).unwrap();
        let dbg = cfg.redacted_debug();
        assert!(dbg.contains(REDACTED));
        assert!(!dbg.contains("super-secret-token"), "token leaked: {dbg}");
        assert!(!dbg.contains("wk-secret"), "worker key leaked: {dbg}");
        assert!(!dbg.contains("otlp-secret"), "otel headers leaked: {dbg}");
        // Non-secret fields remain visible for operability.
        assert!(dbg.contains(DEFAULT_MCP_PATH));
        assert!(dbg.contains("8443"));
        assert!(dbg.contains("/workspace"));
    }
}
