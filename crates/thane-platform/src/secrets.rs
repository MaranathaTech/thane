//! Re-exports of the `SecretStore` trait + `MemorySecretStore` from `thane-core`.
//!
//! The trait lives in `thane-core` (so platform-agnostic modules like
//! `thane_core::audit_keys` can call it without taking a `thane-platform`
//! dependency). Platform-specific implementations live alongside this module
//! in `secrets_macos.rs` / `secrets_linux.rs`.

pub use thane_core::secret_store::{MemorySecretStore, SecretError, SecretStore};
