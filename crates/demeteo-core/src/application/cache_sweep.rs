//! Reclaim the caches and worktrees a project's clone leaked. The verdicts are
//! [`crate::domain::cache_sweep`]'s; this module only observes and deletes.
//!
//! Every deletion re-observes its one entry first and asks the domain again. A
//! single cache is tens of gigabytes, so by the fortieth entry the first
//! observation is minutes old — long enough for a feature to be replayed onto a
//! cache, or for provisioning to re-add a worktree at a name it reuses.
//!
//! [`start_cache_sweep`] runs one at startup and another [`SWEEP_INTERVAL`]
//! after each finishes, in the desktop and in `demeteo-runner` alike — the
//! runner is a long-lived service, and a startup-only sweep there is one that
//! never runs again. Sweeps never overlap, the loop's with each other or with
//! one asked for on demand: two of them racing to delete the same tree would
//! each report the other's work as a failure.

use std::collections::HashSet;
use std::time::Duration;

use serde::Serialize;

use crate::adapters::worktree::git_ops::worktree::{
    delete_worktree_residue, reclaim_worktree_path, worktree_list_request,
};
use crate::domain::cache_release::cache_idle_ttl_days;
use crate::domain::cache_sweep::{
    self, KnownFeature, Observation, Reason, SessionActivity, Sibling, SiblingKind, SweepAction,
    Verdict,
};
use crate::domain::ids::ProjectId;
use crate::domain::models::{Notification, NotificationKind};
use crate::ports::execution::SftpEntry;
use crate::ports::notification::DomainEvent;
use crate::state::AppContext;

#[derive(Debug, Clone, Serialize)]
pub struct SweepReport {
    pub dry_run: bool,
    pub projects: Vec<ProjectSweep>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProjectSweep {
    pub project_id: String,
    pub clone_dir: Option<String>,
    /// Set when nothing of this project was judged.
    pub error: Option<String>,
    /// Set when git could not list the worktrees, which keeps every one of them.
    pub worktree_list_error: Option<String>,
    pub entries: Vec<SweepEntry>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SweepEntry {
    pub path: String,
    pub kind: SiblingKind,
    pub verdict: Verdict,
    pub reason: SweepReason,
    /// `None` when no deletion was attempted.
    pub outcome: Option<Outcome>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", content = "detail", rename_all = "snake_case")]
pub enum Outcome {
    Deleted,
    Failed(String),
    /// Due when observed, no longer due when looked at again just before.
    Spared(SweepReason),
}

impl ProjectSweep {
    /// What the bell says of this project's deletions, or `None` when there
    /// were none. Counts, not bytes: sizing a tree means walking it, and a
    /// cache is tens of gigabytes of small files.
    pub fn reclaimed_notice(&self) -> Option<String> {
        let deleted = |kind| {
            self.entries
                .iter()
                .filter(|e| e.kind == kind && matches!(e.outcome, Some(Outcome::Deleted)))
                .count()
        };
        let parts: Vec<String> = [
            (
                deleted(SiblingKind::Cache),
                "dependency cache",
                "dependency caches",
            ),
            (
                deleted(SiblingKind::Worktree),
                "leaked worktree",
                "leaked worktrees",
            ),
        ]
        .into_iter()
        .filter(|(n, _, _)| *n > 0)
        .map(|(n, one, many)| format!("{n} {}", if n == 1 { one } else { many }))
        .collect();
        if parts.is_empty() {
            return None;
        }
        let place = self.clone_dir.as_deref().unwrap_or(&self.project_id);
        Some(format!("Reclaimed {} beside {place}", parts.join(" and ")))
    }
}

/// A [`Reason`] with its [`Reason::describe`] text attached, so a surface
/// prints the domain's own words instead of keeping a copy that drifts.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct SweepReason {
    pub code: Reason,
    pub text: &'static str,
}

impl From<Reason> for SweepReason {
    fn from(code: Reason) -> Self {
        Self {
            code,
            text: code.describe(),
        }
    }
}

impl SweepReport {
    /// Tell the bell what a sweep nobody asked for deleted, one row per
    /// project. A sweep the user ran from Storage reports there instead.
    pub fn notify(&self, ctx: &AppContext) {
        let now = crate::paths::now_ms();
        for project in &self.projects {
            let Some(message) = project.reclaimed_notice() else {
                continue;
            };
            let row = Notification {
                id: format!("notif-caches-{}-{now}", project.project_id),
                project_id: project.project_id.clone(),
                feature_id: String::new(),
                kind: NotificationKind::CachesReclaimed,
                message: message.clone(),
                feature_url: None,
                read: false,
                created_at: now,
            };
            if let Err(error) = ctx.notifications.add(row) {
                eprintln!("[CacheSweep] could not record the notice: {error}");
            }
            let _ = ctx.notif.emit(&DomainEvent::CachesReclaimed {
                project_id: project.project_id.clone(),
                message,
            });
        }
    }

    pub fn log(&self) {
        for project in &self.projects {
            if let Some(error) = &project.error {
                eprintln!("[CacheSweep] project {}: {}", project.project_id, error);
            }
            if let Some(error) = &project.worktree_list_error {
                eprintln!(
                    "[CacheSweep] project {}: worktrees kept, git could not list them: {}",
                    project.project_id, error
                );
            }
            for entry in &project.entries {
                match &entry.outcome {
                    Some(Outcome::Deleted) => {
                        eprintln!("[CacheSweep] deleted {}: {}", entry.path, entry.reason.text)
                    }
                    Some(Outcome::Failed(error)) => {
                        eprintln!("[CacheSweep] failed to delete {}: {}", entry.path, error)
                    }
                    Some(Outcome::Spared(reason)) => {
                        eprintln!("[CacheSweep] spared {}: {}", entry.path, reason.text)
                    }
                    None if entry.verdict == Verdict::Unknown => eprintln!(
                        "[CacheSweep] left {} alone: {}",
                        entry.path, entry.reason.text
                    ),
                    None if entry.verdict == Verdict::Delete => eprintln!(
                        "[CacheSweep] would delete {}: {}",
                        entry.path, entry.reason.text
                    ),
                    None => {}
                }
            }
        }
    }
}

/// The pause between one sweep finishing and the next starting.
pub const SWEEP_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);

/// Process-wide rather than per context: what it guards is the filesystem.
static SWEEPING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Sweep now, and again [`SWEEP_INTERVAL`] after each sweep ends, for the
/// life of the runtime.
pub fn start_cache_sweep(ctx: AppContext, runtime: &tokio::runtime::Handle) {
    runtime.spawn(async move {
        loop {
            match sweep_feature_caches(&ctx, false).await {
                Ok(report) => {
                    report.log();
                    report.notify(&ctx);
                }
                Err(error) => eprintln!("[CacheSweep] sweep failed: {error}"),
            }
            tokio::time::sleep(SWEEP_INTERVAL).await;
        }
    });
}

/// Judge, and unless `dry_run` delete, every leaked sibling of every project's
/// clone. A project that cannot be judged is reported and skipped. Waits for
/// any sweep already under way.
pub async fn sweep_feature_caches(ctx: &AppContext, dry_run: bool) -> Result<SweepReport, String> {
    let _sweeping = SWEEPING.lock().await;
    let all = ctx.projects.get_projects()?;
    let mut projects = Vec::new();
    for (index, project) in all.iter().enumerate() {
        let progress = Progress {
            ctx,
            dry_run,
            project_id: &project.id,
            project_index: index,
            project_count: all.len(),
        };
        progress.emit(None);
        projects.push(sweep_project(ctx, &progress, dry_run).await);
    }
    let _ = ctx.notif.emit(&DomainEvent::CacheSweepProgress {
        dry_run,
        project_id: None,
        project_index: all.len(),
        project_count: all.len(),
        deleting: None,
    });
    Ok(SweepReport { dry_run, projects })
}

struct Progress<'a> {
    ctx: &'a AppContext,
    dry_run: bool,
    project_id: &'a ProjectId,
    project_index: usize,
    project_count: usize,
}

impl Progress<'_> {
    fn emit(&self, deleting: Option<&str>) {
        let _ = self.ctx.notif.emit(&DomainEvent::CacheSweepProgress {
            dry_run: self.dry_run,
            project_id: Some(self.project_id.0.clone()),
            project_index: self.project_index,
            project_count: self.project_count,
            deleting: deleting.map(str::to_owned),
        });
    }
}

async fn sweep_project(ctx: &AppContext, progress: &Progress<'_>, dry_run: bool) -> ProjectSweep {
    let project_id = progress.project_id;
    let mut out = ProjectSweep {
        project_id: project_id.0.clone(),
        clone_dir: None,
        error: None,
        worktree_list_error: None,
        entries: Vec::new(),
    };
    let site = match Site::resolve(ctx, project_id).await {
        Ok(site) => site,
        Err(error) => {
            out.error = Some(error);
            return out;
        }
    };
    out.clone_dir = Some(site.clone_dir.clone());

    let siblings = match ctx.exec.list_dir(&site.machine_id, &site.parent_dir).await {
        Ok(entries) => entries.iter().map(sibling).collect::<Vec<_>>(),
        Err(error) => {
            out.error = Some(format!("could not list {}: {error}", site.parent_dir));
            return out;
        }
    };
    let facts = match site.facts(ctx).await {
        Ok(facts) => facts,
        Err(error) => {
            out.error = Some(error);
            return out;
        }
    };
    out.worktree_list_error = facts.worktree_list_error.clone();

    for action in cache_sweep::plan(&site.observation(&siblings, &facts)) {
        let path = site.sibling_path(&action.name);
        let outcome = match action.verdict {
            Verdict::Delete if !dry_run => {
                progress.emit(Some(&path));
                Some(site.delete_if_still_due(ctx, &action, &path).await)
            }
            _ => None,
        };
        out.entries.push(SweepEntry {
            path,
            kind: action.kind,
            verdict: action.verdict,
            reason: action.reason.into(),
            outcome,
        });
    }
    out
}

struct Site {
    project_id: ProjectId,
    machine_id: String,
    clone_dir: String,
    clone_name: String,
    parent_dir: String,
    other_repos: Vec<String>,
    default_branch: String,
    branch_prefix: String,
    cache_idle_ttl_days: u32,
}

struct Facts {
    registered: Option<HashSet<String>>,
    worktree_list_error: Option<String>,
    features: Vec<KnownFeature>,
    sessions: Option<SessionActivity>,
    now_secs: u64,
}

impl Site {
    async fn resolve(ctx: &AppContext, project_id: &ProjectId) -> Result<Self, String> {
        let clone = crate::application::worktree::resolve_project_clone(ctx, project_id).await?;
        let path = std::path::Path::new(&clone.clone_dir);
        let clone_name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .ok_or_else(|| format!("{} names no directory", clone.clone_dir))?;
        let parent_dir = path
            .parent()
            .map(|p| p.to_string_lossy().into_owned())
            .filter(|p| !p.is_empty())
            .ok_or_else(|| format!("{} has no parent directory", clone.clone_dir))?;
        let other_repos = ctx
            .projects
            .get_repositories_for(project_id)?
            .iter()
            .map(|r| crate::paths::repo_name_from_path(&r.repo_path))
            .filter(|name| name != &clone_name)
            .collect();
        let settings = ctx
            .projects
            .get_settings(project_id)?
            .unwrap_or_else(crate::adapters::step_executor::setup::fetch_default_settings);
        Ok(Self {
            project_id: project_id.clone(),
            machine_id: clone.machine_id,
            clone_dir: clone.clone_dir,
            clone_name,
            parent_dir,
            other_repos,
            default_branch: settings.worktree_strategy.default_branch,
            branch_prefix: settings.worktree_strategy.branch_prefix,
            cache_idle_ttl_days: cache_idle_ttl_days(settings.cache_idle_ttl_days),
        })
    }

    /// The same concatenation that named it — `feature_cache_dir` and the
    /// worktree paths append to the clone path — so the path matches the one
    /// provisioning uses on every host, whatever its separator.
    fn sibling_path(&self, name: &str) -> String {
        format!(
            "{}{}",
            self.clone_dir,
            name.strip_prefix(self.clone_name.as_str()).unwrap_or(name)
        )
    }

    async fn facts(&self, ctx: &AppContext) -> Result<Facts, String> {
        let activity = ctx.features.last_activity_for_project(&self.project_id)?;
        let features = ctx
            .features
            .get_all_for_project(&self.project_id)?
            .iter()
            .map(|f| KnownFeature {
                branch: f.run_branch(&self.branch_prefix),
                status: f.status.clone(),
                mr_state: f.mr_state.clone(),
                last_activity_ms: activity.get(&f.id.0).copied().unwrap_or(f.created_at),
            })
            .collect();
        let (registered, worktree_list_error) = match ctx
            .exec
            .run_program(&self.machine_id, worktree_list_request(&self.clone_dir))
            .await
        {
            Ok(porcelain) => (
                Some(
                    crate::domain::worktree_listing::parse(&porcelain)
                        .all()
                        .map(|wt| cache_sweep::path_basename(&wt.path).to_string())
                        .collect(),
                ),
                None,
            ),
            Err(error) => (None, Some(error)),
        };
        Ok(Facts {
            registered,
            worktree_list_error,
            features,
            sessions: session_activity(ctx, &self.project_id)
                .map_err(|error| {
                    eprintln!(
                        "[CacheSweep] project {}: default-branch cache kept, sessions unreadable: {error}",
                        self.project_id.0
                    )
                })
                .ok(),
            now_secs: now_secs(),
        })
    }

    fn observation<'a>(&'a self, siblings: &'a [Sibling], facts: &'a Facts) -> Observation<'a> {
        Observation {
            clone_name: &self.clone_name,
            other_repos: &self.other_repos,
            siblings,
            registered: facts.registered.as_ref(),
            features: &facts.features,
            default_branch: &self.default_branch,
            sessions: facts.sessions,
            cache_idle_ttl_days: self.cache_idle_ttl_days,
            now_secs: facts.now_secs,
        }
    }

    async fn delete_if_still_due(
        &self,
        ctx: &AppContext,
        action: &SweepAction,
        path: &str,
    ) -> Outcome {
        let entry = match ctx.exec.get_metadata(&self.machine_id, path).await {
            Ok(entry) => entry,
            Err(error) => return Outcome::Failed(format!("could not look at it again: {error}")),
        };
        let facts = match self.facts(ctx).await {
            Ok(facts) => facts,
            Err(error) => return Outcome::Failed(error),
        };
        let fresh = [Sibling {
            name: action.name.clone(),
            ..sibling(&entry)
        }];
        match cache_sweep::plan(&self.observation(&fresh, &facts)).first() {
            Some(again) if again.verdict == Verdict::Delete => {}
            Some(again) => return Outcome::Spared(again.reason.into()),
            None => return Outcome::Spared(action.reason.into()),
        }
        let deleted = match action.kind {
            SiblingKind::Cache => delete_worktree_residue(&*ctx.exec, &self.machine_id, path).await,
            SiblingKind::Worktree => {
                reclaim_worktree_path(&*ctx.exec, &self.machine_id, &self.clone_dir, path).await
            }
        };
        match deleted {
            Ok(()) => Outcome::Deleted,
            Err(error) => Outcome::Failed(error),
        }
    }
}

/// Every Ask thread and Discovery of the project, whichever host it chose: one
/// on another host keeps this clone's cache a little longer, which is the
/// cheap direction to be wrong in.
fn session_activity(ctx: &AppContext, project_id: &ProjectId) -> Result<SessionActivity, String> {
    let holds = |path: &Option<String>| path.as_deref().is_some_and(|p| !p.is_empty());
    let mut activity = SessionActivity {
        holding_worktree: false,
        last_activity_ms: None,
    };
    for thread in ctx.ask.list_for_project(project_id)? {
        activity.holding_worktree |=
            holds(&thread.worktree_path) || ctx.ask_turns.running(&thread.id);
        activity.last_activity_ms = activity.last_activity_ms.max(Some(thread.updated_at));
    }
    for row in ctx.discoveries.list_for_project(project_id)? {
        let discovery = row.discovery;
        activity.holding_worktree |=
            holds(&discovery.worktree_path) || ctx.discovery_turns.running(&discovery.id);
        activity.last_activity_ms = activity.last_activity_ms.max(Some(discovery.updated_at));
    }
    Ok(activity)
}

fn sibling(entry: &SftpEntry) -> Sibling {
    Sibling {
        name: entry.name.clone(),
        is_dir: entry.is_dir,
        // Both transports report an mtime they could not read as zero.
        modified_secs: (entry.modified != 0).then_some(entry.modified),
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
#[path = "../../tests/application/cache_sweep.rs"]
mod tests;
