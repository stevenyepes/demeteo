use crate::domain::models::{AgentKind, Enforcement, PathContainment, Platform, SandboxSupport};

#[test]
fn codex_is_enforced_on_the_two_posix_hosts() {
    for platform in [Platform::Linux, Platform::MacOS] {
        assert_eq!(
            SandboxSupport::for_agent(AgentKind::Codex, Some(platform)),
            SandboxSupport::Enforced,
            "{platform}",
        );
    }
}

/// Windows is the open question, and the distinction the table exists to hold
/// is `Unknown` vs `Undriven`: the second would be a claim that codex enforces
/// nothing there, which nothing observed supports.
#[test]
fn codex_on_windows_is_unknown_not_undriven() {
    assert_eq!(
        SandboxSupport::for_agent(AgentKind::Codex, Some(Platform::Windows)),
        SandboxSupport::Unknown,
    );
}

#[test]
fn an_unresolved_platform_is_unknown() {
    assert_eq!(
        SandboxSupport::for_agent(AgentKind::Codex, None),
        SandboxSupport::Unknown,
    );
}

#[test]
fn codex_selects_a_sandbox_on_every_platform_and_on_none() {
    for platform in Platform::ALL.map(Some).into_iter().chain([None]) {
        assert!(
            SandboxSupport::for_agent(AgentKind::Codex, platform).selects_sandbox(),
            "{platform:?}",
        );
    }
}

#[test]
fn the_agents_demeteo_does_not_sandbox_select_nothing_anywhere() {
    let undriven = AgentKind::ALL
        .into_iter()
        .filter(|k| *k != AgentKind::Codex)
        .collect::<Vec<_>>();
    assert_eq!(undriven.len(), 4, "a new agent kind needs a table entry");
    for kind in undriven {
        for platform in Platform::ALL.map(Some).into_iter().chain([None]) {
            let support = SandboxSupport::for_agent(kind, platform);
            assert_eq!(support, SandboxSupport::Undriven, "{kind:?} {platform:?}");
            assert!(!support.selects_sandbox(), "{kind:?} {platform:?}");
        }
    }
}

/// Nothing derives a containment answer from the adapter at runtime, so the
/// table is the whole claim — and it is made to a user choosing a harness for a
/// turn that spawns with `PermissionProfile::all_allow`. A kind that fell
/// through to a default would be that claim invented.
///
/// Spelled out dimension by dimension rather than by any summary, because a
/// summary is what cannot be true here: codex's kernel fence covers writes and
/// not reads, opencode's check covers the file tools and not the shell, and
/// any single value has to flatten one of those away.
#[test]
fn every_kind_declares_what_confines_it_on_a_posix_target() {
    let table = [
        (
            AgentKind::Codex,
            PathContainment {
                reads: Enforcement::None,
                writes: Enforcement::Os,
                shell: Enforcement::Os,
            },
        ),
        (
            AgentKind::Opencode,
            PathContainment {
                reads: Enforcement::Harness,
                writes: Enforcement::Harness,
                shell: Enforcement::HarnessPartial,
            },
        ),
        (AgentKind::Hermes, PathContainment::UNFENCED),
        (AgentKind::ClaudeCode, PathContainment::UNFENCED),
        (AgentKind::Pi, PathContainment::UNFENCED),
    ];
    assert_eq!(
        table.len(),
        AgentKind::ALL.len(),
        "a new agent kind needs a containment entry"
    );
    for (kind, expected) in table {
        for platform in [Platform::Linux, Platform::MacOS] {
            assert_eq!(
                PathContainment::for_agent(kind, Some(platform)),
                expected,
                "{kind:?} on {platform}",
            );
        }
    }
}

/// The one place the two tables are deliberately allowed to disagree: codex
/// keeps sending its sandbox selection where the backend is unobserved, and
/// keeps claiming nothing for it.
#[test]
fn codex_claims_no_fence_where_it_still_ships_the_sandbox_selection() {
    for platform in [Some(Platform::Windows), None] {
        assert_eq!(
            PathContainment::for_agent(AgentKind::Codex, platform),
            PathContainment::UNFENCED,
            "{platform:?}",
        );
        assert!(
            SandboxSupport::for_agent(AgentKind::Codex, platform).selects_sandbox(),
            "{platform:?}",
        );
    }
}

#[test]
fn codex_is_the_only_kind_whose_answer_moves_with_the_platform() {
    for kind in AgentKind::ALL
        .into_iter()
        .filter(|k| *k != AgentKind::Codex)
    {
        let posix = PathContainment::for_agent(kind, Some(Platform::Linux));
        for platform in Platform::ALL.map(Some).into_iter().chain([None]) {
            assert_eq!(
                PathContainment::for_agent(kind, platform),
                posix,
                "{kind:?} {platform:?}",
            );
        }
    }
}
