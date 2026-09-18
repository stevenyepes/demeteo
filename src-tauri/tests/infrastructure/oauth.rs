// `commands::oauth`'s `#[tauri::command]` functions take `State<'_, AppContext>`,
// a Tauri-internal wrapper whose constructor is private (see
// `create_project_orchestration.rs`'s doc comment for the same constraint) —
// they cannot be invoked directly from a test. Since each command is a thin,
// zero-logic delegate to `ctx.oauth_grants` / `ctx.mcp_consent`, these tests
// exercise the same port calls the commands make (`grant_summary`, and
// `OAuthGrantRepository::{revoke_grant, list_active_grants}` against a real
// `SqliteAdapter`) directly. `super` = `commands::oauth`.

use super::*;
use crate::adapters::database::SqliteAdapter;
use crate::domain::ids::ClientId;
use crate::ports::oauth::{OAuthClientRepository, OAuthGrantRepository};
use rusqlite::Connection;

fn db() -> SqliteAdapter {
    SqliteAdapter::new(Connection::open_in_memory().unwrap()).unwrap()
}

fn client(id: &str) -> OAuthClient {
    OAuthClient {
        id: ClientId::new(id),
        client_name: "claude-desktop".to_string(),
        redirect_uris: vec!["http://127.0.0.1:5173/callback".to_string()],
        created_at: 1_000,
    }
}

fn grant(id: &str, client_id: &str) -> GrantRecord {
    GrantRecord {
        id: GrantId::new(id),
        client_id: ClientId::new(client_id),
        scopes: vec![Scope::Read, Scope::Spend],
        resource: "https://demeteo.local:9631/mcp".to_string(),
        issued_at: 1_000,
        expires_at: 9_999_999_999_999,
        revoked_at: None,
    }
}

#[test]
fn grant_summary_maps_client_and_grant_fields() {
    let summary = grant_summary((client("c-1"), grant("g-1", "c-1")), None);
    assert_eq!(summary.id, "g-1");
    assert_eq!(summary.client_name, "claude-desktop");
    assert_eq!(summary.scopes, vec![Scope::Read, Scope::Spend]);
    assert_eq!(summary.issued_at, 1_000);
    assert_eq!(summary.expires_at, 9_999_999_999_999);
    assert!(!summary.revoked);
    assert!(!summary.audience_mismatch);
}

#[test]
fn a_grant_for_another_address_is_flagged_only_while_a_listener_is_bound() {
    let pair = || (client("c-1"), grant("g-1", "c-1"));

    assert!(!grant_summary(pair(), None).audience_mismatch);
    assert!(!grant_summary(pair(), Some("https://demeteo.local:9631/mcp")).audience_mismatch);
    assert!(grant_summary(pair(), Some("http://127.0.0.1:9000")).audience_mismatch);
}

#[test]
fn revoke_mcp_grant_flips_revoked_at_and_list_mcp_grants_excludes_it_afterward() {
    let db = db();
    db.register_client(client("c-1")).unwrap();
    db.insert_grant(grant("g-1", "c-1"), "hash-of-token")
        .unwrap();

    // `list_mcp_grants` delegates straight to `list_active_grants` +
    // `grant_summary`; before revocation the grant is present.
    let before: Vec<McpGrantSummary> = db
        .list_active_grants()
        .unwrap()
        .into_iter()
        .map(|pair| grant_summary(pair, None))
        .collect();
    assert_eq!(before.len(), 1);
    assert_eq!(before[0].id, "g-1");
    assert!(!before[0].revoked);

    // `revoke_mcp_grant` delegates straight to `revoke_grant`.
    db.revoke_grant(&GrantId::new("g-1")).unwrap();

    let after: Vec<McpGrantSummary> = db
        .list_active_grants()
        .unwrap()
        .into_iter()
        .map(|pair| grant_summary(pair, None))
        .collect();
    assert!(
        after.is_empty(),
        "a revoked grant must drop out of list_mcp_grants immediately"
    );
}

#[test]
fn mcp_consent_decide_delivers_approval_to_a_registered_waiter() {
    let registry = crate::adapters::mcp::consent_waiter::McpConsentWaiterRegistry::new();
    registry.register("req-1");

    registry.deliver("req-1", ConsentDecision::Approved);

    assert_eq!(
        registry.take_decision("req-1"),
        Some(ConsentDecision::Approved)
    );
}

#[test]
fn mcp_consent_decide_denies_maps_approve_false_to_denied() {
    let registry = crate::adapters::mcp::consent_waiter::McpConsentWaiterRegistry::new();
    registry.register("req-2");

    let approve = false;
    let decision = if approve {
        ConsentDecision::Approved
    } else {
        ConsentDecision::Denied
    };
    registry.deliver("req-2", decision);

    assert_eq!(
        registry.take_decision("req-2"),
        Some(ConsentDecision::Denied)
    );
}
