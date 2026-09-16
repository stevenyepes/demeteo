// Tests extracted from `crates/demeteo-core/src/adapters/database/repos/oauth.rs` (mirrored-tests convention). `super` = that module.

use super::*;
use rusqlite::Connection;

fn db() -> SqliteAdapter {
    SqliteAdapter::new(Connection::open_in_memory().unwrap()).unwrap()
}

fn client(id: &str) -> OAuthClient {
    OAuthClient {
        id: ClientId::from(id.to_string()),
        client_name: "claude-desktop".to_string(),
        redirect_uris: vec!["http://127.0.0.1:5173/callback".to_string()],
        created_at: 1_000,
    }
}

fn grant(id: &str, client_id: &str) -> GrantRecord {
    GrantRecord {
        id: GrantId::from(id.to_string()),
        client_id: ClientId::from(client_id.to_string()),
        scopes: vec![Scope::Read, Scope::Spend],
        resource: "https://demeteo.local:9631/mcp".to_string(),
        issued_at: 1_000,
        // Far beyond any real `now_ms()` this test will ever run under —
        // `list_active_grants` filters against wall-clock time, so an
        // "active" fixture must outlive it rather than a fixed offset from
        // `issued_at`.
        expires_at: 9_999_999_999_999,
        revoked_at: None,
    }
}

#[test]
fn register_and_get_client_round_trips() {
    let db = db();
    db.register_client(client("c-1")).unwrap();

    let found = db.get_client(&ClientId::from("c-1".to_string())).unwrap();
    assert_eq!(found, Some(client("c-1")));

    assert_eq!(
        db.get_client(&ClientId::from("c-missing".to_string()))
            .unwrap(),
        None
    );
}

#[test]
fn insert_and_find_grant_by_token_hash_round_trips() {
    let db = db();
    db.register_client(client("c-1")).unwrap();
    db.insert_grant(grant("g-1", "c-1"), "hash-of-token")
        .unwrap();

    let found = db
        .find_grant_by_token_hash("hash-of-token")
        .unwrap()
        .expect("grant must be found by its token hash");
    assert_eq!(found, grant("g-1", "c-1"));

    assert_eq!(db.find_grant_by_token_hash("no-such-hash").unwrap(), None);
}

#[test]
fn list_active_grants_excludes_revoked_and_expired() {
    let db = db();
    db.register_client(client("c-1")).unwrap();

    db.insert_grant(grant("g-active", "c-1"), "hash-active")
        .unwrap();

    let mut revoked = grant("g-revoked", "c-1");
    revoked.revoked_at = Some(2_000);
    db.insert_grant(revoked, "hash-revoked").unwrap();

    let mut expired = grant("g-expired", "c-1");
    expired.expires_at = 1; // long past
    db.insert_grant(expired, "hash-expired").unwrap();

    let active = db.list_active_grants().unwrap();
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].0, client("c-1"));
    assert_eq!(active[0].1.id, GrantId::from("g-active".to_string()));
}

#[test]
fn revoke_grant_sets_revoked_at_but_stays_findable_by_token_hash() {
    let db = db();
    db.register_client(client("c-1")).unwrap();
    db.insert_grant(grant("g-1", "c-1"), "hash-of-token")
        .unwrap();

    db.revoke_grant(&GrantId::from("g-1".to_string())).unwrap();

    let found = db
        .find_grant_by_token_hash("hash-of-token")
        .unwrap()
        .expect("a revoked grant is a flag, not a deletion");
    assert!(found.revoked_at.is_some());

    assert!(
        db.list_active_grants().unwrap().is_empty(),
        "a revoked grant must drop out of the active list immediately"
    );
}
