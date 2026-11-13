#![cfg(target_os = "linux")]
//! Removing an instance from the Hub through the session API: the passkey
//! challenge, the assertion the Hub verifies itself, and what the fleet,
//! the runs and the live socket show afterwards. The store's half of a
//! removal is `crates/demeteo-hub/tests/removal.rs`; the header of
//! `src-tauri/tests/hub_pairing.rs` says why the API's evidence lives here.

use demeteo_hub::adapters::memory_keys::MemoryKeys;
use demeteo_hub::api::dispatch::cancel;
use demeteo_hub::api::fleet::{fleet, instance_detail};
use demeteo_hub::api::removal::{removal_challenge, remove, RemovalBody, RemovalChallengeView};
use demeteo_hub::api::runs::{run_detail, runs, RunsFilter};
use demeteo_hub::api::testing::MemoryFleet;
use demeteo_hub::api::{ApiContext, ApiStore};
use demeteo_hub::attachments::AttachmentSettings;
use demeteo_hub::auth::ceremonies::Ceremonies;
use demeteo_hub::auth::challenge::base64url_decode;
use demeteo_hub::auth::policy::Status;
use demeteo_hub::auth::testing::SoftAuthenticator;
use demeteo_hub::auth::RelyingParty;
use demeteo_hub::pairing::signing_key::SigningKey;
use demeteo_hub::protocol::{RunEvent, RunEventKind, RunEventPayload, Scope, WebAuthnAssertion};
use demeteo_hub::socket::connections::{Closing, Connections};
use demeteo_hub::store::instances::Instance;
use demeteo_hub::store::passkeys::Passkey;
use demeteo_hub::store::{InstanceId, UserId};
use std::time::Duration;

const SIGNING_KEY: &str = "demeteo-hub-signing";
const NOW: u64 = 1_700_000_000;
const HOLDER: &[u8] = b"the passkey the session signed in with";

struct Parts {
    keys: MemoryKeys,
    signing_key: SigningKey,
    connections: Connections,
    ceremonies: Ceremonies,
    rp: RelyingParty,
    settings: AttachmentSettings,
}

impl Parts {
    fn new() -> Self {
        Self {
            keys: MemoryKeys::with_keys([SIGNING_KEY]),
            signing_key: SigningKey::new(SIGNING_KEY),
            connections: Connections::default(),
            ceremonies: Ceremonies::default(),
            rp: RelyingParty {
                rp_id: "hub.example".to_owned(),
                origin: "https://hub.example".to_owned(),
            },
            settings: AttachmentSettings::default(),
        }
    }

    fn over<'a>(&'a self, store: &'a MemoryFleet) -> ApiContext<'a> {
        ApiContext {
            store,
            keys: &self.keys,
            signing_key: &self.signing_key,
            connections: &self.connections,
            ceremonies: &self.ceremonies,
            rp: &self.rp,
            attachments: store,
            transit_key: SIGNING_KEY,
            settings: &self.settings,
        }
    }
}

/// One tenant with two instances, each with one run mirrored, and the
/// tenant's one trusted passkey.
struct Tenant {
    fleet: MemoryFleet,
    user: UserId,
    doomed: InstanceId,
    kept: InstanceId,
    authenticator: SoftAuthenticator,
    row: Instance,
}

impl Tenant {
    async fn new() -> Self {
        let fleet = MemoryFleet::new();
        let user = fleet.tenant();
        let scopes = [Scope::Read, Scope::Spend, Scope::Gates];
        let doomed = fleet.paired(user, &scopes);
        let kept = fleet.paired(user, &scopes);
        fleet.mirrored(doomed, started("f-doomed")).await;
        fleet.mirrored(kept, started("f-kept")).await;
        let authenticator = SoftAuthenticator::new();
        fleet.trusted(user, vec![authenticator.credential_id()]);
        let row = fleet.instances(user).await.expect("the rows").remove(0);
        Self {
            fleet,
            user,
            doomed,
            kept,
            authenticator,
            row,
        }
    }

    /// The tenant's passkeys as the handler reads them from the store.
    fn passkeys(&self) -> Vec<Passkey> {
        vec![passkey(&self.row, &self.authenticator, Status::Trusted)]
    }
}

/// `row` lends its owner and a timestamp, which the Hub never reads here.
fn passkey(row: &Instance, authenticator: &SoftAuthenticator, status: Status) -> Passkey {
    Passkey {
        user_id: row.user_id,
        credential_id: base64url_decode(&authenticator.credential_id()).expect("base64url"),
        public_key: authenticator.public_key_cose(),
        label: "key".to_owned(),
        sign_count: 0,
        status,
        credential: String::new(),
        created_at: row.created_at,
    }
}

fn started(feature_id: &str) -> RunEvent {
    RunEvent {
        v: 1,
        feature_id: feature_id.to_owned(),
        offset: 1,
        kind: RunEventKind::FeatureStarted,
        occurred_at: NOW,
        payload: RunEventPayload {
            step_id: None,
            step_execution_id: None,
            ticket_id: None,
            failure: None,
        },
    }
}

fn challenge_of(view: &RemovalChallengeView) -> [u8; 32] {
    let text = view.public_key["challenge"].as_str().expect("a challenge");
    base64url_decode(text)
        .expect("base64url")
        .try_into()
        .expect("32 bytes")
}

fn body(view: &RemovalChallengeView, assertion: WebAuthnAssertion) -> RemovalBody {
    serde_json::from_value(serde_json::json!({
        "ceremony_id": view.ceremony_id,
        "assertion": assertion,
    }))
    .expect("a removal body")
}

fn id(instance: InstanceId) -> String {
    instance.0.to_string()
}

#[tokio::test]
async fn the_removal_challenge_is_over_the_instance_and_allows_the_tenants_trusted_passkeys() {
    let t = Tenant::new().await;
    let parts = Parts::new();
    let api = parts.over(&t.fleet);

    let view = removal_challenge(&api, t.user, HOLDER.to_vec(), &id(t.doomed), NOW)
        .await
        .expect("a challenge");

    assert_eq!(
        view.public_key["allowCredentials"],
        serde_json::json!([{"type": "public-key", "id": t.authenticator.credential_id()}])
    );
    assert_eq!(view.public_key["userVerification"], "required");
    assert_eq!(parts.ceremonies.held_under(HOLDER), 1);
}

#[tokio::test]
async fn a_removed_instance_leaves_the_fleet_keeps_its_runs_and_its_socket_is_closed_removed() {
    let t = Tenant::new().await;
    let parts = Parts::new();
    let api = parts.over(&t.fleet);
    let mut live = parts.connections.register(t.doomed).expect("a seat");
    let mut other = parts.connections.register(t.kept).expect("a seat");

    let view = removal_challenge(&api, t.user, HOLDER.to_vec(), &id(t.doomed), NOW)
        .await
        .expect("a challenge");
    let assertion = t.authenticator.assert(&parts.rp, &challenge_of(&view));
    remove(
        &api,
        t.user,
        HOLDER,
        &id(t.doomed),
        body(&view, assertion),
        &t.passkeys(),
        NOW,
    )
    .await
    .expect("removed");

    let listed = fleet(&api, t.user, NOW).await.expect("the fleet");
    let ids: Vec<&str> = listed.instances.iter().map(|i| i.id.as_str()).collect();
    assert_eq!(ids, [id(t.kept)]);
    assert_eq!(
        instance_detail(&api, t.user, &id(t.doomed), NOW)
            .await
            .err()
            .map(|e| e.code()),
        Some("not_found")
    );

    let all = runs(&api, t.user, RunsFilter::default(), NOW)
        .await
        .expect("the runs");
    let flags: Vec<(&str, bool)> = all
        .runs
        .iter()
        .map(|run| (run.feature_id.as_str(), run.instance_removed))
        .collect();
    assert_eq!(flags, [("f-doomed", true), ("f-kept", false)]);
    let detail = run_detail(&api, t.user, &id(t.doomed), "f-doomed", NOW)
        .await
        .expect("a removed instance's run still reads");
    assert!(detail.instance_removed);

    assert_eq!(
        cancel(&api, t.user, &id(t.doomed), "f-doomed", NOW)
            .await
            .err()
            .map(|e| e.code()),
        Some("not_found")
    );
    assert_eq!(
        removal_challenge(&api, t.user, HOLDER.to_vec(), &id(t.doomed), NOW)
            .await
            .err()
            .map(|e| e.code()),
        Some("not_found")
    );

    let closed = tokio::time::timeout(Duration::from_secs(5), live.close.closing()).await;
    assert_eq!(
        closed.ok(),
        Some(Closing::Removed),
        "the live socket was asked to close 4003"
    );
    let untouched = tokio::time::timeout(Duration::from_millis(50), other.close.closing()).await;
    assert!(
        untouched.is_err(),
        "the other instance's socket is left open"
    );
}

#[tokio::test]
async fn an_assertion_the_hub_cannot_verify_removes_nothing_and_spends_the_challenge() {
    let t = Tenant::new().await;
    let parts = Parts::new();
    let api = parts.over(&t.fleet);
    let stranger = SoftAuthenticator::new();
    let revoked = vec![passkey(&t.row, &t.authenticator, Status::Revoked)];

    let cases: [(&str, &SoftAuthenticator, Vec<Passkey>); 3] = [
        (
            "a passkey the tenant does not hold",
            &stranger,
            t.passkeys(),
        ),
        ("a passkey the tenant revoked", &t.authenticator, revoked),
        (
            "an assertion over another challenge",
            &t.authenticator,
            t.passkeys(),
        ),
    ];
    for (case, signer, passkeys) in cases {
        let view = removal_challenge(&api, t.user, HOLDER.to_vec(), &id(t.doomed), NOW)
            .await
            .expect("a challenge");
        let mut signed = challenge_of(&view);
        if case.starts_with("an assertion over another") {
            signed[0] ^= 1;
        }
        let assertion = signer.assert(&parts.rp, &signed);
        let refused = remove(
            &api,
            t.user,
            HOLDER,
            &id(t.doomed),
            body(&view, assertion.clone()),
            &passkeys,
            NOW,
        )
        .await;
        assert_eq!(
            refused.err().map(|e| e.code()),
            Some("assertion_invalid"),
            "{case}"
        );

        let again = remove(
            &api,
            t.user,
            HOLDER,
            &id(t.doomed),
            body(&view, assertion),
            &passkeys,
            NOW,
        )
        .await;
        assert_eq!(
            again.err().map(|e| e.code()),
            Some("ceremony_expired"),
            "{case}"
        );
    }
    assert!(instance_detail(&api, t.user, &id(t.doomed), NOW)
        .await
        .is_ok());
}

/// The challenge names the instance it was held for: a path naming another,
/// another session, or another tenant's instance removes nothing.
#[tokio::test]
async fn a_challenge_removes_only_the_instance_and_session_it_was_held_for() {
    let t = Tenant::new().await;
    let other_tenant = t.fleet.tenant();
    let foreign = t.fleet.paired(other_tenant, &[Scope::Read]);
    let parts = Parts::new();
    let api = parts.over(&t.fleet);

    let attempts: [(&str, String, &[u8], Option<&str>); 3] = [
        (
            "another instance",
            id(t.kept),
            HOLDER,
            Some("ceremony_expired"),
        ),
        (
            "another session",
            id(t.doomed),
            b"another passkey",
            Some("ceremony_expired"),
        ),
        ("another tenant's", id(foreign), HOLDER, Some("not_found")),
    ];
    for (case, path, holder, expected) in attempts {
        let view = removal_challenge(&api, t.user, HOLDER.to_vec(), &id(t.doomed), NOW)
            .await
            .expect("a challenge");
        let assertion = t.authenticator.assert(&parts.rp, &challenge_of(&view));
        let refused = remove(
            &api,
            t.user,
            holder,
            &path,
            body(&view, assertion),
            &t.passkeys(),
            NOW,
        )
        .await;
        assert_eq!(refused.err().map(|e| e.code()), expected, "{case}");
    }
    let listed = fleet(&api, t.user, NOW).await.expect("the fleet");
    assert_eq!(listed.instances.len(), 2);
}
