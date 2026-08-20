use super::*;
use crate::ports::agent_runtime::AgentStartError;
use std::pin::Pin;
use tokio_stream::{empty, Stream};

struct NoopRuntime;
#[async_trait::async_trait]
impl AgentRuntime for NoopRuntime {
    fn kind(&self) -> &'static str {
        "noop"
    }
    fn capabilities(&self) -> crate::ports::agent_runtime::AgentCapabilities {
        crate::ports::agent_runtime::AgentCapabilities {
            display_label: "Noop",
            lists_models: false,
            model_listing: None,
            default_model: None,
            effort_levels: &[],
            personalization: crate::ports::agent_runtime::PersonalizationSupport::Native,
            path_containment: crate::domain::models::PathContainment::UNFENCED,
            windows_agent_shell: crate::domain::models::WindowsAgentShell::Unknown,
        }
    }
    async fn availability(
        &self,
        _exec: &dyn crate::ports::execution::ExecutionPort,
        _machine_id: &str,
    ) -> Availability {
        Availability::Missing
    }
    fn install_command(&self) -> &'static str {
        "echo noop"
    }
    fn start(
        &self,
        _ctx: AgentContext,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<Arc<dyn AgentSession>, AgentStartError>>
                + Send
                + '_,
        >,
    > {
        Box::pin(async { Err(AgentStartError::SpawnFailed("noop".into())) })
    }
}

struct FakeSession;
impl AgentSession for FakeSession {
    fn session_id(&self) -> &str {
        "s-1"
    }
    fn prompt(
        &self,
        _text: &str,
    ) -> Pin<Box<dyn Stream<Item = crate::domain::agent_event::AgentEvent> + Send>> {
        Box::pin(empty())
    }
    fn cancel(&self) -> Result<(), String> {
        Ok(())
    }
    fn set_mode(&self, _mode_id: &str) -> Result<(), String> {
        Ok(())
    }
    fn set_config_option(&self, _config_id: &str, _value: &str) -> Result<(), String> {
        Ok(())
    }
    fn kill(&self) -> Result<(), String> {
        Ok(())
    }
    fn session_info(&self) -> crate::domain::models::SessionInfo {
        crate::domain::models::SessionInfo::default()
    }
}

#[test]
fn runtime_for_returns_registered_kind() {
    let reg = AgentRegistry::new(vec![Arc::new(NoopRuntime)]);
    assert!(reg.runtime_for("noop").is_some());
    assert!(reg.runtime_for("opencode").is_none());
}

/// Whether Demeteo's own spawn flags strip a harness's personalization has no
/// defensible default, so every supported kind answers it here — against its
/// `build_args`, which the value's own rustdoc calls the whole evidence for it.
///
/// A second hand-written copy of the declared table would pass forever after
/// someone taught an adapter to react to `bare_mode`: the declaration would
/// stay `Native`, the comment beside it would stay "reads no `bare_mode`", and
/// the launch surface would keep telling that user Demeteo passes their harness
/// no personalization flags. So the assertion is the argv itself, and the
/// exhaustive `match` survives only to stop a sixth harness compiling until
/// someone has declared for it.
#[test]
fn every_supported_kind_declares_what_bare_mode_does_to_its_personalization() {
    use crate::adapters::agent::cli_runtime::ArgsBuilder;
    use crate::adapters::agent::test_stubs::{StubAgentExec, StubExec};
    use crate::domain::models::AgentKind;
    use crate::ports::agent_runtime::{AgentContext, PersonalizationSupport};

    let ctx = |bare: bool| AgentContext {
        thread_id: "t1".into(),
        machine_id: "local".into(),
        binary: "agent".into(),
        args: vec![],
        env: Default::default(),
        cwd: ".".into(),
        model: None,
        effort: None,
        title: None,
        platform: None,
        agent_exec: Arc::new(StubAgentExec),
        exec: Arc::new(StubExec),
        permissions: crate::domain::permission::PermissionProfile::all_allow(),
        bare_mode: bare,
        keep_harness_personalization: false,
        tool_allowlist: None,
        max_turns: None,
        max_budget_usd: None,
    };

    for kind in AgentKind::ALL {
        let (declared, build): (PersonalizationSupport, ArgsBuilder) = match kind {
            AgentKind::ClaudeCode => {
                let rt = crate::adapters::agent::claude_code::runtime();
                (rt.personalization, rt.build_args)
            }
            AgentKind::Pi => {
                let rt = crate::adapters::agent::pi::runtime();
                (rt.personalization, rt.build_args)
            }
            AgentKind::Codex => {
                let rt = crate::adapters::agent::codex::runtime();
                (rt.personalization, rt.build_args)
            }
            AgentKind::Hermes => {
                let rt = crate::adapters::agent::hermes::runtime();
                (rt.personalization, rt.build_args)
            }
            // opencode wraps its `UnifiedCliRuntime` in a newtype and does not
            // expose it, so the declaration is read through the trait and the
            // builder by name. Same two values, one indirection more.
            AgentKind::Opencode => (
                crate::adapters::agent::opencode::runtime()
                    .capabilities()
                    .personalization,
                crate::adapters::agent::opencode::build_opencode_args as ArgsBuilder,
            ),
        };

        let plain = build(&ctx(false), None, "hi");
        let bare = build(&ctx(true), None, "hi");

        match declared {
            // The claim is "Demeteo passes it no personalization flags either
            // way", which is only true while `bare_mode` changes nothing at all
            // about the argv.
            PersonalizationSupport::Native => assert_eq!(
                plain, bare,
                "{kind} declares Native but its build_args reacts to bare_mode"
            ),
            // Both non-Native declarations are claims that `bare_mode` reaches
            // argv; which of the two it is depends on *what* the flags do, and
            // that is the `personalization` field's own job to say.
            PersonalizationSupport::Loaded | PersonalizationSupport::Suppressed => assert_ne!(
                plain, bare,
                "{kind} declares {declared:?} but its build_args ignores bare_mode"
            ),
        }
    }
}

/// The containment counterpart, and the reason it is not a second copy of the
/// table: the assertion is the fence itself, on the wire. A declaration and a
/// hand-written expectation would agree forever after someone deleted the
/// `external_directory` deny or made the sandbox selection conditional, and the
/// sync pane would keep telling that user their conflict resolution is confined
/// to the worktree.
///
/// Each dimension is bound to the one mechanism that could back it. The kernel
/// sandbox binds both ways — codex is the only harness that sends one and the
/// only one that claims one. The harness denial binds one way only, because
/// Demeteo sends it to a harness that is not credited with reading it; the test
/// below holds that gap open deliberately.
///
/// What no wire test reaches is `Harness` versus `HarnessPartial`: that
/// difference lives inside the harness's own dispatch, and the adapter
/// declaring the partial arm owns the evidence. Asserted here instead is that
/// the arm's two preconditions are both on the wire — the denial exists, and
/// the shell that walks past it is permitted.
///
/// Read for a Linux target because that is what the declaration is for, and
/// because the remote runner is always Linux; the platform-keyed answer is
/// `PathContainment::for_agent`'s own unit tests. The profile is `all_allow`
/// because that is what the sync resolver spawns with, so the argv here is the
/// argv of the turn the claim is made about.
#[test]
fn every_supported_kind_declares_what_confines_its_turn_to_the_worktree() {
    use crate::adapters::agent::cli_runtime::{ArgsBuilder, PermEnvBuilder};
    use crate::adapters::agent::test_stubs::{StubAgentExec, StubExec};
    use crate::domain::models::{AgentKind, Enforcement, PathContainment, Platform};
    use crate::domain::permission::PermissionProfile;
    use crate::ports::agent_runtime::AgentContext;

    let ctx = AgentContext {
        thread_id: "t1".into(),
        machine_id: "local".into(),
        binary: "agent".into(),
        args: vec![],
        env: Default::default(),
        cwd: ".".into(),
        model: None,
        effort: None,
        title: None,
        platform: Some(Platform::Linux),
        agent_exec: Arc::new(StubAgentExec),
        exec: Arc::new(StubExec),
        permissions: PermissionProfile::all_allow(),
        bare_mode: true,
        keep_harness_personalization: false,
        tool_allowlist: None,
        max_turns: None,
        max_budget_usd: None,
    };

    for kind in AgentKind::ALL {
        let (declared, build, perm): (PathContainment, ArgsBuilder, PermEnvBuilder) = match kind {
            AgentKind::ClaudeCode => {
                let rt = crate::adapters::agent::claude_code::runtime();
                (rt.path_containment, rt.build_args, rt.perm_env)
            }
            AgentKind::Pi => {
                let rt = crate::adapters::agent::pi::runtime();
                (rt.path_containment, rt.build_args, rt.perm_env)
            }
            AgentKind::Codex => {
                let rt = crate::adapters::agent::codex::runtime();
                (rt.path_containment, rt.build_args, rt.perm_env)
            }
            AgentKind::Hermes => {
                let rt = crate::adapters::agent::hermes::runtime();
                (rt.path_containment, rt.build_args, rt.perm_env)
            }
            AgentKind::Opencode => (
                crate::adapters::agent::opencode::runtime()
                    .capabilities()
                    .path_containment,
                crate::adapters::agent::opencode::build_opencode_args as ArgsBuilder,
                crate::ports::agent_runtime::opencode_permission_env as PermEnvBuilder,
            ),
        };

        assert_eq!(
            declared,
            PathContainment::for_agent(kind, Some(Platform::Linux)),
            "{kind} declares one thing in its adapter and another in the domain table"
        );

        let argv = build(&ctx, None, "hi").join(" ");
        let perm_env: String = perm(&ctx.permissions)
            .into_values()
            .collect::<Vec<_>>()
            .join(" ");
        let kernel_write_fence = argv.contains("sandbox_mode=workspace-write");
        let outside_denied = perm_env.contains(r#""external_directory":"deny""#);
        let shell_permitted = perm_env.contains(r#""bash":"allow""#);

        assert_eq!(
            declared.writes == Enforcement::Os,
            kernel_write_fence,
            "{kind}: an OS write fence is claimed exactly where the sandbox selection is sent — argv was `{argv}`"
        );
        assert_eq!(
            declared.shell == Enforcement::Os,
            kernel_write_fence,
            "{kind}: that sandbox is process-wide, so it backs the shell dimension and the write one together — argv was `{argv}`"
        );
        assert_ne!(
            declared.reads,
            Enforcement::Os,
            "{kind}: nothing Demeteo puts on a wire refuses a read — the one OS mechanism here is `sandbox_mode`, and `all_allow` selects its write-fencing mode"
        );

        for (dimension, enforcement) in [("reads", declared.reads), ("writes", declared.writes)] {
            let claims_harness = matches!(
                enforcement,
                Enforcement::Harness | Enforcement::HarnessPartial
            );
            assert!(
                !claims_harness || outside_denied,
                "{kind}: a harness fence on {dimension} with no directory denial on the wire — perm env was `{perm_env}`"
            );
        }

        if declared.shell == Enforcement::HarnessPartial {
            assert!(
                outside_denied && shell_permitted,
                "{kind}: the partial arm names a gap in a directory denial that a permitted shell walks past — take either away and it is the wrong word. Perm env was `{perm_env}`"
            );
        }
    }
}

/// Emission is not enforcement, and hermes is where the two come apart. It
/// shares opencode's `perm_env` translator, so it is handed the identical
/// `OPENCODE_PERMISSION` payload — and nothing in this tree establishes that a
/// harness reads a variable in another harness's namespace, nor is hermes
/// installed here to ask. Making the claim agree with the wire is the mistake
/// this exists to catch; what moves hermes is the capture named on its
/// declaration.
#[test]
fn hermes_is_handed_opencodes_directory_denial_and_still_claims_no_fence() {
    use crate::domain::models::{AgentKind, PathContainment, Platform};
    use crate::domain::permission::PermissionProfile;

    let rt = crate::adapters::agent::hermes::runtime();
    let payload = (rt.perm_env)(&PermissionProfile::all_allow());
    assert_eq!(
        payload,
        crate::ports::agent_runtime::opencode_permission_env(&PermissionProfile::all_allow()),
        "hermes no longer shares opencode's translator, so this test is asserting the wrong wire"
    );
    assert!(payload
        .values()
        .any(|v| v.contains(r#""external_directory":"deny""#)));

    assert_eq!(rt.path_containment, PathContainment::UNFENCED);
    assert_eq!(
        PathContainment::for_agent(AgentKind::Hermes, Some(Platform::Linux)),
        PathContainment::UNFENCED
    );
}

/// The half the argv comparison above cannot see: that pi's `bare_mode` block
/// is exactly the four personalization switches, and that a step keeping its
/// personalization gets none of them. Pi is the only harness the flag reaches,
/// so this is where the whole feature is true or false.
#[test]
fn a_step_that_keeps_its_personalization_gets_none_of_pis_suppression_flags() {
    use crate::adapters::agent::test_stubs::{StubAgentExec, StubExec};
    use crate::ports::agent_runtime::AgentContext;

    const SUPPRESSION: [&str; 4] = [
        "--no-skills",
        "--no-extensions",
        "--no-prompt-templates",
        "--no-themes",
    ];

    let ctx = |keep: bool| AgentContext {
        thread_id: "t1".into(),
        machine_id: "local".into(),
        binary: "pi".into(),
        args: vec![],
        env: Default::default(),
        cwd: ".".into(),
        model: None,
        effort: None,
        title: None,
        platform: None,
        agent_exec: Arc::new(StubAgentExec),
        exec: Arc::new(StubExec),
        permissions: crate::domain::permission::PermissionProfile::all_allow(),
        bare_mode: true,
        keep_harness_personalization: keep,
        tool_allowlist: None,
        max_turns: None,
        max_budget_usd: None,
    };

    let build = crate::adapters::agent::pi::runtime().build_args;
    let stripped = build(&ctx(false), None, "hi");
    let kept = build(&ctx(true), None, "hi");

    for flag in SUPPRESSION {
        assert!(
            stripped.contains(&flag.to_string()),
            "a bare pi turn should emit {flag}"
        );
        assert!(
            !kept.contains(&flag.to_string()),
            "a step keeping its personalization should not emit {flag}"
        );
    }

    // The four flags are the *whole* difference: keeping personalization must
    // not also hand back the rest of what `bare_mode` does, which on another
    // harness is the MCP and settings-source isolation this field must never
    // reach.
    let without_suppression: Vec<&String> = stripped
        .iter()
        .filter(|a| !SUPPRESSION.contains(&a.as_str()))
        .collect();
    assert_eq!(without_suppression, kept.iter().collect::<Vec<&String>>());
}

/// A kind nobody recognises is the case where nobody has checked what its
/// command tool runs, so it must answer that rather than inherit a default. A
/// legacy stored kind reaching a Windows prompt would otherwise be told, on no
/// evidence, that POSIX syntax works there.
#[test]
fn an_unrecognised_kind_declares_no_windows_shell() {
    use crate::domain::models::WindowsAgentShell;

    let reg = AgentRegistry::new(vec![Arc::new(NoopRuntime)]);
    assert_eq!(
        reg.windows_agent_shell_for("antigravity"),
        WindowsAgentShell::Unknown
    );
}

#[tokio::test]
async fn get_or_spawn_returns_structured_error_for_unknown_kind() {
    use crate::domain::action::AgentAction;
    use crate::ports::agent_execution::{ActionError, AgentExecutionPort, CommandOutcome};

    struct StubExec;
    #[async_trait::async_trait]
    impl AgentExecutionPort for StubExec {
        async fn submit(&self, _: &str, _: &str, _: AgentAction) -> Result<CommandOutcome, String> {
            Ok(CommandOutcome::Executed {
                output: crate::domain::intercept::ExecutionResult::Bash {
                    output: String::new(),
                },
            })
        }
        async fn submit_agent(
            &self,
            _: &str,
            _: &str,
            _: AgentAction,
            _: Option<String>,
        ) -> Result<CommandOutcome, ActionError> {
            Err(ActionError::Internal {
                message: "stub".into(),
            })
        }
        async fn approve(&self, _: &str) -> Result<(), String> {
            Ok(())
        }
        async fn reject(&self, _: &str, _: String) -> Result<(), String> {
            Ok(())
        }
        async fn register_result_responder(
            &self,
            _: &str,
            _: tokio::sync::oneshot::Sender<
                Result<crate::domain::intercept::ExecutionResult, String>,
            >,
        ) -> Result<(), String> {
            Ok(())
        }
    }
    #[async_trait::async_trait]
    impl crate::ports::execution::ExecutionPort for StubExec {
        async fn test_connection(&self, _: &str) -> Result<(), String> {
            Ok(())
        }
        async fn run_command(&self, _: &str, _: &str) -> Result<String, String> {
            Ok(String::new())
        }
        async fn read_file(&self, _: &str, _: &str) -> Result<String, String> {
            Ok(String::new())
        }
        async fn write_file(&self, _: &str, _: &str, _: &str) -> Result<(), String> {
            Ok(())
        }
        async fn write_file_bytes(&self, _: &str, _: &str, _: &[u8]) -> Result<(), String> {
            Ok(())
        }
        async fn get_metadata(
            &self,
            _: &str,
            path: &str,
        ) -> Result<crate::ports::execution::SftpEntry, String> {
            Ok(crate::ports::execution::SftpEntry {
                name: path.into(),
                path: path.into(),
                is_dir: false,
                size: 0,
                modified: 0,
            })
        }
        async fn list_dir(
            &self,
            _: &str,
            _: &str,
        ) -> Result<Vec<crate::ports::execution::SftpEntry>, String> {
            Ok(vec![])
        }
        async fn setup_worktree(&self, _: &str, _: &str, _: &str, _: &str) -> Result<(), String> {
            Ok(())
        }
        async fn resolve_home(&self, _: &str) -> Result<String, String> {
            Ok("/tmp".to_string())
        }
        async fn resolve_user(&self, _: &str) -> Result<String, String> {
            Ok("test".to_string())
        }
        async fn resolve_platform(
            &self,
            _: &str,
        ) -> Result<crate::domain::models::Platform, String> {
            Err("no platform configured on this stub".to_string())
        }
        async fn control_rpc(
            &self,
            _: &str,
            _: &str,
            _: serde_json::Value,
        ) -> Result<serde_json::Value, String> {
            Err("control_rpc not supported by this stub".to_string())
        }
        fn spawn_interactive(
            &self,
            _: &str,
            _: &str,
            _: &[String],
            _: &str,
            _: &std::collections::HashMap<String, String>,
        ) -> Result<Box<dyn crate::ports::execution::InteractiveHandle>, String> {
            Err("stub".to_string())
        }
    }

    let reg = AgentRegistry::new(vec![Arc::new(NoopRuntime)]);
    let stub = Arc::new(StubExec);
    let err = reg
        .get_or_spawn(
            "t1",
            "opencode",
            AgentContext {
                thread_id: "t1".into(),
                machine_id: "m1".into(),
                binary: "opencode".into(),
                args: vec![],
                env: Default::default(),
                cwd: ".".into(),
                model: None,
                effort: None,
                title: None,
                platform: None,
                agent_exec: stub.clone(),
                exec: stub,
                permissions: crate::domain::permission::PermissionProfile::all_allow(),
                bare_mode: false,
                keep_harness_personalization: false,
                tool_allowlist: None,
                max_turns: None,
                max_budget_usd: None,
            },
        )
        .await
        .err()
        .expect("should error");
    assert!(matches!(err, AgentStartError::NotFound(_)));
}

#[tokio::test]
async fn kill_removes_session() {
    let mut sessions: HashMap<String, Arc<dyn AgentSession>> = HashMap::new();
    sessions.insert("t1".into(), Arc::new(FakeSession) as Arc<dyn AgentSession>);
    let reg = AgentRegistry {
        runtimes: vec![],
        sessions: Mutex::new(sessions),
        availability_cache: tokio::sync::Mutex::new(HashMap::new()),
    };
    reg.kill("t1").await;
    reg.kill("t1").await;
}

/// Runtime that lets the test change the availability answer between probes.
/// Counts how many times it was probed so the cache behavior can be asserted
/// from the call count alone.
struct FlippableRuntime {
    state: tokio::sync::Mutex<Availability>,
    calls: tokio::sync::Mutex<u32>,
}

impl FlippableRuntime {
    fn new(initial: Availability) -> Arc<Self> {
        Arc::new(Self {
            state: tokio::sync::Mutex::new(initial),
            calls: tokio::sync::Mutex::new(0),
        })
    }
    async fn set(&self, next: Availability) {
        *self.state.lock().await = next;
    }
    async fn calls(&self) -> u32 {
        *self.calls.lock().await
    }
}

#[async_trait::async_trait]
impl AgentRuntime for FlippableRuntime {
    fn kind(&self) -> &'static str {
        "flippable"
    }
    fn capabilities(&self) -> crate::ports::agent_runtime::AgentCapabilities {
        crate::ports::agent_runtime::AgentCapabilities {
            display_label: "Flippable",
            lists_models: false,
            model_listing: None,
            default_model: None,
            effort_levels: &[],
            personalization: crate::ports::agent_runtime::PersonalizationSupport::Native,
            path_containment: crate::domain::models::PathContainment::UNFENCED,
            windows_agent_shell: crate::domain::models::WindowsAgentShell::Unknown,
        }
    }
    async fn availability(
        &self,
        _exec: &dyn crate::ports::execution::ExecutionPort,
        _machine_id: &str,
    ) -> Availability {
        let mut c = self.calls.lock().await;
        *c += 1;
        *self.state.lock().await
    }
    fn install_command(&self) -> &'static str {
        "echo flippable"
    }
    fn start(
        &self,
        _ctx: AgentContext,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<Arc<dyn AgentSession>, AgentStartError>>
                + Send
                + '_,
        >,
    > {
        Box::pin(async { Err(AgentStartError::SpawnFailed("flippable".into())) })
    }
}

/// A runtime with a fixed kind and answer that counts its probes, so a test
/// can tell "answered X" apart from "was never asked".
struct FixedRuntime {
    kind: &'static str,
    answer: Availability,
    calls: tokio::sync::Mutex<u32>,
}

impl FixedRuntime {
    fn new(kind: &'static str, answer: Availability) -> Arc<Self> {
        Arc::new(Self {
            kind,
            answer,
            calls: tokio::sync::Mutex::new(0),
        })
    }
    async fn calls(&self) -> u32 {
        *self.calls.lock().await
    }
}

#[async_trait::async_trait]
impl AgentRuntime for FixedRuntime {
    fn kind(&self) -> &'static str {
        self.kind
    }
    fn capabilities(&self) -> crate::ports::agent_runtime::AgentCapabilities {
        crate::ports::agent_runtime::AgentCapabilities {
            display_label: "Fixed",
            lists_models: false,
            model_listing: None,
            default_model: None,
            effort_levels: &[],
            personalization: crate::ports::agent_runtime::PersonalizationSupport::Native,
            path_containment: crate::domain::models::PathContainment::UNFENCED,
            windows_agent_shell: crate::domain::models::WindowsAgentShell::Unknown,
        }
    }
    async fn availability(
        &self,
        _exec: &dyn crate::ports::execution::ExecutionPort,
        _machine_id: &str,
    ) -> Availability {
        *self.calls.lock().await += 1;
        self.answer
    }
    fn install_command(&self) -> &'static str {
        "echo fixed"
    }
    fn start(
        &self,
        _ctx: AgentContext,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<Arc<dyn AgentSession>, AgentStartError>>
                + Send
                + '_,
        >,
    > {
        Box::pin(async { Err(AgentStartError::SpawnFailed("fixed".into())) })
    }
}

/// An `ExecutionPort` the availability tests never read through — the
/// runtimes under test answer from their own state, so this only has to
/// exist to satisfy the signature.
fn noop_exec() -> Arc<dyn crate::ports::execution::ExecutionPort> {
    struct NoopExec;
    #[async_trait::async_trait]
    impl crate::ports::execution::ExecutionPort for NoopExec {
        async fn test_connection(&self, _: &str) -> Result<(), String> {
            Ok(())
        }
        async fn run_command(&self, _: &str, _: &str) -> Result<String, String> {
            Ok(String::new())
        }
        async fn read_file(&self, _: &str, _: &str) -> Result<String, String> {
            Ok(String::new())
        }
        async fn write_file(&self, _: &str, _: &str, _: &str) -> Result<(), String> {
            Ok(())
        }
        async fn write_file_bytes(&self, _: &str, _: &str, _: &[u8]) -> Result<(), String> {
            Ok(())
        }
        async fn get_metadata(
            &self,
            _: &str,
            path: &str,
        ) -> Result<crate::ports::execution::SftpEntry, String> {
            Ok(crate::ports::execution::SftpEntry {
                name: path.into(),
                path: path.into(),
                is_dir: false,
                size: 0,
                modified: 0,
            })
        }
        async fn list_dir(
            &self,
            _: &str,
            _: &str,
        ) -> Result<Vec<crate::ports::execution::SftpEntry>, String> {
            Ok(vec![])
        }
        async fn setup_worktree(&self, _: &str, _: &str, _: &str, _: &str) -> Result<(), String> {
            Ok(())
        }
        async fn resolve_home(&self, _: &str) -> Result<String, String> {
            Ok("/tmp".into())
        }
        async fn resolve_user(&self, _: &str) -> Result<String, String> {
            Ok("test".into())
        }
        async fn resolve_platform(
            &self,
            _: &str,
        ) -> Result<crate::domain::models::Platform, String> {
            Err("no platform configured on this stub".into())
        }
        async fn control_rpc(
            &self,
            _: &str,
            _: &str,
            _: serde_json::Value,
        ) -> Result<serde_json::Value, String> {
            Err("control_rpc not supported by this stub".to_string())
        }
        fn spawn_interactive(
            &self,
            _: &str,
            _: &str,
            _: &[String],
            _: &str,
            _: &std::collections::HashMap<String, String>,
        ) -> Result<Box<dyn crate::ports::execution::InteractiveHandle>, String> {
            Err("noop".into())
        }
    }
    Arc::new(NoopExec)
}

/// When the user's installation toggles the binary on disk mid-session,
/// the next click of the "Re-check availability" button must return the
/// fresh value rather than the cached `false` from the previous probe.
#[tokio::test]
async fn is_available_force_bypasses_cache() {
    let rt = FlippableRuntime::new(Availability::Missing);
    let reg = AgentRegistry::new(vec![rt.clone()]);

    let stub = noop_exec();

    assert!(
        !reg.is_available("flippable", stub.as_ref(), "m1", false)
            .await
    );
    assert_eq!(rt.calls().await, 1, "first call must probe");

    // 2. Cached: subsequent non-forced calls must NOT re-probe.
    assert!(
        !reg.is_available("flippable", stub.as_ref(), "m1", false)
            .await
    );
    assert!(
        !reg.is_available("flippable", stub.as_ref(), "m1", false)
            .await
    );
    assert_eq!(rt.calls().await, 1, "non-forced calls must hit the cache");

    // 3. The user installs the binary. Flip the underlying runtime's
    //    answer to `Installed` and force a re-probe via the refresh button.
    rt.set(Availability::Installed).await;
    assert!(
        reg.is_available("flippable", stub.as_ref(), "m1", true)
            .await
    );
    assert_eq!(rt.calls().await, 2, "forced call must re-probe");

    // 4. The cache now reflects the fresh value.
    assert!(
        reg.is_available("flippable", stub.as_ref(), "m1", false)
            .await
    );
    assert_eq!(rt.calls().await, 2, "fresh value must be cached");
}

/// The cache remembers answers, not failures. A machine that was briefly
/// unreachable must be probed again on the next look — caching `Unknown`
/// would pin every agent on that machine to "missing" until the app
/// restarts, and Project Settings would persist that as user intent.
#[tokio::test]
async fn an_unanswered_probe_is_not_cached_and_is_retried() {
    let rt = FlippableRuntime::new(Availability::Unknown);
    let reg = AgentRegistry::new(vec![rt.clone()]);
    let stub = noop_exec();

    assert_eq!(
        reg.availability("flippable", stub.as_ref(), "m1", false)
            .await,
        Availability::Unknown
    );
    assert_eq!(
        reg.availability("flippable", stub.as_ref(), "m1", false)
            .await,
        Availability::Unknown
    );
    assert_eq!(
        rt.calls().await,
        2,
        "an inconclusive probe must not be answered from the cache"
    );

    rt.set(Availability::Installed).await;
    assert_eq!(
        reg.availability("flippable", stub.as_ref(), "m1", false)
            .await,
        Availability::Installed,
        "the machine coming back must be visible without a forced refresh"
    );
}

/// One unreachable machine, one bill. The settings page probes every kind on
/// one machine, and an inconclusive answer is not cached — so without this,
/// opening settings against a dead host pays the SSH connect timeout and its
/// retries once *per agent kind*, on every open.
#[tokio::test]
async fn one_unreachable_answer_stops_the_probe_for_the_rest_of_the_machine() {
    let first = FixedRuntime::new("first", Availability::Unknown);
    let second = FixedRuntime::new("second", Availability::Installed);
    let reg = AgentRegistry::new(vec![first.clone(), second.clone()]);
    let stub = noop_exec();

    let got = reg
        .availability_of(&["first", "second"], stub.as_ref(), "m1", false)
        .await;

    assert_eq!(
        got,
        vec![
            ("first", Availability::Unknown),
            ("second", Availability::Unknown)
        ],
        "a machine that did not answer did not answer for any kind"
    );
    assert_eq!(first.calls().await, 1);
    assert_eq!(
        second.calls().await,
        0,
        "the second kind must not be probed once the machine is known unreachable"
    );
}

/// The short-circuit is for an unreachable *machine*, not for a missing
/// agent: one kind being absent says nothing about the next.
#[tokio::test]
async fn a_missing_agent_does_not_stop_the_probe_for_the_others() {
    let first = FixedRuntime::new("first", Availability::Missing);
    let second = FixedRuntime::new("second", Availability::Installed);
    let reg = AgentRegistry::new(vec![first.clone(), second.clone()]);
    let stub = noop_exec();

    let got = reg
        .availability_of(&["first", "second"], stub.as_ref(), "m1", false)
        .await;

    assert_eq!(
        got,
        vec![
            ("first", Availability::Missing),
            ("second", Availability::Installed)
        ]
    );
    assert_eq!(second.calls().await, 1);
}

#[test]
fn effort_levels_for_reads_the_runtime_capability() {
    use crate::domain::models::EffortLevel;

    let reg = AgentRegistry::new(vec![
        Arc::new(crate::adapters::agent::claude_code::runtime()),
        Arc::new(crate::adapters::agent::codex::runtime()),
        Arc::new(crate::adapters::agent::hermes::runtime()),
    ]);

    assert_eq!(reg.effort_levels_for("claude-code"), &EffortLevel::ALL[..]);
    // Codex has no `max`.
    assert!(!reg.effort_levels_for("codex").contains(&EffortLevel::Max));
    assert!(reg.effort_levels_for("codex").contains(&EffortLevel::XHigh));
    // Hermes has no per-invocation effort control at all…
    assert!(reg.effort_levels_for("hermes").is_empty());
    // …and neither does a kind we don't know about.
    assert!(reg.effort_levels_for("nonesuch").is_empty());
}
