//! What a Hub Project may hold, judged before anything is written. Pure: the
//! handlers ask this and the store stores what it returns.
//!
//! The repository rules are the ones a runner start was held to when its
//! repository came from a desktop's `remote_url`: a runner clones
//! `https://{host}/{full_name}` as given, so a pair that does not round-trip
//! through that URL is refused rather than repaired.

use crate::store::hub_projects::HubProjectFields;
use crate::workflows;
use demeteo_hub_protocol::Effort;
use serde::Deserialize;

pub const MAX_NAME_CHARS: usize = 200;

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
    })
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

    #[test]
    fn only_github_is_runner_ready() {
        assert!(runner_ready(GITHUB));
        assert!(!runner_ready(GITLAB));
    }
}
