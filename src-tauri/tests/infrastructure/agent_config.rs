// Tests extracted from `src-tauri/src/commands/agent_config.rs` (mirrored-tests
// convention). `super` = that module.

use super::{agent_catalog, agent_config_rows, agent_config_views};
use demeteo_core::adapters::agent::registry::AgentRegistry;
use demeteo_core::domain::models::{
    AgentConfig, Availability, EffortLevel, Enforcement, PathContainment, Platform,
};
use demeteo_core::ports::agent_runtime::AgentRuntime;
use demeteo_core::ports::execution::{ExecutionPort, InteractiveHandle, SftpEntry};
use std::collections::HashMap;
use std::sync::Arc;

/// The same runtime set `composition::build_context` registers, including the
/// internal `noop` runtime the catalog is expected to filter out.
fn production_registry() -> AgentRegistry {
    AgentRegistry::new(vec![
        Arc::new(demeteo_core::adapters::agent::opencode::runtime()) as Arc<dyn AgentRuntime>,
        Arc::new(demeteo_core::adapters::agent::hermes::runtime()) as Arc<dyn AgentRuntime>,
        Arc::new(demeteo_core::adapters::agent::claude_code::runtime()) as Arc<dyn AgentRuntime>,
        Arc::new(demeteo_core::adapters::agent::codex::runtime()) as Arc<dyn AgentRuntime>,
        Arc::new(demeteo_core::adapters::agent::noop::NoopRuntime) as Arc<dyn AgentRuntime>,
    ])
}

fn effort_levels_of(kind: &str) -> Vec<EffortLevel> {
    let catalog = agent_catalog(&production_registry());
    catalog
        .into_iter()
        .find(|e| e.kind == kind)
        .unwrap_or_else(|| panic!("{kind} is missing from the agent catalog"))
        .effort_levels
}

#[test]
fn hermes_reports_no_effort_levels_so_the_ui_cannot_offer_one() {
    // AC5: hermes has no per-invocation effort control. The empty list is what
    // disables the picker — the UI must not invent a ladder for it.
    assert!(effort_levels_of("hermes").is_empty());
}

#[test]
fn claude_code_reports_the_full_ladder() {
    assert_eq!(
        effort_levels_of("claude-code"),
        EffortLevel::ALL.to_vec(),
        "claude's --effort accepts every level; the catalog must say so"
    );
}

#[test]
fn codex_reports_a_ladder_without_max() {
    // `max` only exists on some gpt-5.6 models, so the static table stops at
    // xhigh and `clamp_for` folds Max down into it.
    let levels = effort_levels_of("codex");
    assert!(!levels.is_empty());
    assert!(!levels.contains(&EffortLevel::Max));
    assert!(levels.contains(&EffortLevel::XHigh));
}

/// The UI's own union is spelled in these strings, and a rename on either side
/// fails nothing: the note it drives just stops rendering, which is also what
/// "nobody has declared an answer" looks like there.
#[test]
fn the_catalog_carries_personalization_in_the_spelling_the_ui_reads() {
    let entry = agent_catalog(&production_registry())
        .into_iter()
        .find(|e| e.kind == "claude-code")
        .expect("claude-code is missing from the agent catalog");
    let json = serde_json::to_value(&entry).unwrap();
    assert_eq!(json["personalization"], "loaded");
}

/// The catalog is one list for every machine, and a fence backed by a kernel
/// facility is not one claim across machines: an answer served from here tells
/// a Windows desktop its codex turns are sandboxed. So the assertion is
/// absence and not a safe default — nothing is the one honest thing a list
/// with no machine in it can say, and a surface that finds nothing has to go
/// ask a machine.
#[test]
fn the_global_catalog_answers_nothing_about_containment() {
    for entry in agent_catalog(&production_registry()) {
        let json = serde_json::to_value(&entry).unwrap();
        assert!(
            json.get("path_containment").is_none(),
            "{} carries a containment claim on a list with no machine in it: {json}",
            entry.kind
        );
    }
}

#[test]
fn the_catalog_excludes_internal_runtimes() {
    let kinds: Vec<String> = agent_catalog(&production_registry())
        .into_iter()
        .map(|e| e.kind)
        .collect();
    assert!(!kinds.iter().any(|k| k == "noop"));
    assert_eq!(kinds.len(), 4);
}

fn config(kind: &str, enabled: bool) -> AgentConfig {
    AgentConfig {
        kind: kind.to_string(),
        enabled,
    }
}

/// `enabled` is what the user chose and `available` is what the machine has;
/// the settings table shows both, and the row must not conflate them. A
/// disabled-but-installed agent is the ordinary case of "I turned this off".
#[test]
fn a_row_carries_the_stored_choice_and_the_probe_separately() {
    let views = agent_config_views(
        &production_registry(),
        vec![config("codex", false)],
        &[("codex", Availability::Installed)],
        Some(Platform::Linux),
    );
    assert_eq!(views.len(), 1);
    assert!(!views[0].enabled, "the user's stored choice is untouched");
    assert!(views[0].available, "…and the probe still says it is there");
}

/// The one thing an `Unknown` probe may *not* do is claim availability — the
/// pickers filter on this flag, and offering an agent on a machine that never
/// answered would fail at spawn time instead.
#[test]
fn an_unanswered_probe_is_not_reported_as_available() {
    let views = agent_config_views(
        &production_registry(),
        vec![config("codex", true)],
        &[("codex", Availability::Unknown)],
        Some(Platform::Linux),
    );
    assert!(!views[0].available);
    assert!(
        views[0].enabled,
        "the stored enablement is a separate question from reachability"
    );
}

/// A kind stored by an older build that this one no longer registers still
/// gets a row — the user has to be able to see and clear it — but nothing
/// claims it is installed and there is no install command to offer.
#[test]
fn a_stored_kind_the_registry_no_longer_knows_still_gets_an_unavailable_row() {
    let views = agent_config_views(
        &production_registry(),
        vec![config("antigravity", true)],
        &[("codex", Availability::Installed)],
        Some(Platform::Linux),
    );
    assert_eq!(views.len(), 1);
    assert_eq!(views[0].kind, "antigravity");
    assert!(!views[0].available);
    assert_eq!(
        views[0].path_containment,
        PathContainment::UNFENCED,
        "with no adapter left to have declared a fence, the row may not imply one"
    );
    assert!(views[0].install_command.is_empty());
    assert_eq!(
        views[0].display_label, "antigravity",
        "with no runtime to ask, the kind is the only label available"
    );
}

/// The probe list drives the flag, not position: a row must read the entry
/// matching its own kind.
#[test]
fn each_row_reads_its_own_kinds_probe_result() {
    let views = agent_config_views(
        &production_registry(),
        vec![config("opencode", true), config("codex", true)],
        &[
            ("opencode", Availability::Missing),
            ("codex", Availability::Installed),
        ],
        Some(Platform::Linux),
    );
    let by_kind = |k: &str| views.iter().find(|v| v.kind == k).unwrap();
    assert!(!by_kind("opencode").available);
    assert!(by_kind("codex").available);
    assert_eq!(
        by_kind("codex").display_label,
        "Codex",
        "the label comes from the runtime's declared capabilities"
    );
}

/// An [`ExecutionPort`] that answers `resolve_platform` for the machines a test
/// named and errors on everything else, including on a machine it was not told
/// about.
///
/// A row's containment is exactly one port call, so an accommodating double is
/// what would let this suite pass while wired to nothing: a `Platform::Linux`
/// default is indistinguishable from a desktop answering for itself on a Linux
/// dev box, which is the bug the tests below exist to catch.
struct PlatformOnlyExec(HashMap<String, Platform>);

fn exec_answering(machines: &[(&str, Platform)]) -> PlatformOnlyExec {
    PlatformOnlyExec(
        machines
            .iter()
            .map(|(m, p)| ((*m).to_string(), *p))
            .collect(),
    )
}

impl PlatformOnlyExec {
    fn refused<T>(&self, what: &str) -> Result<T, String> {
        Err(format!("PlatformOnlyExec was never told how to {what}"))
    }
}

#[async_trait::async_trait]
impl ExecutionPort for PlatformOnlyExec {
    async fn resolve_platform(&self, machine_id: &str) -> Result<Platform, String> {
        self.0
            .get(machine_id)
            .copied()
            .ok_or_else(|| format!("no machine {machine_id} answered what it runs"))
    }
    async fn test_connection(&self, _: &str) -> Result<(), String> {
        self.refused("reach a machine")
    }
    async fn read_file(&self, _: &str, _: &str) -> Result<String, String> {
        self.refused("read a file")
    }
    async fn write_file(&self, _: &str, _: &str, _: &str) -> Result<(), String> {
        self.refused("write a file")
    }
    async fn write_file_bytes(&self, _: &str, _: &str, _: &[u8]) -> Result<(), String> {
        self.refused("write a file")
    }
    async fn get_metadata(&self, _: &str, _: &str) -> Result<SftpEntry, String> {
        self.refused("stat a path")
    }
    async fn list_dir(&self, _: &str, _: &str) -> Result<Vec<SftpEntry>, String> {
        self.refused("list a directory")
    }
    async fn setup_worktree(&self, _: &str, _: &str, _: &str, _: &str) -> Result<(), String> {
        self.refused("set up a worktree")
    }
    async fn resolve_home(&self, _: &str) -> Result<String, String> {
        self.refused("resolve a home directory")
    }
    async fn resolve_user(&self, _: &str) -> Result<String, String> {
        self.refused("resolve a user")
    }
    async fn control_rpc(
        &self,
        _: &str,
        _: &str,
        _: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        self.refused("call a runner")
    }
    fn spawn_interactive(
        &self,
        _: &str,
        _: &str,
        _: &[String],
        _: &str,
        _: &HashMap<String, String>,
    ) -> Result<Box<dyn InteractiveHandle>, String> {
        self.refused("spawn a process")
    }
}

async fn containment_on(kind: &str, machine_id: &str, exec: &PlatformOnlyExec) -> PathContainment {
    let views = agent_config_rows(
        &production_registry(),
        vec![config(kind, true)],
        &[(kind, Availability::Installed)],
        exec,
        machine_id,
    )
    .await;
    views[0].path_containment
}

/// The defect this row exists to close, gated through the function the command
/// actually calls: the answer belongs to the machine the turn will run on, and
/// the same desktop reads these rows for several. Codex's two published sandbox
/// backends are POSIX kernel facilities, so a Windows host — and a host that
/// never said what it was — gets the weakest claim, not the one that happens to
/// hold on Linux.
///
/// Three machines disagreeing is what makes this reachable at all. Any single
/// machine agrees with the desktop's own OS on some CI runner, so a suite that
/// named one would go green on a build that answered for the desktop; naming a
/// Linux and a Windows machine leaves that substitution wrong on whichever host
/// runs the suite.
#[tokio::test]
async fn a_row_takes_its_platform_from_the_machine_it_describes() {
    let exec = exec_answering(&[("m-linux", Platform::Linux), ("m-win", Platform::Windows)]);
    assert_eq!(
        containment_on("codex", "m-linux", &exec).await.writes,
        Enforcement::Os
    );
    assert_eq!(
        containment_on("codex", "m-win", &exec).await,
        PathContainment::UNFENCED,
        "no backend has been observed on Windows, so nothing there refuses anything"
    );
    assert_eq!(
        containment_on("codex", "m-offline", &exec).await,
        PathContainment::UNFENCED,
        "a transport that declined to name its OS answered no question about a kernel"
    );
    assert_eq!(
        containment_on("opencode", "m-win", &exec).await.writes,
        Enforcement::Harness,
        "opencode's fence is the harness's own check, which no kernel is behind"
    );
}

/// The wire contract the sync pane's note reads, spelled out: a rename on
/// either side costs the note its rendering rather than the pane its render,
/// and only this test says so before a user finds out.
#[test]
fn a_row_carries_path_containment_in_the_spelling_the_ui_reads() {
    let views = agent_config_views(
        &production_registry(),
        vec![config("opencode", true), config("codex", true)],
        &[
            ("opencode", Availability::Installed),
            ("codex", Availability::Installed),
        ],
        Some(Platform::Linux),
    );
    let json = |kind: &str| {
        let view = views.iter().find(|v| v.kind == kind).unwrap();
        serde_json::to_value(view).unwrap()["path_containment"].clone()
    };
    assert_eq!(
        json("opencode"),
        serde_json::json!({"reads": "harness", "writes": "harness", "shell": "harness-partial"}),
    );
    assert_eq!(
        json("codex"),
        serde_json::json!({"reads": "none", "writes": "os", "shell": "os"}),
    );
}
