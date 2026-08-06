use crate::domain::models::{AgentKind, Platform, SandboxSupport};

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
