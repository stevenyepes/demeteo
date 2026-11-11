//! What a Hub Project may hold, judged before anything is written. Pure: the
//! handlers ask this and the store stores what it returns.
//!
//! The repository rules are the ones a runner start was held to when its
//! repository came from a desktop's `remote_url`: a runner clones
//! `https://{host}/{full_name}` as given, so a pair that does not round-trip
//! through that URL is refused rather than repaired.

use crate::store::hub_projects::{HubProjectFields, ProjectStepOverride};
use crate::workflows;
use demeteo_hub_protocol::{Effort, RunnerSettings};
use serde::Deserialize;

pub const MAX_NAME_CHARS: usize = 200;

/// The desktop's bound on `default_loop_iterations`, which a Hub Project keeps.
pub const LOOP_ITERATIONS: std::ops::RangeInclusive<u32> = 1..=10;

/// The step kinds that run an agent, so have one to override. The browser's
/// `isOverridable` (`hub-web/src/lib/dispatchForm.ts`) holds the same list.
const OVERRIDABLE_KINDS: [&str; 2] = ["agent", "sequence"];

const GITHUB: &str = "github";
const GITLAB: &str = "gitlab";

/// What `POST /v1/projects` and `POST /v1/projects/{id}` carry: every field,
/// since an update replaces the row whole.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectBody {
    pub name: String,
    pub provider_kind: String,
    pub host: String,
    pub full_name: String,
    #[serde(default)]
    pub default_agent_kind: Option<String>,
    #[serde(default)]
    pub default_model: Option<String>,
    #[serde(default)]
    pub default_effort: Option<Effort>,
    #[serde(default)]
    pub default_workflow_id: Option<String>,
    #[serde(default)]
    pub default_max_budget_cents: Option<u64>,
    #[serde(default)]
    pub settings: RunnerSettings,
    #[serde(default)]
    pub step_overrides: Vec<ProjectStepOverride>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectRefusal {
    Name,
    Provider,
    Repository,
    /// A default agent kind or model that is only whitespace or holds NUL.
    Default,
    /// `default_workflow_id` names no embedded starter.
    UnknownWorkflow,
    Budget,
    /// A setting that is only whitespace or holds NUL; a branch prefix a
    /// branch name cannot start with; a folder or file that is not inside the
    /// repository; or a loop budget outside [`LOOP_ITERATIONS`].
    Setting,
    /// A step override naming a starter that does not exist, a step it has
    /// no agent on, the same step twice, or nothing to override.
    StepOverride,
}

/// The fields to store for `body`: the host lowercased, and an empty agent
/// kind, model or workflow read as none.
pub fn project_verdict(body: ProjectBody) -> Result<HubProjectFields, ProjectRefusal> {
    let name_chars = body.name.chars().count();
    if name_chars == 0 || name_chars > MAX_NAME_CHARS || body.name.contains('\0') {
        return Err(ProjectRefusal::Name);
    }
    if body.provider_kind != GITHUB && body.provider_kind != GITLAB {
        return Err(ProjectRefusal::Provider);
    }
    let host = body.host.to_ascii_lowercase();
    let remote = remote_url(&host, &body.full_name);
    if parse_remote(&remote) != Some((host.clone(), body.full_name.clone())) {
        return Err(ProjectRefusal::Repository);
    }
    let default_agent_kind = optional(body.default_agent_kind)?;
    let default_model = optional(body.default_model)?;
    let default_workflow_id = body.default_workflow_id.filter(|id| !id.is_empty());
    if default_workflow_id
        .as_deref()
        .is_some_and(|id| workflows::starter(id).is_none())
    {
        return Err(ProjectRefusal::UnknownWorkflow);
    }
    if body
        .default_max_budget_cents
        .is_some_and(|cents| cents == 0 || i64::try_from(cents).is_err())
    {
        return Err(ProjectRefusal::Budget);
    }
    let settings = settings_verdict(body.settings)?;
    let step_overrides = step_overrides_verdict(body.step_overrides)?;
    Ok(HubProjectFields {
        name: body.name,
        provider_kind: body.provider_kind,
        host,
        full_name: body.full_name,
        default_agent_kind,
        default_model,
        default_effort: body.default_effort,
        default_workflow_id,
        default_max_budget_cents: body.default_max_budget_cents,
        settings,
        step_overrides,
    })
}

/// `settings` with every empty text read as unset. Commands and the review
/// entrypoint are kept as typed: they are run, or read by an agent, as given.
fn settings_verdict(settings: RunnerSettings) -> Result<RunnerSettings, ProjectRefusal> {
    let text = |value: Option<String>| optional(value).map_err(|_| ProjectRefusal::Setting);
    let branch_prefix = text(settings.branch_prefix)?;
    if branch_prefix
        .as_deref()
        .is_some_and(|prefix| !branch_prefix_ok(prefix))
    {
        return Err(ProjectRefusal::Setting);
    }
    let artifact_subdir = text(settings.artifact_subdir)?;
    let conventions_file = text(settings.conventions_file)?;
    if [&artifact_subdir, &conventions_file]
        .into_iter()
        .flatten()
        .any(|relative| !inside_repository(relative))
    {
        return Err(ProjectRefusal::Setting);
    }
    if settings
        .loop_iterations
        .is_some_and(|loops| !LOOP_ITERATIONS.contains(&loops))
    {
        return Err(ProjectRefusal::Setting);
    }
    Ok(RunnerSettings {
        branch_prefix,
        artifact_subdir,
        commit_artifacts: settings.commit_artifacts,
        review_entrypoint: text(settings.review_entrypoint)?,
        test_command: text(settings.test_command)?,
        build_command: text(settings.build_command)?,
        coverage_command: text(settings.coverage_command)?,
        conventions_file,
        loop_iterations: settings.loop_iterations,
        sync_resolver_agent_kind: text(settings.sync_resolver_agent_kind)?,
        sync_resolver_model: text(settings.sync_resolver_model)?,
        sync_resolver_effort: settings.sync_resolver_effort,
        sync_review_before_push: settings.sync_review_before_push,
    })
}

/// What `git check-ref-format` would refuse at the front of a branch name,
/// short of resolving the whole name: the runner appends the Feature's slug.
fn branch_prefix_ok(prefix: &str) -> bool {
    const FORBIDDEN: [char; 8] = ['~', '^', ':', '?', '*', '[', '\\', ' '];
    !prefix.starts_with(['-', '/', '.'])
        && !prefix.contains("..")
        && !prefix.contains("//")
        && !prefix.contains("@{")
        && !prefix.contains(|c: char| c.is_control() || c.is_whitespace() || FORBIDDEN.contains(&c))
}

/// Whether `relative` names something inside the clone, on every OS a runner
/// or a desktop might read it on: not absolute, no drive, no `..` segment.
fn inside_repository(relative: &str) -> bool {
    let drive = relative.as_bytes().get(1) == Some(&b':');
    !relative.starts_with(['/', '\\'])
        && !drive
        && !relative.contains(|c: char| c.is_control())
        && relative.split(['/', '\\']).all(|segment| segment != "..")
}

fn step_overrides_verdict(
    overrides: Vec<ProjectStepOverride>,
) -> Result<Vec<ProjectStepOverride>, ProjectRefusal> {
    let mut seen = std::collections::HashSet::new();
    overrides
        .into_iter()
        .map(|entry| {
            let starter =
                workflows::starter(&entry.workflow_id).ok_or(ProjectRefusal::StepOverride)?;
            let runs_an_agent = starter.graph.nodes.iter().any(|node| {
                node.id == entry.step_id && OVERRIDABLE_KINDS.contains(&node.kind.as_str())
            });
            let text =
                |value: Option<String>| optional(value).map_err(|_| ProjectRefusal::StepOverride);
            let entry = ProjectStepOverride {
                agent_kind: text(entry.agent_kind)?,
                model: text(entry.model)?,
                ..entry
            };
            let empty =
                entry.agent_kind.is_none() && entry.model.is_none() && entry.effort.is_none();
            if !runs_an_agent
                || empty
                || !seen.insert((entry.workflow_id.clone(), entry.step_id.clone()))
            {
                return Err(ProjectRefusal::StepOverride);
            }
            Ok(entry)
        })
        .collect()
}

fn optional(value: Option<String>) -> Result<Option<String>, ProjectRefusal> {
    match value {
        Some(text) if text.is_empty() => Ok(None),
        Some(text) if text.trim().is_empty() || text.contains('\0') => Err(ProjectRefusal::Default),
        other => Ok(other),
    }
}

/// `(host, full_name)` of an `https://host/owner/repo` URL, the host
/// lowercased; `None` for any other shape. A nested group and a host with a
/// port are kept whole. Userinfo, whitespace, control characters, an empty
/// segment and a trailing slash are refused.
pub fn parse_remote(url: &str) -> Option<(String, String)> {
    let rest = url.strip_prefix("https://")?;
    let (host, full_name) = rest.split_once('/')?;
    let well_formed = !host.is_empty()
        && full_name.contains('/')
        && full_name.split('/').all(|segment| !segment.is_empty());
    let doubtful = |c: char| c == '@' || c.is_whitespace() || c.is_control();
    if !well_formed || rest.contains(doubtful) {
        return None;
    }
    Some((host.to_ascii_lowercase(), full_name.to_owned()))
}

/// The URL a runner clones, with no userinfo.
pub fn remote_url(host: &str, full_name: &str) -> String {
    format!("https://{host}/{full_name}")
}

/// Whether a runner can be dispatched against the project today. GitLab has
/// no token minted for a runner yet (decision 71), so a GitLab project can be
/// kept and not run.
pub fn runner_ready(provider_kind: &str) -> bool {
    provider_kind == GITHUB
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(host: &str, full_name: &str) -> ProjectBody {
        ProjectBody {
            name: "Widgets".to_owned(),
            provider_kind: GITHUB.to_owned(),
            host: host.to_owned(),
            full_name: full_name.to_owned(),
            default_agent_kind: None,
            default_model: None,
            default_effort: None,
            default_workflow_id: None,
            default_max_budget_cents: None,
            settings: RunnerSettings::default(),
            step_overrides: Vec::new(),
        }
    }

    fn known_starter() -> String {
        workflows::starters()[0].id.clone()
    }

    #[test]
    fn a_well_formed_body_is_stored_with_its_host_lowercased() {
        let fields = project_verdict(ProjectBody {
            default_agent_kind: Some("claude-code".to_owned()),
            default_model: Some(String::new()),
            default_effort: Some(Effort::High),
            default_workflow_id: Some(known_starter()),
            default_max_budget_cents: Some(250),
            ..body("GitHub.COM", "Acme/Widgets")
        })
        .unwrap();

        assert_eq!(
            fields,
            HubProjectFields {
                name: "Widgets".to_owned(),
                provider_kind: GITHUB.to_owned(),
                host: "github.com".to_owned(),
                full_name: "Acme/Widgets".to_owned(),
                default_agent_kind: Some("claude-code".to_owned()),
                default_model: None,
                default_effort: Some(Effort::High),
                default_workflow_id: Some(known_starter()),
                default_max_budget_cents: Some(250),
                settings: RunnerSettings::default(),
                step_overrides: Vec::new(),
            }
        );
    }

    #[test]
    fn a_nested_group_and_a_host_with_a_port_are_kept_whole() {
        let fields = project_verdict(ProjectBody {
            provider_kind: GITLAB.to_owned(),
            ..body("gitlab.example:8443", "group/sub/repo")
        })
        .unwrap();
        assert_eq!(
            (fields.host.as_str(), fields.full_name.as_str()),
            ("gitlab.example:8443", "group/sub/repo")
        );
    }

    #[test]
    fn a_remote_a_runner_cannot_clone_as_given_is_refused() {
        let cases = [
            ("not https", "http://github.com/acme/widgets"),
            ("scp form", "git@github.com:acme/widgets"),
            ("no owner", "https://github.com/widgets"),
            ("no path", "https://github.com"),
            ("no host", "https:///acme/widgets"),
            ("empty segment", "https://github.com/acme//widgets"),
            ("trailing slash", "https://github.com/acme/widgets/"),
            ("userinfo", "https://tok@github.com/acme/widgets"),
            ("whitespace", "https://github.com/acme/wid gets"),
            ("control", "https://github.com/acme/wid\u{7}gets"),
        ];
        for (name, url) in cases {
            assert_eq!(parse_remote(url), None, "{name}");
        }
    }

    #[test]
    fn a_repository_that_does_not_round_trip_is_refused_by_the_verdict() {
        let cases = [
            ("no owner", body("github.com", "widgets")),
            ("no host", body("", "acme/widgets")),
            ("empty segment", body("github.com", "acme//widgets")),
            ("trailing slash", body("github.com", "acme/widgets/")),
            ("leading slash", body("github.com", "/acme/widgets")),
            ("userinfo", body("tok@github.com", "acme/widgets")),
            ("whitespace", body("github.com", "acme/wid gets")),
            ("path in host", body("github.com/acme", "widgets/x")),
            ("nul", body("github.com", "acme/wid\0gets")),
        ];
        for (name, body) in cases {
            assert_eq!(
                project_verdict(body),
                Err(ProjectRefusal::Repository),
                "{name}"
            );
        }
    }

    #[test]
    fn a_provider_other_than_github_or_gitlab_is_refused() {
        for provider in ["", "GitHub", "bitbucket"] {
            let refused = project_verdict(ProjectBody {
                provider_kind: provider.to_owned(),
                ..body("github.com", "acme/widgets")
            });
            assert_eq!(refused, Err(ProjectRefusal::Provider), "{provider:?}");
        }
    }

    #[test]
    fn a_name_is_one_to_two_hundred_characters_without_nul() {
        let named = |name: String| {
            project_verdict(ProjectBody {
                name,
                ..body("github.com", "acme/widgets")
            })
        };
        assert!(named("é".repeat(MAX_NAME_CHARS)).is_ok());
        for name in [
            String::new(),
            "x".repeat(MAX_NAME_CHARS + 1),
            "a\0b".to_owned(),
        ] {
            assert_eq!(named(name.clone()), Err(ProjectRefusal::Name), "{name:?}");
        }
    }

    #[test]
    fn an_unknown_default_workflow_is_refused_and_a_blank_one_is_none() {
        let with_workflow = |id: &str| {
            project_verdict(ProjectBody {
                default_workflow_id: Some(id.to_owned()),
                ..body("github.com", "acme/widgets")
            })
        };
        assert_eq!(
            with_workflow("wf-starter-nope"),
            Err(ProjectRefusal::UnknownWorkflow)
        );
        assert_eq!(with_workflow("").unwrap().default_workflow_id, None);
        assert_eq!(
            with_workflow(&known_starter()).unwrap().default_workflow_id,
            Some(known_starter())
        );
    }

    #[test]
    fn a_budget_is_positive_and_fits_the_column() {
        let with_budget = |cents: u64| {
            project_verdict(ProjectBody {
                default_max_budget_cents: Some(cents),
                ..body("github.com", "acme/widgets")
            })
        };
        assert!(with_budget(1).is_ok());
        assert!(with_budget(i64::MAX as u64).is_ok());
        for cents in [0, i64::MAX as u64 + 1] {
            assert_eq!(with_budget(cents), Err(ProjectRefusal::Budget), "{cents}");
        }
    }

    #[test]
    fn a_blank_or_nul_default_is_refused() {
        for value in [" ", "a\0b"] {
            let refused = project_verdict(ProjectBody {
                default_model: Some(value.to_owned()),
                ..body("github.com", "acme/widgets")
            });
            assert_eq!(refused, Err(ProjectRefusal::Default), "{value:?}");
        }
    }

    #[test]
    fn parse_remote_splits_and_lowercases_the_host() {
        assert_eq!(
            parse_remote("https://GitHub.com/Acme/Widgets"),
            Some(("github.com".to_owned(), "Acme/Widgets".to_owned()))
        );
        assert_eq!(
            parse_remote("https://gitlab.example:8443/group/sub/repo"),
            Some((
                "gitlab.example:8443".to_owned(),
                "group/sub/repo".to_owned()
            ))
        );
    }

    fn with_settings(settings: RunnerSettings) -> Result<HubProjectFields, ProjectRefusal> {
        project_verdict(ProjectBody {
            settings,
            ..body("github.com", "acme/widgets")
        })
    }

    #[test]
    fn empty_settings_read_as_unset_and_the_rest_are_kept_as_typed() {
        let fields = with_settings(RunnerSettings {
            branch_prefix: Some("hub/".to_owned()),
            artifact_subdir: Some(String::new()),
            test_command: Some("  cargo test --workspace".to_owned()),
            conventions_file: Some("docs/AGENTS.md".to_owned()),
            loop_iterations: Some(10),
            sync_review_before_push: Some(false),
            ..RunnerSettings::default()
        })
        .unwrap();
        assert_eq!(
            fields.settings,
            RunnerSettings {
                branch_prefix: Some("hub/".to_owned()),
                artifact_subdir: None,
                test_command: Some("  cargo test --workspace".to_owned()),
                conventions_file: Some("docs/AGENTS.md".to_owned()),
                loop_iterations: Some(10),
                sync_review_before_push: Some(false),
                ..RunnerSettings::default()
            }
        );
    }

    #[test]
    fn a_setting_a_runner_could_not_use_is_refused() {
        let cases: [(&str, RunnerSettings); 9] = [
            (
                "blank command",
                RunnerSettings {
                    test_command: Some("  ".to_owned()),
                    ..RunnerSettings::default()
                },
            ),
            (
                "NUL",
                RunnerSettings {
                    review_entrypoint: Some("a\0b".to_owned()),
                    ..RunnerSettings::default()
                },
            ),
            (
                "prefix with a space",
                RunnerSettings {
                    branch_prefix: Some("my branch/".to_owned()),
                    ..RunnerSettings::default()
                },
            ),
            (
                "prefix like an option",
                RunnerSettings {
                    branch_prefix: Some("-f".to_owned()),
                    ..RunnerSettings::default()
                },
            ),
            (
                "prefix with ..",
                RunnerSettings {
                    branch_prefix: Some("a..b/".to_owned()),
                    ..RunnerSettings::default()
                },
            ),
            (
                "absolute folder",
                RunnerSettings {
                    artifact_subdir: Some("/tmp/out".to_owned()),
                    ..RunnerSettings::default()
                },
            ),
            (
                "drive",
                RunnerSettings {
                    artifact_subdir: Some("C:\\out".to_owned()),
                    ..RunnerSettings::default()
                },
            ),
            (
                "escaping file",
                RunnerSettings {
                    conventions_file: Some("docs/../../etc".to_owned()),
                    ..RunnerSettings::default()
                },
            ),
            (
                "no loops",
                RunnerSettings {
                    loop_iterations: Some(0),
                    ..RunnerSettings::default()
                },
            ),
        ];
        for (case, settings) in cases {
            assert_eq!(
                with_settings(settings),
                Err(ProjectRefusal::Setting),
                "{case}"
            );
        }
        assert_eq!(
            with_settings(RunnerSettings {
                loop_iterations: Some(11),
                ..RunnerSettings::default()
            }),
            Err(ProjectRefusal::Setting)
        );
    }

    /// A starter with an agent step, and one of its steps that runs none.
    fn agent_and_other_step() -> (String, String, String) {
        workflows::starters()
            .iter()
            .find_map(|starter| {
                let nodes = &starter.graph.nodes;
                let agent = nodes.iter().find(|node| node.kind == "agent")?;
                let other = nodes
                    .iter()
                    .find(|node| !OVERRIDABLE_KINDS.contains(&node.kind.as_str()))?;
                Some((starter.id.clone(), agent.id.clone(), other.id.clone()))
            })
            .expect("a starter with an agent step and a gate")
    }

    fn step_override(workflow_id: &str, step_id: &str) -> ProjectStepOverride {
        ProjectStepOverride {
            workflow_id: workflow_id.to_owned(),
            step_id: step_id.to_owned(),
            agent_kind: Some("codex".to_owned()),
            model: Some(String::new()),
            effort: None,
        }
    }

    fn with_overrides(
        overrides: Vec<ProjectStepOverride>,
    ) -> Result<HubProjectFields, ProjectRefusal> {
        project_verdict(ProjectBody {
            step_overrides: overrides,
            ..body("github.com", "acme/widgets")
        })
    }

    #[test]
    fn a_step_override_of_an_agent_step_is_kept_with_an_empty_model_unset() {
        let (workflow, agent, _) = agent_and_other_step();
        let fields = with_overrides(vec![step_override(&workflow, &agent)]).unwrap();
        assert_eq!(
            fields.step_overrides,
            vec![ProjectStepOverride {
                model: None,
                ..step_override(&workflow, &agent)
            }]
        );
    }

    #[test]
    fn a_step_override_with_nothing_to_override_or_nowhere_to_land_is_refused() {
        let (workflow, agent, other) = agent_and_other_step();
        let cases = [
            ("unknown starter", vec![step_override("wf-nope", &agent)]),
            ("unknown step", vec![step_override(&workflow, "s-nope")]),
            (
                "step that runs no agent",
                vec![step_override(&workflow, &other)],
            ),
            (
                "twice",
                vec![
                    step_override(&workflow, &agent),
                    step_override(&workflow, &agent),
                ],
            ),
            (
                "nothing set",
                vec![ProjectStepOverride {
                    agent_kind: None,
                    ..step_override(&workflow, &agent)
                }],
            ),
        ];
        for (case, overrides) in cases {
            assert_eq!(
                with_overrides(overrides),
                Err(ProjectRefusal::StepOverride),
                "{case}"
            );
        }
    }

    #[test]
    fn only_github_is_runner_ready() {
        assert!(runner_ready(GITHUB));
        assert!(!runner_ready(GITLAB));
    }
}
