//! Glue between `thane-core::config::Config` and the dispatcher.
//!
//! Lives behind both `syslog` and `webhook` features being on (which is the
//! default). Keeps the build logic out of the daemon crate so its constructor
//! stays short.

use std::path::PathBuf;
use std::sync::Arc;

use thane_core::config::Config;
use thane_core::secret_store::SecretStore;

use crate::dispatcher::{AuditDispatcher, DispatcherConfig};
use crate::dlq::DeadLetterQueue;
use crate::filters::parse_min_severity;
use crate::AuditSink;

#[cfg(feature = "syslog")]
use crate::syslog::{SyslogConfig, SyslogSink};

#[cfg(feature = "webhook")]
use crate::webhook::{WebhookConfig, WebhookSink};

#[cfg(feature = "splunk")]
use crate::splunk_hec::{SplunkHecConfig, SplunkHecSink};

#[cfg(feature = "datadog")]
use crate::datadog::{DatadogConfig, DatadogRegion, DatadogSink};

#[cfg(feature = "s3")]
use crate::s3::{ObjectLockKind, S3Config, S3Sink, SseMode};

#[cfg(feature = "loki")]
use crate::loki::{LokiAuth, LokiAuthMode, LokiConfig, LokiSink};

/// Build an [`AuditDispatcher`] from runtime config + the platform secret store.
///
/// `audit_dir` is the directory where the DLQ file is written (same dir as
/// `audit.jsonl`).
///
/// Returns `None` when no sink is enabled, signaling that the caller should
/// skip attaching a forwarder to the AuditLog at all (true no-op path, no
/// tokio task spawned).
pub fn build_dispatcher_from_config(
    config: &Config,
    secret_store: &dyn SecretStore,
    audit_dir: PathBuf,
) -> Option<AuditDispatcher> {
    let mut sinks: Vec<Arc<dyn AuditSink>> = Vec::new();

    #[cfg(feature = "syslog")]
    if let Some(sink) = build_syslog_sink(config) {
        sinks.push(sink);
    }

    #[cfg(feature = "webhook")]
    if let Some(sink) = build_webhook_sink(config, secret_store) {
        sinks.push(sink);
    }

    #[cfg(feature = "splunk")]
    if let Some(sink) = build_splunk_sink(config, secret_store) {
        sinks.push(sink);
    }

    #[cfg(feature = "datadog")]
    if let Some(sink) = build_datadog_sink(config, secret_store) {
        sinks.push(sink);
    }

    #[cfg(feature = "s3")]
    if let Some(sink) = build_s3_sink(config, secret_store) {
        sinks.push(sink);
    }

    #[cfg(feature = "loki")]
    if let Some(sink) = build_loki_sink(config, secret_store) {
        sinks.push(sink);
    }

    // Silence the unused-arg warning on configurations without any
    // secret-store-consuming sinks.
    let _ = secret_store;

    if sinks.is_empty() {
        return None;
    }

    let dlq = Arc::new(DeadLetterQueue::new(audit_dir));
    Some(AuditDispatcher::spawn(DispatcherConfig { sinks, dlq }))
}

#[cfg(feature = "syslog")]
fn build_syslog_sink(config: &Config) -> Option<Arc<dyn AuditSink>> {
    if !config.audit_sink_syslog_enabled() {
        return None;
    }
    let Some(host) = config.audit_sink_syslog_host() else {
        tracing::warn!(
            "audit-sink-syslog-enabled = true but audit-sink-syslog-host not set; sink disabled"
        );
        return None;
    };
    let mut cfg = SyslogConfig::new(host.to_string(), config.audit_sink_syslog_port());
    cfg.use_tls = config.audit_sink_syslog_tls();
    cfg.ca_cert_path = config
        .audit_sink_syslog_ca_cert()
        .map(std::path::PathBuf::from);
    cfg.app_name = config.audit_sink_syslog_app_name();
    cfg.min_severity = parse_min_severity(config.audit_sink_syslog_min_severity());

    match SyslogSink::new(cfg) {
        Ok(s) => Some(Arc::new(s)),
        Err(e) => {
            tracing::error!("syslog sink init failed: {e}");
            None
        }
    }
}

#[cfg(feature = "webhook")]
fn build_webhook_sink(
    config: &Config,
    secret_store: &dyn SecretStore,
) -> Option<Arc<dyn AuditSink>> {
    if !config.audit_sink_webhook_enabled() {
        return None;
    }
    let Some(url) = config.audit_sink_webhook_url() else {
        tracing::warn!(
            "audit-sink-webhook-enabled = true but audit-sink-webhook-url not set; sink disabled"
        );
        return None;
    };
    let secret_id = config.audit_sink_webhook_secret_id();
    let secret = match secret_store.get(&secret_id) {
        Ok(Some(bytes)) if !bytes.is_empty() => bytes,
        Ok(Some(_)) => {
            tracing::error!(
                "webhook secret '{secret_id}' is empty; sink disabled"
            );
            return None;
        }
        Ok(None) => {
            tracing::error!(
                "webhook secret '{secret_id}' not found in secret store; sink disabled"
            );
            return None;
        }
        Err(e) => {
            tracing::error!("failed to load webhook secret '{secret_id}': {e}; sink disabled");
            return None;
        }
    };

    let mut cfg = WebhookConfig::new(url.to_string(), secret);
    cfg.timeout = std::time::Duration::from_secs(config.audit_sink_webhook_timeout_secs());
    cfg.min_severity = parse_min_severity(config.audit_sink_webhook_min_severity());

    match WebhookSink::new(cfg) {
        Ok(s) => Some(Arc::new(s)),
        Err(e) => {
            tracing::error!("webhook sink init failed: {e}");
            None
        }
    }
}

/// Look up a non-empty secret from the store. Logs and returns None on any
/// failure mode (missing entry, empty value, store error) — the sink stays
/// disabled rather than starting in a broken state.
#[cfg(any(feature = "splunk", feature = "datadog"))]
fn load_required_secret(
    secret_store: &dyn SecretStore,
    secret_id: &str,
    sink_name: &str,
) -> Option<Vec<u8>> {
    match secret_store.get(secret_id) {
        Ok(Some(bytes)) if !bytes.is_empty() => Some(bytes),
        Ok(Some(_)) => {
            tracing::error!("{sink_name} secret '{secret_id}' is empty; sink disabled");
            None
        }
        Ok(None) => {
            tracing::error!("{sink_name} secret '{secret_id}' not found; sink disabled");
            None
        }
        Err(e) => {
            tracing::error!("{sink_name} failed to load secret '{secret_id}': {e}; sink disabled");
            None
        }
    }
}

#[cfg(feature = "splunk")]
fn build_splunk_sink(
    config: &Config,
    secret_store: &dyn SecretStore,
) -> Option<Arc<dyn AuditSink>> {
    if !config.audit_sink_splunk_enabled() {
        return None;
    }
    let Some(url) = config.audit_sink_splunk_url() else {
        tracing::warn!(
            "audit-sink-splunk-enabled = true but audit-sink-splunk-url not set; sink disabled"
        );
        return None;
    };
    let secret_id = config.audit_sink_splunk_token_secret_id();
    let token_bytes = load_required_secret(secret_store, &secret_id, "splunk")?;
    let token = match String::from_utf8(token_bytes) {
        Ok(s) => s,
        Err(_) => {
            tracing::error!("splunk token in '{secret_id}' is not UTF-8; sink disabled");
            return None;
        }
    };

    let mut cfg = SplunkHecConfig::new(url.to_string(), token);
    cfg.index = config
        .audit_sink_splunk_index()
        .map(|s| s.to_string());
    cfg.verify_tls = config.audit_sink_splunk_verify_tls();
    cfg.min_severity = parse_min_severity(config.audit_sink_splunk_min_severity());

    match SplunkHecSink::new(cfg) {
        Ok(s) => Some(Arc::new(s)),
        Err(e) => {
            tracing::error!("splunk sink init failed: {e}");
            None
        }
    }
}

#[cfg(feature = "datadog")]
fn build_datadog_sink(
    config: &Config,
    secret_store: &dyn SecretStore,
) -> Option<Arc<dyn AuditSink>> {
    if !config.audit_sink_datadog_enabled() {
        return None;
    }
    let secret_id = config.audit_sink_datadog_api_key_secret_id();
    let api_key_bytes = load_required_secret(secret_store, &secret_id, "datadog")?;
    let api_key = match String::from_utf8(api_key_bytes) {
        Ok(s) => s,
        Err(_) => {
            tracing::error!("datadog API key in '{secret_id}' is not UTF-8; sink disabled");
            return None;
        }
    };

    let region = DatadogRegion::parse(config.audit_sink_datadog_region());
    let mut cfg = DatadogConfig::new(region, api_key);
    cfg.env = config.audit_sink_datadog_env();
    cfg.service = config.audit_sink_datadog_service();
    cfg.min_severity = parse_min_severity(config.audit_sink_datadog_min_severity());

    match DatadogSink::new(cfg) {
        Ok(s) => Some(Arc::new(s)),
        Err(e) => {
            tracing::error!("datadog sink init failed: {e}");
            None
        }
    }
}

#[cfg(feature = "s3")]
fn build_s3_sink(
    config: &Config,
    secret_store: &dyn SecretStore,
) -> Option<Arc<dyn AuditSink>> {
    if !config.audit_sink_s3_enabled() {
        return None;
    }
    let Some(bucket) = config.audit_sink_s3_bucket() else {
        tracing::warn!(
            "audit-sink-s3-enabled = true but audit-sink-s3-bucket not set; sink disabled"
        );
        return None;
    };

    // Static credentials are optional. When both secrets are absent the AWS
    // SDK falls back to its default credential chain (IAM role, env vars,
    // ~/.aws/credentials, IMDS) — see AUDIT_LOG.md operational notes.
    let access_key = secret_store
        .get(&config.audit_sink_s3_access_key_id_secret_id())
        .ok()
        .flatten()
        .and_then(|b| String::from_utf8(b).ok())
        .filter(|s| !s.is_empty());
    let secret_key = secret_store
        .get(&config.audit_sink_s3_secret_key_secret_id())
        .ok()
        .flatten()
        .and_then(|b| String::from_utf8(b).ok())
        .filter(|s| !s.is_empty());

    let mut cfg = S3Config::new(bucket.to_string(), config.audit_sink_s3_region());
    cfg.endpoint_url = config
        .audit_sink_s3_endpoint()
        .map(|s| s.to_string());
    cfg.access_key_id = access_key;
    cfg.secret_access_key = secret_key;
    cfg.prefix = config.audit_sink_s3_prefix();
    cfg.sse_mode = SseMode::parse(config.audit_sink_s3_sse_mode());
    cfg.kms_key_id = config
        .audit_sink_s3_kms_key_id()
        .map(|s| s.to_string());
    cfg.object_lock_kind = ObjectLockKind::parse(config.audit_sink_s3_object_lock_mode());
    cfg.object_lock_days = config.audit_sink_s3_object_lock_days();
    cfg.min_severity = parse_min_severity(config.audit_sink_s3_min_severity());

    match S3Sink::new(cfg) {
        Ok(s) => Some(Arc::new(s)),
        Err(e) => {
            tracing::error!("s3 sink init failed: {e}");
            None
        }
    }
}

#[cfg(feature = "loki")]
fn build_loki_sink(
    config: &Config,
    secret_store: &dyn SecretStore,
) -> Option<Arc<dyn AuditSink>> {
    if !config.audit_sink_loki_enabled() {
        return None;
    }
    let Some(url) = config.audit_sink_loki_url() else {
        tracing::warn!(
            "audit-sink-loki-enabled = true but audit-sink-loki-url not set; sink disabled"
        );
        return None;
    };
    let Some(tenant) = config.audit_sink_loki_tenant() else {
        tracing::warn!(
            "audit-sink-loki-enabled = true but audit-sink-loki-tenant not set; sink disabled \
             (the X-Scope-OrgID header is required by multi-tenant Loki)"
        );
        return None;
    };

    let mode = LokiAuth::parse_mode(config.audit_sink_loki_auth_mode());
    let auth = match mode {
        LokiAuthMode::Bearer => {
            let secret_id = config.audit_sink_loki_auth_secret_id();
            let token = load_loki_token(secret_store, &secret_id)?;
            LokiAuth::Bearer { token }
        }
        LokiAuthMode::Basic => {
            let secret_id = config.audit_sink_loki_auth_secret_id();
            let token = load_loki_token(secret_store, &secret_id)?;
            let user = config
                .audit_sink_loki_basic_user()
                // Per Grafana Cloud convention the tenant id is the Basic user
                // when no separate username is set.
                .map(|s| s.to_string())
                .unwrap_or_else(|| tenant.to_string());
            LokiAuth::Basic { user, token }
        }
        LokiAuthMode::Mtls => {
            let Some(cert_path) = config.audit_sink_loki_client_cert() else {
                tracing::error!(
                    "loki auth-mode = mtls but audit-sink-loki-client-cert not set; sink disabled"
                );
                return None;
            };
            let Some(key_path) = config.audit_sink_loki_client_key() else {
                tracing::error!(
                    "loki auth-mode = mtls but audit-sink-loki-client-key not set; sink disabled"
                );
                return None;
            };
            let cert_pem = match std::fs::read(cert_path) {
                Ok(b) => b,
                Err(e) => {
                    tracing::error!("loki mtls cert read failed: {e}; sink disabled");
                    return None;
                }
            };
            let key_pem = match std::fs::read(key_path) {
                Ok(b) => b,
                Err(e) => {
                    tracing::error!("loki mtls key read failed: {e}; sink disabled");
                    return None;
                }
            };
            LokiAuth::Mtls { cert_pem, key_pem }
        }
        LokiAuthMode::None => LokiAuth::None,
    };

    let mut cfg = LokiConfig::new(url.to_string(), tenant.to_string(), auth);
    cfg.verify_tls = config.audit_sink_loki_verify_tls();
    cfg.compress = config.audit_sink_loki_compress();
    cfg.min_severity = parse_min_severity(config.audit_sink_loki_min_severity());

    if let Some(ca_path) = config.audit_sink_loki_ca_cert() {
        match std::fs::read(ca_path) {
            Ok(b) => cfg.ca_cert_pem = Some(b),
            Err(e) => {
                tracing::warn!(
                    "loki ca cert {ca_path} read failed: {e}; falling back to system trust store"
                );
            }
        }
    }

    match LokiSink::new(cfg) {
        Ok(s) => Some(Arc::new(s)),
        Err(e) => {
            tracing::error!("loki sink init failed: {e}");
            None
        }
    }
}

/// Fetch a Loki auth token from the platform secret store; log + return None
/// on any failure mode so the sink stays disabled rather than starting broken.
#[cfg(feature = "loki")]
fn load_loki_token(secret_store: &dyn SecretStore, secret_id: &str) -> Option<String> {
    match secret_store.get(secret_id) {
        Ok(Some(bytes)) if !bytes.is_empty() => match String::from_utf8(bytes) {
            Ok(s) => Some(s),
            Err(_) => {
                tracing::error!("loki token in '{secret_id}' is not UTF-8; sink disabled");
                None
            }
        },
        Ok(Some(_)) => {
            tracing::error!("loki secret '{secret_id}' is empty; sink disabled");
            None
        }
        Ok(None) => {
            tracing::error!("loki secret '{secret_id}' not found; sink disabled");
            None
        }
        Err(e) => {
            tracing::error!("loki failed to load secret '{secret_id}': {e}; sink disabled");
            None
        }
    }
}
