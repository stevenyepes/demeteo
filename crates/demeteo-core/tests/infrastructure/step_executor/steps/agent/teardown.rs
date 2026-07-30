// Tests for the session kill discipline. `super` = the `teardown` module.
//
// No doubles: the decision is which registry keys die and in which order,
// and that is total over the three dispositions. The `git_ops` call that
// precedes it is unconditional and has nothing to decide.

use super::*;

const SESSION: &str = "f-42::a1b2c3";
const FEATURE: &str = "f-42";

#[test]
fn an_early_return_before_any_session_kills_nothing() {
    assert!(sessions_to_kill(SESSION, FEATURE, SessionDisposition::Keep).is_empty());
}

#[test]
fn an_early_return_after_a_spawn_kills_only_this_steps_session() {
    assert_eq!(
        sessions_to_kill(SESSION, FEATURE, SessionDisposition::Kill),
        vec![SESSION.to_string()],
    );
}

#[test]
fn a_completed_step_kills_the_verifier_and_keeps_the_session() {
    let killed = sessions_to_kill(
        SESSION,
        FEATURE,
        SessionDisposition::Settle { completed: true },
    );
    assert_eq!(
        killed,
        vec!["f-42-verifier".to_string()],
        "the main session survives so the next step can --continue against it"
    );
    assert!(
        !killed.contains(&SESSION.to_string()),
        "killing it here is what breaks session reuse across steps"
    );
}

#[test]
fn an_unfinished_step_kills_the_verifier_first_then_the_session() {
    assert_eq!(
        sessions_to_kill(
            SESSION,
            FEATURE,
            SessionDisposition::Settle { completed: false }
        ),
        vec!["f-42-verifier".to_string(), SESSION.to_string()],
        "the verifier entry must not leak behind a failed step"
    );
}

#[test]
fn the_verifier_key_never_collides_with_the_fingerprint_scoped_session_key() {
    for completed in [true, false] {
        let killed = sessions_to_kill(SESSION, FEATURE, SessionDisposition::Settle { completed });
        assert_eq!(
            killed[0], "f-42-verifier",
            "the verifier is keyed off the bare feature id, the session off its fingerprint"
        );
        assert_ne!(killed[0], SESSION);
    }
}

#[test]
fn the_bare_feature_id_is_never_a_kill_target() {
    for disposition in [
        SessionDisposition::Keep,
        SessionDisposition::Kill,
        SessionDisposition::Settle { completed: true },
        SessionDisposition::Settle { completed: false },
    ] {
        let killed = sessions_to_kill(SESSION, FEATURE, disposition);
        assert!(
            !killed.contains(&FEATURE.to_string()),
            "the feature id no longer identifies a single session; killing by it \
             would take a sibling step's"
        );
    }
}
