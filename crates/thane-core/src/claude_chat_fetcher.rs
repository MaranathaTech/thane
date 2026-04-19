use serde::Deserialize;

/// An organization from the Claude.ai API.
#[derive(Debug, Clone, Deserialize)]
pub struct Organization {
    pub uuid: String,
    pub name: String,
}

/// A conversation summary from the Claude.ai API.
#[derive(Debug, Clone, Deserialize)]
pub struct ConversationSummary {
    pub uuid: String,
    pub name: String,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
}

/// A processed chat record ready for audit logging.
#[derive(Debug, Clone)]
pub struct ChatRecord {
    pub conversation_id: String,
    pub name: String,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub org_name: String,
}

/// Persistent state for the chat fetcher, caching org info and seen conversations.
#[derive(Debug, Default)]
pub struct ChatFetcherState {
    pub org_id: Option<String>,
    pub org_name: Option<String>,
    pub seen_conversation_ids: std::collections::HashSet<String>,
}

/// Fetch organizations from the Claude.ai API.
pub fn fetch_organizations(access_token: &str) -> Option<Vec<Organization>> {
    let output = std::process::Command::new("curl")
        .args([
            "-s", "-f",
            "-H", &format!("Authorization: Bearer {access_token}"),
            "https://claude.ai/api/organizations",
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    serde_json::from_slice(&output.stdout).ok()
}

/// Fetch conversations for an organization from the Claude.ai API.
pub fn fetch_conversations(access_token: &str, org_id: &str) -> Option<Vec<ConversationSummary>> {
    let url = format!("https://claude.ai/api/organizations/{org_id}/chat_conversations");
    let output = std::process::Command::new("curl")
        .args([
            "-s", "-f",
            "-H", &format!("Authorization: Bearer {access_token}"),
            &url,
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    serde_json::from_slice(&output.stdout).ok()
}

/// Process a list of conversations, deduplicating against already-seen IDs.
/// This is the testable core — does not make any network calls.
pub fn process_conversations(
    conversations: Vec<ConversationSummary>,
    org_name: &str,
    state: &mut ChatFetcherState,
) -> Vec<ChatRecord> {
    let mut new_chats = Vec::new();
    for conv in conversations {
        if conv.uuid.is_empty() || !state.seen_conversation_ids.insert(conv.uuid.clone()) {
            continue;
        }
        new_chats.push(ChatRecord {
            conversation_id: conv.uuid,
            name: conv.name,
            created_at: conv.created_at,
            updated_at: conv.updated_at,
            org_name: org_name.to_string(),
        });
    }
    new_chats
}

/// Fetch new Claude.ai conversations. Caches org ID internally.
/// Returns empty vec on any failure (no token, API error, etc.).
pub fn fetch_new_chats(access_token: &str, state: &mut ChatFetcherState) -> Vec<ChatRecord> {
    if state.org_id.is_none()
        && let Some(orgs) = fetch_organizations(access_token)
        && let Some(first_org) = orgs.into_iter().next()
    {
        state.org_name = Some(first_org.name.clone());
        state.org_id = Some(first_org.uuid);
    }
    let org_id = match &state.org_id {
        Some(id) => id.clone(),
        None => return Vec::new(),
    };
    let org_name = state.org_name.clone().unwrap_or_default();
    let conversations = match fetch_conversations(access_token, &org_id) {
        Some(c) => c,
        None => return Vec::new(),
    };
    process_conversations(conversations, &org_name, state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_organization_deserialization() {
        let json = r#"[{"uuid":"org-123","name":"My Org"}]"#;
        let orgs: Vec<Organization> = serde_json::from_str(json).unwrap();
        assert_eq!(orgs.len(), 1);
        assert_eq!(orgs[0].uuid, "org-123");
        assert_eq!(orgs[0].name, "My Org");
    }

    #[test]
    fn test_conversation_summary_deserialization() {
        let json = r#"[{"uuid":"conv-1","name":"Test Chat","created_at":"2025-01-01","updated_at":"2025-01-02"}]"#;
        let convs: Vec<ConversationSummary> = serde_json::from_str(json).unwrap();
        assert_eq!(convs.len(), 1);
        assert_eq!(convs[0].uuid, "conv-1");
        assert_eq!(convs[0].name, "Test Chat");
        assert_eq!(convs[0].created_at.as_deref(), Some("2025-01-01"));
    }

    #[test]
    fn test_conversation_missing_optional_fields() {
        let json = r#"[{"uuid":"conv-1","name":"Chat"}]"#;
        let convs: Vec<ConversationSummary> = serde_json::from_str(json).unwrap();
        assert_eq!(convs.len(), 1);
        assert!(convs[0].created_at.is_none());
        assert!(convs[0].updated_at.is_none());
    }

    #[test]
    fn test_process_conversations_deduplication() {
        let mut state = ChatFetcherState::default();
        state.seen_conversation_ids.insert("conv-1".to_string());

        let convs = vec![
            ConversationSummary { uuid: "conv-1".to_string(), name: "Old".to_string(), created_at: None, updated_at: None },
            ConversationSummary { uuid: "conv-2".to_string(), name: "New".to_string(), created_at: None, updated_at: None },
        ];
        let result = process_conversations(convs, "Org", &mut state);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].conversation_id, "conv-2");
        assert_eq!(result[0].org_name, "Org");
    }

    #[test]
    fn test_process_conversations_skips_empty_uuid() {
        let mut state = ChatFetcherState::default();
        let convs = vec![
            ConversationSummary { uuid: "".to_string(), name: "Empty".to_string(), created_at: None, updated_at: None },
            ConversationSummary { uuid: "conv-1".to_string(), name: "Valid".to_string(), created_at: None, updated_at: None },
        ];
        let result = process_conversations(convs, "Org", &mut state);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].conversation_id, "conv-1");
    }

    #[test]
    fn test_chat_fetcher_state_default() {
        let state = ChatFetcherState::default();
        assert!(state.org_id.is_none());
        assert!(state.org_name.is_none());
        assert!(state.seen_conversation_ids.is_empty());
    }
}
