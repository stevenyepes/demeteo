use crate::domain::models::{AgentKind, EffortLevel};

#[test]
fn parse_as_str_round_trip() {
    for level in EffortLevel::ALL {
        assert_eq!(EffortLevel::parse(level.as_str()), Some(level));
    }
}

#[test]
fn parse_rejects_unknown_values() {
    for garbage in ["", "HIGH", "x-high", "ultra", "medium ", "42"] {
        assert_eq!(EffortLevel::parse(garbage), None, "accepted {garbage:?}");
    }
}

#[test]
fn serde_uses_the_canonical_lowercase_spelling() {
    assert_eq!(
        serde_json::to_string(&EffortLevel::XHigh).unwrap(),
        "\"xhigh\""
    );
    assert_eq!(
        serde_json::from_str::<EffortLevel>("\"xhigh\"").unwrap(),
        EffortLevel::XHigh
    );
    for level in EffortLevel::ALL {
        assert_eq!(
            serde_json::to_string(&level).unwrap(),
            format!("\"{}\"", level.as_str())
        );
    }
}

#[test]
fn ladder_is_ordered_low_to_max() {
    assert!(EffortLevel::Low < EffortLevel::Medium);
    assert!(EffortLevel::Medium < EffortLevel::High);
    assert!(EffortLevel::High < EffortLevel::XHigh);
    assert!(EffortLevel::XHigh < EffortLevel::Max);
    assert_eq!(EffortLevel::DEFAULT, EffortLevel::High);
}

/// AC4: the clamp is total — for every (kind, level) pair it returns either
/// `None` or a level the agent actually declared. No adapter can emit a level
/// outside its own supported set.
#[test]
fn clamp_never_escapes_the_declared_set() {
    for kind in AgentKind::ALL {
        let supported = EffortLevel::supported_for(kind);
        for level in EffortLevel::ALL {
            match EffortLevel::clamp_for(kind, level) {
                None => assert!(
                    supported.is_empty(),
                    "{kind} declares {supported:?} but clamped {level} to None"
                ),
                Some(clamped) => assert!(
                    supported.contains(&clamped),
                    "{kind} clamped {level} to {clamped}, which is not in {supported:?}"
                ),
            }
        }
    }
}

#[test]
fn clamp_keeps_a_supported_level_and_steps_down_otherwise() {
    for level in EffortLevel::ALL {
        assert_eq!(
            EffortLevel::clamp_for(AgentKind::ClaudeCode, level),
            Some(level)
        );
    }
    // Codex has no `max`: it clamps down to the next supported level, not up
    // and not to the floor.
    assert_eq!(
        EffortLevel::clamp_for(AgentKind::Codex, EffortLevel::Max),
        Some(EffortLevel::XHigh)
    );
    assert_eq!(
        EffortLevel::clamp_for(AgentKind::Codex, EffortLevel::High),
        Some(EffortLevel::High)
    );
}

/// AC5: hermes has no per-invocation effort control, so it declares none and
/// the clamp refuses to invent one.
#[test]
fn hermes_supports_no_effort_at_all() {
    assert!(EffortLevel::supported_for(AgentKind::Hermes).is_empty());
    for level in EffortLevel::ALL {
        assert_eq!(EffortLevel::clamp_for(AgentKind::Hermes, level), None);
    }
}

#[test]
fn internal_turn_pins_are_cheap() {
    assert_eq!(EffortLevel::TRIAGE, EffortLevel::Low);
    assert_eq!(EffortLevel::VERIFIER_DEFAULT, EffortLevel::Low);
    assert_eq!(EffortLevel::FINALIZE, EffortLevel::Medium);
}
