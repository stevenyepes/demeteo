use crate::domain::ids::ProjectId;
use crate::domain::models::{ProjectSettings, StepConfig, WorktreeStrategy};
use crate::domain::prompt_context::PromptContext;
use crate::paths;
use crate::ports::db::ProjectRepository;
use crate::ports::execution::ExecutionPort;

/// The (mostly) sync setup phase that runs before the async execution loop.
/// Returns all the pre-computed values the async loop needs.
#[allow(dead_code)]
pub(crate) struct ExecutionSetup {
    pub project_settings: ProjectSettings,
    pub machine_id_opt: Option<String>,
    pub target_dir: String,
    pub branch_name: String,
    pub slug: String,
    pub base_ctx: PromptContext,
    pub steps: Vec<StepConfig>,
    pub test_cmd: String,
    pub build_cmd: String,
    pub coverage_cmd: String,
    pub conventions_content: String,
    pub repo_list_str: String,
    pub repos: Vec<String>,
}

#[allow(dead_code)]
pub(crate) struct ProjectInfo {
    pub compute_type: String,
    pub remote_host: Option<String>,
    pub repo_path: String,
}

#[allow(dead_code)]
pub(crate) fn resolve_project_info(
    projects: &dyn ProjectRepository,
    project_id: &ProjectId,
) -> Result<ProjectInfo, String> {
    let all = projects.get_projects()?;
    let project = all
        .into_iter()
        .find(|p| p.id == *project_id)
        .ok_or_else(|| format!("Project not found: {}", project_id.0))?;

    let repos = projects.get_repositories_for(project_id)?;
    let repo = repos
        .first()
        .ok_or("No repository associated with this project.")?;

    Ok(ProjectInfo {
        compute_type: project.compute_type,
        remote_host: project.remote_host.as_ref().map(|m| m.0.clone()),
        repo_path: repo.repo_path.clone(),
    })
}

#[allow(dead_code)]
pub(crate) fn resolve_path_probe(
    exec: &dyn ExecutionPort,
    project_info: &ProjectInfo,
    _project_id: &ProjectId,
    target_dir: &str,
) -> Result<(), String> {
    let machine_id_for_check = if project_info.compute_type.to_lowercase() == "local" {
        "local".to_string()
    } else {
        project_info
            .remote_host
            .clone()
            .unwrap_or_else(|| "local".to_string())
    };

    let parent_dir = std::path::Path::new(target_dir)
        .parent()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    let probe = format!(
        "echo __DEMETEO_DIAG__ home=\\\"$HOME\\\" pwd=\\\"$PWD\\\"; \
         ls -la {} 2>&1; \
         test -d {} && echo __DEMETEO_DIAG__ exists || echo __DEMETEO_DIAG__ missing",
        paths::shell_escape_posix(&parent_dir),
        paths::shell_escape_posix(target_dir),
    );
    let probe_output = exec
        .run_command(&machine_id_for_check, &probe)
        .unwrap_or_else(|e| format!("probe failed: {}", e));
    let path_ok = probe_output.contains("__DEMETEO_DIAG__ exists");
    if !path_ok {
        return Err(format!(
            "Repository target dir does not exist on '{}': {}\n\
             Remote diagnostic probe output:\n{}\n\n\
             If the parent dir listing is empty, the bootstrap clone \
             did not actually run for this project — re-save the \
             workspace settings to trigger a fresh bootstrap.",
            machine_id_for_check, target_dir, probe_output
        ));
    }
    Ok(())
}

/// Build the feature-level base PromptContext, shared by every step.
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_base_ctx(
    description: &str,
    slug: &str,
    branch_name: &str,
    repo_list_str: &str,
    test_cmd: &str,
    build_cmd: &str,
    coverage_cmd: &str,
    conventions_content: &str,
    project_memory: &str,
) -> PromptContext {
    PromptContext::new()
        .set("feature_description", description)
        .set("feature_slug", slug)
        .set("feature_branch", branch_name)
        .set("repo_list", repo_list_str)
        .set("test_command", test_cmd)
        .set("build_command", build_cmd)
        .set("coverage_command", coverage_cmd)
        .set("project_conventions", conventions_content)
        .set("project_memory", project_memory)
}

const MAX_SLUG_LEN: usize = 50;

/// Generate a URL-safe slug from a feature description string.
pub(crate) fn slug_from_description(description: &str) -> String {
    let slug = description
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("-")
        .to_lowercase();
    if slug.is_empty() {
        return "feature".to_string();
    }
    if slug.len() <= MAX_SLUG_LEN {
        return slug;
    }
    // Truncate at a hyphen boundary to avoid cutting a word in half.
    let truncated = &slug[..MAX_SLUG_LEN];
    if let Some(last_hyphen) = truncated.rfind('-') {
        // Only trim if we'd actually remove characters (keep at least some)
        if last_hyphen > 1 {
            return truncated[..last_hyphen].to_string();
        }
    }
    truncated.to_string()
}

pub(crate) fn fetch_default_settings() -> ProjectSettings {
    ProjectSettings {
        project_id: ProjectId::from(String::new()),
        worktree_strategy: WorktreeStrategy {
            default_branch: "main".to_string(),
            branch_prefix: "demeteo/features/".to_string(),
            test_command: Some("npm test".to_string()),
            build_command: None,
            coverage_command: None,
            conventions_file: None,
            pr_template: None,
            harnesses: None,
        },
        conflict_policy: "always_gate".to_string(),
        feature_lifecycle: "archive".to_string(),
        default_agent_kind: None,
        default_model: None,
    }
}
