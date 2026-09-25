//! The fold against a real git repository: an agent's own commit comes out
//! of the history, and what it carried goes through Demeteo's commit under
//! Demeteo's rules.

use super::*;
use crate::adapters::local::execution::LocalSubprocessAdapter;
use crate::adapters::step_executor::artifacts::commit_worktree_changes;
use crate::domain::agent_commit_fold::AgentCommit;
use crate::paths::shell_escape_posix as esc;

const MACHINE: &str = "local";
const SUBTASK_REF: &str = "refs/heads/feat_subtask_t1";
const AGENT: &str = "Agent Person <agent@example.com>";

struct Repo {
    exec: LocalSubprocessAdapter,
    path: String,
}

impl Repo {
    async fn new(label: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "demeteo_test_fold_{label}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let repo = Self {
            exec: LocalSubprocessAdapter::new(),
            path: dir.to_string_lossy().to_string(),
        };
        repo.git("init -b main").await;
        repo.write("src.rs", "fn a() {}\n").await;
        repo.git("add -A").await;
        repo.agent_git("commit -m base").await;
        repo.git("checkout -b feat_subtask_t1").await;
        repo
    }

    async fn git(&self, args: &str) -> String {
        self.exec
            .run_command(MACHINE, &format!("git -C {} {args}", esc(&self.path)))
            .await
            .unwrap_or_else(|e| panic!("git {args}: {e}"))
    }

    /// Git as the agent runs it: the user's identity, not Demeteo's.
    async fn agent_git(&self, args: &str) -> String {
        self.git(&format!(
            "-c user.name='Agent Person' -c user.email=agent@example.com \
             -c commit.gpgsign=false {args}"
        ))
        .await
    }

    async fn write(&self, rel: &str, body: &str) {
        self.exec
            .write_file(MACHINE, &format!("{}/{rel}", self.path), body)
            .await
            .unwrap();
    }

    async fn head(&self) -> String {
        self.git("rev-parse HEAD").await.trim().to_string()
    }

    async fn demeteo_commit(&self) {
        commit_worktree_changes(
            &self.exec,
            MACHINE,
            &self.path,
            "feat(f-1): task t1",
            "artifacts/",
            false,
            &[],
        )
        .await
        .unwrap();
    }

    async fn files_in_head_commit(&self) -> Vec<String> {
        self.git("show --name-only --format= HEAD")
            .await
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(str::to_string)
            .collect()
    }
}

impl Drop for Repo {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn folded(outcome: FoldOutcome) -> Vec<AgentCommit> {
    match outcome {
        FoldOutcome::Folded { commits } => commits,
        other => panic!("expected a fold, got {other:?}"),
    }
}

/// The incident: the agent committed its code *and* its report. After the
/// fold, Demeteo's commit carries the code, not the report, under Demeteo's
/// identity, directly on top of the pre-turn HEAD.
#[tokio::test]
async fn an_agent_commit_carrying_a_report_is_folded_and_the_report_stays_out() {
    let repo = Repo::new("report").await;
    let pre = capture_head(&repo.exec, MACHINE, &repo.path).await;
    let pre_sha = repo.head().await;

    repo.write("src.rs", "fn b() {}\n").await;
    repo.write("artifacts/implementation-report.md", "# done\n")
        .await;
    repo.agent_git("add -A").await;
    repo.agent_git("commit -m 'feat: the agent did it'").await;

    let commits = folded(fold_agent_commits(&repo.exec, MACHINE, &repo.path, &pre).await);
    assert_eq!(commits.len(), 1);
    assert_eq!(commits[0].author, AGENT);
    assert_eq!(commits[0].subject, "feat: the agent did it");

    repo.demeteo_commit().await;

    assert_eq!(repo.git("rev-parse HEAD~1").await.trim(), pre_sha);
    assert_eq!(
        repo.files_in_head_commit().await,
        vec!["src.rs".to_string()]
    );
    assert_eq!(
        repo.git("log -1 --format=%an%x20%ae HEAD").await.trim(),
        "demeteo demeteo@local"
    );
    assert!(repo
        .git("status --porcelain --untracked-files=all")
        .await
        .contains("?? artifacts/implementation-report.md"));
}

/// `--amend` rewrites the pinned commit itself, so the pin is not an
/// ancestor of the agent's HEAD. The fold must still land on the pin.
#[tokio::test]
async fn an_amend_of_the_pinned_commit_is_folded() {
    let repo = Repo::new("amend").await;
    let pre = capture_head(&repo.exec, MACHINE, &repo.path).await;
    let pre_sha = repo.head().await;

    repo.write("src.rs", "fn amended() {}\n").await;
    repo.agent_git("commit -a --amend -m 'base, amended'").await;
    assert_ne!(repo.head().await, pre_sha);

    let commits = folded(fold_agent_commits(&repo.exec, MACHINE, &repo.path, &pre).await);
    assert_eq!(commits.len(), 1);
    assert_eq!(commits[0].subject, "base, amended");
    assert_eq!(repo.head().await, pre_sha);

    repo.demeteo_commit().await;
    assert_eq!(repo.git("rev-parse HEAD~1").await.trim(), pre_sha);
    assert_eq!(
        repo.files_in_head_commit().await,
        vec!["src.rs".to_string()]
    );
}

/// An agent that switched branches and committed there leaves HEAD off the
/// subtask branch; Demeteo's commit would land where no merge looks.
#[tokio::test]
async fn a_branch_switch_is_folded_back_onto_the_subtask_branch() {
    let repo = Repo::new("switch").await;
    let pre = capture_head(&repo.exec, MACHINE, &repo.path).await;
    let pre_sha = repo.head().await;

    repo.agent_git("checkout -b agent-scratch").await;
    repo.write("new.rs", "fn n() {}\n").await;
    repo.agent_git("add -A").await;
    repo.agent_git("commit -m 'scratch work'").await;

    let commits = folded(fold_agent_commits(&repo.exec, MACHINE, &repo.path, &pre).await);
    assert_eq!(commits.len(), 1);
    assert_eq!(repo.git("symbolic-ref HEAD").await.trim(), SUBTASK_REF);
    assert_eq!(repo.head().await, pre_sha);

    repo.demeteo_commit().await;
    assert_eq!(
        repo.git("rev-parse feat_subtask_t1~1").await.trim(),
        pre_sha
    );
    assert_eq!(
        repo.files_in_head_commit().await,
        vec!["new.rs".to_string()]
    );
}

#[tokio::test]
async fn an_agent_that_only_wrote_files_is_left_alone() {
    let repo = Repo::new("clean").await;
    let pre = capture_head(&repo.exec, MACHINE, &repo.path).await;
    repo.write("src.rs", "fn c() {}\n").await;

    assert_eq!(
        fold_agent_commits(&repo.exec, MACHINE, &repo.path, &pre).await,
        FoldOutcome::Clean
    );
    assert!(repo.git("status --porcelain").await.contains("src.rs"));
}

#[tokio::test]
async fn the_pin_names_the_subtask_branch() {
    let repo = Repo::new("pin").await;
    let pre = capture_head(&repo.exec, MACHINE, &repo.path).await;
    assert_eq!(pre.sha.as_deref(), Some(repo.head().await.as_str()));
    assert_eq!(pre.branch.as_deref(), Some(SUBTASK_REF));
}
