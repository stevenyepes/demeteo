// domain::oauth's own vocabulary. `super` = `domain::oauth`.

use super::*;

const RESOURCE: &str = "https://demeteo.local/mcp";

fn grant(
    scopes: &[Scope],
    resource: &str,
    expires_at: i64,
    revoked_at: Option<i64>,
) -> GrantRecord {
    GrantRecord {
        id: GrantId::new("grant-1"),
        client_id: ClientId::new("client-1"),
        scopes: scopes.to_vec(),
        resource: resource.to_string(),
        issued_at: 0,
        expires_at,
        revoked_at,
    }
}

#[test]
fn revoked_grant_is_rejected_before_anything_else() {
    let g = grant(&[Scope::Read], RESOURCE, 1_000, Some(500));
    assert_eq!(
        validate_grant(&g, 100, RESOURCE, Scope::Read),
        Err(OAuthError::TokenRevoked)
    );
}

#[test]
fn expired_grant_is_rejected() {
    let g = grant(&[Scope::Read], RESOURCE, 100, None);
    assert_eq!(
        validate_grant(&g, 200, RESOURCE, Scope::Read),
        Err(OAuthError::TokenExpired)
    );
}

#[test]
fn resource_mismatch_is_rejected() {
    let g = grant(&[Scope::Read], RESOURCE, 1_000, None);
    assert_eq!(
        validate_grant(&g, 100, "https://other.example/mcp", Scope::Read),
        Err(OAuthError::AudienceMismatch)
    );
}

#[test]
fn read_only_grant_cannot_spend() {
    let g = grant(&[Scope::Read], RESOURCE, 1_000, None);
    assert_eq!(
        validate_grant(&g, 100, RESOURCE, Scope::Spend),
        Err(OAuthError::InsufficientScope {
            required: Scope::Spend
        })
    );
}

#[test]
fn matching_grant_is_ok() {
    let g = grant(&[Scope::Read, Scope::Spend], RESOURCE, 1_000, None);
    assert_eq!(validate_grant(&g, 100, RESOURCE, Scope::Spend), Ok(()));
}
