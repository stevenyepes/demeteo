// Contract test for `SequenceResumeRepository`, included from
// `crates/demeteo-core/src/ports/db.rs` (mirrored-tests convention).
// `super` = that module.
//
// One test body, two implementations. Every `#[test]` here runs against
// the real `SqliteAdapter` *and* an in-memory double, so the double
// cannot quietly drift from the thing it stands in for — which is the
// only reason a double for this port is worth having. Drop either arm of
// `against_every_impl` and these stop being contract tests.
//
// What they pin is the distinction the sequence step's crash-resume
// depends on and which nothing else asserts at the port level:
//
//   * `sequence_checkpoint_record` **unions** — it merges ids and the
//     `produced` payload into whatever the row already holds,
//     deduplicating, and returns the total.
//   * `sequence_checkpoint_set` **replaces** — the row becomes exactly
//     what was named, dropping everything else.
//
// Getting that backwards is not a visible bug: a `set` that unioned
// would leave a discarded attempt's commits in the checkpoint, and the
// next attempt would `reset --hard` onto work the rollback threw away.

use super::*;

use crate::adapters::database::SqliteAdapter;
use crate::domain::models::{CheckpointProduced, SequenceCheckpoint};
use rusqlite::Connection;
use std::collections::HashMap;
use std::sync::Mutex;

// ── The double ────────────────────────────────────────────────────────

/// In-memory `SequenceResumeRepository`, deliberately written as a
/// transcription of `adapters/database/repos/sequence_state.rs` rather
/// than a simplification — a double that only approximates the real
/// merge rules is worse than none, because the sequence step would test
/// green against semantics production does not have.
#[derive(Default)]
struct InMemorySequenceResume {
    checkpoints: Mutex<HashMap<(String, String), SequenceCheckpoint>>,
    plans: Mutex<HashMap<(String, String), String>>,
}

fn key(feature_id: &FeatureId, step_id: &str) -> (String, String) {
    (feature_id.0.clone(), step_id.to_string())
}

impl SequenceResumeRepository for InMemorySequenceResume {
    fn sequence_checkpoint_get(
        &self,
        feature_id: &FeatureId,
        step_id: &str,
    ) -> Result<SequenceCheckpoint, String> {
        Ok(self
            .checkpoints
            .lock()
            .unwrap()
            .get(&key(feature_id, step_id))
            .cloned()
            .unwrap_or_default())
    }

    fn sequence_checkpoint_record(
        &self,
        feature_id: &FeatureId,
        step_id: &str,
        landed_task_ids: &[String],
        anchor_sha: Option<&str>,
        produced: Option<&CheckpointProduced>,
        _now: i64,
    ) -> Result<u32, String> {
        let mut store = self.checkpoints.lock().unwrap();
        let existing = store
            .get(&key(feature_id, step_id))
            .cloned()
            .unwrap_or_default();
        let had_landed = !existing.landed_task_ids.is_empty();

        let mut merged = existing.landed_task_ids;
        for id in landed_task_ids {
            if !merged.contains(id) {
                merged.push(id.clone());
            }
        }

        // The anchor names the *tip* of the landed prefix, so the newest
        // write wins; `None` means the caller could not read a HEAD, and
        // knows less than the row does.
        let anchor = anchor_sha
            .map(|s| s.to_string())
            .or(existing.anchor_sha)
            .filter(|s| !s.trim().is_empty());

        let merged_produced = match (existing.produced, produced) {
            (existing, None) => existing,
            // A pre-V36 row that already claims tasks cannot be
            // completed from here: a partial payload would read as a
            // complete one.
            (None, Some(_)) if had_landed => None,
            (existing, Some(new)) => {
                let mut acc: CheckpointProduced = existing.unwrap_or_default();
                for r in &new.artifact_refs {
                    if !acc.artifact_refs.contains(r) {
                        acc.artifact_refs.push(r.clone());
                    }
                }
                for d in &new.satisfied_decls {
                    if !acc.satisfied_decls.contains(d) {
                        acc.satisfied_decls.push(d.clone());
                    }
                }
                Some(acc)
            }
        };

        let total = merged.len() as u32;
        store.insert(
            key(feature_id, step_id),
            SequenceCheckpoint {
                landed_task_ids: merged,
                anchor_sha: anchor,
                produced: merged_produced,
            },
        );
        Ok(total)
    }

    fn sequence_checkpoint_set(
        &self,
        feature_id: &FeatureId,
        step_id: &str,
        landed_task_ids: &[String],
        anchor_sha: Option<&str>,
        produced: Option<&CheckpointProduced>,
        _now: i64,
    ) -> Result<(), String> {
        self.checkpoints.lock().unwrap().insert(
            key(feature_id, step_id),
            SequenceCheckpoint {
                landed_task_ids: landed_task_ids.to_vec(),
                anchor_sha: anchor_sha
                    .map(|s| s.to_string())
                    .filter(|s| !s.trim().is_empty()),
                produced: produced.cloned(),
            },
        );
        Ok(())
    }

    fn sequence_checkpoint_clear(
        &self,
        feature_id: &FeatureId,
        step_id: &str,
    ) -> Result<(), String> {
        self.checkpoints
            .lock()
            .unwrap()
            .remove(&key(feature_id, step_id));
        Ok(())
    }

    fn plan_cache_get(
        &self,
        feature_id: &FeatureId,
        step_id: &str,
    ) -> Result<Option<String>, String> {
        Ok(self
            .plans
            .lock()
            .unwrap()
            .get(&key(feature_id, step_id))
            .cloned())
    }

    fn plan_cache_put(
        &self,
        feature_id: &FeatureId,
        step_id: &str,
        plan_json: &str,
        _attempt_no: Option<u32>,
        _now: i64,
    ) -> Result<(), String> {
        self.plans
            .lock()
            .unwrap()
            .insert(key(feature_id, step_id), plan_json.to_string());
        Ok(())
    }
}

// ── Harness ───────────────────────────────────────────────────────────

/// Run one test body against every implementation of the port. The
/// `&str` is the implementation's name, threaded into each assertion so
/// a failure says *which* side of the contract broke.
fn against_every_impl(body: impl Fn(&dyn SequenceResumeRepository, &str)) {
    let sqlite = SqliteAdapter::new(Connection::open_in_memory().unwrap()).unwrap();
    body(&sqlite, "SqliteAdapter");

    let double = InMemorySequenceResume::default();
    body(&double, "InMemorySequenceResume");
}

fn fid(s: &str) -> FeatureId {
    FeatureId::from(s.to_string())
}

fn produced(refs: &[&str], decls: &[&str]) -> CheckpointProduced {
    CheckpointProduced {
        artifact_refs: refs.iter().map(|s| s.to_string()).collect(),
        satisfied_decls: decls.iter().map(|s| s.to_string()).collect(),
    }
}

fn ids(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

// ── The contract ──────────────────────────────────────────────────────

/// `record` grows the id list: landed order preserved, duplicates folded
/// away, and the return value is the total after the merge (not this
/// call's contribution).
#[test]
fn record_unions_ids_and_returns_the_total() {
    against_every_impl(|repo, who| {
        let f = fid("f-1");
        assert_eq!(
            repo.sequence_checkpoint_record(&f, "s-impl", &ids(&["a", "b"]), None, None, 100)
                .unwrap(),
            2,
            "{who}: first record returns its own count"
        );
        assert_eq!(
            repo.sequence_checkpoint_record(&f, "s-impl", &ids(&["b", "c"]), None, None, 200)
                .unwrap(),
            3,
            "{who}: the return is the merged total, with `b` folded away"
        );
        assert_eq!(
            repo.sequence_checkpoint_get(&f, "s-impl")
                .unwrap()
                .landed_task_ids,
            ids(&["a", "b", "c"]),
            "{who}: landed order survives the union"
        );
    });
}

/// `set` is the counterpart: the row becomes exactly what was named. This
/// is the only write that can make a checkpoint *smaller*, which a
/// discarded attempt's rollback needs.
#[test]
fn set_replaces_the_ids_outright() {
    against_every_impl(|repo, who| {
        let f = fid("f-1");
        repo.sequence_checkpoint_record(&f, "s-impl", &ids(&["a", "b", "c"]), None, None, 100)
            .unwrap();
        repo.sequence_checkpoint_set(&f, "s-impl", &ids(&["a"]), None, None, 200)
            .unwrap();
        assert_eq!(
            repo.sequence_checkpoint_get(&f, "s-impl")
                .unwrap()
                .landed_task_ids,
            ids(&["a"]),
            "{who}: set drops the ids it did not name (a union would keep b and c)"
        );
    });
}

/// The same split on the anchor: `record`'s `None` means "I could not
/// read a HEAD" and leaves the stored one alone; `set`'s `None` means
/// "there is no anchor any more" and clears it.
#[test]
fn a_none_anchor_is_left_alone_by_record_and_cleared_by_set() {
    against_every_impl(|repo, who| {
        let f = fid("f-1");
        repo.sequence_checkpoint_record(&f, "s-impl", &ids(&["a"]), Some("sha-a"), None, 100)
            .unwrap();
        repo.sequence_checkpoint_record(&f, "s-impl", &ids(&["b"]), None, None, 200)
            .unwrap();
        assert_eq!(
            repo.sequence_checkpoint_get(&f, "s-impl")
                .unwrap()
                .anchor_sha,
            Some("sha-a".to_string()),
            "{who}: record leaves the anchor it was not given"
        );

        repo.sequence_checkpoint_set(&f, "s-impl", &ids(&["a"]), None, None, 300)
            .unwrap();
        assert_eq!(
            repo.sequence_checkpoint_get(&f, "s-impl")
                .unwrap()
                .anchor_sha,
            None,
            "{who}: set clears the anchor it was not given"
        );
    });
}

/// The `produced` payload unions the same way the ids do — including
/// across the two lists independently, and deduplicating each.
#[test]
fn record_unions_the_produced_payload() {
    against_every_impl(|repo, who| {
        let f = fid("f-1");
        repo.sequence_checkpoint_record(
            &f,
            "s-impl",
            &ids(&["a"]),
            Some("sha-a"),
            Some(&produced(&["art-a"], &["report"])),
            100,
        )
        .unwrap();
        repo.sequence_checkpoint_record(
            &f,
            "s-impl",
            &ids(&["b"]),
            Some("sha-b"),
            Some(&produced(&["art-a", "art-b"], &["report"])),
            200,
        )
        .unwrap();
        assert_eq!(
            repo.sequence_checkpoint_get(&f, "s-impl").unwrap().produced,
            Some(produced(&["art-a", "art-b"], &["report"])),
            "{who}: both lists union and deduplicate"
        );
    });
}

/// `None` adds nothing rather than erasing: a caller with no payload to
/// contribute knows less than the row does.
#[test]
fn record_with_no_payload_keeps_what_the_row_knows() {
    against_every_impl(|repo, who| {
        let f = fid("f-1");
        repo.sequence_checkpoint_record(
            &f,
            "s-impl",
            &ids(&["a"]),
            None,
            Some(&produced(&["art-a"], &[])),
            100,
        )
        .unwrap();
        repo.sequence_checkpoint_record(&f, "s-impl", &ids(&["b"]), None, None, 200)
            .unwrap();
        assert_eq!(
            repo.sequence_checkpoint_get(&f, "s-impl").unwrap().produced,
            Some(produced(&["art-a"], &[])),
            "{who}: a None payload contributes nothing and erases nothing"
        );
    });
}

/// The deliberate refusal: a row that already names tasks but carries no
/// payload was written before V36. Unioning this task's output into it
/// would yield a set that *looks* complete while silently omitting the
/// earlier tasks' artifacts — exactly the input the declared-deliverable
/// check would then misjudge. Such a row stays "unknown" for the rest of
/// the step's life.
#[test]
fn record_refuses_to_complete_a_pre_v36_row_that_already_claims_tasks() {
    against_every_impl(|repo, who| {
        let f = fid("f-1");
        // A pre-V36 shape: landed ids, no payload.
        repo.sequence_checkpoint_record(&f, "s-impl", &ids(&["a"]), None, None, 100)
            .unwrap();
        repo.sequence_checkpoint_record(
            &f,
            "s-impl",
            &ids(&["b"]),
            None,
            Some(&produced(&["art-b"], &["report"])),
            200,
        )
        .unwrap();
        let cp = repo.sequence_checkpoint_get(&f, "s-impl").unwrap();
        assert_eq!(
            cp.produced, None,
            "{who}: the payload stays unknown rather than becoming a partial set \
             that reads as complete"
        );
        assert_eq!(
            cp.landed_task_ids,
            ids(&["a", "b"]),
            "{who}: the refusal is about the payload only — the ids still union"
        );
    });
}

/// A payload on a row with *no* landed ids yet is not the pre-V36 case:
/// there is nothing earlier for the new payload to be misleadingly
/// partial about, so it is adopted.
#[test]
fn record_adopts_a_payload_on_a_row_that_claims_no_tasks() {
    against_every_impl(|repo, who| {
        let f = fid("f-1");
        repo.sequence_checkpoint_record(&f, "s-impl", &[], None, None, 100)
            .unwrap();
        repo.sequence_checkpoint_record(
            &f,
            "s-impl",
            &ids(&["a"]),
            None,
            Some(&produced(&["art-a"], &[])),
            200,
        )
        .unwrap();
        assert_eq!(
            repo.sequence_checkpoint_get(&f, "s-impl").unwrap().produced,
            Some(produced(&["art-a"], &[])),
            "{who}: an empty row has no earlier tasks to omit"
        );
    });
}

/// `set` replaces the payload along with the ids, and for the same
/// reason: a rewound row that kept this attempt's artifact references
/// would hand the next attempt output belonging to discarded commits.
#[test]
fn set_replaces_the_produced_payload() {
    against_every_impl(|repo, who| {
        let f = fid("f-1");
        repo.sequence_checkpoint_record(
            &f,
            "s-impl",
            &ids(&["a", "b"]),
            Some("sha-b"),
            Some(&produced(&["art-a", "art-b"], &["report"])),
            100,
        )
        .unwrap();
        repo.sequence_checkpoint_set(
            &f,
            "s-impl",
            &ids(&["a"]),
            Some("sha-a"),
            Some(&produced(&["art-a"], &[])),
            200,
        )
        .unwrap();
        assert_eq!(
            repo.sequence_checkpoint_get(&f, "s-impl").unwrap().produced,
            Some(produced(&["art-a"], &[])),
            "{who}: set drops art-b and the satisfied declaration it belonged to"
        );
    });
}

/// Reading a (feature, node) that never checkpointed is the empty
/// checkpoint, not an error — and `clear` puts a row back into that
/// state, because a stale skip-list would exempt tasks from a full
/// re-run.
#[test]
fn an_unwritten_or_cleared_checkpoint_reads_empty() {
    against_every_impl(|repo, who| {
        let f = fid("f-1");
        assert_eq!(
            repo.sequence_checkpoint_get(&f, "s-impl").unwrap(),
            SequenceCheckpoint::default(),
            "{who}: never checkpointed"
        );

        repo.sequence_checkpoint_record(
            &f,
            "s-impl",
            &ids(&["a"]),
            Some("sha-a"),
            Some(&produced(&["art-a"], &[])),
            100,
        )
        .unwrap();
        repo.sequence_checkpoint_clear(&f, "s-impl").unwrap();
        assert_eq!(
            repo.sequence_checkpoint_get(&f, "s-impl").unwrap(),
            SequenceCheckpoint::default(),
            "{who}: cleared"
        );
        // Idempotent — the step-complete path clears unconditionally.
        repo.sequence_checkpoint_clear(&f, "s-impl").unwrap();
    });
}

/// Both the checkpoint and the plan are keyed per (feature, node): a
/// workflow may hold several sequence nodes, and a write to one must not
/// be visible from another.
#[test]
fn state_is_scoped_to_one_feature_and_node() {
    against_every_impl(|repo, who| {
        repo.sequence_checkpoint_record(&fid("f-1"), "s-a", &ids(&["a"]), None, None, 100)
            .unwrap();
        repo.plan_cache_put(&fid("f-1"), "s-a", r#"{"tasks":[]}"#, Some(1), 100)
            .unwrap();

        for (feature, step) in [("f-1", "s-b"), ("f-2", "s-a")] {
            assert_eq!(
                repo.sequence_checkpoint_get(&fid(feature), step)
                    .unwrap()
                    .landed_task_ids,
                Vec::<String>::new(),
                "{who}: checkpoint leaked into ({feature}, {step})"
            );
            assert_eq!(
                repo.plan_cache_get(&fid(feature), step).unwrap(),
                None,
                "{who}: plan leaked into ({feature}, {step})"
            );
        }
    });
}

/// The plan cache is a plain upsert — unlike the checkpoint it never
/// merges, because a plan is the whole decomposition or it is nothing.
#[test]
fn the_plan_cache_round_trips_and_overwrites() {
    against_every_impl(|repo, who| {
        let f = fid("f-1");
        assert_eq!(
            repo.plan_cache_get(&f, "s-impl").unwrap(),
            None,
            "{who}: never planned"
        );

        repo.plan_cache_put(&f, "s-impl", r#"{"tasks":[]}"#, Some(1), 100)
            .unwrap();
        assert_eq!(
            repo.plan_cache_get(&f, "s-impl").unwrap().as_deref(),
            Some(r#"{"tasks":[]}"#),
            "{who}: round-trips verbatim"
        );

        repo.plan_cache_put(&f, "s-impl", r#"{"tasks":[{"id":"t"}]}"#, Some(2), 200)
            .unwrap();
        assert_eq!(
            repo.plan_cache_get(&f, "s-impl").unwrap().as_deref(),
            Some(r#"{"tasks":[{"id":"t"}]}"#),
            "{who}: the re-plan replaces, it does not merge"
        );
    });
}
