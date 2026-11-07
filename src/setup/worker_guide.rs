//! The worker-model guidance the menu promises (bugfix 2026-11-07): the
//! features screen says "setup will show you how to connect one after the
//! install" — this module is that step. It also asks a local worker
//! endpoint for its model names (the Ollama-compatible `/api/tags` list),
//! so the guidance can suggest models that are actually installed.
//!
//! Nothing here blocks or fails the install: an endpoint that does not
//! answer, or answers in an unknown shape, simply adds no suggestions.

use std::time::Duration;

/// Ask the endpoint behind `chat_completions_url` (the worker `api_url`,
/// by default a local Ollama) for its model names. `None`: not reachable,
/// not Ollama-shaped or no models — never an error the user sees.
pub fn local_models(chat_completions_url: &str, timeout: Duration) -> Option<Vec<String>> {
    let url = tags_url(chat_completions_url)?;
    let client = self_update::reqwest::blocking::Client::builder()
        .timeout(timeout)
        .build()
        .ok()?;
    let body = client.get(url).send().ok()?.error_for_status().ok()?;
    let text = body.text().ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    let names = value
        .get("models")?
        .as_array()?
        .iter()
        .filter_map(|m| m.get("name").and_then(|n| n.as_str()))
        .map(str::to_owned)
        .collect::<Vec<_>>();
    (!names.is_empty()).then_some(names)
}

/// `…/v1/chat/completions` → `…/api/tags` (the Ollama shape); any other
/// endpoint shape gives `None` — `/api/tags` is Ollama's list, not a
/// general OpenAI-compatible one.
fn tags_url(chat_completions_url: &str) -> Option<String> {
    let base = chat_completions_url.strip_suffix("/v1/chat/completions")?;
    Some(format!("{base}/api/tags"))
}

#[cfg(test)]
#[path = "worker_guide_tests.rs"]
mod tests;
