//! The profile dimension (local-mode Req 2.5, 3.2-3.4; design.md "Local
//! profile config").
//!
//! One `Config` type, two loading rules: `Config::from_env` keeps the serve
//! semantics byte-identical (token + roots required — the existing suite
//! pins them), while `Config::from_env_local` starts with ZERO mandatory
//! variables: token optional (empty and unused — stdio opens no listener),
//! confinement roots optional (empty = unconfined), and per-OS user-level
//! path defaults from [`super::local_dirs`]. The store busy-timeout and
//! audit-rotation knobs parse identically in both profiles (Req 6.2, 7.3).

use std::path::PathBuf;

use super::local_dirs;
use super::mmhmem_file::MmhmemFile;
use super::{
    Config, ConfigError, DEFAULT_AUDIT_MAX_BYTES_LOCAL, DEFAULT_AUDIT_ROTATE_KEEP,
    DEFAULT_BUSY_TIMEOUT_MS, DEFAULT_HOST, DEFAULT_MCP_PATH, DEFAULT_OTEL_SERVICE_NAME,
    DEFAULT_WORKER_API_URL, ENV_ALLOWED_ROOTS, ENV_AUDIT_MAX_BYTES, ENV_AUDIT_ROTATE_KEEP,
    ENV_BUSY_TIMEOUT_MS, ENV_DATA_DIR, ENV_HOST, ENV_LOG_FILE, ENV_MCP_PATH, ENV_OTEL_ENDPOINT,
    ENV_OTEL_HEADERS, ENV_OTEL_SERVICE_NAME, ENV_TOKEN, ENV_WORKER_API_KEY, ENV_WORKER_API_URL,
    ENV_WORKER_MODEL, optional, parse_port, parse_shunt_min_lines, present,
};

/// Operating profile of a loaded `Config`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Profile {
    /// The daemon: system paths, required token + roots, HTTP transport.
    Serve,
    /// A spawned stdio child: per-user paths, no token, stdio transport.
    Local,
}

impl Config {
    /// Loads the local-profile config: worker settings from the process
    /// environment merged with `.mmhmem.json` (project over global,
    /// Req 7.1); everything else stays env-only.
    pub fn load_local() -> Result<Self, ConfigError> {
        let file = MmhmemFile::load_default_locations()?;
        let env = |key: &str| std::env::var(key).ok();
        let merged = MmhmemFile::merged_lookup(&env, &file);
        Self::from_env_local(&merged)
    }

    /// Pure local-profile parser; tests inject a lookup closure.
    pub fn from_env_local(get: &dyn Fn(&str) -> Option<String>) -> Result<Self, ConfigError> {
        let data_dir = match present(get, ENV_DATA_DIR) {
            Some(dir) => PathBuf::from(dir),
            None => local_dirs::local_data_dir(get)?,
        };
        let log_file = match present(get, ENV_LOG_FILE) {
            Some(file) => PathBuf::from(file),
            None => local_dirs::local_audit_file(&data_dir),
        };
        let allowed_roots = match present(get, ENV_ALLOWED_ROOTS) {
            None => Vec::new(),
            Some(raw) => raw
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(PathBuf::from)
                .collect(),
        };
        Ok(Self {
            data_dir,
            log_file,
            allowed_roots,
            // Req 3.2: no token for a spawned child inside the trust
            // boundary; stdio opens no listener, so the value is never read.
            token: present(get, ENV_TOKEN).unwrap_or_default(),
            host: optional(get, ENV_HOST, DEFAULT_HOST),
            port: parse_port(get)?,
            mcp_path: optional(get, ENV_MCP_PATH, DEFAULT_MCP_PATH),
            shunt_min_lines: parse_shunt_min_lines(get)?,
            worker_api_url: optional(get, ENV_WORKER_API_URL, DEFAULT_WORKER_API_URL),
            worker_model: present(get, ENV_WORKER_MODEL),
            worker_api_key: present(get, ENV_WORKER_API_KEY),
            otel_endpoint: present(get, ENV_OTEL_ENDPOINT),
            otel_headers: present(get, ENV_OTEL_HEADERS),
            otel_service_name: optional(get, ENV_OTEL_SERVICE_NAME, DEFAULT_OTEL_SERVICE_NAME),
            busy_timeout_ms: parse_busy_timeout_ms(get)?,
            audit_max_bytes: parse_audit_max_bytes(get, DEFAULT_AUDIT_MAX_BYTES_LOCAL)?,
            audit_rotate_keep: parse_audit_rotate_keep(get)?,
            profile: Profile::Local,
        })
    }
}

/// `AGENT_MEM_BUSY_TIMEOUT_MS` parsed as u64 (0 = fail fast on busy).
pub fn parse_busy_timeout_ms(get: &dyn Fn(&str) -> Option<String>) -> Result<u64, ConfigError> {
    match present(get, ENV_BUSY_TIMEOUT_MS) {
        Some(raw) => raw
            .parse::<u64>()
            .map_err(|_| ConfigError::InvalidBusyTimeoutMs(raw)),
        None => Ok(DEFAULT_BUSY_TIMEOUT_MS),
    }
}

/// `AGENT_MEM_AUDIT_MAX_BYTES` parsed as u64 with a per-profile default
/// (serve: 0 = rotation off; local: 10 MiB).
pub fn parse_audit_max_bytes(
    get: &dyn Fn(&str) -> Option<String>,
    default: u64,
) -> Result<u64, ConfigError> {
    match present(get, ENV_AUDIT_MAX_BYTES) {
        Some(raw) => raw
            .parse::<u64>()
            .map_err(|_| ConfigError::InvalidAuditMaxBytes(raw)),
        None => Ok(default),
    }
}

/// `AGENT_MEM_AUDIT_ROTATE_KEEP` parsed as u32 ≥ 1.
pub fn parse_audit_rotate_keep(get: &dyn Fn(&str) -> Option<String>) -> Result<u32, ConfigError> {
    match present(get, ENV_AUDIT_ROTATE_KEEP) {
        Some(raw) => {
            let n = raw
                .parse::<u32>()
                .map_err(|_| ConfigError::InvalidAuditRotateKeep(raw.clone()))?;
            if n == 0 {
                return Err(ConfigError::InvalidAuditRotateKeep(raw));
            }
            Ok(n)
        }
        None => Ok(DEFAULT_AUDIT_ROTATE_KEEP),
    }
}

#[cfg(test)]
mod tests {
    use super::super::{DEFAULT_AUDIT_MAX_BYTES, ENV_SHUNT_MIN_LINES};
    use super::*;
    use std::collections::HashMap;

    fn lookup(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    /// Serve-profile base env, exactly like the config.rs tests.
    fn serve_base(overrides: &[(&str, &str)]) -> HashMap<String, String> {
        let mut map = lookup(&[(ENV_TOKEN, "test-token"), (ENV_ALLOWED_ROOTS, "/workspace")]);
        for (k, v) in overrides {
            map.insert(k.to_string(), v.to_string());
        }
        map
    }

    /// A local-profile env with only the per-user location variables set —
    /// everything else is optional (Req 2.5 zero mandatory env).
    fn local_user_env(overrides: &[(&str, &str)]) -> HashMap<String, String> {
        let mut map = lookup(&[
            ("XDG_DATA_HOME", "/xdg-data"),
            ("HOME", "/home/u"),
            ("APPDATA", r"C:\Users\u\AppData\Roaming"),
        ]);
        for (k, v) in overrides {
            map.insert(k.to_string(), v.to_string());
        }
        map
    }

    fn get(map: &HashMap<String, String>) -> impl Fn(&str) -> Option<String> + '_ {
        move |k| map.get(k).cloned()
    }

    #[test]
    fn serve_from_env_still_requires_token_and_roots() {
        // Req 9.2: the serve contract is unchanged by the profile split.
        assert_eq!(
            Config::from_env(&|_| None).unwrap_err(),
            ConfigError::MissingToken
        );
        let map = lookup(&[(ENV_TOKEN, "t")]);
        assert_eq!(
            Config::from_env(&get(&map)).unwrap_err(),
            ConfigError::MissingAllowedRoots
        );
    }

    #[test]
    fn serve_defaults_busy_timeout_5000_rotation_off() {
        let map = serve_base(&[]);
        let cfg = Config::from_env(&get(&map)).unwrap();
        assert_eq!(cfg.busy_timeout_ms, DEFAULT_BUSY_TIMEOUT_MS);
        assert_eq!(cfg.audit_max_bytes, DEFAULT_AUDIT_MAX_BYTES);
        assert_eq!(cfg.profile, Profile::Serve);
    }

    #[test]
    fn local_defaults_with_zero_mandatory_env() {
        let map = local_user_env(&[]);
        let cfg = Config::from_env_local(&get(&map)).unwrap();
        let expected_data = local_dirs::local_data_dir(&get(&map)).unwrap();
        assert_eq!(cfg.profile, Profile::Local);
        assert_eq!(cfg.token, "", "token is optional-empty in local mode");
        assert!(cfg.allowed_roots.is_empty(), "roots optional in local mode");
        assert_eq!(cfg.data_dir, expected_data);
        assert_eq!(cfg.log_file, local_dirs::local_audit_file(&expected_data));
        assert_eq!(cfg.busy_timeout_ms, DEFAULT_BUSY_TIMEOUT_MS);
        assert_eq!(cfg.audit_max_bytes, DEFAULT_AUDIT_MAX_BYTES_LOCAL);
        assert_eq!(cfg.audit_rotate_keep, DEFAULT_AUDIT_ROTATE_KEEP);
    }

    #[test]
    fn local_honors_data_dir_and_log_file_overrides() {
        let map = local_user_env(&[
            (ENV_DATA_DIR, "/custom/data"),
            (ENV_LOG_FILE, "/custom/audit.jsonl"),
        ]);
        let cfg = Config::from_env_local(&get(&map)).unwrap();
        assert_eq!(cfg.data_dir, PathBuf::from("/custom/data"));
        assert_eq!(cfg.log_file, PathBuf::from("/custom/audit.jsonl"));
    }

    #[test]
    fn local_parses_token_and_roots_when_set() {
        let map = local_user_env(&[(ENV_TOKEN, "local-token"), (ENV_ALLOWED_ROOTS, " /a , /b ")]);
        let cfg = Config::from_env_local(&get(&map)).unwrap();
        assert_eq!(cfg.token, "local-token");
        assert_eq!(
            cfg.allowed_roots,
            vec![PathBuf::from("/a"), PathBuf::from("/b")]
        );
    }

    #[test]
    fn local_honors_shunt_and_worker_overrides() {
        let map = local_user_env(&[
            (ENV_SHUNT_MIN_LINES, "250"),
            (ENV_WORKER_API_URL, "http://worker:9/v1/chat/completions"),
            (ENV_WORKER_MODEL, "qwen3"),
        ]);
        let cfg = Config::from_env_local(&get(&map)).unwrap();
        assert_eq!(cfg.shunt_min_lines, 250);
        assert_eq!(cfg.worker_api_url, "http://worker:9/v1/chat/completions");
        assert_eq!(cfg.worker_model.as_deref(), Some("qwen3"));
        assert!(cfg.worker_api_key.is_none());
    }

    #[test]
    fn local_invalid_port_and_shunt_keep_serve_errors() {
        let map = local_user_env(&[(ENV_SHUNT_MIN_LINES, "0")]);
        assert!(matches!(
            Config::from_env_local(&get(&map)),
            Err(ConfigError::InvalidShuntMinLines(_))
        ));
        let map = local_user_env(&[("AGENT_MEM_PORT", "abc")]);
        assert!(matches!(
            Config::from_env_local(&get(&map)),
            Err(ConfigError::InvalidPort(_))
        ));
    }

    #[test]
    fn busy_timeout_parses_override_and_zero_is_valid() {
        let map = serve_base(&[(ENV_BUSY_TIMEOUT_MS, "12000")]);
        assert_eq!(
            Config::from_env(&get(&map)).unwrap().busy_timeout_ms,
            12_000
        );
        let map = local_user_env(&[(ENV_BUSY_TIMEOUT_MS, "0")]);
        assert_eq!(
            Config::from_env_local(&get(&map)).unwrap().busy_timeout_ms,
            0
        );
    }

    #[test]
    fn busy_timeout_invalid_is_an_error() {
        for raw in ["abc", "-1", "1.5"] {
            let map = serve_base(&[(ENV_BUSY_TIMEOUT_MS, raw)]);
            assert!(
                matches!(
                    Config::from_env(&get(&map)),
                    Err(ConfigError::InvalidBusyTimeoutMs(_))
                ),
                "raw={raw:?}"
            );
        }
    }

    #[test]
    fn audit_max_bytes_defaults_and_overrides_per_profile() {
        let map = serve_base(&[(ENV_AUDIT_MAX_BYTES, "12345")]);
        assert_eq!(
            Config::from_env(&get(&map)).unwrap().audit_max_bytes,
            12_345
        );
        let map = local_user_env(&[(ENV_AUDIT_MAX_BYTES, "12345")]);
        assert_eq!(
            Config::from_env_local(&get(&map)).unwrap().audit_max_bytes,
            12_345
        );
        let map = local_user_env(&[(ENV_AUDIT_MAX_BYTES, "0")]);
        assert_eq!(
            Config::from_env_local(&get(&map)).unwrap().audit_max_bytes,
            0,
            "explicit zero disables rotation in local mode too"
        );
    }

    #[test]
    fn audit_max_bytes_invalid_is_an_error() {
        for raw in ["abc", "-5"] {
            let map = serve_base(&[(ENV_AUDIT_MAX_BYTES, raw)]);
            assert!(
                matches!(
                    Config::from_env(&get(&map)),
                    Err(ConfigError::InvalidAuditMaxBytes(_))
                ),
                "raw={raw:?}"
            );
        }
    }

    #[test]
    fn rotate_keep_defaults_to_five_and_rejects_zero() {
        let map = serve_base(&[]);
        assert_eq!(
            Config::from_env(&get(&map)).unwrap().audit_rotate_keep,
            DEFAULT_AUDIT_ROTATE_KEEP
        );
        let map = local_user_env(&[(ENV_AUDIT_ROTATE_KEEP, "3")]);
        assert_eq!(
            Config::from_env_local(&get(&map))
                .unwrap()
                .audit_rotate_keep,
            3
        );
        for raw in ["0", "abc", "-1"] {
            let map = local_user_env(&[(ENV_AUDIT_ROTATE_KEEP, raw)]);
            assert!(
                matches!(
                    Config::from_env_local(&get(&map)),
                    Err(ConfigError::InvalidAuditRotateKeep(_))
                ),
                "raw={raw:?}"
            );
        }
    }
}
