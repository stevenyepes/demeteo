//! `GET /authorize` — the PKCE-gated, human-consent-gated authorization
//! request.
//!
//! implementation-spec.md §6 requires PKCE to be validated before any
//! consent UI is shown or grant is created, and AC3 requires that ordering
//! to hold even under rejection: this handler checks PKCE first and returns
//! before touching `ctx.oauth_clients` or `ctx.notif` at all when it fails,
//! so a rejected request emits zero `DomainEvent`s. `client_id` is resolved
//! only after PKCE passes, so an unknown `client_id` never reveals anything
//! a PKCE failure wouldn't already have.
//!
//! `resource` (RFC 8707) is mandatory and must equal [`canonical_uri`]
//! exactly — never defaulted or inferred (implementation-spec.md §6).
//!
//! A [`PendingAuthorization`] lives only in memory, from the moment
//! validation passes to the moment `POST /token` (a later ticket) consumes
//! its single-use code via [`take_pending_authorization`] — losing one on
//! restart is cheap to retry, unlike a `Gate` a driver may already be
//! blocked on.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::get;
use axum::Json;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use serde::Deserialize;
use serde_json::json;

use crate::domain::ids::ClientId;
use crate::domain::oauth::{pkce, Scope};
use crate::ports::notification::DomainEvent;
use crate::state::AppContext;

use super::canonical_uri;
use super::consent_waiter::ConsentDecision;

/// How long a parked `/authorize` request waits for a human decision before
/// resolving as if denied.
const CONSENT_TIMEOUT: Duration = Duration::from_secs(5 * 60);

/// Mounted onto the shared router by [`super::router`]. `pub(super)` — same
/// visibility as `metadata::routes` and `register::routes`.
pub(super) fn routes() -> axum::Router<AppContext> {
    axum::Router::new().route("/authorize", get(authorize))
}

#[derive(Debug, Deserialize)]
struct AuthorizeQuery {
    #[serde(default)]
    client_id: String,
    #[serde(default)]
    redirect_uri: String,
    #[serde(default)]
    code_challenge: String,
    #[serde(default)]
    code_challenge_method: String,
    #[serde(default)]
    resource: String,
    #[serde(default)]
    scope: String,
}

/// A validated, not-yet-decided `/authorize` request: PKCE, `resource`, the
/// client, its `redirect_uri`, and `scope` have all already checked out.
/// Kept for the lifetime of the parked request and, on approval, moved into
/// [`issue_code`]'s store for `POST /token` to consume — see the module docs.
#[derive(Debug, Clone)]
pub struct PendingAuthorization {
    pub request_id: String,
    pub client_id: ClientId,
    pub redirect_uri: String,
    pub code_challenge: String,
    pub resource: String,
    pub scopes: Vec<Scope>,
}

fn pending_codes() -> &'static Mutex<HashMap<String, PendingAuthorization>> {
    static CODES: OnceLock<Mutex<HashMap<String, PendingAuthorization>>> = OnceLock::new();
    CODES.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Consume and return the single-use authorization code's pending
/// authorization, if any — the seam `POST /token` (a later ticket) exchanges
/// a code through.
///
/// `pub`, not `pub(super)`: no caller exists in this crate yet, and an
/// unused `pub(super)` fn is indistinguishable from dead code to
/// `-D warnings` clippy — same reasoning as `guard::check`.
pub fn take_pending_authorization(code: &str) -> Option<PendingAuthorization> {
    pending_codes().lock().unwrap().remove(code)
}

/// Fills 256 bits from the OS CSPRNG and base64url-encodes them — used for
/// the `request_id`, the issued authorization code, and (via `token.rs`) the
/// issued access token. Never a predictable source (timestamp, counter): a
/// guessable code or token lets an attacker complete or hijack someone
/// else's authorization flow.
pub(super) fn generate_token() -> String {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("OS CSPRNG is unavailable");
    URL_SAFE_NO_PAD.encode(bytes)
}

fn issue_code(pending: PendingAuthorization) -> String {
    let code = generate_token();
    pending_codes()
        .lock()
        .unwrap()
        .insert(code.clone(), pending);
    code
}

/// Shared with `token.rs`: both endpoints report a rejected request the
/// same `{"error": "..."}` shape, per RFC 6749 §5.2.
pub(super) fn oauth_bad_request(error: &str) -> Response {
    (StatusCode::BAD_REQUEST, Json(json!({ "error": error }))).into_response()
}

/// `scope` is a single space-separated string, per RFC 6749 §3.3 and the
/// `oauth_grants.scopes` column's own storage convention — never repeated
/// query keys. `None` for an empty or all-unrecognized value: implementation-
/// spec.md requires "one or more" valid scopes, not zero.
fn parse_scopes(raw: &str) -> Option<Vec<Scope>> {
    let scopes: Option<Vec<Scope>> = raw.split_whitespace().map(Scope::parse).collect();
    scopes.filter(|scopes| !scopes.is_empty())
}

/// Appends a `key=value` pair to `uri`, respecting a query string `uri` may
/// already carry — a registered `redirect_uri` is free to have its own query
/// params, and blindly appending `?code=...` after one would build an
/// invalid URL.
fn append_query(uri: &str, pair: &str) -> String {
    let separator = if uri.contains('?') { '&' } else { '?' };
    format!("{uri}{separator}{pair}")
}

async fn authorize(State(ctx): State<AppContext>, Query(q): Query<AuthorizeQuery>) -> Response {
    if pkce::validate_challenge_request(&q.code_challenge, &q.code_challenge_method).is_err() {
        return oauth_bad_request("invalid_request");
    }

    let Some(canonical) = canonical_uri() else {
        return oauth_bad_request("invalid_target");
    };
    if q.resource != canonical {
        return oauth_bad_request("invalid_target");
    }

    let client_id = ClientId::new(q.client_id.clone());
    let client = match ctx.oauth_clients.get_client(&client_id) {
        Ok(Some(client)) => client,
        _ => return oauth_bad_request("invalid_request"),
    };

    if !client.redirect_uris.contains(&q.redirect_uri) {
        return oauth_bad_request("invalid_request");
    }

    let Some(scopes) = parse_scopes(&q.scope) else {
        return oauth_bad_request("invalid_scope");
    };

    let request_id = generate_token();
    let pending = PendingAuthorization {
        request_id: request_id.clone(),
        client_id,
        redirect_uri: q.redirect_uri.clone(),
        code_challenge: q.code_challenge,
        resource: q.resource.clone(),
        scopes: scopes.clone(),
    };
    let redirect_uri = pending.redirect_uri.clone();

    let _ = ctx.notif.emit(&DomainEvent::McpConsentRequested {
        request_id: request_id.clone(),
        client_name: client.client_name,
        requested_scopes: scopes,
        resource: q.resource,
    });
    ctx.notif.raise_main_window();

    let notify = ctx.mcp_consent.register(&request_id);
    let _ = tokio::time::timeout(CONSENT_TIMEOUT, notify.notified()).await;
    let decision = ctx.mcp_consent.take_decision(&request_id);

    match decision {
        Some(ConsentDecision::Approved) => {
            let code = issue_code(pending);
            Redirect::to(&append_query(&redirect_uri, &format!("code={code}"))).into_response()
        }
        Some(ConsentDecision::Denied) | None => {
            Redirect::to(&append_query(&redirect_uri, "error=access_denied")).into_response()
        }
    }
}

#[cfg(test)]
#[path = "../../../tests/adapters/mcp/authorize_pkce.rs"]
mod tests;
