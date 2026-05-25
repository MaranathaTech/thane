//! Process-wide accessor for the audit-sink dispatcher (Phase 5).
//!
//! The dispatcher must outlive every cloned [`thane_audit_sink::DispatcherHandle`]
//! we hand to [`thane_core::audit::AuditLog::set_forwarder`]. Stashing it in
//! a `OnceLock<Arc<_>>` is the simplest way to do that without rewiring every
//! AuditLog owner.
//!
//! Spawned lazily on first request. Returns `None` when no sink is configured
//! (in which case nothing is forwarded and we don't spawn a tokio task at all).

use std::sync::{Arc, OnceLock};

use thane_audit_sink::AuditDispatcher;
use thane_core::config::Config;

static DISPATCHER: OnceLock<Option<Arc<AuditDispatcher>>> = OnceLock::new();

/// Get or initialize the dispatcher.
///
/// Subsequent calls return the same Arc (or the same `None` if no sink was
/// enabled at first init time — config changes require an app restart, matching
/// the rest of the audit pipeline).
pub fn dispatcher(config: &Config) -> Option<Arc<AuditDispatcher>> {
    DISPATCHER
        .get_or_init(|| {
            use thane_platform::traits::PlatformDirs;
            #[cfg(target_os = "linux")]
            let dirs = thane_platform::LinuxDirs;
            #[cfg(target_os = "macos")]
            let dirs = thane_platform::MacosDirs;
            let audit_dir = dirs.sessions_dir();

            let secret_store = thane_platform::default_secret_store();
            let built = thane_audit_sink::build_dispatcher_from_config(
                config,
                secret_store.as_ref(),
                audit_dir,
            );
            if built.is_some() {
                tracing::info!("audit external sinks active");
            }
            built.map(Arc::new)
        })
        .clone()
}
