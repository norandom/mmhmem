//! Tests of the worker-model guidance (bugfix 2026-11-07).

use super::*;

#[test]
fn tags_url_takes_only_the_ollama_shape() {
    assert_eq!(
        tags_url("http://127.0.0.1:11434/v1/chat/completions").as_deref(),
        Some("http://127.0.0.1:11434/api/tags")
    );
    assert_eq!(
        tags_url("https://host/v1/chat/completions").as_deref(),
        Some("https://host/api/tags")
    );
    assert_eq!(tags_url("http://127.0.0.1:11434/api/generate"), None);
    assert_eq!(tags_url("https://api.example.com/chat/completions"), None);
}

#[test]
fn a_local_endpoint_with_no_models_or_no_answer_gives_none() {
    // Port 1 on loopback: nothing answers there; the short timeout keeps
    // the test fast. This pins the fail-soft contract, not one endpoint.
    let none = local_models(
        "http://127.0.0.1:1/v1/chat/completions",
        Duration::from_millis(200),
    );
    assert!(none.is_none());
}
