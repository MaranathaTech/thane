use thane_core::session::AppSnapshot;
use serde::{Deserialize, Serialize};

/// Wrapper for versioned snapshot serialization.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionedSnapshot {
    pub version: u32,
    #[serde(flatten)]
    pub snapshot: AppSnapshot,
}

impl VersionedSnapshot {
    pub fn new(snapshot: AppSnapshot) -> Self {
        Self {
            version: AppSnapshot::CURRENT_VERSION,
            snapshot,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_versioned_snapshot_new() {
        let app = AppSnapshot::new(vec![], None);
        let versioned = VersionedSnapshot::new(app);
        assert_eq!(versioned.version, AppSnapshot::CURRENT_VERSION);
        assert!(versioned.snapshot.workspaces.is_empty());
    }

    #[test]
    fn test_versioned_snapshot_serializes_with_version() {
        let app = AppSnapshot::new(vec![], None);
        let versioned = VersionedSnapshot::new(app);
        let json = serde_json::to_string(&versioned).unwrap();
        // The flattened AppSnapshot also has a version field; verify the JSON
        // contains the expected version and the other snapshot fields.
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["version"], AppSnapshot::CURRENT_VERSION);
        assert_eq!(value["workspaces"], serde_json::json!([]));
        assert!(value["timestamp"].is_string());
    }
}
