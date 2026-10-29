#![cfg(target_os = "linux")]
//! The desktop's half of Hub requests (docs/HUB.md §6.1), driven against a
//! real migrated SQLite database.

mod launched_by {
    use std::time::{SystemTime, UNIX_EPOCH};

    use demeteo_core::adapters::database::SqliteAdapter;
    use demeteo_core::domain::feature_origin::FeatureOrigin;
    use demeteo_core::domain::ids::{FeatureId, ProjectId};
    use demeteo_core::domain::models::{Feature, LaunchedBy, Project};
    use demeteo_core::ports::db::{FeatureRepository, ProjectRepository};

    const DAY_MS: i64 = 86_400_000;

    fn store() -> (SqliteAdapter, std::path::PathBuf) {
        let dir = demeteo_core::test_dir::scratch("demeteo-hub-requests-launched-by");
        let conn = demeteo_core::db::init_db(dir.clone()).expect("init temp db");
        let adapter = SqliteAdapter::new(conn).expect("build sqlite adapter");
        ProjectRepository::add(
            &adapter,
            Project {
                id: ProjectId::new("p-1"),
                name: "demeteo".into(),
                compute_type: "local".into(),
                remote_host: None,
                status: "active".into(),
                nodes: 0,
                spend: 0.0,
                tokens: 0,
                created_at: 0,
            },
        )
        .expect("add project");
        (adapter, dir)
    }

    fn now_ms() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_millis() as i64
    }

    fn feature(
        id: &str,
        launched_by: LaunchedBy,
        created_at: i64,
        max_budget_usd: Option<f64>,
    ) -> Feature {
        Feature {
            id: FeatureId::new(id),
            project_id: ProjectId::new("p-1"),
            workflow_id: None,
            workflow_version_id: None,
            title: id.into(),
            description: String::new(),
            status: "running".into(),
            total_cost: 0.0,
            duration: "0s".into(),
            tokens: 0,
            created_at,
            agent_kind: None,
            model: None,
            effort: None,
            mr_url: None,
            mr_state: Some("none".into()),
            pr_title: None,
            pr_body: None,
            commit_artifacts: None,
            loop_iterations: None,
            max_budget_usd,
            step_overrides: Vec::new(),
            attachments: Vec::new(),
            harness_baseline: None,
            origin: FeatureOrigin::Branch {
                base: "release/1.2".into(),
            },
            diff_base_branch: None,
            resolved_branch: None,
            launched_by,
        }
    }

    fn read_back(store: &SqliteAdapter, id: &str) -> Feature {
        FeatureRepository::get(store, &FeatureId::new(id))
            .expect("get feature")
            .expect("feature exists")
    }

    #[test]
    fn a_hub_launch_reads_back_as_hub_with_its_origin_untouched() {
        let (store, dir) = store();
        let written = feature("f-hub", LaunchedBy::Hub, now_ms(), Some(1.0));
        FeatureRepository::add(&store, written.clone()).expect("add feature");

        let read = read_back(&store, "f-hub");
        assert_eq!(read.launched_by, LaunchedBy::Hub);
        assert_eq!(read.origin, written.origin);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_default_launch_reads_back_as_desktop_with_its_origin_untouched() {
        let (store, dir) = store();
        let written = feature("f-desk", LaunchedBy::default(), now_ms(), Some(1.0));
        FeatureRepository::add(&store, written.clone()).expect("add feature");

        let read = read_back(&store, "f-desk");
        assert_eq!(read.launched_by, LaunchedBy::Desktop);
        assert_eq!(read.origin, written.origin);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The window is inclusive at its start, counts only Hub launches, and
    /// sums caps in rounded cents: `0.29 * 100` is `28.999…` in floating
    /// point, so truncating instead of rounding would undercount by a cent.
    #[test]
    fn the_committed_total_counts_only_hub_rows_inside_the_last_day() {
        let (store, dir) = store();
        let now = now_ms();
        let since = now - DAY_MS;
        for f in [
            feature("hub-at-window-start", LaunchedBy::Hub, since, Some(1.25)),
            feature("hub-now", LaunchedBy::Hub, now, Some(0.29)),
            feature("hub-uncapped", LaunchedBy::Hub, now, None),
            feature("hub-1ms-too-old", LaunchedBy::Hub, since - 1, Some(7.0)),
            feature("desktop-now", LaunchedBy::Desktop, now, Some(3.0)),
        ] {
            FeatureRepository::add(&store, f).expect("add feature");
        }

        let committed = store
            .hub_committed_cents_since(since)
            .expect("committed total");
        assert_eq!(committed, 125 + 29);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_committed_total_is_zero_with_no_hub_launches() {
        let (store, dir) = store();
        let now = now_ms();
        FeatureRepository::add(
            &store,
            feature("desktop", LaunchedBy::Desktop, now, Some(3.0)),
        )
        .expect("add feature");

        assert_eq!(
            store
                .hub_committed_cents_since(now - DAY_MS)
                .expect("total"),
            0
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}

/// AC1: the pure decision, end to end from pinned scopes to a verdict, with
/// no store and no executor behind it.
mod decision {
    use demeteo_core::application::attachments::{
        MAX_AGENT_ATTACHMENT_BATCH_BYTES, MAX_ATTACHMENTS_PER_FEATURE, MAX_ATTACHMENT_BYTES,
    };
    use demeteo_core::domain::hub_request::{
        attachment_refusal, decide, effective_scopes, precheck, AttachmentLimits, Consumption,
        DropReason, ExecuteKind, HubLocalPolicy, Precheck, SignatureCheck, Verdict,
    };
    use demeteo_hub::protocol::{
        AttachmentRef, CancelFeature, Decision, GateDecision, PasskeyEndorsement,
        PasskeyRevocation, RefusalReason, RequestPayload, Scope, SetStepAssignment, SignedRequest,
        StartFeature, WebAuthnAssertion,
    };

    const INSTANCE: &str = "i-7f3a9c";
    const EXPIRES_AT: u64 = 1_791_020_400;
    const LIVE: u64 = EXPIRES_AT - 1;
    const ALL: &[Scope] = &[Scope::Read, Scope::Spend, Scope::Gates];
    const NO_SPEND: &[Scope] = &[Scope::Read, Scope::Gates];

    fn start(cap_cents: Option<u64>) -> RequestPayload {
        RequestPayload::StartFeature(StartFeature {
            project_id: "p-1".into(),
            workflow_id: "w-1".into(),
            title: "t".into(),
            description: "d".into(),
            agent_kind: None,
            model: None,
            effort: None,
            step_overrides: Vec::new(),
            max_budget_cents: cap_cents,
            max_wall_clock_secs: None,
            attachments: Vec::new(),
        })
    }

    fn assertion() -> WebAuthnAssertion {
        WebAuthnAssertion {
            credential_id: "c".into(),
            authenticator_data: "a".into(),
            client_data_json: "j".into(),
            signature: "s".into(),
        }
    }

    fn gate() -> RequestPayload {
        RequestPayload::GateDecision(GateDecision {
            feature_id: "f-1".into(),
            step_execution_id: "se-1".into(),
            decision: Decision::Approve,
            feedback: None,
            assertion: assertion(),
        })
    }

    fn policy(spend: bool, gates: bool, run: Option<u64>, daily: Option<u64>) -> HubLocalPolicy {
        HubLocalPolicy {
            read_enabled: true,
            spend_enabled: spend,
            gates_enabled: gates,
            run_ceiling_cents: run,
            daily_ceiling_cents: daily,
        }
    }

    /// Spend and gates on, 500 cents a run, 2000 a day.
    fn open() -> HubLocalPolicy {
        policy(true, true, Some(500), Some(2_000))
    }

    struct Row {
        name: &'static str,
        payload: RequestPayload,
        pinned: &'static [Scope],
        policy: HubLocalPolicy,
        signature: SignatureCheck,
        instance_id: &'static str,
        now: u64,
        consumption: Consumption,
        committed: u64,
        expected: Kind,
    }

    fn row(name: &'static str, payload: RequestPayload, expected: Kind) -> Row {
        Row {
            name,
            payload,
            pinned: ALL,
            policy: open(),
            signature: SignatureCheck::Valid,
            instance_id: INSTANCE,
            now: LIVE,
            consumption: Consumption::Fresh,
            committed: 0,
            expected,
        }
    }

    /// A [`Verdict`] with an executed payload reduced to its kind, so a row
    /// can name what it expects without borrowing the request `run` builds.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Kind {
        Start,
        Cancel,
        Assign,
        Gate,
        Revoke,
        Refuse(RefusalReason),
        Drop(DropReason),
        Defer,
    }

    fn kind(verdict: Verdict<'_>) -> Kind {
        match verdict {
            Verdict::Execute(ExecuteKind::StartFeature(_)) => Kind::Start,
            Verdict::Execute(ExecuteKind::CancelFeature(_)) => Kind::Cancel,
            Verdict::Execute(ExecuteKind::SetStepAssignment(_)) => Kind::Assign,
            Verdict::Execute(ExecuteKind::GateDecision(_)) => Kind::Gate,
            Verdict::Execute(ExecuteKind::PasskeyRevocation(_)) => Kind::Revoke,
            Verdict::Refuse(reason) => Kind::Refuse(reason),
            Verdict::Drop(reason) => Kind::Drop(reason),
            Verdict::Defer => Kind::Defer,
        }
    }

    fn run(row: &Row) -> Kind {
        let request = SignedRequest {
            v: 1,
            request_id: "Zk3m0R8pQnVx2tL5aYw9bA".into(),
            instance_id: row.instance_id.into(),
            issued_at: EXPIRES_AT - 600,
            expires_at: EXPIRES_AT,
            payload: row.payload.clone(),
            signature: String::new(),
        };
        let scopes = effective_scopes(row.pinned, &row.policy);
        kind(
            match precheck(&request, row.signature, INSTANCE, row.now, &scopes) {
                Precheck::Settled(verdict) => verdict,
                Precheck::Consume => decide(
                    &request.payload,
                    row.consumption,
                    Some(row.committed),
                    &row.policy,
                ),
            },
        )
    }

    const START: Kind = Kind::Start;
    const fn refuse(reason: RefusalReason) -> Kind {
        Kind::Refuse(reason)
    }

    #[test]
    fn every_row_gets_its_verdict() {
        use RefusalReason::*;
        let rows = [
            // (a) spend effectively off, both ways round.
            Row {
                policy: policy(false, true, Some(500), Some(2_000)),
                ..row("a: spend toggle off", start(Some(100)), refuse(ScopeOff))
            },
            Row {
                pinned: NO_SPEND,
                ..row(
                    "a: spend toggled on, never pinned",
                    start(Some(100)),
                    refuse(ScopeOff),
                )
            },
            Row {
                pinned: NO_SPEND,
                policy: policy(false, true, Some(500), Some(2_000)),
                ..row(
                    "a: cancel with spend off",
                    RequestPayload::CancelFeature(CancelFeature {
                        feature_id: "f-1".into(),
                    }),
                    refuse(ScopeOff),
                )
            },
            // (b)
            row("b: cap absent", start(None), refuse(OverRunCeiling)),
            // (c)
            row(
                "c: cap one cent over run",
                start(Some(501)),
                refuse(OverRunCeiling),
            ),
            // (d)
            Row {
                committed: 1_501,
                ..row(
                    "d: total one cent over daily",
                    start(Some(500)),
                    refuse(OverRollingCeiling),
                )
            },
            // (e)
            row("e: well within both", start(Some(100)), START),
            row("e: cap == run ceiling", start(Some(500)), START),
            Row {
                committed: 1_500,
                ..row("e: total + cap == daily", start(Some(500)), START)
            },
            // (f)
            Row {
                policy: policy(true, true, None, Some(2_000)),
                ..row(
                    "f: run ceiling unset",
                    start(Some(1)),
                    refuse(OverRunCeiling),
                )
            },
            Row {
                policy: policy(true, true, Some(500), None),
                ..row(
                    "f: daily ceiling unset",
                    start(Some(1)),
                    refuse(OverRollingCeiling),
                )
            },
            Row {
                policy: policy(true, true, Some(u64::MAX), Some(u64::MAX)),
                committed: 1,
                ..row(
                    "total + cap overflows u64",
                    start(Some(u64::MAX)),
                    refuse(OverRollingCeiling),
                )
            },
            // Expiry boundary.
            Row {
                now: EXPIRES_AT - 1,
                ..row("one second before expiry", start(Some(100)), START)
            },
            Row {
                now: EXPIRES_AT,
                ..row("at expiry", start(Some(100)), refuse(Expired))
            },
            // Instance, replay, signature.
            Row {
                instance_id: "i-other",
                ..row("wrong instance", start(Some(100)), refuse(WrongInstance))
            },
            Row {
                consumption: Consumption::AlreadyConsumed,
                ..row("replayed", start(Some(100)), refuse(Replayed))
            },
            Row {
                signature: SignatureCheck::Invalid,
                ..row(
                    "bad signature",
                    start(Some(100)),
                    Kind::Drop(DropReason::BadSignature),
                )
            },
            // Order of checks.
            Row {
                policy: policy(false, true, Some(500), Some(2_000)),
                now: EXPIRES_AT,
                ..row(
                    "scope off before expiry",
                    start(Some(100)),
                    refuse(ScopeOff),
                )
            },
            Row {
                signature: SignatureCheck::Invalid,
                now: EXPIRES_AT,
                ..row(
                    "bad signature before expiry",
                    start(Some(100)),
                    Kind::Drop(DropReason::BadSignature),
                )
            },
            Row {
                now: EXPIRES_AT,
                consumption: Consumption::AlreadyConsumed,
                ..row("expiry before replay", start(Some(100)), refuse(Expired))
            },
            Row {
                policy: policy(false, true, Some(500), Some(2_000)),
                signature: SignatureCheck::Invalid,
                ..row(
                    "bad signature before spend off",
                    start(Some(100)),
                    Kind::Drop(DropReason::BadSignature),
                )
            },
            Row {
                policy: policy(true, false, Some(500), Some(2_000)),
                signature: SignatureCheck::Invalid,
                ..row(
                    "bad signature before gates off",
                    gate(),
                    Kind::Drop(DropReason::BadSignature),
                )
            },
            // Other spend kinds carry no cap and ignore the ceilings.
            Row {
                policy: policy(true, false, None, None),
                ..row(
                    "cancel",
                    RequestPayload::CancelFeature(CancelFeature {
                        feature_id: "f-1".into(),
                    }),
                    Kind::Cancel,
                )
            },
            row(
                "set step assignment",
                RequestPayload::SetStepAssignment(SetStepAssignment {
                    feature_id: "f-1".into(),
                    step_id: "s-1".into(),
                    agent_kind: None,
                    model: None,
                    effort: None,
                }),
                Kind::Assign,
            ),
            // Gates and passkeys.
            row("gate decision, gates on", gate(), Kind::Gate),
            Row {
                policy: policy(true, false, Some(500), Some(2_000)),
                ..row("gate decision, gates off", gate(), refuse(ScopeOff))
            },
            Row {
                pinned: &[Scope::Read, Scope::Spend],
                ..row(
                    "gate decision, gates never pinned",
                    gate(),
                    refuse(ScopeOff),
                )
            },
            Row {
                pinned: &[],
                policy: HubLocalPolicy::default(),
                ..row(
                    "passkey endorsement, every scope off",
                    RequestPayload::PasskeyEndorsement(PasskeyEndorsement {
                        new_credential_id: "c2".into(),
                        new_public_key: "k2".into(),
                        assertion: assertion(),
                    }),
                    refuse(UnknownKind),
                )
            },
            Row {
                pinned: &[],
                policy: HubLocalPolicy::default(),
                ..row(
                    "passkey revocation, every scope off",
                    RequestPayload::PasskeyRevocation(PasskeyRevocation {
                        credential_id: "c".into(),
                        assertion: assertion(),
                    }),
                    Kind::Revoke,
                )
            },
        ];

        let failures: Vec<String> = rows
            .iter()
            .filter_map(|r| {
                let got = run(r);
                (got != r.expected)
                    .then(|| format!("{}: expected {:?}, got {:?}", r.name, r.expected, got))
            })
            .collect();
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    /// A start whose 24-hour total the caller never loaded is refused, not
    /// read as zero.
    #[test]
    fn a_start_without_a_committed_total_is_refused() {
        assert_eq!(
            decide(&start(Some(1)), Consumption::Fresh, None, &open()),
            Verdict::Refuse(RefusalReason::OverRollingCeiling)
        );
    }

    fn sized(sizes: &[u64]) -> Vec<AttachmentRef> {
        sizes
            .iter()
            .map(|&size| AttachmentRef {
                id: "0b5e4c1a-2f7d-4e8b-9a3c-6d1f0e2b7a94".into(),
                size,
                sha256: String::new(),
            })
            .collect()
    }

    #[test]
    fn declared_attachments_past_a_limit_are_refused_and_those_at_it_are_not() {
        const DESKTOP: AttachmentLimits = AttachmentLimits {
            max_count: MAX_ATTACHMENTS_PER_FEATURE,
            max_bytes: MAX_ATTACHMENT_BYTES,
            max_total_bytes: MAX_AGENT_ATTACHMENT_BATCH_BYTES,
        };
        // Only a sum can overflow here: the desktop's own limits keep ten
        // sizes far below `u64::MAX`.
        const UNBOUNDED: AttachmentLimits = AttachmentLimits {
            max_count: MAX_ATTACHMENTS_PER_FEATURE,
            max_bytes: u64::MAX,
            max_total_bytes: u64::MAX,
        };
        let mismatch = Some(RefusalReason::AttachmentMismatch);
        let rest = MAX_AGENT_ATTACHMENT_BATCH_BYTES - 2 * MAX_ATTACHMENT_BYTES;
        let rows: Vec<(
            &str,
            Vec<AttachmentRef>,
            AttachmentLimits,
            Option<RefusalReason>,
        )> = vec![
            ("none", Vec::new(), DESKTOP, None),
            (
                "count at the cap",
                sized(&[1; MAX_ATTACHMENTS_PER_FEATURE]),
                DESKTOP,
                None,
            ),
            (
                "count over the cap",
                sized(&[1; MAX_ATTACHMENTS_PER_FEATURE + 1]),
                DESKTOP,
                mismatch,
            ),
            ("size zero", sized(&[1, 0]), DESKTOP, mismatch),
            (
                "size at the cap",
                sized(&[MAX_ATTACHMENT_BYTES]),
                DESKTOP,
                None,
            ),
            (
                "size over the cap",
                sized(&[1, MAX_ATTACHMENT_BYTES + 1]),
                DESKTOP,
                mismatch,
            ),
            (
                "total at the cap",
                sized(&[MAX_ATTACHMENT_BYTES, MAX_ATTACHMENT_BYTES, rest]),
                DESKTOP,
                None,
            ),
            (
                "total over the cap",
                sized(&[MAX_ATTACHMENT_BYTES, MAX_ATTACHMENT_BYTES, rest + 1]),
                DESKTOP,
                mismatch,
            ),
            ("one at u64::MAX", sized(&[u64::MAX]), UNBOUNDED, None),
            (
                "a sum that overflows",
                sized(&[u64::MAX, 1]),
                UNBOUNDED,
                mismatch,
            ),
        ];
        let failures: Vec<String> = rows
            .iter()
            .filter_map(|(name, refs, limits, expected)| {
                let got = attachment_refusal(refs, *limits);
                (got != *expected).then(|| format!("{name}: expected {expected:?}, got {got:?}"))
            })
            .collect();
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    fn read_has_no_toggle_and_toggles_never_widen_the_pinned_set() {
        let all_on = policy(true, true, None, None);
        assert_eq!(
            effective_scopes(&[Scope::Read], &HubLocalPolicy::default()),
            [Scope::Read]
        );
        assert_eq!(effective_scopes(&[Scope::Read], &all_on), [Scope::Read]);
        assert_eq!(
            effective_scopes(ALL, &policy(false, true, None, None)),
            [Scope::Read, Scope::Gates]
        );
    }
}

/// What a launch failure's local detail keeps once the request's own text is
/// withheld from it.
mod launch_failure {
    use demeteo_core::domain::hub_request::log::{launch_failure_reason, MIN_WITHHELD_CHARS};

    const DETAIL: &str = "branch demeteo/features/a-b-c already exists";

    #[test]
    fn a_withheld_string_too_short_to_be_the_request_leaves_the_detail_intact() {
        let short = "a".repeat(MIN_WITHHELD_CHARS - 1);
        for withheld in ["a", "e", short.as_str()] {
            assert_eq!(
                launch_failure_reason(DETAIL, &[withheld]),
                format!("malformed_body: {DETAIL}"),
                "{withheld}"
            );
        }
    }

    #[test]
    fn a_withheld_string_inside_another_does_not_leave_a_fragment_of_it() {
        assert_eq!(
            launch_failure_reason(
                "branch demeteo/features/from-the-hub already exists",
                &["from-the", "from-the-hub"],
            ),
            "malformed_body: branch demeteo/features/[withheld] already exists"
        );
    }
}

/// AC2, the signature half: a request the Hub signed verifies under the key
/// the desktop pinned, and one bit changed anywhere does not.
mod signature {
    use demeteo_core::domain::hub_request::{verify_signature, SignatureCheck};
    use demeteo_hub::adapters::memory_keys::MemoryKeys;
    use demeteo_hub::auth::challenge::{base64url_decode, base64url_encode};
    use demeteo_hub::pairing::signing_key::SigningKey;
    use demeteo_hub::ports::KeyService;
    use demeteo_hub::protocol::{RequestPayload, SignedRequest, StartFeature};

    const SIGNING_KEY: &str = "request-signing";

    /// The request and the pinned key, signed as `requests::signed` signs one:
    /// `SigningKey::sign` over `signing_bytes()`, encoded by the Hub's own
    /// base64url.
    async fn signed() -> (SignedRequest, String) {
        let keys = MemoryKeys::with_keys([SIGNING_KEY]);
        let signing_key = SigningKey::new(SIGNING_KEY);
        let mut request = SignedRequest {
            v: 1,
            request_id: "Zk3m0R8pQnVx2tL5aYw9bA".into(),
            instance_id: "i-7f3a9c".into(),
            issued_at: 1_791_019_800,
            expires_at: 1_791_020_400,
            payload: RequestPayload::StartFeature(StartFeature {
                project_id: "p-1".into(),
                workflow_id: "w-1".into(),
                title: "t".into(),
                description: "d".into(),
                agent_kind: None,
                model: None,
                effort: None,
                step_overrides: Vec::new(),
                max_budget_cents: Some(500),
                max_wall_clock_secs: None,
                attachments: Vec::new(),
            }),
            signature: String::new(),
        };
        let message = request.signing_bytes().expect("encode the request");
        let signature = signing_key
            .sign(&keys, &message)
            .await
            .expect("sign the request");
        request.signature = base64url_encode(&signature);
        let public_key = keys
            .verifying_key(SIGNING_KEY)
            .await
            .expect("read the public key");
        (request, base64url_encode(&public_key))
    }

    fn flipped(b64url: &str) -> String {
        let mut bytes = base64url_decode(b64url).expect("decode");
        bytes[0] ^= 1;
        base64url_encode(&bytes)
    }

    #[tokio::test]
    async fn a_request_the_hub_signed_verifies_under_the_pinned_key() {
        let (request, key) = signed().await;

        assert_eq!(verify_signature(&request, &key), SignatureCheck::Valid);
    }

    #[tokio::test]
    async fn a_flipped_signature_bit_is_invalid() {
        let (mut request, key) = signed().await;
        request.signature = flipped(&request.signature);

        assert_eq!(verify_signature(&request, &key), SignatureCheck::Invalid);
    }

    #[tokio::test]
    async fn a_flipped_payload_bit_is_invalid() {
        let (mut request, key) = signed().await;
        let RequestPayload::StartFeature(start) = &mut request.payload else {
            unreachable!("signed() builds a StartFeature");
        };
        start.max_budget_cents = Some(500 ^ 1);

        assert_eq!(verify_signature(&request, &key), SignatureCheck::Invalid);
    }

    #[tokio::test]
    async fn a_flipped_key_bit_is_invalid() {
        let (request, key) = signed().await;

        assert_eq!(
            verify_signature(&request, &flipped(&key)),
            SignatureCheck::Invalid
        );
    }

    #[tokio::test]
    async fn a_key_or_signature_that_is_not_base64url_is_invalid() {
        let (request, key) = signed().await;
        let mut garbled = request.clone();
        garbled.signature = "not base64!".into();

        assert_eq!(
            verify_signature(&request, "not base64!"),
            SignatureCheck::Invalid
        );
        assert_eq!(verify_signature(&garbled, &key), SignatureCheck::Invalid);
        assert_eq!(verify_signature(&request, ""), SignatureCheck::Invalid);
    }
}

/// AC2, AC5, AC6: the consumed ids, the activity log and the local policy,
/// over a real database file that is reopened where persistence is the point.
mod repository {
    use demeteo_core::adapters::database::SqliteAdapter;
    use demeteo_core::domain::hub_pairing::{HubPins, PinnedPasskey};
    use demeteo_core::domain::hub_request::passkeys::{TrustWrite, TrustedPasskey};
    use demeteo_core::domain::hub_request::{
        Consumption, HubLocalPolicy, HubRequestLogEntry, HubRequestLogRow, LogKind, LogOutcome,
    };
    use demeteo_core::ports::hub::{
        HubPasskeyTrust, HubPinRepository, HubPolicyReader, HubPolicyWriter, HubRequestRepository,
    };
    use demeteo_hub::protocol::Scope;

    const NOW: u64 = 1_800_000_000;

    fn open(dir: &std::path::Path) -> SqliteAdapter {
        let conn = demeteo_core::db::init_db(dir.to_path_buf()).expect("init temp db");
        SqliteAdapter::new(conn).expect("build sqlite adapter")
    }

    fn scratch(tag: &str) -> std::path::PathBuf {
        demeteo_core::test_dir::scratch(&format!("demeteo-hub-requests-{tag}"))
    }

    #[test]
    fn an_id_is_fresh_once_and_already_consumed_after() {
        let dir = scratch("consume");
        let store = open(&dir);

        assert_eq!(
            store.try_consume("r-1", NOW + 60, NOW).expect("consume"),
            Consumption::Fresh
        );
        assert_eq!(
            store.try_consume("r-1", NOW + 60, NOW).expect("consume"),
            Consumption::AlreadyConsumed
        );
        assert_eq!(
            store.try_consume("r-2", NOW + 60, NOW).expect("consume"),
            Consumption::Fresh
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_consumed_id_survives_reopening_the_database() {
        let dir = scratch("consume-restart");
        let first = open(&dir);
        assert_eq!(
            first.try_consume("r-1", NOW + 60, NOW).expect("consume"),
            Consumption::Fresh
        );
        drop(first);

        let reopened = open(&dir);
        assert_eq!(
            reopened
                .try_consume("r-1", NOW + 60, NOW + 1)
                .expect("consume"),
            Consumption::AlreadyConsumed
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The Hub may run behind this clock, so an id it would still deliver is
    /// kept until it is past the skew margin, not merely past `expires_at`.
    #[test]
    fn pruning_keeps_ids_inside_the_skew_margin() {
        let dir = scratch("prune");
        let store = open(&dir);
        for (id, expires_at) in [
            ("live", NOW + 60),
            ("just-expired", NOW - 1),
            ("inside-margin", NOW - 600),
            ("long-gone", NOW - 601),
        ] {
            store
                .try_consume(id, expires_at, NOW - 900)
                .expect("consume");
        }

        store.prune_consumed(NOW).expect("prune");

        for kept in ["live", "just-expired", "inside-margin"] {
            assert_eq!(
                store.try_consume(kept, NOW + 60, NOW).expect("consume"),
                Consumption::AlreadyConsumed,
                "{kept} must still be consumed"
            );
        }
        assert_eq!(
            store
                .try_consume("long-gone", NOW + 60, NOW)
                .expect("consume"),
            Consumption::Fresh
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_missing_policy_row_fails_closed_and_the_writer_round_trips() {
        let dir = scratch("policy");
        let store = open(&dir);

        let missing = HubPolicyReader::load(&store).expect("load");
        assert_eq!(missing, HubLocalPolicy::default());
        assert!(!missing.spend_enabled && !missing.gates_enabled);
        assert_eq!(missing.run_ceiling_cents, None);
        assert_eq!(missing.daily_ceiling_cents, None);

        let set = HubLocalPolicy {
            read_enabled: true,
            spend_enabled: true,
            gates_enabled: false,
            run_ceiling_cents: Some(500),
            daily_ceiling_cents: Some(2_000),
        };
        HubPolicyWriter::save(&store, &set).expect("save");
        assert_eq!(HubPolicyReader::load(&store).expect("load"), set);

        let cleared = HubLocalPolicy {
            gates_enabled: true,
            ..HubLocalPolicy::default()
        };
        HubPolicyWriter::save(&store, &cleared).expect("save");
        assert_eq!(HubPolicyReader::load(&open(&dir)).expect("load"), cleared);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_log_lists_newest_first_with_every_field_preserved() {
        let dir = scratch("log");
        let store = open(&dir);
        let entries = [
            HubRequestLogEntry {
                request_id: "r-1".into(),
                kind: LogKind::StartFeature,
                scope: Some(Scope::Spend),
                outcome: LogOutcome::Accepted,
                reason: None,
                feature_id: Some("f-1".into()),
                received_at_ms: 1_000,
            },
            HubRequestLogEntry {
                request_id: "r-2".into(),
                kind: LogKind::CancelFeature,
                scope: Some(Scope::Spend),
                outcome: LogOutcome::Refused,
                reason: Some("unknown_feature".into()),
                feature_id: Some("f-9".into()),
                received_at_ms: 2_000,
            },
            HubRequestLogEntry {
                request_id: "r-3".into(),
                kind: LogKind::GateDecision,
                scope: Some(Scope::Gates),
                outcome: LogOutcome::Deferred,
                reason: None,
                feature_id: None,
                received_at_ms: 3_000,
            },
            HubRequestLogEntry {
                request_id: "r-4".into(),
                kind: LogKind::Unknown,
                scope: None,
                outcome: LogOutcome::Dropped,
                reason: Some("signature".into()),
                feature_id: None,
                received_at_ms: 4_000,
            },
        ];
        for entry in &entries {
            store.append_log(entry).expect("append");
        }

        let entries_of = |rows: Vec<HubRequestLogRow>| -> Vec<HubRequestLogEntry> {
            rows.into_iter().map(|row| row.entry).collect()
        };
        let listed = entries_of(store.list_log(10).expect("list"));
        let newest_first: Vec<_> = entries.iter().rev().cloned().collect();
        assert_eq!(listed, newest_first);
        assert_eq!(
            entries_of(store.list_log(2).expect("list")),
            newest_first[..2]
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A deferred id is logged again when it is later handled, so the
    /// `request_id` cannot key the Settings list; the row's own id does.
    #[test]
    fn two_rows_of_one_request_id_are_both_listed_with_distinct_ids() {
        let dir = scratch("log-repeat");
        let store = open(&dir);
        let deferred = HubRequestLogEntry {
            request_id: "r-1".into(),
            kind: LogKind::GateDecision,
            scope: Some(Scope::Gates),
            outcome: LogOutcome::Deferred,
            reason: None,
            feature_id: None,
            received_at_ms: 1_000,
        };
        let accepted = HubRequestLogEntry {
            outcome: LogOutcome::Accepted,
            received_at_ms: 2_000,
            ..deferred.clone()
        };
        store.append_log(&deferred).expect("append");
        store.append_log(&accepted).expect("append");

        let listed = store.list_log(10).expect("list");
        assert_eq!(listed.len(), 2);
        assert_ne!(listed[0].id, listed[1].id);
        assert_eq!(listed[0].entry, accepted);
        assert_eq!(listed[1].entry, deferred);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Pinned before `B` and sorting after it, so pairing order is not
    /// mistaken for the primary key's.
    const A: &str = "yubikey";
    const B: &str = "laptop";

    fn pins_of(credential_ids: &[&str]) -> HubPins {
        HubPins {
            hub_origin: "https://hub.example".into(),
            instance_id: "i-1".into(),
            request_signing_key: "signing-key".into(),
            rp_id: "hub.example".into(),
            scopes: vec!["gates".into()],
            device_jkt: "jkt".into(),
            paired_at: 0,
            passkeys: credential_ids
                .iter()
                .map(|id| PinnedPasskey {
                    credential_id: (*id).into(),
                    public_key: format!("cose-{id}"),
                })
                .collect(),
        }
    }

    fn counts(store: &SqliteAdapter) -> Vec<(String, u32)> {
        store
            .trusted()
            .expect("trusted")
            .into_iter()
            .map(|passkey| (passkey.credential_id, passkey.sign_count))
            .collect()
    }

    #[test]
    fn trusted_lists_the_pins_in_pairing_order_at_count_zero() {
        let dir = scratch("trusted");
        let store = open(&dir);
        HubPinRepository::save(&store, &pins_of(&[A, B])).expect("save");

        assert_eq!(
            store.trusted().expect("trusted"),
            [A, B].map(|id| TrustedPasskey {
                credential_id: id.into(),
                public_key: format!("cose-{id}"),
                sign_count: 0,
            })
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn advance_count_writes_only_over_the_count_it_expects() {
        let dir = scratch("advance");
        let store = open(&dir);
        HubPinRepository::save(&store, &pins_of(&[A, B])).expect("save");

        assert!(store.advance_count(A, 0, 7).expect("advance"));
        assert_eq!(counts(&store), [(A.into(), 7), (B.into(), 0)]);

        assert!(!store.advance_count(A, 0, 9).expect("stale advance"));
        assert!(!store.advance_count("unknown", 0, 1).expect("unknown"));
        assert_eq!(counts(&open(&dir)), [(A.into(), 7), (B.into(), 0)]);

        // A synced passkey reports 0 every time: writing the count it already
        // holds is still a match, not a missing row.
        assert!(store.advance_count(B, 0, 0).expect("synced advance"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn re_pairing_starts_every_count_over() {
        let dir = scratch("re-pair");
        let store = open(&dir);
        HubPinRepository::save(&store, &pins_of(&[A, B])).expect("save");
        assert!(store.advance_count(A, 0, 7).expect("advance"));

        HubPinRepository::save(&store, &pins_of(&[A, B])).expect("re-save");

        assert_eq!(counts(&store), [(A.into(), 0), (B.into(), 0)]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_stored_count_outside_u32_fails_to_read_rather_than_clamping() {
        for stored in [-1, i64::from(u32::MAX) + 1] {
            let dir = scratch("out-of-range");
            let store = open(&dir);
            HubPinRepository::save(&store, &pins_of(&[A])).expect("save");
            rusqlite::Connection::open(dir.join("demeteo.db"))
                .expect("a second connection")
                .execute("UPDATE hub_pinned_passkeys SET sign_count = ?1", [stored])
                .expect("write a corrupt count");

            assert!(store.trusted().is_err(), "{stored} must not read");
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    const C: &str = "phone";
    const C_KEY: &str = "pQECAyYgASFYIPx-_3kQ0aZ9";

    /// AC3, with `revocation_verdict` bypassed. The racing revocation pins the
    /// signer's compare-and-set: its signer is already gone, so it stops
    /// before the delete. The self-revocation on the shrunk set pins the
    /// store's own last-key guard. It is the only write that can reach that
    /// guard, since a distinct signer and target are two rows, and the domain
    /// refuses it as `self_revocation`. The reopened store still at count 1
    /// shows the signer's counter write rolled back with the refused delete.
    #[test]
    fn revoke_never_deletes_the_last_key_even_unchecked() {
        let dir = scratch("revoke-guard");
        let store = open(&dir);
        HubPinRepository::save(&store, &pins_of(&[A, B])).expect("save");

        assert_eq!(
            store.revoke(A, 0, 1, B).expect("revoke"),
            TrustWrite::Applied
        );
        assert_eq!(counts(&store), [(A.into(), 1)]);

        assert_eq!(
            store.revoke(B, 0, 1, A).expect("racing revoke"),
            TrustWrite::SignerChanged
        );
        assert_eq!(
            store.revoke(A, 1, 2, A).expect("self revoke"),
            TrustWrite::LastCredential
        );
        assert_eq!(counts(&open(&dir)), [(A.into(), 1)]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn revoking_the_only_key_is_last_credential_and_writes_nothing() {
        let dir = scratch("revoke-last");
        let store = open(&dir);
        HubPinRepository::save(&store, &pins_of(&[A])).expect("save");

        assert_eq!(
            store.revoke(A, 0, 1, A).expect("revoke"),
            TrustWrite::LastCredential
        );
        assert_eq!(counts(&store), [(A.into(), 0)]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn revoking_an_unknown_target_is_target_gone_and_keeps_the_count() {
        let dir = scratch("revoke-gone");
        let store = open(&dir);
        HubPinRepository::save(&store, &pins_of(&[A, B])).expect("save");

        assert_eq!(
            store.revoke(A, 0, 1, "unknown").expect("revoke"),
            TrustWrite::TargetGone
        );
        assert_eq!(counts(&store), [(A.into(), 0), (B.into(), 0)]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn endorse_appends_the_key_exactly_as_given() {
        let dir = scratch("endorse");
        let store = open(&dir);
        HubPinRepository::save(&store, &pins_of(&[A])).expect("save");

        assert_eq!(
            store.endorse(A, 0, 4, C, C_KEY, 1).expect("endorse"),
            TrustWrite::Applied
        );
        assert_eq!(
            open(&dir).trusted().expect("trusted"),
            [
                TrustedPasskey {
                    credential_id: A.into(),
                    public_key: format!("cose-{A}"),
                    sign_count: 4,
                },
                TrustedPasskey {
                    credential_id: C.into(),
                    public_key: C_KEY.into(),
                    sign_count: 0,
                },
            ]
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn endorsing_a_pinned_id_is_already_pinned_and_replaces_nothing() {
        let dir = scratch("endorse-pinned");
        let store = open(&dir);
        HubPinRepository::save(&store, &pins_of(&[A])).expect("save");
        let before = store.trusted().expect("trusted");

        assert_eq!(
            store.endorse(A, 0, 4, A, C_KEY, 1).expect("endorse"),
            TrustWrite::AlreadyPinned
        );
        assert_eq!(store.trusted().expect("trusted"), before);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn endorsing_over_a_stale_count_is_signer_changed_and_adds_no_row() {
        let dir = scratch("endorse-stale");
        let store = open(&dir);
        HubPinRepository::save(&store, &pins_of(&[A])).expect("save");
        assert!(store.advance_count(A, 0, 3).expect("advance"));

        assert_eq!(
            store.endorse(A, 0, 4, C, C_KEY, 1).expect("endorse"),
            TrustWrite::SignerChanged
        );
        assert_eq!(counts(&store), [(A.into(), 3)]);
        let _ = std::fs::remove_dir_all(dir);
    }
}

/// The strict presenter AC4 and AC5 swap into `ctx.presenter`: it accepts the
/// one call it was scripted with, once, and refuses the rest.
mod gate_presenter_double {
    use demeteo_core::adapters::hub::testing::ScriptedGatePresenter;
    use demeteo_core::error::AppError;
    use demeteo_core::ports::step_executor::GatePresenter;

    const STEP: &str = "se-gate";

    fn scripted() -> ScriptedGatePresenter {
        ScriptedGatePresenter::expecting(STEP, "approve", Some("ship it"))
    }

    fn the_scripted_call() -> (String, String, Option<String>) {
        (STEP.into(), "approve".into(), Some("ship it".into()))
    }

    #[tokio::test]
    async fn the_scripted_call_is_accepted_and_recorded() {
        let presenter = scripted();

        presenter
            .gate_decide(STEP, "approve", Some("ship it"))
            .await
            .expect("the scripted call is accepted");

        assert_eq!(presenter.calls(), [the_scripted_call()]);
        assert_eq!(presenter.unexpected(), 0);
    }

    #[tokio::test]
    async fn a_repeat_of_the_scripted_call_is_refused() {
        let presenter = scripted();
        presenter
            .gate_decide(STEP, "approve", Some("ship it"))
            .await
            .expect("the scripted call is accepted");

        let repeat = presenter
            .gate_decide(STEP, "approve", Some("ship it"))
            .await;

        assert!(
            matches!(repeat, Err(AppError::Validation { .. })),
            "{repeat:?}"
        );
        assert_eq!(presenter.unexpected(), 1);
        assert_eq!(presenter.calls(), [the_scripted_call()]);
    }

    #[tokio::test]
    async fn a_call_with_other_arguments_is_refused() {
        for (step, decision, feedback) in [
            (STEP, "cancel", Some("ship it")),
            (STEP, "approve", None),
            (STEP, "approve", Some("ship it!")),
            ("se-other", "approve", Some("ship it")),
        ] {
            let presenter = scripted();

            let refused = presenter.gate_decide(step, decision, feedback).await;

            assert!(
                matches!(refused, Err(AppError::Validation { .. })),
                "{step} {decision} {feedback:?}: {refused:?}"
            );
            assert_eq!(presenter.unexpected(), 1, "{step} {decision} {feedback:?}");
            assert!(
                presenter.calls().is_empty(),
                "{step} {decision} {feedback:?}"
            );
        }
    }

    #[tokio::test]
    async fn gate_pending_for_run_is_refused() {
        let presenter = scripted();

        let pending = presenter.gate_pending_for_run("f-1").await;

        assert!(pending.is_err(), "{pending:?}");
        assert_eq!(presenter.unexpected(), 1);
        assert!(presenter.calls().is_empty());
    }

    #[tokio::test]
    async fn refusing_all_refuses_every_gate_decision() {
        let presenter = ScriptedGatePresenter::refusing_all();

        let refused = presenter
            .gate_decide(STEP, "approve", Some("ship it"))
            .await;

        assert!(
            matches!(refused, Err(AppError::Validation { .. })),
            "{refused:?}"
        );
        assert_eq!(presenter.unexpected(), 1);
        assert!(presenter.calls().is_empty());
    }
}

/// AC2, AC3: the handler over a real context, judged by what reached an
/// executor that refuses everything but the launches it records.
mod handler {
    use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use demeteo_core::adapters::database::SqliteAdapter;
    use demeteo_core::adapters::hub::testing::{ScriptedAttachments, ScriptedGatePresenter};
    use demeteo_core::adapters::notification_noop::NoopNotificationAdapter;
    use demeteo_core::application::attachments::{
        MAX_ATTACHMENTS_PER_FEATURE, MAX_ATTACHMENT_BYTES,
    };
    use demeteo_core::application::hub_requests::{HubRequestPorts, HubRequests};
    use demeteo_core::composition::{
        build_core_context, build_core_context_with_policy_writer, CoreConfig, ExecutionMode,
    };
    use demeteo_core::domain::feature_origin::FeatureOrigin;
    use demeteo_core::domain::hub_pairing::{HubPins, PinnedPasskey};
    use demeteo_core::domain::hub_request::{
        HubLocalPolicy, LogKind, LogOutcome, MAX_REQUEST_ID_LEN,
    };
    use demeteo_core::domain::ids::{
        FeatureId, GateDecisionId, ProjectId, StepExecutionId, StepId, WorkflowId,
    };
    use demeteo_core::domain::models::{
        EffortLevel, Feature, FeatureDivergence, FeatureDrift, GateDecision, LaunchedBy, Project,
        StepExecution, StepOverride, Workflow,
    };
    use demeteo_core::domain::step_assignment::StepAssignment;
    use demeteo_core::domain::sync_resolver::SyncResolverChoice;
    use demeteo_core::domain::upstream_feature::DivergenceReconcile;
    use demeteo_core::error::AppError;
    use demeteo_core::ports::hub::{
        AttachmentFetchError, HubAttachmentFetcher, HubPasskeyTrust, HubPinRepository,
        HubRequestHandler,
    };
    use demeteo_core::ports::step_executor::{
        FeatureLaunch, StepExecutor, SyncOutcomeView, SyncResolverView,
    };
    use demeteo_core::ports::sync_session::SyncSessionView;
    use demeteo_core::state::AppContext;
    use demeteo_hub::adapters::memory_keys::MemoryKeys;
    use demeteo_hub::auth::challenge::{base64url_decode, base64url_encode};
    use demeteo_hub::pairing::signing_key::SigningKey;
    use demeteo_hub::ports::KeyService;
    use demeteo_hub::protocol::{
        AttachmentRef, CancelFeature, Effort, RefusalReason, RequestOutcome, RequestPayload,
        RequestResult, SetStepAssignment, SignedRequest, StartFeature,
        StepOverride as WireStepOverride,
    };
    use sha2::{Digest, Sha256};

    const SIGNING_KEY: &str = "request-signing";
    const INSTANCE: &str = "i-7f3a9c";
    const NOW: u64 = 1_800_000_000;
    const ATTACHMENT_ID: &str = "0b5e4c1a-2f7d-4e8b-9a3c-6d1f0e2b7a94";

    /// Records each launch, cancel and assignment it is given; every other
    /// method panics, so no refusal can pass by reaching some other effect.
    /// A `start_error` makes `feature_start` fail with it after recording.
    #[derive(Default)]
    struct ExecutorSpy {
        start_error: Mutex<Option<String>>,
        launches: Mutex<Vec<FeatureLaunch>>,
        cancels: Mutex<Vec<String>>,
        assignments: Mutex<Vec<(String, StepAssignment)>>,
    }

    impl ExecutorSpy {
        fn launches(&self) -> Vec<FeatureLaunch> {
            self.launches.lock().expect("lock").clone()
        }

        fn cancels(&self) -> Vec<String> {
            self.cancels.lock().expect("lock").clone()
        }

        fn assignments(&self) -> Vec<(String, StepAssignment)> {
            self.assignments.lock().expect("lock").clone()
        }

        fn effects(&self) -> usize {
            self.launches().len() + self.cancels().len() + self.assignments().len()
        }
    }

    fn feature(id: &str, project_id: &str) -> Feature {
        Feature {
            id: FeatureId::from(id.to_string()),
            project_id: ProjectId::from(project_id.to_string()),
            workflow_id: None,
            workflow_version_id: None,
            title: id.into(),
            description: String::new(),
            status: "running".to_string(),
            total_cost: 0.0,
            duration: String::new(),
            tokens: 0,
            created_at: 0,
            agent_kind: None,
            model: None,
            effort: None,
            mr_url: None,
            mr_state: Some("none".into()),
            pr_title: None,
            pr_body: None,
            commit_artifacts: None,
            loop_iterations: None,
            max_budget_usd: None,
            step_overrides: Vec::new(),
            attachments: Vec::new(),
            harness_baseline: None,
            origin: FeatureOrigin::DefaultBranch,
            diff_base_branch: None,
            resolved_branch: None,
            launched_by: LaunchedBy::Desktop,
        }
    }

    fn step(id: &str, step_id: &str, step_kind: &str, status: &str, at: i64) -> StepExecution {
        StepExecution {
            id: StepExecutionId::from(id.to_string()),
            feature_id: FeatureId::from("f-1".to_string()),
            step_id: StepId::from(step_id.to_string()),
            step_index: 0,
            step_kind: step_kind.into(),
            status: status.into(),
            cost_usd: None,
            tokens: None,
            wall_clock_secs: None,
            artifact_path: None,
            artifact_paths: Vec::new(),
            error_message: None,
            iteration_count: 0,
            cache_read_input_tokens: None,
            cache_creation_input_tokens: None,
            last_failure_fingerprint: None,
            created_at: at,
            updated_at: at,
        }
    }

    fn unexpected() -> ! {
        panic!("unexpected StepExecutor call")
    }

    #[async_trait]
    impl StepExecutor for ExecutorSpy {
        async fn feature_start(&self, launch: FeatureLaunch) -> Result<Feature, String> {
            let feature = Feature {
                launched_by: launch.launched_by,
                ..feature("f-hub", &launch.project_id)
            };
            self.launches.lock().expect("lock").push(launch);
            match self.start_error.lock().expect("lock").clone() {
                Some(error) => Err(error),
                None => Ok(feature),
            }
        }
        async fn feature_pause(&self, _: &str) -> Result<(), String> {
            unexpected()
        }
        async fn feature_resume(&self, _: &str) -> Result<(), String> {
            unexpected()
        }
        async fn feature_cancel(&self, feature_id: &str) -> Result<(), String> {
            self.cancels.lock().expect("lock").push(feature_id.into());
            Ok(())
        }
        async fn step_get(&self, _: &str) -> Result<StepExecution, String> {
            unexpected()
        }
        async fn step_retry(
            &self,
            _: &str,
            _: Option<&str>,
            _: Option<&str>,
            _: Option<EffortLevel>,
        ) -> Result<(), AppError> {
            unexpected()
        }
        async fn step_set_assignment(
            &self,
            execution_id: &str,
            assignment: StepAssignment,
        ) -> Result<(), AppError> {
            self.assignments
                .lock()
                .expect("lock")
                .push((execution_id.into(), assignment));
            Ok(())
        }
        async fn replay_from_step(
            &self,
            _: &str,
            _: Option<&str>,
            _: Option<&str>,
            _: Option<EffortLevel>,
        ) -> Result<(), String> {
            unexpected()
        }
        async fn step_list_for_run(&self, _: &str) -> Result<Vec<StepExecution>, String> {
            unexpected()
        }
        async fn feature_sync(&self, _: &str) -> Result<SyncOutcomeView, String> {
            unexpected()
        }
        async fn feature_drift(&self, _: &str, _: bool) -> Result<FeatureDrift, String> {
            unexpected()
        }
        async fn feature_reconcile(
            &self,
            _: &str,
            _: DivergenceReconcile,
        ) -> Result<Option<SyncSessionView>, String> {
            unexpected()
        }
        async fn feature_divergence(&self, _: &str) -> Result<Option<FeatureDivergence>, String> {
            unexpected()
        }
        async fn feature_resolve_sync_conflicts(
            &self,
            _: &str,
            _: &SyncResolverChoice,
        ) -> Result<SyncOutcomeView, String> {
            unexpected()
        }
        async fn feature_continue_sync(&self, _: &str) -> Result<SyncOutcomeView, String> {
            unexpected()
        }
        async fn feature_sync_resolver(&self, _: &str) -> Result<SyncResolverView, String> {
            unexpected()
        }
    }

    /// The Hub's signing key, whose public half the desktop pinned.
    struct Hub {
        keys: MemoryKeys,
    }

    impl Hub {
        fn new() -> Self {
            Self {
                keys: MemoryKeys::with_keys([SIGNING_KEY]),
            }
        }

        async fn public_key(&self) -> String {
            let key = self
                .keys
                .verifying_key(SIGNING_KEY)
                .await
                .expect("read the public key");
            base64url_encode(&key)
        }

        /// Signed as `requests::signed` signs one, and serialized as the
        /// socket carries it.
        async fn signed(&self, mut request: SignedRequest) -> String {
            let message = request.signing_bytes().expect("encode the request");
            let signature = SigningKey::new(SIGNING_KEY)
                .sign(&self.keys, &message)
                .await
                .expect("sign the request");
            request.signature = base64url_encode(&signature);
            serde_json::to_string(&request).expect("serialize the request")
        }
    }

    fn sha256_hex(bytes: &[u8]) -> String {
        format!("{:x}", Sha256::digest(bytes))
    }

    fn start(request_id: &str, attachments: Vec<AttachmentRef>) -> SignedRequest {
        SignedRequest {
            v: 1,
            request_id: request_id.into(),
            instance_id: INSTANCE.into(),
            issued_at: NOW,
            expires_at: NOW + 600,
            payload: RequestPayload::StartFeature(StartFeature {
                project_id: "p-1".into(),
                workflow_id: "w-1".into(),
                title: "From the Hub".into(),
                description: "Started from a phone".into(),
                agent_kind: Some("claude-code".into()),
                model: Some("opus".into()),
                effort: Some(Effort::High),
                step_overrides: vec![
                    WireStepOverride {
                        step_id: "s-plan".into(),
                        agent_kind: Some("codex".into()),
                        model: Some("gpt-5".into()),
                        effort: Some(Effort::Xhigh),
                    },
                    WireStepOverride {
                        step_id: "s-build".into(),
                        agent_kind: Some("opencode".into()),
                        model: None,
                        effort: Some(Effort::Low),
                    },
                ],
                max_budget_cents: Some(250),
                max_wall_clock_secs: None,
                attachments,
            }),
            signature: String::new(),
        }
    }

    /// One desktop: a context over `dir` whose executor is the spy, and the
    /// handler built over it. Seeding is separate, so a second `Desktop`
    /// over the same `dir` is this desktop restarted.
    struct Desktop {
        ctx: Arc<AppContext>,
        spy: Arc<ExecutorSpy>,
        attachments: Arc<ScriptedAttachments>,
        clock: Arc<AtomicU64>,
        handler: HubRequests,
    }

    impl Desktop {
        fn open(dir: &std::path::Path) -> Self {
            Self::open_with(dir, |scripted| scripted)
        }

        /// [`Desktop::open`], deciding gates through `presenter`.
        fn presenting(dir: &std::path::Path, presenter: Arc<ScriptedGatePresenter>) -> Self {
            Self::build(dir, |scripted| scripted, Some(presenter), |store| store)
        }

        /// [`Desktop::presenting`], whose handler reaches the trusted set
        /// through what `passkeys` makes of the desktop's own store.
        fn presenting_through(
            dir: &std::path::Path,
            presenter: Arc<ScriptedGatePresenter>,
            passkeys: impl FnOnce(Arc<SqliteAdapter>) -> Arc<dyn HubPasskeyTrust>,
        ) -> Self {
            Self::build(dir, |scripted| scripted, Some(presenter), passkeys)
        }

        /// `fetcher` is handed the scripted attachments and returns what the
        /// handler fetches through.
        fn open_with(
            dir: &std::path::Path,
            fetcher: impl FnOnce(Arc<ScriptedAttachments>) -> Arc<dyn HubAttachmentFetcher>,
        ) -> Self {
            Self::build(dir, fetcher, None, |store| store)
        }

        fn build(
            dir: &std::path::Path,
            fetcher: impl FnOnce(Arc<ScriptedAttachments>) -> Arc<dyn HubAttachmentFetcher>,
            presenter: Option<Arc<ScriptedGatePresenter>>,
            passkeys: impl FnOnce(Arc<SqliteAdapter>) -> Arc<dyn HubPasskeyTrust>,
        ) -> Self {
            let mut ctx = build_core_context(
                CoreConfig {
                    app_data_dir: dir.to_path_buf(),
                    execution_mode: ExecutionMode::LocalOnly,
                },
                Arc::new(NoopNotificationAdapter),
                tokio::runtime::Handle::current(),
            );
            let spy = Arc::new(ExecutorSpy::default());
            ctx.executor = spy.clone();
            if let Some(presenter) = presenter {
                ctx.presenter = presenter;
            }
            let ctx = Arc::new(ctx);
            let pins = demeteo_core::db::init_db(dir.to_path_buf()).expect("init temp db");
            let pins = Arc::new(SqliteAdapter::new(pins).expect("build sqlite adapter"));
            let attachments = Arc::new(ScriptedAttachments::new());
            let clock = Arc::new(AtomicU64::new(NOW * 1000));
            let now = clock.clone();
            let handler = HubRequests::new(
                ctx.clone(),
                HubRequestPorts {
                    requests: ctx.hub_requests.clone(),
                    policy: ctx.hub_policy.clone(),
                    pins: pins.clone(),
                    passkeys: passkeys(pins),
                    attachments: fetcher(attachments.clone()),
                    now_ms: Arc::new(move || now.load(Ordering::SeqCst)),
                },
            );
            Self {
                ctx,
                spy,
                attachments,
                clock,
                handler,
            }
        }

        fn with_fetch_deadline(self, deadline: std::time::Duration) -> Self {
            Self {
                handler: self.handler.with_fetch_deadline(deadline),
                ..self
            }
        }

        async fn handle(&self, text: &str) -> Option<RequestResult> {
            self.handler.handle(text).await
        }

        fn last_log(&self) -> (LogKind, LogOutcome, Option<String>) {
            let entry = self
                .ctx
                .hub_requests
                .list_log(1)
                .expect("list the log")
                .pop()
                .expect("a log row")
                .entry;
            (entry.kind, entry.outcome, entry.reason)
        }

        fn log_len(&self) -> usize {
            self.ctx.hub_requests.list_log(100).expect("list").len()
        }

        /// Feature `f-1`, whose node `s-build` ran twice and `s-review` is
        /// running.
        fn seed_run(&self) {
            self.ctx
                .features
                .add(feature("f-1", "p-1"))
                .expect("add the feature");
            for row in [
                step("se-build-1", "s-build", "agent", "failed", 10),
                step("se-build-2", "s-build", "agent", "failed", 20),
                step("se-review", "s-review", "agent", "running", 30),
            ] {
                self.ctx.features.step_create(row).expect("add the step");
            }
        }

        /// Feature `f-1`, whose gate step `step_execution_id` awaits a
        /// decision.
        fn seed_pending_gate(&self, step_execution_id: &str) {
            self.ctx
                .features
                .add(feature("f-1", "p-1"))
                .expect("add the feature");
            self.ctx
                .features
                .step_create(step(
                    step_execution_id,
                    "s-gate",
                    "gate",
                    "awaiting_gate",
                    40,
                ))
                .expect("add the gate step");
            self.ctx
                .gates
                .create(GateDecision {
                    id: GateDecisionId::new("gd-1"),
                    step_execution_id: StepExecutionId::new(step_execution_id),
                    decision: None,
                    feedback: None,
                    created_at: 40,
                    auto_approved: false,
                })
                .expect("add the undecided gate");
        }
    }

    fn save_policy(dir: &std::path::Path, policy: &HubLocalPolicy) {
        let (_, writer) = build_core_context_with_policy_writer(
            CoreConfig {
                app_data_dir: dir.to_path_buf(),
                execution_mode: ExecutionMode::LocalOnly,
            },
            Arc::new(NoopNotificationAdapter),
            tokio::runtime::Handle::current(),
        );
        writer.save(policy).expect("save the policy");
    }

    /// A paired desktop with spend on, both ceilings well above 250 cents,
    /// and the project and workflow a start names.
    async fn paired(tag: &str, hub: &Hub) -> std::path::PathBuf {
        paired_trusting(tag, hub, Vec::new()).await
    }

    /// [`paired`], with `passkeys` as the trusted set.
    async fn paired_trusting(
        tag: &str,
        hub: &Hub,
        passkeys: Vec<PinnedPasskey>,
    ) -> std::path::PathBuf {
        let dir = demeteo_core::test_dir::scratch(&format!("demeteo-hub-handler-{tag}"));
        save_policy(
            &dir,
            &HubLocalPolicy {
                read_enabled: true,
                spend_enabled: true,
                gates_enabled: false,
                run_ceiling_cents: Some(1_000),
                daily_ceiling_cents: Some(10_000),
            },
        );
        let ctx = build_core_context(
            CoreConfig {
                app_data_dir: dir.clone(),
                execution_mode: ExecutionMode::LocalOnly,
            },
            Arc::new(NoopNotificationAdapter),
            tokio::runtime::Handle::current(),
        );
        let pins = SqliteAdapter::new(demeteo_core::db::init_db(dir.clone()).expect("init db"))
            .expect("build sqlite adapter");
        HubPinRepository::save(
            &pins,
            &HubPins {
                hub_origin: "https://hub.example".into(),
                instance_id: INSTANCE.into(),
                request_signing_key: hub.public_key().await,
                rp_id: "hub.example".into(),
                scopes: vec!["read".into(), "spend".into(), "gates".into()],
                device_jkt: "jkt".into(),
                paired_at: 0,
                passkeys,
            },
        )
        .expect("save the pins");
        ctx.projects
            .add(Project {
                id: ProjectId::new("p-1"),
                name: "demeteo".into(),
                compute_type: "local".into(),
                remote_host: None,
                status: "idle".into(),
                nodes: 0,
                spend: 0.0,
                tokens: 0,
                created_at: 0,
            })
            .expect("add the project");
        ctx.workflows
            .create(Workflow {
                id: WorkflowId::from("w-1".to_string()),
                name: "W".into(),
                description: String::new(),
                is_starter: false,
                created_at: 0,
                updated_at: 0,
                schedule: None,
            })
            .expect("add the workflow");
        dir
    }

    fn refused(reason: RefusalReason) -> RequestOutcome {
        RequestOutcome::Refused { reason }
    }

    fn request(request_id: &str, payload: RequestPayload) -> SignedRequest {
        SignedRequest {
            v: 1,
            request_id: request_id.into(),
            instance_id: INSTANCE.into(),
            issued_at: NOW,
            expires_at: NOW + 600,
            payload,
            signature: String::new(),
        }
    }

    fn cancel(request_id: &str, feature_id: &str) -> SignedRequest {
        request(
            request_id,
            RequestPayload::CancelFeature(CancelFeature {
                feature_id: feature_id.into(),
            }),
        )
    }

    fn assign(request_id: &str, feature_id: &str, step_id: &str) -> SignedRequest {
        assign_to(request_id, feature_id, step_id, "opencode")
    }

    fn assign_to(
        request_id: &str,
        feature_id: &str,
        step_id: &str,
        agent_kind: &str,
    ) -> SignedRequest {
        request(
            request_id,
            RequestPayload::SetStepAssignment(SetStepAssignment {
                feature_id: feature_id.into(),
                step_id: step_id.into(),
                agent_kind: Some(agent_kind.into()),
                model: Some("sonnet".into()),
                effort: Some(Effort::Xhigh),
            }),
        )
    }

    #[tokio::test]
    async fn an_accepted_start_launches_locally_with_every_requested_field() {
        let hub = Hub::new();
        let dir = paired("accepted", &hub).await;
        let desktop = Desktop::open(&dir);
        let bytes = b"attached from the phone".to_vec();
        desktop.attachments.stage(ATTACHMENT_ID, &bytes);
        let attachment = AttachmentRef {
            id: ATTACHMENT_ID.into(),
            size: bytes.len() as u64,
            sha256: sha256_hex(&bytes),
        };
        let text = hub.signed(start("r-accepted", vec![attachment])).await;

        let result = desktop.handle(&text).await.expect("an answer");

        assert_eq!(result.request_id, "r-accepted");
        assert_eq!(result.outcome, RequestOutcome::Accepted);
        let launches = desktop.spy.launches();
        assert_eq!(launches.len(), 1);
        let launch = &launches[0];
        assert_eq!(
            launch.step_overrides,
            vec![
                StepOverride {
                    step_id: "s-plan".into(),
                    agent_kind: Some("codex".into()),
                    model: Some("gpt-5".into()),
                    effort: Some(EffortLevel::XHigh),
                },
                StepOverride {
                    step_id: "s-build".into(),
                    agent_kind: Some("opencode".into()),
                    model: None,
                    effort: Some(EffortLevel::Low),
                },
            ]
        );
        assert_eq!(launch.agent_kind.as_deref(), Some("claude-code"));
        assert_eq!(launch.model.as_deref(), Some("opus"));
        assert_eq!(launch.effort, Some(EffortLevel::High));
        assert_eq!(launch.max_budget_usd, Some(2.5));
        assert_eq!(launch.launched_by, LaunchedBy::Hub);
        assert_eq!(launch.origin, FeatureOrigin::default());
        assert_eq!(launch.staged_attachments.len(), 1);
        assert_eq!(launch.staged_attachments[0].bytes.as_ref(), Some(&bytes));
        assert_eq!(launch.staged_attachments[0].source_path, "");
        assert_eq!(
            launch.staged_attachments[0].mime.as_deref(),
            Some("text/plain")
        );
        assert_eq!(launch.staged_attachments[0].source_filename, None);
        assert_eq!(desktop.attachments.fetched(), vec![ATTACHMENT_ID]);
        assert_eq!(
            desktop.last_log(),
            (LogKind::StartFeature, LogOutcome::Accepted, None)
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn an_attachment_that_does_not_match_is_refused_without_a_launch() {
        let hub = Hub::new();
        let dir = paired("mismatch", &hub).await;
        let desktop = Desktop::open(&dir);
        let bytes = b"attached from the phone".to_vec();
        desktop.attachments.stage(ATTACHMENT_ID, &bytes);
        let attachment = AttachmentRef {
            id: ATTACHMENT_ID.into(),
            size: bytes.len() as u64,
            sha256: sha256_hex(b"something else"),
        };
        let text = hub.signed(start("r-mismatch", vec![attachment])).await;

        let result = desktop.handle(&text).await.expect("an answer");

        assert_eq!(result.outcome, refused(RefusalReason::AttachmentMismatch));
        assert!(desktop.spy.launches().is_empty());
        assert_eq!(
            desktop.last_log(),
            (
                LogKind::StartFeature,
                LogOutcome::Refused,
                Some("attachment_mismatch".into())
            )
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The spy stops at the launch, so this carries what it was handed the
    /// rest of the way: through the commit the bootstrap makes.
    #[tokio::test]
    async fn a_fetched_png_and_txt_commit_through_the_real_bootstrap_path() {
        let hub = Hub::new();
        let dir = paired("typed", &hub).await;
        let desktop = Desktop::open(&dir);
        let mut png = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        png.extend_from_slice(b"\0\0\0\rIHDR");
        let txt = "notes from the phone — ünïcode".as_bytes().to_vec();
        let mut refs = Vec::new();
        for (id, bytes) in [("a-png", &png), ("a-txt", &txt)] {
            desktop.attachments.stage(id, bytes);
            refs.push(AttachmentRef {
                id: id.into(),
                size: bytes.len() as u64,
                sha256: sha256_hex(bytes),
            });
        }
        let text = hub.signed(start("r-typed", refs)).await;

        let result = desktop.handle(&text).await.expect("an answer");

        assert_eq!(result.outcome, RequestOutcome::Accepted);
        let launch = desktop.spy.launches().remove(0);
        desktop
            .ctx
            .features
            .add(feature("f-hub", "p-1"))
            .expect("add the feature");
        let committed = demeteo_core::application::attachments::commit_staged_attachments(
            &desktop.ctx.features,
            &desktop.ctx.attachment_json,
            &desktop.ctx.attachments,
            "f-hub",
            launch.staged_attachments,
        )
        .expect("the bootstrap commits what the handler staged");
        let mimes: Vec<&str> = committed.iter().map(|a| a.mime.as_str()).collect();
        assert_eq!(mimes, vec!["image/png", "text/plain"]);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// `image/tiff` has no canonical extension, so with no filename it would
    /// be stored and shown to the agent as `<sha256>.bin`.
    #[tokio::test]
    async fn a_fetched_tiff_is_stored_and_looked_up_under_a_tiff_extension() {
        let hub = Hub::new();
        let dir = paired("tiff", &hub).await;
        let desktop = Desktop::open(&dir);
        let tiff = b"II*\0\x08\0\0\0\x01\0\0\x01\x03\0".to_vec();
        desktop.attachments.stage(ATTACHMENT_ID, &tiff);
        let attachment = AttachmentRef {
            id: ATTACHMENT_ID.into(),
            size: tiff.len() as u64,
            sha256: sha256_hex(&tiff),
        };
        let text = hub.signed(start("r-tiff", vec![attachment])).await;

        let result = desktop.handle(&text).await.expect("an answer");

        assert_eq!(result.outcome, RequestOutcome::Accepted);
        let launch = desktop.spy.launches().remove(0);
        let staged = &launch.staged_attachments[0];
        assert_eq!(staged.mime.as_deref(), Some("image/tiff"));
        let resolved = demeteo_core::application::attachments::resolve_attachment_type(
            staged.mime.as_deref(),
            staged.source_filename.as_deref(),
            std::path::Path::new(""),
        );
        assert_eq!(resolved.ext, "tiff");
        desktop
            .ctx
            .features
            .add(feature("f-hub", "p-1"))
            .expect("add the feature");
        let committed = demeteo_core::application::attachments::commit_staged_attachments(
            &desktop.ctx.features,
            &desktop.ctx.attachment_json,
            &desktop.ctx.attachments,
            "f-hub",
            launch.staged_attachments,
        )
        .expect("the bootstrap commits what the handler staged");
        assert_eq!(
            demeteo_core::domain::attachment::resolved_ext(&committed[0]),
            "tiff"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn an_attachment_of_an_unsupported_type_is_refused_without_a_launch() {
        let hub = Hub::new();
        let dir = paired("untyped", &hub).await;
        let desktop = Desktop::open(&dir);
        let bytes = vec![0x00, 0x01, 0xFF, 0x00];
        desktop.attachments.stage(ATTACHMENT_ID, &bytes);
        let attachment = AttachmentRef {
            id: ATTACHMENT_ID.into(),
            size: bytes.len() as u64,
            sha256: sha256_hex(&bytes),
        };
        let text = hub.signed(start("r-untyped", vec![attachment])).await;

        let result = desktop.handle(&text).await.expect("an answer");

        assert_eq!(result.outcome, refused(RefusalReason::AttachmentMismatch));
        assert_eq!(desktop.attachments.fetched(), vec![ATTACHMENT_ID]);
        assert!(desktop.spy.launches().is_empty());
        assert!(desktop
            .ctx
            .features
            .get_all_for_project(&ProjectId::new("p-1"))
            .expect("list the features")
            .is_empty());
        assert_eq!(
            desktop.last_log(),
            (
                LogKind::StartFeature,
                LogOutcome::Refused,
                Some("attachment_mismatch".into())
            )
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The Hub deletes what it hands over and the fetcher buffers up to the
    /// declared size, so neither limit can wait for the bootstrap.
    #[tokio::test]
    async fn an_oversized_or_overcounted_start_is_refused_before_any_fetch() {
        let hub = Hub::new();
        let bytes = b"attached from the phone".to_vec();
        let fits = || AttachmentRef {
            id: ATTACHMENT_ID.into(),
            size: bytes.len() as u64,
            sha256: sha256_hex(&bytes),
        };
        let cases = [
            ("overcounted", vec![fits(); MAX_ATTACHMENTS_PER_FEATURE + 1]),
            (
                "oversized",
                vec![AttachmentRef {
                    size: MAX_ATTACHMENT_BYTES + 1,
                    ..fits()
                }],
            ),
        ];
        for (tag, refs) in cases {
            let dir = paired(tag, &hub).await;
            let desktop = Desktop::open(&dir);
            desktop.attachments.stage(ATTACHMENT_ID, &bytes);
            let text = hub.signed(start(&format!("r-{tag}"), refs)).await;

            let result = desktop.handle(&text).await.expect("an answer");

            assert_eq!(
                result.outcome,
                refused(RefusalReason::AttachmentMismatch),
                "{tag}"
            );
            assert!(desktop.attachments.fetched().is_empty(), "{tag}");
            assert!(desktop.spy.launches().is_empty(), "{tag}");
            assert_eq!(
                desktop.last_log(),
                (
                    LogKind::StartFeature,
                    LogOutcome::Refused,
                    Some("attachment_mismatch".into())
                ),
                "{tag}"
            );
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    /// Holds the first fetch until released; every fetch is then the
    /// scripted one.
    struct GatedAttachments {
        inner: Arc<ScriptedAttachments>,
        entered: AtomicUsize,
        held: AtomicBool,
        arrived: tokio::sync::Notify,
        release: tokio::sync::Notify,
    }

    #[async_trait]
    impl HubAttachmentFetcher for GatedAttachments {
        async fn fetch(&self, attachment: &AttachmentRef) -> Result<Vec<u8>, AttachmentFetchError> {
            self.entered.fetch_add(1, Ordering::SeqCst);
            if !self.held.swap(true, Ordering::SeqCst) {
                self.arrived.notify_one();
                self.release.notified().await;
            }
            self.inner.fetch(attachment).await
        }
    }

    fn gated(dir: &std::path::Path) -> (Desktop, Arc<GatedAttachments>) {
        let gate: std::sync::OnceLock<Arc<GatedAttachments>> = std::sync::OnceLock::new();
        let desktop = Desktop::open_with(dir, |inner| {
            let gated = Arc::new(GatedAttachments {
                inner,
                entered: AtomicUsize::new(0),
                held: AtomicBool::new(false),
                arrived: tokio::sync::Notify::new(),
                release: tokio::sync::Notify::new(),
            });
            let _ = gate.set(gated.clone());
            gated
        });
        let gate = gate.get().expect("the gated fetcher").clone();
        (desktop, gate)
    }

    /// The handler is bound once per process and shared by every socket, so
    /// a re-paired socket cannot judge a second start beside the first.
    #[tokio::test]
    async fn a_second_start_is_not_judged_while_the_first_is_in_flight() {
        let hub = Hub::new();
        let dir = paired("one-at-a-time", &hub).await;
        let (desktop, gate) = gated(&dir);
        let mut texts = Vec::new();
        for (request_id, attachment_id) in [("r-first", ATTACHMENT_ID), ("r-second", "a-second")] {
            let bytes = request_id.as_bytes();
            desktop.attachments.stage(attachment_id, bytes);
            let attachment = AttachmentRef {
                id: attachment_id.into(),
                size: bytes.len() as u64,
                sha256: sha256_hex(bytes),
            };
            texts.push(hub.signed(start(request_id, vec![attachment])).await);
        }

        let watched = async {
            gate.arrived.notified().await;
            for _ in 0..50 {
                tokio::task::yield_now().await;
            }
            let entered = gate.entered.load(Ordering::SeqCst);
            gate.release.notify_one();
            entered
        };
        let (first, second, entered) =
            tokio::time::timeout(std::time::Duration::from_secs(10), async {
                tokio::join!(
                    desktop.handle(&texts[0]),
                    desktop.handle(&texts[1]),
                    watched
                )
            })
            .await
            .expect("both starts complete once the gate is released");

        assert_eq!(
            entered, 1,
            "the second start fetched while the first was held"
        );
        assert_eq!(first.expect("an answer").outcome, RequestOutcome::Accepted);
        assert_eq!(second.expect("an answer").outcome, RequestOutcome::Accepted);
        assert_eq!(desktop.attachments.fetched().len(), 2);
        assert_eq!(desktop.spy.launches().len(), 2);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A cancel is how the Hub's user stops spend, so it must not queue
    /// behind a start's fetches the way a second start does.
    #[tokio::test]
    async fn a_cancel_is_answered_while_a_start_is_held_in_its_fetch() {
        let hub = Hub::new();
        let dir = paired("cancel-beside-start", &hub).await;
        let (desktop, gate) = gated(&dir);
        desktop.seed_run();
        let bytes = b"attached from the phone";
        desktop.attachments.stage(ATTACHMENT_ID, bytes);
        let attachment = AttachmentRef {
            id: ATTACHMENT_ID.into(),
            size: bytes.len() as u64,
            sha256: sha256_hex(bytes),
        };
        let started = hub.signed(start("r-start", vec![attachment])).await;
        let cancelled = hub.signed(cancel("r-cancel", "f-1")).await;

        let beside = async {
            gate.arrived.notified().await;
            let answer = tokio::time::timeout(
                std::time::Duration::from_secs(5),
                desktop.handle(&cancelled),
            )
            .await;
            let launched_meanwhile = desktop.spy.launches().len();
            gate.release.notify_one();
            (answer, launched_meanwhile)
        };
        let (started, (cancelled, launched_meanwhile)) =
            tokio::join!(desktop.handle(&started), beside);

        let cancelled = cancelled
            .expect("the cancel waited behind the held start")
            .expect("an answer");
        assert_eq!(cancelled.outcome, RequestOutcome::Accepted);
        assert_eq!(desktop.spy.cancels(), vec!["f-1".to_string()]);
        assert_eq!(launched_meanwhile, 0, "the start launched before release");
        assert_eq!(
            started.expect("an answer").outcome,
            RequestOutcome::Accepted
        );
        assert_eq!(desktop.spy.launches().len(), 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Each fetch takes `delay`, or never resolves with none.
    struct SlowAttachments {
        inner: Arc<ScriptedAttachments>,
        delay: Option<std::time::Duration>,
    }

    #[async_trait]
    impl HubAttachmentFetcher for SlowAttachments {
        async fn fetch(&self, attachment: &AttachmentRef) -> Result<Vec<u8>, AttachmentFetchError> {
            match self.delay {
                Some(delay) => tokio::time::sleep(delay).await,
                None => std::future::pending::<()>().await,
            }
            self.inner.fetch(attachment).await
        }
    }

    /// The deadline is the start's, not each fetch's: two fetches that each
    /// fit inside it are refused once they overrun it together.
    #[tokio::test]
    async fn a_start_whose_fetches_overrun_the_deadline_is_refused() {
        let deadline = std::time::Duration::from_millis(200);
        let cases = [("never", None, 1), ("together", Some(deadline * 3 / 5), 2)];
        for (tag, delay, count) in cases {
            let hub = Hub::new();
            let dir = paired(&format!("deadline-{tag}"), &hub).await;
            let desktop =
                Desktop::open_with(&dir, |inner| Arc::new(SlowAttachments { inner, delay }))
                    .with_fetch_deadline(deadline);
            let attachments = (0..count)
                .map(|i| {
                    let id = format!("a-{i}");
                    let bytes = id.as_bytes().to_vec();
                    desktop.attachments.stage(&id, &bytes);
                    AttachmentRef {
                        size: bytes.len() as u64,
                        sha256: sha256_hex(&bytes),
                        id,
                    }
                })
                .collect();
            let text = hub.signed(start("r-slow", attachments)).await;

            let result = tokio::time::timeout(deadline * 50, desktop.handle(&text))
                .await
                .expect("the start outlived its deadline")
                .expect("an answer");

            assert_eq!(
                result.outcome,
                refused(RefusalReason::AttachmentMismatch),
                "{tag}"
            );
            assert!(desktop.spy.launches().is_empty(), "{tag}");
            assert_eq!(
                desktop.last_log(),
                (
                    LogKind::StartFeature,
                    LogOutcome::Refused,
                    Some("attachment_mismatch".into())
                ),
                "{tag}"
            );
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    #[tokio::test]
    async fn a_redelivered_request_is_replayed_in_process_and_after_a_restart() {
        let hub = Hub::new();
        let dir = paired("replay", &hub).await;
        let desktop = Desktop::open(&dir);
        let text = hub.signed(start("r-replay", Vec::new())).await;

        let first = desktop.handle(&text).await.expect("an answer");
        let second = desktop.handle(&text).await.expect("an answer");

        assert_eq!(first.outcome, RequestOutcome::Accepted);
        assert_eq!(second.outcome, refused(RefusalReason::Replayed));
        assert_eq!(desktop.spy.launches().len(), 1);
        assert_eq!(desktop.log_len(), 2);

        let restarted = Desktop::open(&dir);
        let third = restarted.handle(&text).await.expect("an answer");

        assert_eq!(third.outcome, refused(RefusalReason::Replayed));
        assert!(restarted.spy.launches().is_empty());
        assert_eq!(
            restarted.last_log(),
            (
                LogKind::StartFeature,
                LogOutcome::Refused,
                Some("replayed".into())
            )
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn a_signature_altered_by_one_byte_is_dropped_unanswered() {
        let hub = Hub::new();
        let dir = paired("bad-signature", &hub).await;
        let desktop = Desktop::open(&dir);
        let text = hub.signed(start("r-forged", Vec::new())).await;
        let mut request: SignedRequest = serde_json::from_str(&text).expect("parse");
        let mut signature = base64url_decode(&request.signature).expect("decode");
        signature[0] ^= 0xff;
        request.signature = base64url_encode(&signature);
        let forged = serde_json::to_string(&request).expect("serialize");

        assert_eq!(desktop.handle(&forged).await, None);
        assert!(desktop.spy.launches().is_empty());
        assert_eq!(
            desktop.last_log(),
            (
                LogKind::StartFeature,
                LogOutcome::Dropped,
                Some("signature".into())
            )
        );

        // The id was not spent by the forgery: the genuine request still runs.
        let genuine = desktop.handle(&text).await.expect("an answer");
        assert_eq!(genuine.outcome, RequestOutcome::Accepted);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn a_request_at_its_expiry_is_refused_expired() {
        let hub = Hub::new();
        let dir = paired("expired", &hub).await;
        let desktop = Desktop::open(&dir);
        let request = start("r-expired", Vec::new());
        desktop
            .clock
            .store(request.expires_at * 1000, Ordering::SeqCst);
        let text = hub.signed(request).await;

        let result = desktop.handle(&text).await.expect("an answer");

        assert_eq!(result.outcome, refused(RefusalReason::Expired));
        assert!(desktop.spy.launches().is_empty());
        assert_eq!(
            desktop.last_log(),
            (
                LogKind::StartFeature,
                LogOutcome::Refused,
                Some("expired".into())
            )
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn a_request_for_another_instance_is_refused_wrong_instance() {
        let hub = Hub::new();
        let dir = paired("wrong-instance", &hub).await;
        let desktop = Desktop::open(&dir);
        let mut request = start("r-elsewhere", Vec::new());
        request.instance_id = "i-someone-else".into();
        let text = hub.signed(request).await;

        let result = desktop.handle(&text).await.expect("an answer");

        assert_eq!(result.outcome, refused(RefusalReason::WrongInstance));
        assert!(desktop.spy.launches().is_empty());
        assert_eq!(
            desktop.last_log(),
            (
                LogKind::StartFeature,
                LogOutcome::Refused,
                Some("wrong_instance".into())
            )
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    fn consumed_ids(dir: &std::path::Path) -> Vec<String> {
        let conn = rusqlite::Connection::open(dir.join("demeteo.db")).expect("open the DB");
        let mut stmt = conn
            .prepare("SELECT request_id FROM hub_consumed_requests ORDER BY request_id")
            .expect("prepare");
        stmt.query_map([], |row| row.get(0))
            .expect("query")
            .collect::<Result<_, _>>()
            .expect("read the ids")
    }

    /// A cancel of `f-1` delivered `secs` after `NOW`, live for 600 s.
    fn cancel_at(request_id: &str, secs: u64) -> SignedRequest {
        SignedRequest {
            issued_at: NOW + secs,
            expires_at: NOW + secs + 600,
            ..cancel(request_id, "f-1")
        }
    }

    #[tokio::test]
    async fn a_consumed_id_past_the_skew_margin_is_pruned_and_never_fresh_again() {
        let hub = Hub::new();
        let dir = paired("prune", &hub).await;
        let desktop = Desktop::open(&dir);
        desktop.seed_run();
        let a = hub.signed(cancel_at("r-a", 0)).await;
        assert_eq!(
            desktop.handle(&a).await.expect("an answer").outcome,
            RequestOutcome::Accepted
        );

        // Past A's `expires_at` plus the 600 s skew margin.
        let later = 600 + 600 + 1;
        desktop.clock.store((NOW + later) * 1000, Ordering::SeqCst);
        let b = hub.signed(cancel_at("r-b", later)).await;
        assert_eq!(
            desktop.handle(&b).await.expect("an answer").outcome,
            RequestOutcome::Accepted
        );
        assert_eq!(consumed_ids(&dir), vec!["r-b".to_string()]);

        let again = desktop.handle(&a).await.expect("an answer");
        assert_eq!(again.outcome, refused(RefusalReason::Expired));
        assert_eq!(desktop.spy.effects(), 2);
        assert_eq!(consumed_ids(&dir), vec!["r-b".to_string()]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn a_prune_that_fails_changes_neither_the_answer_nor_the_log() {
        let hub = Hub::new();
        let dir = paired("prune-fails", &hub).await;
        let desktop = Desktop::open(&dir);
        desktop.seed_run();
        rusqlite::Connection::open(dir.join("demeteo.db"))
            .expect("open the DB")
            .execute_batch(
                "INSERT INTO hub_consumed_requests (request_id, expires_at, consumed_at)
                 VALUES ('r-long-gone', 0, 0);
                 CREATE TRIGGER no_prune BEFORE DELETE ON hub_consumed_requests
                 BEGIN SELECT RAISE(ABORT, 'pruning refused'); END",
            )
            .expect("seed an expired id and refuse its deletion");
        let text = hub.signed(cancel("r-cancel", "f-1")).await;

        let result = desktop.handle(&text).await.expect("an answer");

        assert_eq!(
            consumed_ids(&dir),
            vec!["r-cancel".to_string(), "r-long-gone".to_string()]
        );
        assert_eq!(result.outcome, RequestOutcome::Accepted);
        assert_eq!(desktop.spy.cancels(), vec!["f-1".to_string()]);
        assert_eq!(
            desktop.last_log(),
            (LogKind::CancelFeature, LogOutcome::Accepted, None)
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn an_accepted_cancel_reaches_feature_cancel_with_the_id() {
        let hub = Hub::new();
        let dir = paired("cancel", &hub).await;
        let desktop = Desktop::open(&dir);
        desktop.seed_run();
        let text = hub.signed(cancel("r-cancel", "f-1")).await;

        let result = desktop.handle(&text).await.expect("an answer");

        assert_eq!(result.outcome, RequestOutcome::Accepted);
        assert_eq!(desktop.spy.cancels(), vec!["f-1".to_string()]);
        assert_eq!(desktop.spy.effects(), 1);
        assert_eq!(
            desktop.last_log(),
            (LogKind::CancelFeature, LogOutcome::Accepted, None)
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn an_accepted_assignment_lands_on_the_latest_execution_of_the_step() {
        let hub = Hub::new();
        let dir = paired("assign", &hub).await;
        let desktop = Desktop::open(&dir);
        desktop.seed_run();
        let text = hub.signed(assign("r-assign", "f-1", "s-build")).await;

        let result = desktop.handle(&text).await.expect("an answer");

        assert_eq!(result.outcome, RequestOutcome::Accepted);
        assert_eq!(
            desktop.spy.assignments(),
            vec![(
                "se-build-2".to_string(),
                StepAssignment {
                    agent_kind: Some("opencode".into()),
                    model: Some("sonnet".into()),
                    effort: Some(EffortLevel::XHigh),
                }
            )]
        );
        assert_eq!(desktop.spy.effects(), 1);
        assert_eq!(
            desktop.last_log(),
            (LogKind::SetStepAssignment, LogOutcome::Accepted, None)
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn a_cancel_or_assignment_that_cannot_land_is_refused_without_an_effect() {
        let hub = Hub::new();
        let cases = [
            (
                "unknown-feature-cancel",
                cancel("r-1", "f-missing"),
                LogKind::CancelFeature,
                RefusalReason::UnknownFeature,
                "unknown_feature",
            ),
            (
                "unknown-feature-assign",
                assign("r-2", "f-missing", "s-build"),
                LogKind::SetStepAssignment,
                RefusalReason::UnknownFeature,
                "unknown_feature",
            ),
            (
                "unknown-step",
                assign("r-3", "f-1", "s-missing"),
                LogKind::SetStepAssignment,
                RefusalReason::UnknownStep,
                "unknown_step",
            ),
            (
                "running-step",
                assign("r-4", "f-1", "s-review"),
                LogKind::SetStepAssignment,
                RefusalReason::StepNotReassignable,
                "step_not_reassignable",
            ),
            (
                "unregistered-harness",
                assign_to("r-5", "f-1", "s-build", "not-a-harness"),
                LogKind::SetStepAssignment,
                RefusalReason::HarnessNotRegistered,
                "harness_not_registered",
            ),
        ];
        for (tag, request, kind, reason, logged) in cases {
            let dir = paired(tag, &hub).await;
            let desktop = Desktop::open(&dir);
            desktop.seed_run();
            let text = hub.signed(request).await;

            let result = desktop.handle(&text).await.expect("an answer");

            assert_eq!(result.outcome, refused(reason), "{tag}");
            assert_eq!(desktop.spy.effects(), 0, "{tag}");
            assert_eq!(desktop.log_len(), 1, "{tag}");
            assert_eq!(
                desktop.last_log(),
                (kind, LogOutcome::Refused, Some(logged.into())),
                "{tag}"
            );
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    #[tokio::test]
    async fn with_spend_off_a_cancel_and_an_assignment_are_refused_scope_off() {
        let hub = Hub::new();
        let dir = paired("spend-off", &hub).await;
        save_policy(&dir, &HubLocalPolicy::default());
        let desktop = Desktop::open(&dir);
        desktop.seed_run();

        for request in [
            cancel("r-cancel", "f-1"),
            assign("r-assign", "f-1", "s-build"),
        ] {
            let text = hub.signed(request).await;
            let result = desktop.handle(&text).await.expect("an answer");
            assert_eq!(result.outcome, refused(RefusalReason::ScopeOff));
        }

        assert_eq!(desktop.spy.effects(), 0);
        assert_eq!(desktop.log_len(), 2);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// AC5: every delivery leaves exactly one activity row, and every
    /// verified request that is not deferred gets exactly one answer.
    mod activity_log {
        use super::*;
        use demeteo_core::domain::hub_request::HubRequestLogEntry;
        use demeteo_hub::auth::testing::SoftAuthenticator;
        use demeteo_hub::protocol::{Decision, GateDecision, Scope, WebAuthnAssertion};

        const TITLE: &str = "From the Hub";
        const DESCRIPTION: &str = "Started from a phone";
        const ATTACHED: &[u8] = b"attached from the phone";

        /// How a case's request reaches the handler.
        enum Delivery {
            Signed,
            /// Signed, then one signature byte flipped.
            Forged,
            /// Delivered with this desktop's clock at its `expires_at`.
            AtExpiry,
            /// Delivered once before the delivery under test.
            Redelivered,
        }

        struct Case {
            tag: &'static str,
            policy: Option<HubLocalPolicy>,
            request: SignedRequest,
            delivery: Delivery,
            answer: Option<RequestOutcome>,
            row: (LogKind, Option<Scope>, LogOutcome, Option<&'static str>),
            /// Executor calls on the spy once the delivery under test is done.
            effects: usize,
            /// Attachment fetches once the delivery under test is done.
            fetches: usize,
        }

        fn case(
            tag: &'static str,
            request: SignedRequest,
            answer: Option<RequestOutcome>,
            row: (LogKind, Option<Scope>, LogOutcome, Option<&'static str>),
        ) -> Case {
            let fetches = match &request.payload {
                RequestPayload::StartFeature(start) => start.attachments.len(),
                _ => 0,
            };
            Case {
                tag,
                policy: None,
                fetches,
                request,
                delivery: Delivery::Signed,
                effects: usize::from(answer == Some(RequestOutcome::Accepted)),
                answer,
                row,
            }
        }

        fn with_start(request_id: &str, edit: impl FnOnce(&mut StartFeature)) -> SignedRequest {
            let mut request = super::start(request_id, Vec::new());
            if let RequestPayload::StartFeature(start) = &mut request.payload {
                edit(start);
            }
            request
        }

        /// A start carrying an attachment that would fetch and match, edited
        /// into one refused `reason` before anything is fetched.
        fn refused_unfetched(
            tag: &'static str,
            reason: (RefusalReason, &'static str),
            edit: impl FnOnce(&mut StartFeature),
        ) -> Case {
            let mut request = super::start(&format!("r-{tag}"), vec![attached()]);
            if let RequestPayload::StartFeature(start) = &mut request.payload {
                edit(start);
            }
            let row = (
                LogKind::StartFeature,
                Some(Scope::Spend),
                LogOutcome::Refused,
                Some(reason.1),
            );
            Case {
                fetches: 0,
                ..case(tag, request, Some(refused(reason.0)), row)
            }
        }

        fn assertion() -> WebAuthnAssertion {
            WebAuthnAssertion {
                credential_id: "c".into(),
                authenticator_data: "a".into(),
                client_data_json: "j".into(),
                signature: "s".into(),
            }
        }

        fn gate(request_id: &str) -> SignedRequest {
            request(
                request_id,
                RequestPayload::GateDecision(GateDecision {
                    feature_id: "f-1".into(),
                    step_execution_id: "se-review".into(),
                    decision: Decision::Approve,
                    feedback: None,
                    assertion: assertion(),
                }),
            )
        }

        fn policy(spend: bool, gates: bool, run: u64, daily: u64) -> HubLocalPolicy {
            HubLocalPolicy {
                read_enabled: true,
                spend_enabled: spend,
                gates_enabled: gates,
                run_ceiling_cents: Some(run),
                daily_ceiling_cents: Some(daily),
            }
        }

        fn forged(text: &str) -> String {
            let mut request: SignedRequest = serde_json::from_str(text).expect("parse");
            let mut signature = base64url_decode(&request.signature).expect("decode");
            signature[0] ^= 0xff;
            request.signature = base64url_encode(&signature);
            serde_json::to_string(&request).expect("serialize")
        }

        fn log(desktop: &Desktop) -> Vec<HubRequestLogEntry> {
            desktop
                .ctx
                .hub_requests
                .list_log(100)
                .expect("list the log")
                .into_iter()
                .map(|row| row.entry)
                .collect()
        }

        /// Hands `text` over and returns its answer and the one row it added.
        async fn deliver(
            desktop: &Desktop,
            text: &str,
        ) -> (Option<RequestResult>, HubRequestLogEntry) {
            let before = log(desktop).len();
            let result = desktop.handle(text).await;
            let mut after = log(desktop);
            assert_eq!(after.len(), before + 1, "exactly one new row");
            (result, after.remove(0))
        }

        fn row_of(
            entry: &HubRequestLogEntry,
        ) -> (LogKind, Option<Scope>, LogOutcome, Option<&str>) {
            (
                entry.kind,
                entry.scope,
                entry.outcome,
                entry.reason.as_deref(),
            )
        }

        /// Nothing the request carried lands in its row.
        fn assert_no_content(entry: &HubRequestLogEntry, tag: &str) {
            let row = format!("{entry:?}");
            for content in [TITLE, DESCRIPTION, &String::from_utf8_lossy(ATTACHED)] {
                assert!(!row.contains(content), "{tag}: {row}");
            }
        }

        fn attached() -> AttachmentRef {
            AttachmentRef {
                id: ATTACHMENT_ID.into(),
                size: ATTACHED.len() as u64,
                sha256: sha256_hex(ATTACHED),
            }
        }

        fn mismatched() -> AttachmentRef {
            AttachmentRef {
                id: ATTACHMENT_ID.into(),
                size: ATTACHED.len() as u64,
                sha256: sha256_hex(b"something else"),
            }
        }

        #[tokio::test]
        async fn every_request_leaves_one_row_and_every_answered_one_one_result() {
            let hub = Hub::new();
            let spend = Some(Scope::Spend);
            let start = LogKind::StartFeature;
            let accepted = |kind| (kind, spend, LogOutcome::Accepted, None);
            let refusal = |kind, reason| (kind, spend, LogOutcome::Refused, Some(reason));
            let malformed = (RefusalReason::MalformedBody, "malformed_body");
            let unregistered = (
                RefusalReason::HarnessNotRegistered,
                "harness_not_registered",
            );
            let cases = [
                case(
                    "start",
                    super::start("r-start", Vec::new()),
                    Some(RequestOutcome::Accepted),
                    accepted(start),
                ),
                case(
                    "cancel",
                    cancel("r-cancel", "f-1"),
                    Some(RequestOutcome::Accepted),
                    accepted(LogKind::CancelFeature),
                ),
                case(
                    "assign",
                    assign("r-assign", "f-1", "s-build"),
                    Some(RequestOutcome::Accepted),
                    accepted(LogKind::SetStepAssignment),
                ),
                Case {
                    policy: Some(HubLocalPolicy::default()),
                    ..case(
                        "spend-off",
                        super::start("r-spend-off", Vec::new()),
                        Some(refused(RefusalReason::ScopeOff)),
                        refusal(start, "scope_off"),
                    )
                },
                case(
                    "cap-absent",
                    with_start("r-no-cap", |s| s.max_budget_cents = None),
                    Some(refused(RefusalReason::OverRunCeiling)),
                    refusal(start, "over_run_ceiling"),
                ),
                case(
                    "over-run-ceiling",
                    with_start("r-over-run", |s| s.max_budget_cents = Some(1_001)),
                    Some(refused(RefusalReason::OverRunCeiling)),
                    refusal(start, "over_run_ceiling"),
                ),
                Case {
                    policy: Some(policy(true, false, 1_000, 249)),
                    ..case(
                        "over-daily-ceiling",
                        super::start("r-over-daily", Vec::new()),
                        Some(refused(RefusalReason::OverRollingCeiling)),
                        refusal(start, "over_rolling_ceiling"),
                    )
                },
                Case {
                    delivery: Delivery::Redelivered,
                    effects: 1,
                    ..case(
                        "replayed",
                        super::start("r-replayed", Vec::new()),
                        Some(refused(RefusalReason::Replayed)),
                        refusal(start, "replayed"),
                    )
                },
                Case {
                    delivery: Delivery::AtExpiry,
                    ..case(
                        "expired",
                        super::start("r-expired", Vec::new()),
                        Some(refused(RefusalReason::Expired)),
                        refusal(start, "expired"),
                    )
                },
                case(
                    "wrong-instance",
                    SignedRequest {
                        instance_id: "i-someone-else".into(),
                        ..super::start("r-elsewhere", Vec::new())
                    },
                    Some(refused(RefusalReason::WrongInstance)),
                    refusal(start, "wrong_instance"),
                ),
                Case {
                    delivery: Delivery::Forged,
                    ..case(
                        "bad-signature",
                        super::start("r-forged", Vec::new()),
                        None,
                        (start, spend, LogOutcome::Dropped, Some("signature")),
                    )
                },
                case(
                    "attachment-mismatch",
                    super::start("r-mismatch", vec![mismatched()]),
                    Some(refused(RefusalReason::AttachmentMismatch)),
                    refusal(start, "attachment_mismatch"),
                ),
                case(
                    "unknown-project",
                    with_start("r-no-project", |s| s.project_id = "p-missing".into()),
                    Some(refused(RefusalReason::UnknownProject)),
                    refusal(start, "unknown_project"),
                ),
                case(
                    "unknown-workflow",
                    with_start("r-no-workflow", |s| s.workflow_id = "w-missing".into()),
                    Some(refused(RefusalReason::UnknownWorkflow)),
                    refusal(start, "unknown_workflow"),
                ),
                refused_unfetched("flag-model", malformed, |s| s.model = Some("-x".into())),
                refused_unfetched("spaced-model", malformed, |s| {
                    s.model = Some("opus --yolo".into())
                }),
                refused_unfetched("override-flag-model", malformed, |s| {
                    s.step_overrides[0].model = Some("-x".into())
                }),
                refused_unfetched("unregistered-harness", unregistered, |s| {
                    s.agent_kind = Some("aider".into())
                }),
                refused_unfetched("override-unregistered-harness", unregistered, |s| {
                    s.step_overrides[1].agent_kind = Some("aider".into())
                }),
            ];
            for Case {
                tag,
                policy,
                request,
                delivery,
                answer,
                row,
                effects,
                fetches,
            } in cases
            {
                let dir = paired(&format!("log-{tag}"), &hub).await;
                if let Some(policy) = &policy {
                    save_policy(&dir, policy);
                }
                let desktop = Desktop::open(&dir);
                desktop.seed_run();
                desktop.attachments.stage(ATTACHMENT_ID, ATTACHED);
                let request_id = request.request_id.clone();
                let expires_at = request.expires_at;
                let mut text = hub.signed(request).await;
                match delivery {
                    Delivery::Signed => {}
                    Delivery::Forged => text = forged(&text),
                    Delivery::AtExpiry => desktop.clock.store(expires_at * 1000, Ordering::SeqCst),
                    Delivery::Redelivered => {
                        desktop.handle(&text).await.expect("a first answer");
                        assert_eq!(desktop.spy.launches().len(), 1, "{tag}: first delivery");
                    }
                }

                let (result, entry) = deliver(&desktop, &text).await;

                let expected = answer.map(|outcome| RequestResult {
                    v: 1,
                    request_id: request_id.clone(),
                    outcome,
                });
                assert_eq!(result, expected, "{tag}");
                assert_eq!(row_of(&entry), row, "{tag}");
                assert_eq!(desktop.spy.effects(), effects, "{tag}");
                assert_eq!(desktop.attachments.fetched().len(), fetches, "{tag}");
                assert_eq!(entry.request_id, request_id, "{tag}");
                assert_no_content(&entry, tag);
                let _ = std::fs::remove_dir_all(dir);
            }
        }

        /// A forgery is not answered `scope_off` either: the Hub keeps the
        /// first outcome it records, so that answer would be the forger's.
        #[tokio::test]
        async fn a_forged_start_with_spend_off_is_dropped_unanswered_and_unspent() {
            let hub = Hub::new();
            let dir = paired("log-forged-spend-off", &hub).await;
            save_policy(&dir, &HubLocalPolicy::default());
            let desktop = Desktop::open(&dir);
            let text = hub.signed(super::start("r-forged-off", Vec::new())).await;

            let (result, entry) = deliver(&desktop, &forged(&text)).await;

            assert_eq!(result, None);
            assert_eq!(
                row_of(&entry),
                (
                    LogKind::StartFeature,
                    Some(Scope::Spend),
                    LogOutcome::Dropped,
                    Some("signature")
                )
            );
            assert!(consumed_ids(&dir).is_empty());
            assert_eq!(desktop.spy.effects(), 0);

            let (result, _) = deliver(&desktop, &text).await;
            assert_eq!(
                result.expect("an answer").outcome,
                refused(RefusalReason::ScopeOff)
            );
            let _ = std::fs::remove_dir_all(dir);
        }

        /// What Settings' activity log is handed for a start refused with
        /// `spend` off, spelled as it crosses IPC.
        #[tokio::test]
        async fn a_start_refused_with_spend_off_reads_back_through_the_log_command() {
            let hub = Hub::new();
            let dir = paired("log-command-spend-off", &hub).await;
            save_policy(&dir, &HubLocalPolicy::default());
            let desktop = Desktop::open(&dir);
            let text = hub.signed(super::start("r-log-off", Vec::new())).await;

            desktop.handle(&text).await.expect("an answer");
            let log =
                demeteo_lib::commands::hub::request_log_core(desktop.ctx.hub_requests.as_ref(), 10)
                    .expect("read the log");

            let [entry] = &log[..] else {
                panic!("one row: {log:?}");
            };
            let entry = serde_json::to_value(entry).expect("serialisable");
            assert_eq!(entry["request_id"], "r-log-off");
            assert_eq!(entry["outcome"], "refused");
            assert_eq!(entry["scope"], "spend");
            assert_eq!(entry["reason"], "scope_off");
            let _ = std::fs::remove_dir_all(dir);
        }

        /// The gate awaits a decision, so the placeholder assertion is what
        /// refuses the first delivery; its id is spent all the same.
        #[tokio::test]
        async fn a_gate_decision_with_gates_on_is_answered_then_replayed() {
            let hub = Hub::new();
            let dir = paired("log-gate-on", &hub).await;
            save_policy(&dir, &policy(true, true, 1_000, 10_000));
            let presenter = Arc::new(ScriptedGatePresenter::refusing_all());
            let desktop = Desktop::presenting(&dir, presenter.clone());
            desktop.seed_pending_gate("se-review");
            let text = hub.signed(gate("r-gate")).await;

            for (delivery, reason, logged) in [
                (
                    "first",
                    RefusalReason::AssertionInvalid,
                    "assertion_invalid",
                ),
                ("second", RefusalReason::Replayed, "replayed"),
            ] {
                let (result, entry) = deliver(&desktop, &text).await;
                assert_eq!(
                    result.expect("an answer").outcome,
                    refused(reason),
                    "{delivery}"
                );
                assert_eq!(
                    row_of(&entry),
                    (
                        LogKind::GateDecision,
                        Some(Scope::Gates),
                        LogOutcome::Refused,
                        Some(logged)
                    ),
                    "{delivery}"
                );
            }
            assert!(presenter.calls().is_empty());
            assert_eq!(presenter.unexpected(), 0);
            assert_eq!(desktop.spy.effects(), 0);
            let _ = std::fs::remove_dir_all(dir);
        }

        /// The total is read by a query naming `features.launched_by`;
        /// renaming that column under the open context is the one failure
        /// no earlier step of the handler reads.
        fn rename_launched_by(dir: &std::path::Path, from: &str, to: &str) {
            rusqlite::Connection::open(dir.join("demeteo.db"))
                .expect("open the desktop's DB")
                .execute_batch(&format!(
                    "ALTER TABLE features RENAME COLUMN {from} TO {to}"
                ))
                .expect("rename the column");
        }

        #[tokio::test]
        async fn a_start_whose_committed_total_cannot_load_is_deferred_unspent() {
            let hub = Hub::new();
            let dir = paired("log-total-unloaded", &hub).await;
            let desktop = Desktop::open(&dir);
            let text = hub.signed(super::start("r-total", Vec::new())).await;
            rename_launched_by(&dir, "launched_by", "launched_by_away");

            let (result, entry) = deliver(&desktop, &text).await;

            assert_eq!(result, None);
            assert_eq!(
                row_of(&entry),
                (
                    LogKind::StartFeature,
                    Some(Scope::Spend),
                    LogOutcome::Deferred,
                    None
                )
            );
            assert_eq!(desktop.spy.effects(), 0);

            rename_launched_by(&dir, "launched_by_away", "launched_by");
            let (result, entry) = deliver(&desktop, &text).await;

            assert_eq!(result.expect("an answer").outcome, RequestOutcome::Accepted);
            assert_eq!(entry.outcome, LogOutcome::Accepted);
            assert_eq!(desktop.spy.launches().len(), 1);
            let _ = std::fs::remove_dir_all(dir);
        }

        /// Gates are pinned and toggled off, and A's assertion would verify:
        /// the toggle is the only thing refusing it.
        #[tokio::test]
        async fn a_gate_decision_with_gates_off_is_refused_scope_off() {
            let hub = Hub::new();
            let a = SoftAuthenticator::new().with_sign_count(7);
            let dir = paired_trusting("log-gate-off", &hub, vec![trust::pinned(&a)]).await;
            let presenter = Arc::new(ScriptedGatePresenter::refusing_all());
            let desktop = Desktop::presenting(&dir, presenter.clone());
            desktop.seed_pending_gate(trust::STEP_EXECUTION);
            let text = hub
                .signed(trust::gate("r-gate", Decision::Approve, &a))
                .await;

            let (result, entry) = deliver(&desktop, &text).await;

            let result = result.expect("an answer");
            assert_eq!(result.request_id, "r-gate");
            assert_eq!(result.outcome, refused(RefusalReason::ScopeOff));
            assert_eq!(
                row_of(&entry),
                (
                    LogKind::GateDecision,
                    Some(Scope::Gates),
                    LogOutcome::Refused,
                    Some("scope_off")
                )
            );
            assert!(presenter.calls().is_empty(), "{:?}", presenter.calls());
            assert_eq!(presenter.unexpected(), 0);
            assert_eq!(trust::stored_count(&dir, &a), 0);
            assert_eq!(desktop.spy.effects(), 0);
            let _ = std::fs::remove_dir_all(dir);
        }

        /// The passkey kinds spend no scope, so turning every toggle off
        /// leaves an endorsement to its standing refusal and a revocation to
        /// its assertion.
        #[tokio::test]
        async fn the_passkey_kinds_are_answered_even_with_every_scope_off() {
            let hub = Hub::new();
            let a = SoftAuthenticator::new();
            let c = trust::pinned(&SoftAuthenticator::new());
            let dir =
                paired_trusting("log-passkeys", &hub, vec![trust::pinned(&a), c.clone()]).await;
            save_policy(&dir, &HubLocalPolicy::default());
            let desktop = Desktop::open(&dir);

            let d = trust::pinned(&SoftAuthenticator::new());
            let text = hub
                .signed(trust::endorsement(
                    "r-endorse",
                    &d.credential_id,
                    &d.public_key,
                    &a,
                ))
                .await;
            let (result, entry) = deliver(&desktop, &text).await;
            assert_eq!(
                result.expect("an answer").outcome,
                RequestOutcome::Refused {
                    reason: RefusalReason::UnknownKind
                }
            );
            assert_eq!(
                row_of(&entry),
                (
                    LogKind::PasskeyEndorsement,
                    None,
                    LogOutcome::Refused,
                    Some("unknown_kind")
                )
            );

            let text = hub
                .signed(trust::revocation("r-revoke", &c.credential_id, &a))
                .await;
            let (result, entry) = deliver(&desktop, &text).await;
            assert_eq!(result.expect("an answer").outcome, RequestOutcome::Accepted);
            let revoked = format!(
                "revoked {}, signed by {}",
                c.credential_id,
                a.credential_id()
            );
            assert_eq!(
                row_of(&entry),
                (
                    LogKind::PasskeyRevocation,
                    None,
                    LogOutcome::Accepted,
                    Some(revoked.as_str())
                )
            );
            let _ = std::fs::remove_dir_all(dir);
        }

        #[tokio::test]
        async fn an_unknown_kind_or_a_malformed_body_is_answered_from_its_header() {
            let hub = Hub::new();
            let dir = paired("log-unparsed", &hub).await;
            let desktop = Desktop::open(&dir);
            let text = hub.signed(super::start("r-template", Vec::new())).await;
            let template: serde_json::Value = serde_json::from_str(&text).expect("parse");
            let mut unknown = template.clone();
            unknown["request_id"] = "r-unknown".into();
            unknown["payload"]["kind"] = "launch_everything".into();
            let mut malformed = template;
            malformed["request_id"] = "r-malformed".into();
            malformed["payload"]["body"]
                .as_object_mut()
                .expect("an object")
                .remove("project_id")
                .expect("a project_id to remove");

            for (body, request_id, reason, logged) in [
                (
                    unknown,
                    "r-unknown",
                    RefusalReason::UnknownKind,
                    "unknown_kind",
                ),
                (
                    malformed,
                    "r-malformed",
                    RefusalReason::MalformedBody,
                    "malformed_body",
                ),
            ] {
                let (result, entry) = deliver(&desktop, &body.to_string()).await;
                let result = result.expect("an answer");
                assert_eq!(result.request_id, request_id);
                assert_eq!(result.outcome, refused(reason));
                assert_eq!(entry.request_id, request_id);
                assert_eq!(
                    row_of(&entry),
                    (LogKind::Unknown, None, LogOutcome::Refused, Some(logged))
                );
                assert_no_content(&entry, request_id);
            }
            assert_eq!(desktop.spy.effects(), 0);
            let _ = std::fs::remove_dir_all(dir);
        }

        #[tokio::test]
        async fn an_unparsed_body_with_an_oversized_request_id_is_neither_logged_nor_answered() {
            let hub = Hub::new();
            let dir = paired("log-unparsed-long-id", &hub).await;
            let desktop = Desktop::open(&dir);
            let text = hub.signed(super::start("r-template", Vec::new())).await;
            let mut body: serde_json::Value = serde_json::from_str(&text).expect("parse");
            body["payload"]["kind"] = "launch_everything".into();

            body["request_id"] = "r".repeat(MAX_REQUEST_ID_LEN + 1).into();
            assert_eq!(desktop.handle(&body.to_string()).await, None);
            assert_eq!(desktop.log_len(), 0);

            let at_the_limit = "r".repeat(MAX_REQUEST_ID_LEN);
            body["request_id"] = at_the_limit.clone().into();
            let (result, entry) = deliver(&desktop, &body.to_string()).await;
            assert_eq!(result.expect("an answer").request_id, at_the_limit);
            assert_eq!(entry.request_id, at_the_limit);
            let _ = std::fs::remove_dir_all(dir);
        }

        #[tokio::test]
        async fn a_parsed_request_with_an_oversized_request_id_is_neither_logged_nor_answered() {
            let hub = Hub::new();
            let dir = paired("log-parsed-long-id", &hub).await;
            let desktop = Desktop::open(&dir);
            let text = hub.signed(super::start("r-template", Vec::new())).await;
            let mut body: serde_json::Value = serde_json::from_str(&text).expect("parse");

            body["request_id"] = "r".repeat(MAX_REQUEST_ID_LEN + 1).into();
            assert_eq!(desktop.handle(&body.to_string()).await, None);
            assert_eq!(desktop.log_len(), 0);
            assert!(consumed_ids(&dir).is_empty());

            let at_the_limit = "r".repeat(MAX_REQUEST_ID_LEN);
            body["request_id"] = at_the_limit.clone().into();
            let (result, entry) = deliver(&desktop, &body.to_string()).await;
            assert_eq!(result, None);
            assert_eq!(entry.request_id, at_the_limit);
            assert_eq!(entry.kind, LogKind::StartFeature);
            assert!(desktop.spy.launches().is_empty());
            assert_eq!(desktop.spy.effects(), 0);
            let _ = std::fs::remove_dir_all(dir);
        }

        #[tokio::test]
        async fn a_wall_clock_cap_is_ignored_and_noted_on_an_accepted_start() {
            let hub = Hub::new();
            let dir = paired("log-wall-clock", &hub).await;
            let desktop = Desktop::open(&dir);
            let request = with_start("r-wall-clock", |s| s.max_wall_clock_secs = Some(3_600));
            let text = hub.signed(request).await;

            let (result, entry) = deliver(&desktop, &text).await;

            assert_eq!(result.expect("an answer").outcome, RequestOutcome::Accepted);
            assert_eq!(desktop.spy.launches().len(), 1);
            assert_eq!(
                row_of(&entry),
                (
                    LogKind::StartFeature,
                    Some(Scope::Spend),
                    LogOutcome::Accepted,
                    Some("max_wall_clock_secs ignored")
                )
            );
            assert_eq!(entry.feature_id.as_deref(), Some("f-hub"));
            let _ = std::fs::remove_dir_all(dir);
        }

        #[tokio::test]
        async fn a_launch_that_fails_is_refused_malformed_body_and_stays_consumed() {
            let hub = Hub::new();
            let dir = paired("log-launch-fails", &hub).await;
            let desktop = Desktop::open(&dir);
            *desktop.spy.start_error.lock().expect("lock") = Some(format!(
                "branch for '{TITLE}' ({DESCRIPTION}) failed: \
                 https://user:ghp_0123456789abcdefghijklmnopqrstuvwxyz@github.com/o/r"
            ));
            let text = hub.signed(super::start("r-fails", Vec::new())).await;

            let (result, entry) = deliver(&desktop, &text).await;

            assert_eq!(
                result.expect("an answer").outcome,
                refused(RefusalReason::MalformedBody)
            );
            assert_eq!(
                (entry.kind, entry.scope, entry.outcome),
                (
                    LogKind::StartFeature,
                    Some(Scope::Spend),
                    LogOutcome::Refused
                )
            );
            let reason = entry.reason.clone().expect("a reason");
            assert!(reason.starts_with("malformed_body: "), "{reason}");
            assert!(reason.contains("failed"), "{reason}");
            assert!(!reason.contains("ghp_0123456789"), "{reason}");
            assert_no_content(&entry, "launch-fails");

            let (again, _) = deliver(&desktop, &text).await;
            assert_eq!(
                again.expect("an answer").outcome,
                refused(RefusalReason::Replayed)
            );
            assert_eq!(desktop.spy.launches().len(), 1);
            let _ = std::fs::remove_dir_all(dir);
        }

        /// The failure reason withholds the slugs of the title and the
        /// description, so CJK text over the 50-byte slug cap is slugged
        /// on the way to the answer.
        async fn assert_cjk_launch_failure_is_answered(tag: &str, title: &str) {
            let hub = Hub::new();
            let dir = paired(tag, &hub).await;
            let desktop = Desktop::open(&dir);
            *desktop.spy.start_error.lock().expect("lock") = Some("launch failed".to_owned());
            let request = with_start(&format!("r-{tag}"), |s| {
                s.title = title.into();
                s.description = "日".repeat(17);
            });
            let text = hub.signed(request).await;

            let (result, entry) = deliver(&desktop, &text).await;

            assert_eq!(
                result.expect("an answer").outcome,
                refused(RefusalReason::MalformedBody)
            );
            assert_eq!(
                (entry.kind, entry.outcome),
                (LogKind::StartFeature, LogOutcome::Refused)
            );
            let reason = entry.reason.clone().expect("a reason");
            assert!(reason.starts_with("malformed_body: "), "{reason}");
            let _ = std::fs::remove_dir_all(dir);
        }

        #[tokio::test]
        async fn a_failed_launch_with_a_blank_title_and_a_cjk_description_is_answered() {
            assert_cjk_launch_failure_is_answered("log-launch-cjk-blank", "  ").await;
        }

        #[tokio::test]
        async fn a_failed_launch_with_a_titled_cjk_description_is_answered() {
            assert_cjk_launch_failure_is_answered("log-launch-cjk-titled", TITLE).await;
        }

        #[tokio::test]
        async fn a_launch_failure_quoting_the_slugged_title_or_description_withholds_it() {
            let hub = Hub::new();
            let dir = paired("log-launch-slug", &hub).await;
            let desktop = Desktop::open(&dir);
            *desktop.spy.start_error.lock().expect("lock") = Some(
                "branch demeteo/features/from-the-hub already exists \
                 (worktree started-from-a-phone)"
                    .to_owned(),
            );
            let text = hub.signed(super::start("r-slug", Vec::new())).await;

            let (result, entry) = deliver(&desktop, &text).await;

            assert_eq!(
                result.expect("an answer").outcome,
                refused(RefusalReason::MalformedBody)
            );
            let reason = entry.reason.clone().expect("a reason");
            assert!(
                reason.contains(
                    "branch demeteo/features/[withheld] already exists (worktree [withheld])"
                ),
                "{reason}"
            );
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    /// AC6: no request kind, and no body that fails to parse as one, changes
    /// the scope toggles or ceilings. The handler holds only a
    /// `HubPolicyReader`; this is what proves no path around it writes.
    mod policy_immutable {
        use super::*;
        use demeteo_hub::protocol::{
            Decision, GateDecision, PasskeyEndorsement, PasskeyRevocation, WebAuthnAssertion,
        };

        fn assertion() -> WebAuthnAssertion {
            WebAuthnAssertion {
                credential_id: "c".into(),
                authenticator_data: "a".into(),
                client_data_json: "j".into(),
                signature: "s".into(),
            }
        }

        #[tokio::test]
        async fn no_request_kind_or_malformed_body_changes_the_policy() {
            let hub = Hub::new();
            let dir = paired("policy-immutable", &hub).await;
            let seeded = HubLocalPolicy {
                read_enabled: true,
                spend_enabled: true,
                gates_enabled: true,
                run_ceiling_cents: Some(1_234),
                daily_ceiling_cents: Some(56_789),
            };
            save_policy(&dir, &seeded);
            let desktop = Desktop::open(&dir);
            desktop.seed_run();
            let before = desktop.ctx.hub_policy.load().expect("load the policy");
            assert_eq!(before, seeded);

            let mut texts = Vec::new();
            for request in [
                super::start("r-start", Vec::new()),
                cancel("r-cancel", "f-1"),
                assign("r-assign", "f-1", "s-build"),
                request(
                    "r-gate",
                    RequestPayload::GateDecision(GateDecision {
                        feature_id: "f-1".into(),
                        step_execution_id: "se-review".into(),
                        decision: Decision::Approve,
                        feedback: None,
                        assertion: assertion(),
                    }),
                ),
                request(
                    "r-endorse",
                    RequestPayload::PasskeyEndorsement(PasskeyEndorsement {
                        new_credential_id: "c2".into(),
                        new_public_key: "k2".into(),
                        assertion: assertion(),
                    }),
                ),
                request(
                    "r-revoke",
                    RequestPayload::PasskeyRevocation(PasskeyRevocation {
                        credential_id: "c".into(),
                        assertion: assertion(),
                    }),
                ),
            ] {
                texts.push(hub.signed(request).await);
            }
            let mut malformed: serde_json::Value =
                serde_json::from_str(&hub.signed(super::start("r-malformed", Vec::new())).await)
                    .expect("parse");
            malformed["payload"]["body"]
                .as_object_mut()
                .expect("an object")
                .remove("project_id")
                .expect("a project_id to remove");
            texts.push(malformed.to_string());

            for text in &texts {
                desktop.handle(text).await;
            }

            assert_eq!(desktop.log_len(), texts.len(), "every request was judged");
            assert_eq!(
                desktop.spy.effects(),
                3,
                "the start, cancel and assignment executed"
            );
            assert_eq!(
                desktop.ctx.hub_policy.load().expect("load the policy"),
                before
            );
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    /// Spec AC2 to AC4: an endorsement, a revocation and a gate decision a
    /// trusted passkey signed, judged over real SQLite, a gate applied through a
    /// presenter that refuses any call it was not scripted with.
    mod trust {
        use super::*;
        use demeteo_core::domain::hub_request::assertion::{FLAG_USER_PRESENT, FLAG_USER_VERIFIED};
        use demeteo_core::domain::hub_request::passkeys::{
            endorsement_challenge_digest, gate_challenge_digest, revocation_challenge_digest,
            TrustWrite, TrustedPasskey,
        };
        use demeteo_hub::auth::testing::SoftAuthenticator;
        use demeteo_hub::auth::RelyingParty;
        use demeteo_hub::protocol::{
            Decision, GateDecision as GatePayload, PasskeyEndorsement, PasskeyRevocation, Scope,
            WebAuthnAssertion,
        };

        const FEATURE: &str = "f-1";
        pub(super) const STEP_EXECUTION: &str = "se-gate";
        /// Every gate decision carries it and no presenter is scripted with
        /// it: the desktop drops Hub feedback, so if it ever reaches
        /// `gate_decide` the strict double refuses the call.
        const FEEDBACK: &str = "ship it";
        const EXPIRES_AT: u64 = NOW + 600;

        fn rp() -> RelyingParty {
            RelyingParty {
                rp_id: "hub.example".into(),
                origin: "https://hub.example".into(),
            }
        }

        pub(super) fn pinned(authenticator: &SoftAuthenticator) -> PinnedPasskey {
            PinnedPasskey {
                credential_id: authenticator.credential_id(),
                public_key: base64url_encode(&authenticator.public_key_cose()),
            }
        }

        /// A paired desktop with gates on and pinned, trusting `trusted`.
        async fn trusting(
            tag: &str,
            hub: &Hub,
            trusted: &[&SoftAuthenticator],
        ) -> std::path::PathBuf {
            let dir = paired_trusting(
                &format!("trust-{tag}"),
                hub,
                trusted.iter().map(|a| pinned(a)).collect(),
            )
            .await;
            save_policy(
                &dir,
                &HubLocalPolicy {
                    read_enabled: true,
                    spend_enabled: false,
                    gates_enabled: true,
                    run_ceiling_cents: None,
                    daily_ceiling_cents: None,
                },
            );
            dir
        }

        /// `decision` on [`STEP_EXECUTION`], asserted by `signer` over the
        /// challenge the envelope commits it to.
        pub(super) fn gate(
            request_id: &str,
            decision: Decision,
            signer: &SoftAuthenticator,
        ) -> SignedRequest {
            gate_asserted(request_id, FEATURE, decision, |challenge| {
                signer.assert(&rp(), challenge)
            })
        }

        /// `decision` on [`STEP_EXECUTION`] naming `feature_id`, carrying
        /// whatever `assert` makes of the challenge that pair commits it to.
        fn gate_asserted(
            request_id: &str,
            feature_id: &str,
            decision: Decision,
            assert: impl FnOnce(&[u8; 32]) -> WebAuthnAssertion,
        ) -> SignedRequest {
            let challenge = gate_challenge_digest(
                INSTANCE,
                feature_id,
                STEP_EXECUTION,
                decision,
                request_id,
                EXPIRES_AT,
            )
            .expect("encode the gate challenge");
            SignedRequest {
                expires_at: EXPIRES_AT,
                ..request(
                    request_id,
                    RequestPayload::GateDecision(GatePayload {
                        feature_id: feature_id.into(),
                        step_execution_id: STEP_EXECUTION.into(),
                        decision,
                        feedback: Some(FEEDBACK.into()),
                        assertion: assert(&challenge),
                    }),
                )
            }
        }

        fn store(dir: &std::path::Path) -> SqliteAdapter {
            SqliteAdapter::new(demeteo_core::db::init_db(dir.to_path_buf()).expect("init db"))
                .expect("build sqlite adapter")
        }

        fn trusted_set(dir: &std::path::Path) -> Vec<TrustedPasskey> {
            store(dir).trusted().expect("load the trusted set")
        }

        pub(super) fn stored_count(
            dir: &std::path::Path,
            authenticator: &SoftAuthenticator,
        ) -> u32 {
            trusted_set(dir)
                .into_iter()
                .find(|passkey| passkey.credential_id == authenticator.credential_id())
                .expect("the passkey is still trusted")
                .sign_count
        }

        async fn assert_decided(tag: &str, decision: Decision, word: &str) {
            let hub = Hub::new();
            let a = SoftAuthenticator::new().with_sign_count(7);
            let dir = trusting(tag, &hub, &[&a]).await;
            let presenter = Arc::new(ScriptedGatePresenter::expecting(STEP_EXECUTION, word, None));
            let desktop = Desktop::presenting(&dir, presenter.clone());
            desktop.seed_pending_gate(STEP_EXECUTION);
            let text = hub.signed(gate("r-gate", decision, &a)).await;

            let result = desktop.handle(&text).await.expect("an answer");

            assert_eq!(result.outcome, RequestOutcome::Accepted, "{tag}");
            let entry = desktop
                .ctx
                .hub_requests
                .list_log(1)
                .expect("list the log")
                .pop()
                .expect("a log row")
                .entry;
            assert_eq!(
                (
                    entry.kind,
                    entry.scope,
                    entry.outcome,
                    entry.reason.as_deref(),
                    entry.feature_id.as_deref()
                ),
                (
                    LogKind::GateDecision,
                    Some(Scope::Gates),
                    LogOutcome::Accepted,
                    None,
                    Some(FEATURE)
                ),
                "{tag}"
            );
            assert!(!format!("{entry:?}").contains(FEEDBACK), "{tag}: {entry:?}");
            assert_eq!(
                presenter.calls(),
                [(STEP_EXECUTION.to_owned(), word.to_owned(), None)],
                "{tag}"
            );
            assert_eq!(presenter.unexpected(), 0, "{tag}");
            assert_eq!(stored_count(&dir, &a), 7, "{tag}");
            assert_eq!(desktop.spy.effects(), 0, "{tag}");
            let _ = std::fs::remove_dir_all(dir);
        }

        #[tokio::test]
        async fn an_approval_by_a_trusted_key_is_the_presenters_gate_decide() {
            assert_decided("approve", Decision::Approve, "approve").await;
        }

        #[tokio::test]
        async fn a_cancel_by_a_trusted_key_is_the_presenters_gate_decide() {
            assert_decided("cancel", Decision::Cancel, "cancel").await;
        }

        /// A desktop trusting `trusted` whose presenter takes no call. The
        /// gate is each test's to seed.
        async fn refusing(
            tag: &str,
            hub: &Hub,
            trusted: &[&SoftAuthenticator],
        ) -> (std::path::PathBuf, Desktop, Arc<ScriptedGatePresenter>) {
            let dir = trusting(tag, hub, trusted).await;
            let presenter = Arc::new(ScriptedGatePresenter::refusing_all());
            let desktop = Desktop::presenting(&dir, presenter.clone());
            (dir, desktop, presenter)
        }

        async fn assert_refused_untouched(
            desktop: &Desktop,
            presenter: &ScriptedGatePresenter,
            text: &str,
            reason: (RefusalReason, &str),
        ) {
            let result = desktop.handle(text).await.expect("an answer");
            assert_eq!(result.outcome, refused(reason.0));
            assert_eq!(
                desktop.last_log(),
                (
                    LogKind::GateDecision,
                    LogOutcome::Refused,
                    Some(reason.1.to_owned())
                )
            );
            assert!(presenter.calls().is_empty(), "{:?}", presenter.calls());
            assert_eq!(presenter.unexpected(), 0);
            assert_eq!(desktop.spy.effects(), 0);
        }

        #[tokio::test]
        async fn a_wrong_origin_assertion_is_assertion_invalid() {
            let hub = Hub::new();
            let a = SoftAuthenticator::new().with_sign_count(7);
            let (dir, desktop, presenter) = refusing("wrong-origin", &hub, &[&a]).await;
            desktop.seed_pending_gate(STEP_EXECUTION);
            let request = gate_asserted("r-gate", FEATURE, Decision::Approve, |challenge| {
                a.assert_with(
                    &rp().rp_id,
                    "https://evil.example",
                    FLAG_USER_PRESENT | FLAG_USER_VERIFIED,
                    challenge,
                )
            });
            let text = hub.signed(request).await;

            assert_refused_untouched(
                &desktop,
                &presenter,
                &text,
                (RefusalReason::AssertionInvalid, "assertion_invalid"),
            )
            .await;
            assert_eq!(stored_count(&dir, &a), 0);
            let _ = std::fs::remove_dir_all(dir);
        }

        #[tokio::test]
        async fn an_assertion_reporting_the_stored_count_again_is_assertion_invalid() {
            let hub = Hub::new();
            let a = SoftAuthenticator::new().with_sign_count(7);
            let (dir, desktop, presenter) = refusing("count-again", &hub, &[&a]).await;
            desktop.seed_pending_gate(STEP_EXECUTION);
            assert!(store(&dir)
                .advance_count(&a.credential_id(), 0, 7)
                .expect("advance the count"));
            let text = hub
                .signed(gate("r-gate-again", Decision::Approve, &a))
                .await;

            assert_refused_untouched(
                &desktop,
                &presenter,
                &text,
                (RefusalReason::AssertionInvalid, "assertion_invalid"),
            )
            .await;
            assert_eq!(stored_count(&dir, &a), 7);
            let _ = std::fs::remove_dir_all(dir);
        }

        #[tokio::test]
        async fn a_gate_already_decided_is_gate_not_pending() {
            let hub = Hub::new();
            let a = SoftAuthenticator::new().with_sign_count(7);
            let (dir, desktop, presenter) = refusing("decided", &hub, &[&a]).await;
            desktop
                .ctx
                .features
                .add(feature(FEATURE, "p-1"))
                .expect("add the feature");
            desktop
                .ctx
                .features
                .step_create(step(STEP_EXECUTION, "s-gate", "gate", "completed", 40))
                .expect("add the gate step");
            desktop
                .ctx
                .gates
                .create(GateDecision {
                    id: GateDecisionId::new("gd-1"),
                    step_execution_id: StepExecutionId::new(STEP_EXECUTION),
                    decision: Some("approve".into()),
                    feedback: None,
                    created_at: 40,
                    auto_approved: false,
                })
                .expect("add the decided gate");
            let text = hub.signed(gate("r-gate", Decision::Approve, &a)).await;

            assert_refused_untouched(
                &desktop,
                &presenter,
                &text,
                (RefusalReason::GateNotPending, "gate_not_pending"),
            )
            .await;
            assert_eq!(stored_count(&dir, &a), 0);
            let _ = std::fs::remove_dir_all(dir);
        }

        /// The gate row seeded here, which a mirrored run does not have,
        /// passes the pending check, so only whose run it is refuses it.
        #[tokio::test]
        async fn a_gate_decision_on_a_runner_owned_run_is_runner_owned() {
            let hub = Hub::new();
            let a = SoftAuthenticator::new().with_sign_count(7);
            let (dir, desktop, presenter) = refusing("runner-owned", &hub, &[&a]).await;
            desktop.seed_pending_gate(STEP_EXECUTION);
            desktop
                .ctx
                .remote_run_mirror
                .upsert_submitted("m-1", "run-1", Some("p-1"), Some(FEATURE), "t", 0)
                .expect("mirror the run");
            let text = hub.signed(gate("r-gate", Decision::Approve, &a)).await;

            assert_refused_untouched(
                &desktop,
                &presenter,
                &text,
                (RefusalReason::RunnerOwned, "runner_owned"),
            )
            .await;
            assert_eq!(stored_count(&dir, &a), 0);
            let _ = std::fs::remove_dir_all(dir);
        }

        #[tokio::test]
        async fn an_assertion_by_an_untrusted_key_is_credential_not_pinned() {
            let hub = Hub::new();
            let a = SoftAuthenticator::new();
            let b = SoftAuthenticator::new().with_sign_count(7);
            let (dir, desktop, presenter) = refusing("untrusted", &hub, &[&a]).await;
            desktop.seed_pending_gate(STEP_EXECUTION);
            let text = hub.signed(gate("r-gate", Decision::Approve, &b)).await;

            assert_refused_untouched(
                &desktop,
                &presenter,
                &text,
                (RefusalReason::CredentialNotPinned, "credential_not_pinned"),
            )
            .await;
            assert_eq!(stored_count(&dir, &a), 0);
            let _ = std::fs::remove_dir_all(dir);
        }

        /// The assertion is over the challenge for the feature the request
        /// names, so the step's owner is the only thing refusing it.
        #[tokio::test]
        async fn a_gate_decision_for_a_step_of_another_feature_is_unknown_step() {
            let hub = Hub::new();
            let a = SoftAuthenticator::new().with_sign_count(7);
            let (dir, desktop, presenter) = refusing("another-feature", &hub, &[&a]).await;
            desktop.seed_pending_gate(STEP_EXECUTION);
            desktop
                .ctx
                .features
                .add(feature("f-2", "p-1"))
                .expect("add the other feature");
            let request = gate_asserted("r-gate", "f-2", Decision::Approve, |challenge| {
                a.assert(&rp(), challenge)
            });
            let text = hub.signed(request).await;

            assert_refused_untouched(
                &desktop,
                &presenter,
                &text,
                (RefusalReason::UnknownStep, "unknown_step"),
            )
            .await;
            assert_eq!(stored_count(&dir, &a), 0);
            let _ = std::fs::remove_dir_all(dir);
        }

        /// Fails every read that selects `table.from` until it is renamed
        /// back. Nothing may open the store meanwhile: `init_db` re-adds
        /// any column it adds only if missing, under the old name.
        fn rename_column(dir: &std::path::Path, table: &str, from: &str, to: &str) {
            rusqlite::Connection::open(dir.join("demeteo.db"))
                .expect("open the desktop's DB")
                .execute_batch(&format!("ALTER TABLE {table} RENAME COLUMN {from} TO {to}"))
                .expect("rename the column");
        }

        /// `trusted()` is the one read naming `sign_count`; the pins load
        /// before it names only the other passkey columns.
        fn rename_sign_count(dir: &std::path::Path, from: &str, to: &str) {
            rename_column(dir, "hub_pinned_passkeys", from, to);
        }

        /// A's approval is deferred unspent, the gate never asked, while
        /// `table.column` is renamed away, and its redelivery is decided
        /// once it is back.
        async fn assert_gate_deferred_unspent(tag: &str, table: &str, column: &str) {
            let hub = Hub::new();
            let a = SoftAuthenticator::new().with_sign_count(7);
            let dir = trusting(tag, &hub, &[&a]).await;
            let presenter = Arc::new(ScriptedGatePresenter::expecting(
                STEP_EXECUTION,
                "approve",
                None,
            ));
            let desktop = Desktop::presenting(&dir, presenter.clone());
            desktop.seed_pending_gate(STEP_EXECUTION);
            let text = hub.signed(gate("r-gate", Decision::Approve, &a)).await;
            let away = format!("{column}_away");
            rename_column(&dir, table, column, &away);

            assert_eq!(desktop.handle(&text).await, None, "{tag}");
            assert_eq!(
                desktop.last_log(),
                (LogKind::GateDecision, LogOutcome::Deferred, None),
                "{tag}"
            );
            assert!(
                presenter.calls().is_empty(),
                "{tag}: {:?}",
                presenter.calls()
            );
            assert_eq!(presenter.unexpected(), 0, "{tag}");

            rename_column(&dir, table, &away, column);
            let result = desktop.handle(&text).await.expect("an answer");

            assert_eq!(result.outcome, RequestOutcome::Accepted, "{tag}");
            assert_eq!(
                presenter.calls(),
                [(STEP_EXECUTION.to_owned(), "approve".to_owned(), None)],
                "{tag}"
            );
            assert_eq!(presenter.unexpected(), 0, "{tag}");
            assert_eq!(stored_count(&dir, &a), 7, "{tag}");
            let _ = std::fs::remove_dir_all(dir);
        }

        #[tokio::test]
        async fn a_gate_decision_whose_trusted_set_cannot_load_is_deferred_unspent() {
            assert_gate_deferred_unspent("trusted-unloaded", "hub_pinned_passkeys", "sign_count")
                .await;
        }

        /// `step_get` selects `artifact_paths`; nothing a gate decision
        /// reads before it does.
        #[tokio::test]
        async fn a_gate_decision_whose_step_cannot_load_is_deferred_unspent() {
            assert_gate_deferred_unspent("step-unloaded", "step_executions", "artifact_paths")
                .await;
        }

        /// A gate decision reads `remote_run_mirror` only through
        /// `find_mirror_for_feature`, last.
        #[tokio::test]
        async fn a_gate_decision_whose_run_owner_cannot_load_is_deferred_unspent() {
            assert_gate_deferred_unspent(
                "mirror-unloaded",
                "remote_run_mirror",
                "last_notified_status",
            )
            .await;
        }

        /// The one passkey write a [`ScriptedPasskeys`] answers, and how.
        #[derive(Clone, Copy)]
        enum Script {
            Advance(fn() -> Result<bool, AppError>),
            Revoke(fn() -> Result<TrustWrite, AppError>),
        }

        /// Reads the desktop's own trusted set and answers every call of
        /// the write `script` names, counted by `writes`, writing nothing.
        /// Any other write is an `Err` counted by `unexpected`.
        struct ScriptedPasskeys {
            store: Arc<SqliteAdapter>,
            script: Script,
            writes: AtomicUsize,
            unexpected: AtomicUsize,
        }

        impl ScriptedPasskeys {
            fn scripted<T>(&self, answer: fn() -> Result<T, AppError>) -> Result<T, AppError> {
                self.writes.fetch_add(1, Ordering::SeqCst);
                answer()
            }

            fn unscripted<T>(&self, call: &str) -> Result<T, AppError> {
                self.unexpected.fetch_add(1, Ordering::SeqCst);
                Err(AppError::internal(format!(
                    "unscripted passkey write: {call}"
                )))
            }
        }

        impl HubPasskeyTrust for ScriptedPasskeys {
            fn trusted(&self) -> Result<Vec<TrustedPasskey>, AppError> {
                self.store.trusted()
            }

            fn advance_count(&self, _: &str, _: u32, _: u32) -> Result<bool, AppError> {
                match self.script {
                    Script::Advance(answer) => self.scripted(answer),
                    _ => self.unscripted("advance_count"),
                }
            }

            fn endorse(
                &self,
                _: &str,
                _: u32,
                _: u32,
                _: &str,
                _: &str,
                _: i64,
            ) -> Result<TrustWrite, AppError> {
                self.unscripted("endorse")
            }

            fn revoke(&self, _: &str, _: u32, _: u32, _: &str) -> Result<TrustWrite, AppError> {
                match self.script {
                    Script::Revoke(answer) => self.scripted(answer),
                    _ => self.unscripted("revoke"),
                }
            }
        }

        /// A desktop over `dir` deciding gates through `presenter`, whose
        /// passkey writes are `script`'s.
        fn scripted(
            dir: &std::path::Path,
            presenter: Arc<ScriptedGatePresenter>,
            script: Script,
        ) -> (Desktop, Arc<ScriptedPasskeys>) {
            let mut passkeys = None;
            let desktop = Desktop::presenting_through(dir, presenter, |store| {
                let scripted = Arc::new(ScriptedPasskeys {
                    store,
                    script,
                    writes: AtomicUsize::new(0),
                    unexpected: AtomicUsize::new(0),
                });
                passkeys = Some(scripted.clone());
                scripted
            });
            let passkeys = passkeys.expect("the handler is built over the scripted passkeys");
            (desktop, passkeys)
        }

        /// The assertion verifies, so only the counter write can refuse it.
        async fn assert_count_unwritten(
            tag: &str,
            advance: fn() -> Result<bool, AppError>,
            reason: (RefusalReason, &str),
        ) {
            let hub = Hub::new();
            let a = SoftAuthenticator::new().with_sign_count(7);
            let dir = trusting(tag, &hub, &[&a]).await;
            let presenter = Arc::new(ScriptedGatePresenter::refusing_all());
            let (desktop, counter) = scripted(&dir, presenter.clone(), Script::Advance(advance));
            desktop.seed_pending_gate(STEP_EXECUTION);
            let text = hub.signed(gate("r-gate", Decision::Approve, &a)).await;

            assert_refused_untouched(&desktop, &presenter, &text, reason).await;
            assert_eq!(counter.writes.load(Ordering::SeqCst), 1, "{tag}");
            assert_eq!(counter.unexpected.load(Ordering::SeqCst), 0, "{tag}");
            assert_eq!(stored_count(&dir, &a), 0, "{tag}");
            let _ = std::fs::remove_dir_all(dir);
        }

        #[tokio::test]
        async fn a_counter_write_lost_to_a_race_is_credential_not_pinned() {
            assert_count_unwritten(
                "lost-race",
                || Ok(false),
                (RefusalReason::CredentialNotPinned, "credential_not_pinned"),
            )
            .await;
        }

        #[tokio::test]
        async fn a_counter_write_that_fails_is_malformed_body() {
            assert_count_unwritten(
                "count-unwritten",
                || Err(AppError::database("disk I/O error")),
                (RefusalReason::MalformedBody, "malformed_body"),
            )
            .await;
        }

        /// The count is written before the gate is asked, and a gate that
        /// refuses does not give it back.
        #[tokio::test]
        async fn a_gate_that_refuses_the_decision_is_gate_not_pending_and_keeps_the_count() {
            let hub = Hub::new();
            let a = SoftAuthenticator::new().with_sign_count(7);
            let (dir, desktop, presenter) = refusing("gate-refuses", &hub, &[&a]).await;
            desktop.seed_pending_gate(STEP_EXECUTION);
            let text = hub.signed(gate("r-gate", Decision::Approve, &a)).await;

            let result = desktop.handle(&text).await.expect("an answer");

            assert_eq!(result.outcome, refused(RefusalReason::GateNotPending));
            assert_eq!(
                desktop.last_log(),
                (
                    LogKind::GateDecision,
                    LogOutcome::Refused,
                    Some("gate_not_pending".to_owned())
                )
            );
            assert!(presenter.calls().is_empty(), "{:?}", presenter.calls());
            assert_eq!(presenter.unexpected(), 1);
            assert_eq!(stored_count(&dir, &a), 7);
            assert_eq!(desktop.spy.effects(), 0);
            let _ = std::fs::remove_dir_all(dir);
        }

        /// Were the second delivery judged, the count the first wrote would
        /// still refuse it, as `assertion_invalid`: only the reason shows it
        /// was refused as a replay.
        #[tokio::test]
        async fn an_accepted_gate_decision_delivered_again_is_replayed_and_decided_once() {
            let hub = Hub::new();
            let a = SoftAuthenticator::new().with_sign_count(7);
            let dir = trusting("redelivered", &hub, &[&a]).await;
            let presenter = Arc::new(ScriptedGatePresenter::expecting(
                STEP_EXECUTION,
                "approve",
                None,
            ));
            let desktop = Desktop::presenting(&dir, presenter.clone());
            desktop.seed_pending_gate(STEP_EXECUTION);
            let text = hub.signed(gate("r-gate", Decision::Approve, &a)).await;

            for (delivery, outcome, logged, reason) in [
                (
                    "first",
                    RequestOutcome::Accepted,
                    LogOutcome::Accepted,
                    None,
                ),
                (
                    "second",
                    refused(RefusalReason::Replayed),
                    LogOutcome::Refused,
                    Some("replayed".to_owned()),
                ),
            ] {
                let result = desktop.handle(&text).await.expect("an answer");
                assert_eq!(result.outcome, outcome, "{delivery}");
                assert_eq!(
                    desktop.last_log(),
                    (LogKind::GateDecision, logged, reason),
                    "{delivery}"
                );
            }
            assert_eq!(
                presenter.calls(),
                [(STEP_EXECUTION.to_owned(), "approve".to_owned(), None)]
            );
            assert_eq!(presenter.unexpected(), 0);
            assert_eq!(stored_count(&dir, &a), 7);
            assert_eq!(desktop.spy.effects(), 0);
            let _ = std::fs::remove_dir_all(dir);
        }

        /// `signer`'s endorsement of `new_credential_id` with
        /// `new_public_key`, over the challenge the envelope commits them to.
        pub(super) fn endorsement(
            request_id: &str,
            new_credential_id: &str,
            new_public_key: &str,
            signer: &SoftAuthenticator,
        ) -> SignedRequest {
            let challenge = endorsement_challenge_digest(
                INSTANCE,
                request_id,
                EXPIRES_AT,
                new_credential_id,
                new_public_key,
            )
            .expect("encode the endorsement challenge");
            SignedRequest {
                expires_at: EXPIRES_AT,
                ..request(
                    request_id,
                    RequestPayload::PasskeyEndorsement(PasskeyEndorsement {
                        new_credential_id: new_credential_id.into(),
                        new_public_key: new_public_key.into(),
                        assertion: signer.assert(&rp(), &challenge),
                    }),
                )
            }
        }

        /// Trusting A, `endorse` builds a request from A, a new key C and an
        /// untrusted key D, and it is refused `unknown_kind` with the set,
        /// every stored key and counter among it, as it was (docs/HUB.md §11
        /// item 20).
        async fn assert_endorsement_refused(
            tag: &str,
            endorse: impl FnOnce(
                &SoftAuthenticator,
                &SoftAuthenticator,
                &SoftAuthenticator,
            ) -> SignedRequest,
        ) {
            let hub = Hub::new();
            let a = SoftAuthenticator::new().with_sign_count(3);
            let c = SoftAuthenticator::new().with_sign_count(3);
            let d = SoftAuthenticator::new().with_sign_count(3);
            let (dir, desktop, presenter) = refusing(tag, &hub, &[&a]).await;
            let before = trusted_set(&dir);
            let text = hub.signed(endorse(&a, &c, &d)).await;

            let result = desktop.handle(&text).await.expect("an answer");

            assert_eq!(result.outcome, refused(RefusalReason::UnknownKind), "{tag}");
            let entry = desktop
                .ctx
                .hub_requests
                .list_log(1)
                .expect("list the log")
                .pop()
                .expect("a log row")
                .entry;
            assert_eq!(
                (
                    entry.kind,
                    entry.scope,
                    entry.outcome,
                    entry.reason.as_deref()
                ),
                (
                    LogKind::PasskeyEndorsement,
                    None,
                    LogOutcome::Refused,
                    Some("unknown_kind")
                ),
                "{tag}"
            );
            assert_eq!(trusted_set(&dir), before, "{tag}");
            assert!(
                presenter.calls().is_empty(),
                "{tag}: {:?}",
                presenter.calls()
            );
            assert_eq!(presenter.unexpected(), 0, "{tag}");
            let _ = std::fs::remove_dir_all(dir);
        }

        /// A's assertion over C's endorsement challenge verifies, and A's
        /// stored count stays 0 although A reports 3.
        #[tokio::test]
        async fn an_endorsement_by_a_trusted_key_is_unknown_kind_and_trusts_nothing() {
            assert_endorsement_refused("endorse", |a, c, _| {
                let new = pinned(c);
                endorsement("r-endorse", &new.credential_id, &new.public_key, a)
            })
            .await;
        }

        #[tokio::test]
        async fn an_endorsement_signed_by_the_key_it_endorses_is_unknown_kind() {
            assert_endorsement_refused("endorse-self", |_, c, _| {
                let new = pinned(c);
                endorsement("r-endorse", &new.credential_id, &new.public_key, c)
            })
            .await;
        }

        #[tokio::test]
        async fn an_endorsement_signed_by_an_untrusted_key_is_unknown_kind() {
            assert_endorsement_refused("endorse-untrusted", |_, c, d| {
                let new = pinned(c);
                endorsement("r-endorse", &new.credential_id, &new.public_key, d)
            })
            .await;
        }

        /// A signs for its own id with C's key: a trusted key is never
        /// replaced, by its own word or any other.
        #[tokio::test]
        async fn an_endorsement_reusing_a_trusted_id_is_unknown_kind() {
            assert_endorsement_refused("endorse-reused-id", |a, c, _| {
                endorsement("r-endorse", &a.credential_id(), &pinned(c).public_key, a)
            })
            .await;
        }

        /// The substituted-ceremony attack: A's valid endorsement of C, which
        /// the Hub holds, is refused, so C's own approval of a pending gate
        /// is refused too, through a presenter that would have taken it.
        #[tokio::test]
        async fn a_key_whose_endorsement_was_refused_cannot_decide_a_gate() {
            let hub = Hub::new();
            let a = SoftAuthenticator::new();
            let c = SoftAuthenticator::new().with_sign_count(7);
            let dir = trusting("endorsed-gate", &hub, &[&a]).await;
            let presenter = Arc::new(ScriptedGatePresenter::expecting(
                STEP_EXECUTION,
                "approve",
                None,
            ));
            let desktop = Desktop::presenting(&dir, presenter.clone());
            desktop.seed_pending_gate(STEP_EXECUTION);
            let before = trusted_set(&dir);
            let new = pinned(&c);
            let text = hub
                .signed(endorsement(
                    "r-endorse",
                    &new.credential_id,
                    &new.public_key,
                    &a,
                ))
                .await;
            let endorsed = desktop.handle(&text).await.expect("an answer");
            assert_eq!(endorsed.outcome, refused(RefusalReason::UnknownKind));

            let text = hub.signed(gate("r-gate", Decision::Approve, &c)).await;
            let result = desktop.handle(&text).await.expect("an answer");

            assert_eq!(result.outcome, refused(RefusalReason::CredentialNotPinned));
            assert_eq!(
                desktop.last_log(),
                (
                    LogKind::GateDecision,
                    LogOutcome::Refused,
                    Some("credential_not_pinned".to_owned())
                )
            );
            assert!(presenter.calls().is_empty(), "{:?}", presenter.calls());
            assert_eq!(presenter.unexpected(), 0);
            assert_eq!(trusted_set(&dir), before);
            assert_eq!(desktop.spy.effects(), 0);
            let _ = std::fs::remove_dir_all(dir);
        }

        /// `signer`'s revocation of `target`, over the challenge the envelope
        /// commits it to.
        pub(super) fn revocation(
            request_id: &str,
            target: &str,
            signer: &SoftAuthenticator,
        ) -> SignedRequest {
            let challenge = revocation_challenge_digest(INSTANCE, request_id, EXPIRES_AT, target)
                .expect("encode the revocation challenge");
            SignedRequest {
                expires_at: EXPIRES_AT,
                ..request(
                    request_id,
                    RequestPayload::PasskeyRevocation(PasskeyRevocation {
                        credential_id: target.into(),
                        assertion: signer.assert(&rp(), &challenge),
                    }),
                )
            }
        }

        /// Hands `text` over, checks no gate was touched, and returns its
        /// answer with the outcome and reason its log row holds.
        async fn revoked(
            desktop: &Desktop,
            presenter: &ScriptedGatePresenter,
            text: &str,
        ) -> (RequestOutcome, LogOutcome, Option<String>) {
            let result = desktop.handle(text).await.expect("an answer");
            let (kind, outcome, reason) = desktop.last_log();
            assert_eq!(kind, LogKind::PasskeyRevocation);
            assert!(presenter.calls().is_empty(), "{:?}", presenter.calls());
            assert_eq!(presenter.unexpected(), 0);
            (result.outcome, outcome, reason)
        }

        fn last_credential() -> (RequestOutcome, LogOutcome, Option<String>) {
            (
                refused(RefusalReason::LastCredential),
                LogOutcome::Refused,
                Some("last_credential".to_owned()),
            )
        }

        #[tokio::test]
        async fn a_revocation_of_the_last_key_is_last_credential_and_keeps_it() {
            let hub = Hub::new();
            let a = SoftAuthenticator::new().with_sign_count(3);
            let (dir, desktop, presenter) = refusing("revoke-last", &hub, &[&a]).await;
            let before = trusted_set(&dir);
            let text = hub
                .signed(revocation("r-revoke", &a.credential_id(), &a))
                .await;

            assert_eq!(
                revoked(&desktop, &presenter, &text).await,
                last_credential()
            );
            assert_eq!(trusted_set(&dir), before);
            let _ = std::fs::remove_dir_all(dir);
        }

        /// The last revocation is signed by a key no longer trusted, and is
        /// still `last_credential`: that check runs before the signer is
        /// looked up (spec §5).
        #[tokio::test]
        async fn a_revocation_removes_another_trusted_key_but_never_the_last() {
            let hub = Hub::new();
            let a = SoftAuthenticator::new().with_sign_count(3);
            let b = SoftAuthenticator::new().with_sign_count(5);
            let (dir, desktop, presenter) = refusing("revoke-pair", &hub, &[&a, &b]).await;
            let pair = trusted_set(&dir);
            let only_a = [TrustedPasskey {
                sign_count: 3,
                ..pair[0].clone()
            }];

            let text = hub
                .signed(revocation("r-self", &b.credential_id(), &b))
                .await;
            assert_eq!(
                revoked(&desktop, &presenter, &text).await,
                (
                    refused(RefusalReason::SelfRevocation),
                    LogOutcome::Refused,
                    Some("self_revocation".to_owned())
                )
            );
            assert_eq!(trusted_set(&dir), pair);

            let text = hub
                .signed(revocation("r-by-a", &b.credential_id(), &a))
                .await;
            assert_eq!(
                revoked(&desktop, &presenter, &text).await,
                (
                    RequestOutcome::Accepted,
                    LogOutcome::Accepted,
                    Some(format!(
                        "revoked {}, signed by {}",
                        b.credential_id(),
                        a.credential_id()
                    ))
                )
            );
            assert_eq!(trusted_set(&dir), only_a);

            let text = hub
                .signed(revocation("r-by-b", &a.credential_id(), &b))
                .await;
            assert_eq!(
                revoked(&desktop, &presenter, &text).await,
                last_credential()
            );
            assert_eq!(trusted_set(&dir), only_a);
            let _ = std::fs::remove_dir_all(dir);
        }

        #[tokio::test]
        async fn a_revocation_whose_trusted_set_cannot_load_is_deferred_unspent() {
            let hub = Hub::new();
            let a = SoftAuthenticator::new().with_sign_count(3);
            let b = SoftAuthenticator::new();
            let (dir, desktop, presenter) = refusing("revoke-unloaded", &hub, &[&a, &b]).await;
            let text = hub
                .signed(revocation("r-revoke", &b.credential_id(), &a))
                .await;
            rename_sign_count(&dir, "sign_count", "sign_count_away");

            assert_eq!(desktop.handle(&text).await, None);
            assert_eq!(
                desktop.last_log(),
                (LogKind::PasskeyRevocation, LogOutcome::Deferred, None)
            );

            rename_sign_count(&dir, "sign_count_away", "sign_count");
            assert_eq!(
                revoked(&desktop, &presenter, &text).await,
                (
                    RequestOutcome::Accepted,
                    LogOutcome::Accepted,
                    Some(format!(
                        "revoked {}, signed by {}",
                        b.credential_id(),
                        a.credential_id()
                    ))
                )
            );
            assert_eq!(trusted_set(&dir).len(), 1);
            let _ = std::fs::remove_dir_all(dir);
        }

        /// Nothing is loaded for an endorsement, so a broken trusted-set
        /// table does not defer it: its id is spent and it is refused, and
        /// its redelivery is a replay.
        #[tokio::test]
        async fn an_endorsement_whose_trusted_set_cannot_load_is_still_unknown_kind() {
            let hub = Hub::new();
            let a = SoftAuthenticator::new().with_sign_count(3);
            let c = pinned(&SoftAuthenticator::new());
            let (dir, desktop, presenter) = refusing("endorse-unloaded", &hub, &[&a]).await;
            let before = trusted_set(&dir);
            let text = hub
                .signed(endorsement(
                    "r-endorse",
                    &c.credential_id,
                    &c.public_key,
                    &a,
                ))
                .await;
            rename_sign_count(&dir, "sign_count", "sign_count_away");

            let result = desktop.handle(&text).await.expect("an answer");

            assert_eq!(result.outcome, refused(RefusalReason::UnknownKind));
            assert_eq!(
                desktop.last_log(),
                (
                    LogKind::PasskeyEndorsement,
                    LogOutcome::Refused,
                    Some("unknown_kind".to_owned())
                )
            );

            rename_sign_count(&dir, "sign_count_away", "sign_count");
            let result = desktop.handle(&text).await.expect("an answer");

            assert_eq!(result.outcome, refused(RefusalReason::Replayed));
            assert_eq!(trusted_set(&dir), before);
            assert!(presenter.calls().is_empty(), "{:?}", presenter.calls());
            assert_eq!(presenter.unexpected(), 0);
            let _ = std::fs::remove_dir_all(dir);
        }

        /// Trusting A and B, `request` makes a request A signs that
        /// verifies, so only `script`'s write can refuse it, and it is
        /// refused `reason` with its id spent and nothing written.
        async fn assert_trust_write_refused(
            tag: &str,
            kind: LogKind,
            request: impl FnOnce(&SoftAuthenticator, &SoftAuthenticator) -> SignedRequest,
            script: Script,
            reason: (RefusalReason, &str),
        ) {
            let hub = Hub::new();
            let a = SoftAuthenticator::new().with_sign_count(3);
            let b = SoftAuthenticator::new().with_sign_count(5);
            let dir = trusting(tag, &hub, &[&a, &b]).await;
            let presenter = Arc::new(ScriptedGatePresenter::refusing_all());
            let (desktop, passkeys) = scripted(&dir, presenter.clone(), script);
            let before = trusted_set(&dir);
            let text = hub.signed(request(&a, &b)).await;

            for (delivery, (refusal, logged)) in [
                ("first", reason),
                ("again", (RefusalReason::Replayed, "replayed")),
            ] {
                let result = desktop.handle(&text).await.expect("an answer");
                assert_eq!(result.outcome, refused(refusal), "{tag}: {delivery}");
                assert_eq!(
                    desktop.last_log(),
                    (kind, LogOutcome::Refused, Some(logged.to_owned())),
                    "{tag}: {delivery}"
                );
            }
            assert_eq!(trusted_set(&dir), before, "{tag}");
            assert_eq!(passkeys.writes.load(Ordering::SeqCst), 1, "{tag}");
            assert_eq!(passkeys.unexpected.load(Ordering::SeqCst), 0, "{tag}");
            assert!(
                presenter.calls().is_empty(),
                "{tag}: {:?}",
                presenter.calls()
            );
            assert_eq!(presenter.unexpected(), 0, "{tag}");
            let _ = std::fs::remove_dir_all(dir);
        }

        fn revoking_b(a: &SoftAuthenticator, b: &SoftAuthenticator) -> SignedRequest {
            revocation("r-revoke", &b.credential_id(), a)
        }

        /// No script answers `endorse`, so a call to it would be counted
        /// as unexpected.
        #[tokio::test]
        async fn an_endorsement_never_reaches_the_passkey_store() {
            let hub = Hub::new();
            let a = SoftAuthenticator::new().with_sign_count(3);
            let dir = trusting("endorse-unwritten", &hub, &[&a]).await;
            let presenter = Arc::new(ScriptedGatePresenter::refusing_all());
            let (desktop, passkeys) =
                scripted(&dir, presenter.clone(), Script::Advance(|| Ok(true)));
            let new = pinned(&SoftAuthenticator::new());
            let text = hub
                .signed(endorsement(
                    "r-endorse",
                    &new.credential_id,
                    &new.public_key,
                    &a,
                ))
                .await;

            let result = desktop.handle(&text).await.expect("an answer");

            assert_eq!(result.outcome, refused(RefusalReason::UnknownKind));
            assert_eq!(passkeys.writes.load(Ordering::SeqCst), 0);
            assert_eq!(passkeys.unexpected.load(Ordering::SeqCst), 0);
            assert!(presenter.calls().is_empty(), "{:?}", presenter.calls());
            assert_eq!(presenter.unexpected(), 0);
            let _ = std::fs::remove_dir_all(dir);
        }

        #[tokio::test]
        async fn a_revocation_whose_write_fails_is_malformed_body() {
            assert_trust_write_refused(
                "revoke-unwritten",
                LogKind::PasskeyRevocation,
                revoking_b,
                Script::Revoke(|| Err(AppError::database("disk I/O error"))),
                (RefusalReason::MalformedBody, "malformed_body"),
            )
            .await;
        }

        #[tokio::test]
        async fn a_revocation_whose_signer_changed_before_the_write_is_credential_not_pinned() {
            assert_trust_write_refused(
                "revoke-signer-changed",
                LogKind::PasskeyRevocation,
                revoking_b,
                Script::Revoke(|| Ok(TrustWrite::SignerChanged)),
                (RefusalReason::CredentialNotPinned, "credential_not_pinned"),
            )
            .await;
        }

        #[tokio::test]
        async fn a_revocation_whose_target_went_before_the_write_is_credential_not_pinned() {
            assert_trust_write_refused(
                "revoke-target-gone",
                LogKind::PasskeyRevocation,
                revoking_b,
                Script::Revoke(|| Ok(TrustWrite::TargetGone)),
                (RefusalReason::CredentialNotPinned, "credential_not_pinned"),
            )
            .await;
        }
    }
}
