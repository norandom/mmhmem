//! Optional OTel observability (task 9.1; requirements.md Req 11.1-11.3;
//! design.md "Components and Interfaces" Telemetry row).
//!
//! Compiled behind the `otel` cargo feature (Docker image only, Req 11.2).
//! Without the feature this module is a no-op shim with the same API, so the
//! plain release binary contains no OTLP code path (`cargo tree -e normal`
//! shows zero opentelemetry crates). When the feature is on but
//! `OTEL_EXPORTER_OTLP_ENDPOINT` is unset, nothing is exported (off by
//! default, Req 11.1). Every attribute value passes the audit redaction
//! filter (Req 11.3). The JSONL audit remains the system of record.

use crate::config::Config;
use crate::services::stats::OtelSink;

#[cfg(feature = "otel")]
mod imp {
    use std::collections::HashMap;
    use std::time::Duration;

    use opentelemetry::metrics::MeterProvider as _;
    use opentelemetry::trace::{Span, Tracer, TracerProvider as _};
    use opentelemetry::KeyValue;
    use opentelemetry_otlp::{WithExportConfig, WithHttpConfig};
    use opentelemetry_sdk::logs::SdkLoggerProvider;
    use opentelemetry_sdk::metrics::SdkMeterProvider;
    use opentelemetry_sdk::trace::SdkTracerProvider;
    use opentelemetry_sdk::Resource;

    use super::*;
    use crate::storage::audit::redact_secrets;

    /// Real OTLP-backed telemetry (feature `otel`).
    pub struct Telemetry {
        enabled: bool,
        tracer_provider: Option<SdkTracerProvider>,
        meter_provider: Option<SdkMeterProvider>,
        logger_provider: Option<SdkLoggerProvider>,
        counters: Counters,
    }

    struct Counters {
        avoided_in: opentelemetry::metrics::Counter<u64>,
        avoided_out: opentelemetry::metrics::Counter<u64>,
        worker_prompt: opentelemetry::metrics::Counter<u64>,
        worker_completion: opentelemetry::metrics::Counter<u64>,
        cache: opentelemetry::metrics::Counter<u64>,
    }

    /// A tool-call span that records outcome + latency on finish.
    pub struct ToolSpan {
        span: Option<opentelemetry_sdk::trace::Span>,
    }

    impl ToolSpan {
        /// Ends the span with outcome + latency_ms attributes.
        pub fn finish(mut self, outcome: &str, latency_ms: u64) {
            if let Some(span) = self.span.as_mut() {
                span.set_attribute(KeyValue::new("outcome", redact_secrets(outcome)));
                span.set_attribute(KeyValue::new("latency_ms", latency_ms as i64));
                span.end();
            }
        }
    }

    impl Telemetry {
        pub fn is_enabled(&self) -> bool {
            self.enabled
        }

        /// Starts a span for one MCP tool call; attributes are redacted.
        pub fn start_tool_span(&self, tool: &str, harness: &str, project: &str) -> ToolSpan {
            let span = if self.enabled {
                // enabled implies both providers are Some
                let tracer = self
                    .tracer_provider
                    .as_ref()
                    .expect("enabled implies tracer provider")
                    .tracer("mmhmem");
                let mut span = tracer.start(tool.to_string());
                span.set_attribute(KeyValue::new("tool", redact_secrets(tool)));
                span.set_attribute(KeyValue::new("agent_harness", redact_secrets(harness)));
                span.set_attribute(KeyValue::new("project", redact_secrets(project)));
                Some(span)
            } else {
                None
            };
            ToolSpan { span }
        }

        /// Force-flush + shutdown all providers (graceful shutdown, task 10.4).
        pub fn shutdown(self) {
            if let Some(p) = self.tracer_provider {
                let _ = p.shutdown();
            }
            if let Some(p) = self.meter_provider {
                let _ = p.shutdown();
            }
            if let Some(p) = self.logger_provider {
                let _ = p.shutdown();
            }
        }
    }

    impl OtelSink for Telemetry {
        fn incr_tokens_avoided(&self, input_est: u64, output_est: u64) {
            self.counters.avoided_in.add(input_est, &[]);
            self.counters.avoided_out.add(output_est, &[]);
        }
        fn incr_worker_tokens(&self, prompt: u64, completion: u64) {
            self.counters.worker_prompt.add(prompt, &[]);
            self.counters.worker_completion.add(completion, &[]);
        }
        fn incr_cache(&self, hit: bool) {
            self.counters.cache.add(1, &[KeyValue::new("hit", hit)]);
        }
    }

    /// Builds telemetry from env-only config. Disabled when the endpoint is
    /// unset (Req 11.1: off by default, zero OTLP traffic).
    pub fn init_from_config(config: &Config) -> Telemetry {
        let Some(endpoint) = config
            .otel_endpoint
            .clone()
            .filter(|e| !e.trim().is_empty())
        else {
            return Telemetry {
                enabled: false,
                tracer_provider: None,
                meter_provider: None,
                logger_provider: None,
                counters: noop_counters(),
            };
        };
        let headers = parse_headers(config.otel_headers.as_deref().unwrap_or(""));
        let resource = Resource::builder()
            .with_service_name(config.otel_service_name.clone())
            .build();
        let base = endpoint.trim_end_matches('/').to_string();

        let span_exporter = opentelemetry_otlp::SpanExporter::builder()
            .with_http()
            .with_endpoint(format!("{base}/v1/traces"))
            .with_headers(headers.clone())
            .build();
        let tracer_provider = match span_exporter {
            Ok(exp) => Some(
                // Simple processor: exports inline on span end. At per-tool-call
                // volume this is fine, and it avoids the thread-based batch
                // processor (incompatible with the rt-tokio runtime on 0.33).
                SdkTracerProvider::builder()
                    .with_simple_exporter(exp)
                    .with_resource(resource.clone())
                    .build(),
            ),
            Err(e) => {
                tracing::warn!("otel span exporter init failed: {e}");
                None
            }
        };

        let metric_exporter = opentelemetry_otlp::MetricExporter::builder()
            .with_http()
            .with_endpoint(format!("{base}/v1/metrics"))
            .with_headers(headers.clone())
            .build();
        let meter_provider = match metric_exporter {
            Ok(exp) => {
                let reader = opentelemetry_sdk::metrics::PeriodicReader::builder(exp)
                    .with_interval(Duration::from_secs(10))
                    .build();
                Some(
                    SdkMeterProvider::builder()
                        .with_reader(reader)
                        .with_resource(resource.clone())
                        .build(),
                )
            }
            Err(e) => {
                tracing::warn!("otel metric exporter init failed: {e}");
                None
            }
        };

        let log_exporter = opentelemetry_otlp::LogExporter::builder()
            .with_http()
            .with_endpoint(format!("{base}/v1/logs"))
            .with_headers(headers)
            .build();
        let logger_provider = match log_exporter {
            Ok(exp) => Some(
                SdkLoggerProvider::builder()
                    .with_batch_exporter(exp)
                    .with_resource(resource)
                    .build(),
            ),
            Err(e) => {
                tracing::warn!("otel log exporter init failed: {e}");
                None
            }
        };

        let enabled = tracer_provider.is_some() && meter_provider.is_some();
        let counters = if let Some(mp) = &meter_provider {
            let meter = mp.meter("mmhmem");
            Counters {
                avoided_in: meter.u64_counter("mmhmem.tokens_avoided_input_est").build(),
                avoided_out: meter
                    .u64_counter("mmhmem.tokens_avoided_output_est")
                    .build(),
                worker_prompt: meter.u64_counter("mmhmem.worker_prompt_tokens").build(),
                worker_completion: meter.u64_counter("mmhmem.worker_completion_tokens").build(),
                cache: meter.u64_counter("mmhmem.shunt_cache_events").build(),
            }
        } else {
            noop_counters()
        };
        Telemetry {
            enabled,
            tracer_provider,
            meter_provider,
            logger_provider,
            counters,
        }
    }

    fn noop_counters() -> Counters {
        let meter = opentelemetry_sdk::metrics::SdkMeterProvider::builder()
            .build()
            .meter("mmhmem-noop");
        Counters {
            avoided_in: meter.u64_counter("noop").build(),
            avoided_out: meter.u64_counter("noop").build(),
            worker_prompt: meter.u64_counter("noop").build(),
            worker_completion: meter.u64_counter("noop").build(),
            cache: meter.u64_counter("noop").build(),
        }
    }

    /// Parses `OTEL_EXPORTER_OTLP_HEADERS` (`k=v,k2=v2`) into a map.
    fn parse_headers(raw: &str) -> HashMap<String, String> {
        raw.split(',')
            .filter_map(|pair| {
                let (k, v) = pair.split_once('=')?;
                let k = k.trim();
                if k.is_empty() {
                    None
                } else {
                    Some((k.to_string(), v.trim().to_string()))
                }
            })
            .collect()
    }

    /// For tests: force-flush the providers.
    #[cfg(test)]
    pub fn force_flush(t: &Telemetry) {
        if let Some(p) = &t.tracer_provider {
            let _ = p.force_flush();
        }
        if let Some(p) = &t.meter_provider {
            let _ = p.force_flush();
        }
    }
}

#[cfg(not(feature = "otel"))]
mod imp {
    use super::*;

    /// No-op telemetry shim (plain release binary, Req 11.2: no OTLP code path).
    pub struct Telemetry {
        enabled: bool,
    }

    pub struct ToolSpan;

    impl ToolSpan {
        /// No-op finish.
        pub fn finish(self, _outcome: &str, _latency_ms: u64) {}
    }

    impl Telemetry {
        /// Always false without the `otel` feature.
        pub fn is_enabled(&self) -> bool {
            false
        }

        /// No-op span.
        pub fn start_tool_span(&self, _tool: &str, _harness: &str, _project: &str) -> ToolSpan {
            let _ = self.enabled;
            ToolSpan
        }

        /// No-op shutdown.
        pub fn shutdown(self) {}
    }

    impl OtelSink for Telemetry {}

    /// Without the feature, always disabled even if the endpoint env is set.
    pub fn init_from_config(_config: &Config) -> Telemetry {
        Telemetry { enabled: false }
    }
}

pub use imp::{init_from_config, Telemetry, ToolSpan};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, ENV_ALLOWED_ROOTS, ENV_OTEL_ENDPOINT, ENV_TOKEN};
    use std::collections::HashMap;

    fn cfg_with(endpoint: Option<&str>) -> Config {
        let mut env: HashMap<String, String> = HashMap::new();
        env.insert(ENV_TOKEN.to_string(), "tok".to_string());
        env.insert(ENV_ALLOWED_ROOTS.to_string(), "/ws".to_string());
        if let Some(e) = endpoint {
            env.insert(ENV_OTEL_ENDPOINT.to_string(), e.to_string());
        }
        Config::from_env(&|k| env.get(k).cloned()).unwrap()
    }

    #[test]
    fn disabled_without_endpoint() {
        let t = init_from_config(&cfg_with(None));
        assert!(!t.is_enabled());
    }

    #[cfg(not(feature = "otel"))]
    #[test]
    fn feature_off_means_disabled_even_with_endpoint() {
        // Req 11.2: plain release binary has no OTLP path.
        let t = init_from_config(&cfg_with(Some("http://127.0.0.1:4318")));
        assert!(!t.is_enabled());
        let span = t.start_tool_span("smart_read", "opencode", "p");
        span.finish("ok", 12);
        t.shutdown();
    }

    #[cfg(feature = "otel")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn feature_on_with_endpoint_exports_span_to_stub() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;
        use tokio::io::AsyncReadExt;
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let posts = Arc::new(AtomicUsize::new(0));
        let pc = Arc::clone(&posts);
        tokio::spawn(async move {
            use tokio::io::AsyncWriteExt;
            while let Ok((mut sock, _)) = listener.accept().await {
                pc.fetch_add(1, Ordering::SeqCst);
                let mut buf = vec![0u8; 262144];
                let _ = sock.read(&mut buf).await;
                // The OTLP HTTP exporter expects a response; answer 200.
                let _ = sock
                    .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\nconnection: close\r\n\r\n")
                    .await;
            }
        });

        let t = init_from_config(&cfg_with(Some(&format!("http://{addr}"))));
        assert!(t.is_enabled());
        let span = t.start_tool_span("smart_read", "opencode", "proj");
        span.finish("shunted", 42);
        imp::force_flush(&t);
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        assert!(
            posts.load(Ordering::SeqCst) >= 1,
            "expected at least one OTLP POST"
        );
        t.shutdown();
    }
}
