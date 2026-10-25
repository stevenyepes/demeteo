#![cfg(target_os = "linux")]
//! The Hub's attachments, asserted from the desktop package: this file is the
//! evidence a Demeteo run has for them. The header of
//! `src-tauri/tests/hub_pairing.rs` says why that evidence has to live here
//! and not beside the code.

use async_trait::async_trait;
use demeteo_hub::adapters::memory_keys::MemoryKeys;
use demeteo_hub::api::attachments::{upload, UploadedView};
use demeteo_hub::api::dispatch::{dispatch, DispatchBody, EnqueuedView};
use demeteo_hub::api::testing::MemoryFleet;
use demeteo_hub::api::ApiContext;
use demeteo_hub::attachments::fetch::{admitted, fetch, fetch_htu};
use demeteo_hub::attachments::sweep::sweep_once;
use demeteo_hub::attachments::{AttachmentSettings, MAX_ATTACHMENTS_PER_DISPATCH};
use demeteo_hub::auth::ceremonies::Ceremonies;
use demeteo_hub::auth::challenge::{base64url_decode, base64url_encode};
use demeteo_hub::auth::handlers::ApiError;
use demeteo_hub::auth::testing::{answered, lazy_state, serve};
use demeteo_hub::auth::RelyingParty;
use demeteo_hub::config::{Config, ConfigError};
use demeteo_hub::pairing::access::Admission;
use demeteo_hub::pairing::grant::{FreshPair, TokenSecret};
use demeteo_hub::pairing::signing_key::SigningKey;
use demeteo_hub::pairing::testing::{claims, verifies_ed25519, MemoryFamily, SoftDeviceKey};
use demeteo_hub::ports::{DataKey, KeyService, KeyServiceError};
use demeteo_hub::protocol::{AttachmentRef, RequestOutcome, RequestPayload, Scope};
use demeteo_hub::routes::{self, AppState};
use demeteo_hub::socket::connections::Connections;
use demeteo_hub::socket::port::SocketStore;
use demeteo_hub::socket::requests::{self, OutcomeChange};
use demeteo_hub::socket::testing::MemoryStore;
use demeteo_hub::socket::REQUEST_TTL_SECS;
use demeteo_hub::store::requests::{RequestId, RequestState};
use demeteo_hub::store::tokens::TokenHash;
use demeteo_hub::store::{InstanceId, StoreError, UserId};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::future::ready;
use std::net::Ipv4Addr;
use std::path::Path;

const MAX_BYTES: &str = "DEMETEO_HUB_ATTACHMENT_MAX_BYTES";
const TTL_SECS: &str = "DEMETEO_HUB_ATTACHMENT_TTL_SECS";
const SWEEP_SECS: &str = "DEMETEO_HUB_ATTACHMENT_SWEEP_SECS";

fn required_env() -> HashMap<&'static str, String> {
    HashMap::from([
        ("DATABASE_URL", "postgres://hub:pw@db:5432/hub".to_owned()),
        (
            "DEMETEO_HUB_PUBLIC_URL",
            "https://hub.example.com".to_owned(),
        ),
        ("DEMETEO_HUB_RP_ID", "example.com".to_owned()),
        ("OPENBAO_ADDR", "http://bao:8200".to_owned()),
        ("OPENBAO_TOKEN", "s.token".to_owned()),
    ])
}

fn load(env: &HashMap<&'static str, String>) -> Result<Config, ConfigError> {
    Config::from_lookup(|name| env.get(name).cloned())
}

#[test]
fn attachment_settings_default_when_none_of_their_variables_is_set() {
    let config = load(&required_env()).unwrap();
    assert_eq!(
        config.attachments,
        AttachmentSettings {
            max_bytes: 26_214_400,
            ttl_secs: 3600,
            sweep_secs: 60,
        }
    );
}

#[test]
fn attachment_settings_are_read_from_their_variables() {
    let mut env = required_env();
    env.insert(MAX_BYTES, "1048576".to_owned());
    env.insert(TTL_SECS, "120".to_owned());
    env.insert(SWEEP_SECS, "5".to_owned());
    assert_eq!(
        load(&env).unwrap().attachments,
        AttachmentSettings {
            max_bytes: 1_048_576,
            ttl_secs: 120,
            sweep_secs: 5,
        }
    );
}

#[test]
fn an_attachment_setting_that_is_not_a_positive_integer_is_refused_by_name() {
    for var in [MAX_BYTES, TTL_SECS, SWEEP_SECS] {
        for value in ["0", "abc", "-5"] {
            let mut env = required_env();
            env.insert(var, value.to_owned());
            match load(&env) {
                Err(ConfigError::Invalid { var: named, .. }) => {
                    assert_eq!(named, var, "{var}={value}");
                }
                other => panic!("{var}={value} was not refused: {:?}", other.map(|_| ())),
            }
        }
    }
}

const SIGNING_KEY: &str = "demeteo-hub-signing";
const TRANSIT_KEY: &str = "demeteo-hub-transit";
const NOW: u64 = 1_700_000_000;
const MAX: u64 = 256;
const TTL: u64 = 900;

/// What an [`ApiContext`] borrows, owned by the test. `keys` is replaceable so
/// a test can make the key service sealed.
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
        Self::with_keys(MemoryKeys::with_keys([SIGNING_KEY, TRANSIT_KEY]))
    }

    fn with_keys(keys: MemoryKeys) -> Self {
        Self {
            keys,
            signing_key: SigningKey::new(SIGNING_KEY),
            connections: Connections::default(),
            ceremonies: Ceremonies::default(),
            rp: RelyingParty {
                rp_id: "hub.example".to_owned(),
                origin: "https://hub.example".to_owned(),
            },
            settings: AttachmentSettings {
                max_bytes: MAX,
                ttl_secs: TTL,
                sweep_secs: 60,
            },
        }
    }

    fn over<'a>(&'a self, fleet: &'a MemoryFleet) -> ApiContext<'a> {
        ApiContext {
            store: fleet,
            keys: &self.keys,
            signing_key: &self.signing_key,
            connections: &self.connections,
            ceremonies: &self.ceremonies,
            rp: &self.rp,
            attachments: fleet,
            transit_key: TRANSIT_KEY,
            settings: &self.settings,
        }
    }
}

fn paired(fleet: &MemoryFleet, scopes: &[Scope]) -> (UserId, InstanceId) {
    let user = fleet.tenant();
    (user, fleet.paired(user, scopes))
}

async fn uploaded(
    parts: &Parts,
    fleet: &MemoryFleet,
    user: UserId,
    instance: InstanceId,
    bytes: &[u8],
) -> Result<UploadedView, (u16, Value)> {
    match upload(
        &parts.over(fleet),
        user,
        &instance.0.to_string(),
        bytes,
        NOW,
    )
    .await
    {
        Ok(view) => Ok(view),
        Err(error) => Err(answered(error).await),
    }
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    needle.is_empty()
        || haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

fn lowercase_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn distinctive_plaintext() -> Vec<u8> {
    b"DEMETEO-ATTACHMENT-CANARY: a file the user attached, long enough to scan for. 0123456789"
        .to_vec()
}

#[test]
fn the_leak_scan_finds_a_plaintext_where_it_is() {
    let plaintext = distinctive_plaintext();
    let mut buffer = b"prefix ".to_vec();
    buffer.extend_from_slice(&plaintext);
    buffer.extend_from_slice(b" suffix");

    assert!(contains(&buffer, &plaintext));
    assert!(!contains(&buffer[1..buffer.len() - 8], &plaintext));
}

#[tokio::test]
async fn a_stored_attachment_holds_no_plaintext_no_plaintext_digest_and_a_wrapped_key() {
    let parts = Parts::new();
    let fleet = MemoryFleet::new();
    let (user, instance) = paired(&fleet, &[Scope::Spend]);
    let plaintext = distinctive_plaintext();
    assert!(plaintext.len() >= 64);
    let digest: [u8; 32] = Sha256::digest(&plaintext).into();
    let hex = lowercase_hex(&digest);

    let view = uploaded(&parts, &fleet, user, instance, &plaintext)
        .await
        .unwrap();

    let rows = fleet.attachments(instance);
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row.id.to_string(), view.attachment_id);
    let fields: [(&str, Vec<u8>); 6] = [
        ("ciphertext", row.ciphertext.clone()),
        ("wrapped_key", row.wrapped_key.as_bytes().to_vec()),
        ("sealed_digest", row.sealed_digest.clone()),
        ("id", row.id.as_bytes().to_vec()),
        ("instance_id", row.instance_id.0.as_bytes().to_vec()),
        ("whole row", format!("{row:?}").into_bytes()),
    ];
    for (name, field) in &fields {
        for (what, needle) in [
            ("plaintext", plaintext.as_slice()),
            ("raw digest", digest.as_slice()),
            ("hex digest", hex.as_bytes()),
        ] {
            assert!(!contains(field, needle), "{what} found in {name}");
        }
    }
    assert!(row.wrapped_key.starts_with("vault:"), "{}", row.wrapped_key);
    assert_eq!(view.sha256, hex);
    assert_eq!(view.size, plaintext.len() as u64);
}

#[tokio::test]
async fn an_upload_expires_its_ttl_after_now() {
    let parts = Parts::new();
    let fleet = MemoryFleet::new();
    let (user, instance) = paired(&fleet, &[Scope::Spend]);

    let view = uploaded(&parts, &fleet, user, instance, b"content")
        .await
        .unwrap();

    assert_eq!(view.expires_at, NOW + TTL);
    assert_eq!(fleet.attachments(instance)[0].expires_at, NOW + TTL);
}

fn too_large() -> (u16, Value) {
    (413, json!({"error": "attachment_too_large"}))
}

#[tokio::test]
async fn an_upload_over_the_limit_is_refused_and_stores_nothing() {
    let parts = Parts::new();
    let fleet = MemoryFleet::new();
    let (user, instance) = paired(&fleet, &[Scope::Spend]);
    let over = vec![b'x'; MAX as usize + 1];

    let refused = uploaded(&parts, &fleet, user, instance, &over).await;

    assert_eq!(refused.err(), Some(too_large()));
    assert!(fleet.attachments(instance).is_empty());
}

#[tokio::test]
async fn the_size_is_judged_before_the_key_service_is_asked() {
    let parts = Parts::with_keys(MemoryKeys::sealed());
    let fleet = MemoryFleet::new();
    let (user, instance) = paired(&fleet, &[Scope::Spend]);
    let over = vec![b'x'; MAX as usize + 1];

    let refused = uploaded(&parts, &fleet, user, instance, &over).await;

    assert_eq!(refused.err(), Some(too_large()));
    assert!(fleet.attachments(instance).is_empty());
}

#[tokio::test]
async fn an_upload_at_the_limit_and_an_empty_one_are_accepted() {
    let parts = Parts::new();
    let fleet = MemoryFleet::new();
    let (user, instance) = paired(&fleet, &[Scope::Spend]);

    for len in [MAX as usize, 0] {
        let view = uploaded(&parts, &fleet, user, instance, &vec![b'x'; len])
            .await
            .unwrap_or_else(|refused| panic!("{len} bytes refused: {refused:?}"));
        assert_eq!(view.size, len as u64);
    }
    assert_eq!(fleet.attachments(instance).len(), 2);
}

#[tokio::test]
async fn a_sealed_key_service_is_unavailable_and_stores_nothing() {
    let parts = Parts::with_keys(MemoryKeys::sealed());
    let fleet = MemoryFleet::new();
    let (user, instance) = paired(&fleet, &[Scope::Spend]);

    let refused = uploaded(&parts, &fleet, user, instance, b"content").await;

    assert_eq!(
        refused.err(),
        Some((503, json!({"error": "key_service_unavailable"})))
    );
    assert!(fleet.attachments(instance).is_empty());
}

#[tokio::test]
async fn another_tenants_instance_is_not_found_and_spend_off_is_refused_by_name() {
    let parts = Parts::new();
    let fleet = MemoryFleet::new();
    let (owner, instance) = paired(&fleet, &[Scope::Spend]);
    let (stranger, _) = paired(&fleet, &[Scope::Spend]);
    let (user, without_spend) = paired(&fleet, &[Scope::Read, Scope::Gates]);
    let not_found = (404, json!({"error": "not_found"}));

    assert_eq!(
        uploaded(&parts, &fleet, stranger, instance, b"x")
            .await
            .err(),
        Some(not_found.clone())
    );
    let api = parts.over(&fleet);
    let refused = upload(&api, owner, "not-a-uuid", b"x", NOW).await.err();
    assert_eq!(refused, Some(ApiError::NOT_FOUND));
    assert_eq!(
        uploaded(
            &parts,
            &fleet,
            user,
            without_spend,
            &[b'x'; MAX as usize + 1]
        )
        .await
        .err(),
        Some((409, json!({"error": "scope_not_granted", "scope": "spend"})))
    );
    assert!(fleet.attachments(instance).is_empty());
    assert!(fleet.attachments(without_spend).is_empty());
}

async fn dispatched(
    parts: &Parts,
    fleet: &MemoryFleet,
    user: UserId,
    instance: InstanceId,
    attachment_ids: &[String],
    now: u64,
) -> Result<EnqueuedView, (u16, Value)> {
    let body: DispatchBody = serde_json::from_value(json!({
        "project_id": "p-1",
        "workflow_id": "w-1",
        "title": "t",
        "description": "d",
        "attachment_ids": attachment_ids,
    }))
    .expect("a dispatch body");
    match dispatch(&parts.over(fleet), user, &instance.0.to_string(), body, now).await {
        Ok(view) => Ok(view),
        Err(error) => Err(answered(error).await),
    }
}

/// A well-formed UUID no upload minted: the upload's are random v4 ids, and
/// these are sequential.
fn nobodys(n: usize) -> String {
    format!("00000000-0000-4000-8000-{n:012}")
}

async fn staged_id(
    parts: &Parts,
    fleet: &MemoryFleet,
    user: UserId,
    instance: InstanceId,
    bytes: &[u8],
) -> String {
    uploaded(parts, fleet, user, instance, bytes)
        .await
        .expect("an upload")
        .attachment_id
}

#[tokio::test]
async fn a_dispatch_naming_an_attachment_it_may_not_bind_is_refused_and_changes_nothing() {
    let parts = Parts::new();
    let fleet = MemoryFleet::new();
    let (user, instance) = paired(&fleet, &[Scope::Spend]);
    let sibling = fleet.paired(user, &[Scope::Spend]);
    let (stranger, strangers_instance) = paired(&fleet, &[Scope::Spend]);

    let own = staged_id(&parts, &fleet, user, instance, b"own").await;
    let foreign = staged_id(&parts, &fleet, stranger, strangers_instance, b"foreign").await;
    let of_sibling = staged_id(&parts, &fleet, user, sibling, b"sibling").await;
    let bound = staged_id(&parts, &fleet, user, instance, b"bound").await;
    dispatched(
        &parts,
        &fleet,
        user,
        instance,
        std::slice::from_ref(&bound),
        NOW,
    )
    .await
    .expect("the first dispatch binds it");

    let not_found = (404, json!({"error": "not_found"}));
    let unavailable = (409, json!({"error": "attachment_unavailable"}));
    let invalid = (400, json!({"error": "invalid_request"}));
    let too_many: Vec<String> = (0..=MAX_ATTACHMENTS_PER_DISPATCH).map(nobodys).collect();
    let cases = [
        (
            "another user's",
            vec![foreign.clone()],
            NOW,
            not_found.clone(),
        ),
        (
            "another instance's",
            vec![of_sibling.clone()],
            NOW,
            not_found.clone(),
        ),
        ("unknown", vec![nobodys(0)], NOW, not_found.clone()),
        (
            "not a uuid",
            vec!["att-1".to_owned()],
            NOW,
            not_found.clone(),
        ),
        ("bound", vec![bound.clone()], NOW, unavailable.clone()),
        ("expired", vec![own.clone()], NOW + TTL, unavailable),
        (
            "duplicate",
            vec![own.clone(), own.clone()],
            NOW,
            invalid.clone(),
        ),
        ("too many", too_many, NOW, invalid),
        (
            "valid and foreign",
            vec![own.clone(), foreign.clone()],
            NOW,
            not_found,
        ),
    ];

    let watched = [instance, sibling, strangers_instance];
    let requests_before: Vec<_> = watched.iter().map(|i| fleet.requests(*i)).collect();
    let rows_before: Vec<_> = watched.iter().map(|i| fleet.attachments(*i)).collect();
    for (name, ids, now, expected) in cases {
        let refused = dispatched(&parts, &fleet, user, instance, &ids, now).await;
        assert_eq!(refused.err(), Some(expected), "{name}");
        let requests: Vec<_> = watched.iter().map(|i| fleet.requests(*i)).collect();
        let rows: Vec<_> = watched.iter().map(|i| fleet.attachments(*i)).collect();
        assert_eq!(requests, requests_before, "{name} queued a request");
        assert_eq!(rows, rows_before, "{name} changed an attachment row");
    }
}

#[tokio::test]
async fn a_dispatch_naming_two_own_attachments_signs_their_refs_in_order_and_binds_both() {
    let parts = Parts::new();
    let fleet = MemoryFleet::new();
    let (user, instance) = paired(&fleet, &[Scope::Spend]);
    let first = b"the first file".to_vec();
    let second = b"a second, longer file the user attached".to_vec();
    let first_id = staged_id(&parts, &fleet, user, instance, &first).await;
    let second_id = staged_id(&parts, &fleet, user, instance, &second).await;

    let answer = dispatched(
        &parts,
        &fleet,
        user,
        instance,
        &[second_id.clone(), first_id.clone()],
        NOW,
    )
    .await
    .expect("both attachments are the tenant's own and staged");

    let queued = fleet.requests(instance);
    assert_eq!(queued.len(), 1);
    let request = &queued[0];
    assert_eq!(answer.request_id, base64url_encode(&request.id));
    let RequestPayload::StartFeature(start) = &request.envelope.payload else {
        panic!("{:?}", request.envelope.payload);
    };
    let reference = |id: &str, bytes: &[u8]| AttachmentRef {
        id: id.to_owned(),
        size: bytes.len() as u64,
        sha256: lowercase_hex(&Sha256::digest(bytes)),
    };
    assert_eq!(
        start.attachments,
        vec![reference(&second_id, &second), reference(&first_id, &first)]
    );

    let advertised = parts
        .signing_key
        .public_key(&parts.keys)
        .await
        .expect("the signing key's public half");
    let signature = base64url_decode(&request.envelope.signature).expect("a base64url signature");
    let signed = request.envelope.signing_bytes().expect("signing bytes");
    assert!(verifies_ed25519(&advertised, &signed, &signature));

    let rows = fleet.attachments(instance);
    assert_eq!(rows.len(), 2);
    for row in &rows {
        assert_eq!(row.request_id, Some(request.id), "{}", row.id);
        assert_eq!(row.expires_at, request.expires_at, "{}", row.id);
    }
}

/// Who `instance` of `user` is admitted as. The fleet's tenants are minted by
/// [`MemoryFleet::tenant`], so a test cannot make up the user it names.
fn admitted_as(user: UserId, instance: InstanceId) -> Admission {
    Admission {
        user,
        instance,
        token_expires_at: NOW + 3600,
    }
}

async fn fetched(
    keys: &MemoryKeys,
    fleet: &MemoryFleet,
    admission: &Admission,
    attachment_id: &str,
    now: u64,
) -> Result<Vec<u8>, (u16, Value)> {
    match fetch(fleet, keys, TRANSIT_KEY, admission, attachment_id, now).await {
        Ok(bytes) => Ok(bytes),
        Err(error) => Err(answered(error).await),
    }
}

/// Uploads `bytes` for the instance and binds it to a fresh dispatch of it,
/// answering the attachment's id and the dispatch's request id.
async fn bound_id(
    parts: &Parts,
    fleet: &MemoryFleet,
    user: UserId,
    instance: InstanceId,
    bytes: &[u8],
) -> (String, String) {
    let id = staged_id(parts, fleet, user, instance, bytes).await;
    let view = dispatched(parts, fleet, user, instance, std::slice::from_ref(&id), NOW)
        .await
        .expect("a dispatch binding the attachment");
    (id, view.request_id)
}

fn held(fleet: &MemoryFleet, instance: InstanceId, id: &str) -> bool {
    fleet
        .attachments(instance)
        .iter()
        .any(|row| row.id.to_string() == id)
}

fn not_found() -> (u16, Value) {
    (404, json!({"error": "not_found"}))
}

#[tokio::test]
async fn a_bound_attachment_is_fetched_once_and_then_is_not_found() {
    let parts = Parts::new();
    let fleet = MemoryFleet::new();
    let (user, instance) = paired(&fleet, &[Scope::Spend]);
    let plaintext = distinctive_plaintext();
    let (id, _) = bound_id(&parts, &fleet, user, instance, &plaintext).await;
    let admission = admitted_as(user, instance);

    let first = fetched(&parts.keys, &fleet, &admission, &id, NOW + 1).await;

    assert_eq!(first, Ok(plaintext));
    assert!(!held(&fleet, instance, &id));
    let second = fetched(&parts.keys, &fleet, &admission, &id, NOW + 1).await;
    assert_eq!(second, Err(not_found()));
}

#[tokio::test]
async fn a_sealed_key_service_is_unavailable_and_the_attachment_stays_fetchable() {
    let parts = Parts::new();
    let fleet = MemoryFleet::new();
    let (user, instance) = paired(&fleet, &[Scope::Spend]);
    let (id, _) = bound_id(&parts, &fleet, user, instance, b"content").await;
    let admission = admitted_as(user, instance);

    let refused = fetched(&MemoryKeys::sealed(), &fleet, &admission, &id, NOW + 1).await;

    assert_eq!(
        refused,
        Err((503, json!({"error": "key_service_unavailable"})))
    );
    assert!(held(&fleet, instance, &id));
    let retried = fetched(&parts.keys, &fleet, &admission, &id, NOW + 1).await;
    assert_eq!(retried, Ok(b"content".to_vec()));
}

#[tokio::test]
async fn a_fetch_not_admitted_as_the_owning_instance_is_not_found_and_changes_nothing() {
    let parts = Parts::new();
    let fleet = MemoryFleet::new();
    let (user, instance) = paired(&fleet, &[Scope::Spend]);
    let sibling = fleet.paired(user, &[Scope::Spend]);
    let (stranger, strangers_instance) = paired(&fleet, &[Scope::Spend]);
    let plaintext = distinctive_plaintext();
    let (id, _) = bound_id(&parts, &fleet, user, instance, &plaintext).await;
    let owner = admitted_as(user, instance);
    let rows_before = fleet.attachments(instance);

    let cases = [
        (
            "another instance of the user",
            admitted_as(user, sibling),
            id.clone(),
        ),
        (
            "another user's instance",
            admitted_as(stranger, strangers_instance),
            id.clone(),
        ),
        (
            "another user naming the owner's instance",
            admitted_as(stranger, instance),
            id.clone(),
        ),
        ("an unknown id", owner, nobodys(0)),
        ("not a uuid", owner, "att-1".to_owned()),
    ];
    for (name, admission, named) in cases {
        let refused = fetched(&parts.keys, &fleet, &admission, &named, NOW + 1).await;
        assert_eq!(refused, Err(not_found()), "{name}");
        assert_eq!(fleet.attachments(instance), rows_before, "{name}");
    }

    let owned = fetched(&parts.keys, &fleet, &owner, &id, NOW + 1).await;
    assert_eq!(owned, Ok(plaintext));
}

#[tokio::test]
async fn an_unbound_an_answered_and_an_expired_attachment_are_not_found() {
    let parts = Parts::new();
    let fleet = MemoryFleet::new();
    let (user, instance) = paired(&fleet, &[Scope::Spend]);
    let admission = admitted_as(user, instance);

    let unbound = staged_id(&parts, &fleet, user, instance, b"unbound").await;
    let (answered_id, request_id) = bound_id(&parts, &fleet, user, instance, b"answered").await;
    let request = requests::named_request(&request_id).expect("the dispatch's request id");
    let decide = |state| requests::outcome_change(state, RequestOutcome::Accepted);
    let recorded = fleet
        .store(user, instance)
        .outcome_recorded(request, &decide, NOW + 1)
        .await
        .expect("the outcome is recorded");
    assert!(recorded);
    let (expired, _) = bound_id(&parts, &fleet, user, instance, b"expired").await;
    let expires_at = fleet
        .attachments(instance)
        .iter()
        .find(|row| row.id.to_string() == expired)
        .expect("the bound row")
        .expires_at;

    for (name, id, now, kept) in [
        ("unbound", &unbound, NOW + 1, true),
        ("answered", &answered_id, NOW + 1, false),
        ("expired", &expired, expires_at, true),
    ] {
        let refused = fetched(&parts.keys, &fleet, &admission, id, now).await;
        assert_eq!(refused, Err(not_found()), "{name}");
        assert_eq!(held(&fleet, instance, id), kept, "{name}");
    }
}

async fn outcome_of(
    fleet: &MemoryFleet,
    user: UserId,
    instance: InstanceId,
    request: RequestId,
    decide: &(dyn Fn(RequestState) -> OutcomeChange + Sync),
) -> bool {
    fleet
        .store(user, instance)
        .outcome_recorded(request, decide, NOW + 1)
        .await
        .expect("the outcome store answers")
}

fn ids_held(fleet: &MemoryFleet, instance: InstanceId) -> Vec<String> {
    fleet
        .attachments(instance)
        .iter()
        .map(|row| row.id.to_string())
        .collect()
}

#[tokio::test]
async fn recording_an_outcome_deletes_that_requests_attachments_and_no_others() {
    let parts = Parts::new();
    let fleet = MemoryFleet::new();
    let (user, instance) = paired(&fleet, &[Scope::Spend]);
    let (stranger, strangers_instance) = paired(&fleet, &[Scope::Spend]);
    let first = staged_id(&parts, &fleet, user, instance, b"first").await;
    let second = staged_id(&parts, &fleet, user, instance, b"second").await;
    let answered = dispatched(&parts, &fleet, user, instance, &[first, second], NOW)
        .await
        .expect("a dispatch binding both");
    let (other, _) = bound_id(&parts, &fleet, user, instance, b"other").await;
    let unbound = staged_id(&parts, &fleet, user, instance, b"unbound").await;
    let (strangers, _) = bound_id(&parts, &fleet, stranger, strangers_instance, b"x").await;
    let request = requests::named_request(&answered.request_id).expect("the dispatch's id");

    let decide = |state| requests::outcome_change(state, RequestOutcome::Accepted);
    assert!(outcome_of(&fleet, user, instance, request, &decide).await);

    assert_eq!(ids_held(&fleet, instance), vec![other, unbound]);
    assert_eq!(ids_held(&fleet, strangers_instance), vec![strangers]);
}

/// The forced `Keep` is the decision on a request still queued, which
/// [`requests::outcome_change`] never makes; the expired one with no delivery
/// is the `Keep` it does make, and its row is the sweep's to delete.
#[tokio::test]
async fn a_kept_outcome_and_an_unknown_request_delete_nothing() {
    let parts = Parts::new();
    let fleet = MemoryFleet::new();
    let (user, instance) = paired(&fleet, &[Scope::Spend]);
    let (_, queued) = bound_id(&parts, &fleet, user, instance, b"queued").await;
    let (_, expired) = bound_id(&parts, &fleet, user, instance, b"expired").await;
    let queued = requests::named_request(&queued).expect("the dispatch's id");
    let expired = requests::named_request(&expired).expect("the dispatch's id");
    fleet
        .store(user, instance)
        .request_expired(expired)
        .await
        .expect("the request is marked expired");
    let rows_before = fleet.attachments(instance);

    let keep = |_| OutcomeChange::Keep;
    assert!(outcome_of(&fleet, user, instance, queued, &keep).await);
    let decide = |state| requests::outcome_change(state, RequestOutcome::Accepted);
    assert!(outcome_of(&fleet, user, instance, expired, &decide).await);
    assert!(!outcome_of(&fleet, user, instance, [0xee; 16], &decide).await);

    assert_eq!(fleet.attachments(instance), rows_before);
}

#[derive(Clone, Copy, Debug)]
enum Lapse {
    Answered,
    Expired,
}

/// [`MemoryKeys`], except that unwrapping a data key first ends `request` the
/// way `lapse` says: the window between the fetch's read and its delete,
/// which is the one Transit round trip, made to hold an outcome.
struct LapsingMidFetch<'a> {
    keys: &'a MemoryKeys,
    store: MemoryStore,
    request: RequestId,
    lapse: Lapse,
}

#[async_trait]
impl KeyService for LapsingMidFetch<'_> {
    async fn encrypt(&self, key: &str, plaintext: &[u8]) -> Result<String, KeyServiceError> {
        self.keys.encrypt(key, plaintext).await
    }

    async fn decrypt(&self, key: &str, ciphertext: &str) -> Result<Vec<u8>, KeyServiceError> {
        match self.lapse {
            Lapse::Answered => {
                let decide = |state| requests::outcome_change(state, RequestOutcome::Accepted);
                let recorded = self
                    .store
                    .outcome_recorded(self.request, &decide, NOW + 1)
                    .await;
                assert!(matches!(recorded, Ok(true)));
            }
            Lapse::Expired => {
                let expired = self.store.request_expired(self.request).await;
                assert!(expired.is_ok());
            }
        }
        self.keys.decrypt(key, ciphertext).await
    }

    async fn data_key(&self, key: &str) -> Result<DataKey, KeyServiceError> {
        self.keys.data_key(key).await
    }

    async fn verifying_key(&self, key: &str) -> Result<[u8; 32], KeyServiceError> {
        self.keys.verifying_key(key).await
    }

    async fn sign(&self, key: &str, message: &[u8]) -> Result<[u8; 64], KeyServiceError> {
        self.keys.sign(key, message).await
    }
}

#[tokio::test]
async fn a_request_that_ends_while_its_attachment_is_opened_hands_over_nothing() {
    for lapse in [Lapse::Answered, Lapse::Expired] {
        let parts = Parts::new();
        let fleet = MemoryFleet::new();
        let (user, instance) = paired(&fleet, &[Scope::Spend]);
        let (id, request_id) = bound_id(&parts, &fleet, user, instance, b"content").await;
        let keys = LapsingMidFetch {
            keys: &parts.keys,
            store: fleet.store(user, instance),
            request: requests::named_request(&request_id).expect("the dispatch's request id"),
            lapse,
        };

        let refused = fetch(
            &fleet,
            &keys,
            TRANSIT_KEY,
            &admitted_as(user, instance),
            &id,
            NOW + 1,
        )
        .await;

        assert_eq!(refused.err(), Some(ApiError::NOT_FOUND), "{lapse:?}");
        let kept = matches!(lapse, Lapse::Expired);
        assert_eq!(held(&fleet, instance, &id), kept, "{lapse:?}");
    }
}

#[tokio::test]
async fn a_tampered_ciphertext_is_an_internal_error_and_the_row_is_kept() {
    let parts = Parts::new();
    let fleet = MemoryFleet::new();
    let (user, instance) = paired(&fleet, &[Scope::Spend]);
    let (id, _) = bound_id(&parts, &fleet, user, instance, b"content").await;
    fleet.tampered(instance, id.parse().expect("a uuid"));

    let refused = fetched(
        &parts.keys,
        &fleet,
        &admitted_as(user, instance),
        &id,
        NOW + 1,
    )
    .await;

    assert_eq!(refused, Err((500, json!({"error": "internal"}))));
    assert!(held(&fleet, instance, &id));
}

/// Swept at `NOW + TTL`, the instant both the staged and the bound row expire:
/// a sweep that spared a row at exactly its `expires_at` would leave two.
#[tokio::test]
async fn a_sweep_deletes_the_expired_staged_and_bound_rows_and_keeps_the_live_one() {
    let parts = Parts::new();
    let fleet = MemoryFleet::new();
    let (user, instance) = paired(&fleet, &[Scope::Spend]);
    let api = parts.over(&fleet);
    let swept_at = NOW + TTL;

    let staged = staged_id(&parts, &fleet, user, instance, b"staged").await;
    let bound = upload(&api, user, &instance.0.to_string(), b"bound", NOW)
        .await
        .expect("an upload")
        .attachment_id;
    let dispatched_at = swept_at - REQUEST_TTL_SECS;
    dispatched(
        &parts,
        &fleet,
        user,
        instance,
        std::slice::from_ref(&bound),
        dispatched_at,
    )
    .await
    .expect("a dispatch binding the attachment");
    let live = upload(&api, user, &instance.0.to_string(), b"live", NOW + 1)
        .await
        .expect("an upload")
        .attachment_id;
    let rows = fleet.attachments(instance);
    assert_eq!(rows.len(), 3);
    assert!(rows.iter().any(|row| row.request_id.is_some()));

    let deleted = sweep_once(&fleet, swept_at).await.expect("the sweep");

    assert_eq!(deleted, 2);
    let remaining: Vec<String> = fleet
        .attachments(instance)
        .iter()
        .map(|row| row.id.to_string())
        .collect();
    assert_eq!(remaining, vec![live]);
    let admission = admitted_as(user, instance);
    for id in [&staged, &bound] {
        let refused = fetched(&parts.keys, &fleet, &admission, id, dispatched_at).await;
        assert_eq!(refused, Err(not_found()), "{id}");
    }
}

/// The URLs a proof is made for, written out here and not built by the Hub: a
/// helper of the Hub's would agree with the Hub about a spelling both had
/// wrong.
const SOCKET_URL: &str = "https://hub.example/socket";

fn fetch_url(attachment_id: &str) -> String {
    format!("https://hub.example/attachments/{attachment_id}")
}

/// A Hub whose store is never dialled, and one paired instance as that store
/// would hold it: the token rows, and the one device key. Must be made inside
/// a Tokio runtime, as `lazy_state` requires.
struct Fetcher {
    state: AppState,
    key: SoftDeviceKey,
    issued: FreshPair,
    tokens: MemoryFamily,
    instance: InstanceId,
}

impl Fetcher {
    fn new() -> Self {
        let rp = RelyingParty {
            rp_id: "hub.example".to_owned(),
            origin: "https://hub.example".to_owned(),
        };
        let state = lazy_state("postgres://hub@db.invalid/hub", rp).expect("a parsable URL");
        let key = SoftDeviceKey::new();
        let issued = FreshPair::generate().expect("the system random source");
        let tokens = MemoryFamily::paired(&issued, &key.thumbprint(), NOW);
        Self {
            state,
            key,
            issued,
            tokens,
            instance: InstanceId::generate(),
        }
    }

    fn token(&self) -> &TokenSecret {
        &self.issued.access
    }

    /// A `GET` proof for `htu` at `now`, naming the access token. Every proof
    /// needs a `jti` of its own, or it is refused as a replay.
    fn proof(&self, htu: &str, now: u64, jti: &str) -> String {
        let mut claims = claims("GET", htu, now, jti);
        claims["ath"] = json!(base64url_encode(&Sha256::digest(
            self.token().text().as_bytes()
        )));
        self.key.proof_with(&self.key.header(), &claims)
    }

    /// A fetch of `attachment_id` presenting the access token and `proof`, put
    /// to the admission the fetch route uses.
    async fn answer(&self, attachment_id: &str, proof: &str) -> Result<Admission, (u16, Value)> {
        let user = self.tokens.rows()[0].user_id;
        let jkt = self.key.thumbprint();
        let find_token =
            |hash: TokenHash| ready(Ok::<_, StoreError>(self.tokens.find(&hash).cloned()));
        let instance_of_key = |owner: UserId, key: String| {
            let held = (owner == user && key == jkt).then_some(self.instance);
            ready(Ok::<_, StoreError>(held))
        };
        let authorization = format!("DPoP {}", self.token().text());
        let outcome = admitted(
            &self.state,
            attachment_id,
            Some(&authorization),
            Some(proof),
            NOW,
            find_token,
            instance_of_key,
        )
        .await;
        match outcome {
            Ok(admission) => Ok(admission),
            Err(refusal) => Err(answered(refusal).await),
        }
    }
}

fn invalid_token() -> (u16, Value) {
    (401, json!({"error": "invalid_token"}))
}

#[tokio::test]
async fn the_fetch_htu_is_the_configured_origin_and_the_attachment_path() {
    let hub = Fetcher::new();
    let id = nobodys(1);
    assert_eq!(fetch_htu(&hub.state, &id), fetch_url(&id));
}

#[tokio::test]
async fn a_fetch_presenting_a_proof_made_for_the_socket_is_invalid_token() {
    let hub = Fetcher::new();
    let proof = hub.proof(SOCKET_URL, NOW, "jti-socket");
    assert_eq!(hub.answer(&nobodys(1), &proof).await, Err(invalid_token()));
}

#[tokio::test]
async fn a_fetch_presenting_a_proof_made_for_another_attachment_is_invalid_token() {
    let hub = Fetcher::new();
    let proof = hub.proof(&fetch_url(&nobodys(2)), NOW, "jti-other");
    assert_eq!(hub.answer(&nobodys(1), &proof).await, Err(invalid_token()));
}

#[tokio::test]
async fn a_fetch_proof_for_this_attachment_is_admitted_as_its_instance() {
    let hub = Fetcher::new();
    let id = nobodys(1);

    let proof = hub.proof(&fetch_url(&id), NOW, "jti-own");

    assert_eq!(
        hub.answer(&id, &proof).await,
        Ok(Admission {
            user: hub.tokens.rows()[0].user_id,
            instance: hub.instance,
            token_expires_at: NOW + 900,
        })
    );
}

/// The Hub's real router, asked for an attachment with no `Origin` and no
/// credentials. It answers the DPoP refusal, challenge and all, so the route
/// is mounted, is admitted as the socket is, and is not behind `/v1`'s guard.
#[tokio::test]
async fn the_fetch_route_answers_a_request_with_no_credentials_with_the_dpop_401() {
    let hub = Fetcher::new();
    let web_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("no-such-web-dir");
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .expect("a free loopback port");
    let addr = listener.local_addr().expect("the bound address");
    serve(listener, routes::router(hub.state.clone(), &web_dir));

    let response = reqwest::get(format!("http://{addr}/attachments/{}", nobodys(1)))
        .await
        .expect("an answer from the Hub");

    assert_eq!(response.status().as_u16(), 401);
    let challenge = response.headers().get("www-authenticate").cloned();
    assert_eq!(
        challenge.as_ref().map(|value| value.to_str().ok()),
        Some(Some("DPoP"))
    );
    let body: Value = response.json().await.expect("a JSON body");
    assert_eq!(body, json!({"error": "invalid_token"}));
}
