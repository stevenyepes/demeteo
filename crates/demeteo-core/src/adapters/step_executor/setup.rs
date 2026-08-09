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

/// The three commands a project configures for its own gates.
///
/// Bundled because they are three adjacent `&str`s that mean different things:
/// positionally, transposing two of them compiles, renders, and produces a
/// wrong prompt for the rest of the run. Naming them makes that impossible.
pub(crate) struct ProjectCommands<'a> {
    pub test: &'a str,
    pub build: &'a str,
    pub coverage: &'a str,
}

/// How the feature names itself: to a human, to a path, to git.
pub(crate) struct FeatureIdentity<'a> {
    pub description: &'a str,
    pub slug: &'a str,
    pub branch_name: &'a str,
}

/// Build the feature-level base PromptContext, shared by every step.
pub(crate) fn build_base_ctx(
    feature: FeatureIdentity<'_>,
    repo_list_str: &str,
    commands: ProjectCommands<'_>,
    conventions_content: &str,
    project_memory: &str,
    artifact_dir: &str,
    session_resume_summary: &str,
) -> PromptContext {
    PromptContext::new()
        .set("feature_description", feature.description)
        .set("feature_slug", feature.slug)
        .set("feature_branch", feature.branch_name)
        .set("repo_list", repo_list_str)
        .set("test_command", commands.test)
        .set("build_command", commands.build)
        .set("coverage_command", commands.coverage)
        .set("project_conventions", conventions_content)
        .set("project_memory", project_memory)
        .set("artifact_dir", artifact_dir)
        // `report_dir` is the clearer-name alias for `artifact_dir`. The
        // `{{report_dir}}` token is what new workflows should use — it
        // describes the folder's role (per-step change-summary report,
        // surfaced in the UI as an artifact, NOT the deliverable) instead
        // of the misleading historical `artifact_dir`. Both names resolve
        // to the same value so old `{{artifact_dir}}` templates keep
        // rendering unchanged. `PromptContext`'s
        // `collapse_unknown_placeholders` reduces unknown tokens to "", so
        // dropping the alias is safe once every starter workflow + UI hint
        // has migrated.
        .set("report_dir", artifact_dir)
        .set("session_resume_summary", session_resume_summary)
}

/// Probe the feature worktree's state as a comparable fingerprint
/// (P1.14): `"<repo HEAD>:<dirty|clean>"`. Recorded on every
/// `step_attempts` row at node start; on resume of an interrupted node,
/// a mismatch against the live workspace surfaces as the Decision-14
/// synthetic gate instead of blind re-execution.
///
/// `None` on any probe failure (no repo yet, dead transport, git
/// missing) — a fingerprint that can't be read must never block a run,
/// so callers treat `None` as "unknown, proceed".
///
/// Dirtiness uses plain `git status --porcelain`, untracked included:
/// agent steps write uncommitted artifacts mid-feature, so mid-run
/// fingerprints are routinely `dirty` on both sides of a compare — the
/// signal is the *change* of the pair, not dirtiness itself.
pub(crate) async fn workspace_fingerprint(
    exec: &dyn ExecutionPort,
    machine_id: &str,
    target_dir: &str,
) -> Option<String> {
    let d = paths::shell_escape_posix(target_dir);
    let script = format!(
        "cd {d} && git rev-parse HEAD 2>/dev/null && git status --porcelain 2>/dev/null | head -1"
    );
    let out = exec.run_command(machine_id, &script).await.ok()?;
    let mut lines = out.lines();
    let head = lines.next()?.trim();
    // `git rev-parse HEAD` yields a 40-hex sha; anything else means the
    // probe ran in a broken repo — treat as unknown.
    if head.len() != 40 || !head.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let dirty = lines.next().is_some_and(|l| !l.trim().is_empty());
    Some(format!("{head}:{}", if dirty { "dirty" } else { "clean" }))
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

pub fn fetch_default_settings() -> ProjectSettings {
    ProjectSettings {
        default_effort: None,
        default_workflow_id: None,
        project_id: ProjectId::from(String::new()),
        worktree_strategy: WorktreeStrategy {
            default_branch: "main".to_string(),
            branch_prefix: "demeteo/features/".to_string(),
            // Deliberately `None`, not a guess. This is the fallback for a
            // project with *no saved settings row* — we know nothing about it,
            // not even its ecosystem, so `npm test` was never justified: a real
            // project gets its commands from `detect_worktree_strategy` at
            // bootstrap, which at least reads the repo.
            //
            // It used to be a cheap wrong answer that failed late at
            // `s-validate`. Since HB1's preflight it is an expensive one: the
            // launch is *blocked* when the binary does not resolve, which is
            // precisely the false positive `preflight.rs` is built to avoid —
            // refusing to start work over a command the user never configured.
            // `run-topology-conformance.sh` caught exactly that, and `npm run
            // checks` did not.
            test_command: None,
            build_command: None,
            coverage_command: None,
            conventions_file: None,
            pr_template: None,
            harnesses: None,
            validation_gates: None,
            prepare_command: None,
            extra_writable_paths: Vec::new(),
        },
        conflict_policy: "always_gate".to_string(),
        feature_lifecycle: "archive".to_string(),
        default_agent_kind: None,
        default_model: None,
        artifact_subdir: "artifacts/".to_string(),
        commit_artifacts: false,
        default_loop_iterations: None,
        default_max_budget_usd: None,
    }
}
