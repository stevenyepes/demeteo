use super::{git_request, GitOpsHelper};
use crate::domain::sync_failure::SyncBlockedStage;
use crate::domain::upstream_feature::{DivergedBranch, DivergenceReconcile, FeatureUpstream};
use crate::ports::execution::ExecutionPort;
use crate::ports::worktree_ops::MergeGate;
use crate::ports::worktree_ops::{SyncFailure, SyncOutcome, SyncWorktreeObserver};

/// One sync of a feature branch: where it runs, which branches, what the
/// merged tree must prove, and what a person already decided about a
/// divergence.
///
/// A parameter object because the alternative is one more positional argument
/// on a call that already takes seven, three of them adjacent strings.
pub struct SyncRequest<'a> {
    pub machine_id: Option<&'a str>,
    pub repo_dir: &'a str,
    pub feature_branch: &'a str,
    pub base_branch: &'a str,
    pub gate: MergeGate<'a>,
    /// `None` is a sync that classifies the divergence for itself.
    pub reconcile: Option<DivergenceReconcile>,
}

impl GitOpsHelper {
    /// Fetch the latest state of `default_branch` from `origin` and update
    /// the local copy of that branch to match. This is the one-time
    /// "snapshot" call used at feature start so the user's local
    /// `<default>` ref doesn't lag behind upstream — which the validate
    /// step would otherwise read as "extra changes not in main" when
    /// comparing the freshly-cut feature branch (which IS based on
    /// `origin/<default>`) against a stale local ref.
    ///
    /// Idempotent and safe to re-invoke: it does not touch any feature
    /// branches, only the local `<default>` ref.
    ///
    /// On success, the local `<default>` ref matches `origin/<default>`.
    /// On a non-fatal fallback (HEAD on `<default>` with a dirty working
    /// tree), the function returns `Err` with a clear message that the
    /// caller surfaces as a soft bootstrap detail. The feature branch is
    /// still cut correctly in the next phase regardless, so the pipeline
    /// always proceeds.
    pub async fn ensure_default_branch_updated(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
        default_branch: &str,
    ) -> Result<(), String> {
        let machine_str = machine_id.unwrap_or(crate::domain::ids::LOCAL_MACHINE);
        // 1. Fetch the latest refs from origin. The fetch is best-effort:
        //    if origin is unreachable, we leave the local branch alone and
        //    warn via stderr (which the executor surfaces to the UI logs).
        let _ = self
            .exec
            .run_program(
                machine_str,
                git_request(repo_dir, ["fetch", "origin", "--", default_branch]),
            )
            .await;

        // 2. Resolve the remote tracking branch (origin/<default>). If
        //    the ref doesn't exist (offline / no remote), bail with a
        //    soft error so the caller can decide to proceed with the
        //    local branch anyway.
        let tracking = format!("origin/{}", default_branch);
        let _ = self
            .exec
            .run_program(
                machine_str,
                git_request(repo_dir, ["rev-parse", "--verify", &tracking]),
            )
            .await
            .map_err(|_| {
                format!(
                    "Local default branch '{}' has no upstream on origin; \
                     proceeding with whatever is local.",
                    default_branch
                )
            })?;

        // 3. Update the local default-branch ref to match origin.
        //    Prefer the ref-only fast-forward (`git fetch origin +src:dst`),
        //    which never moves HEAD or the working tree. This works when
        //    HEAD is on any branch other than `<default>` — and the
        //    previous code relied on this path even when HEAD *was* on
        //    `<default>`, swallowing the inevitable Err ("refusing to
        //    fetch into current branch") and leaving the local ref stale.
        //
        //    The stale local ref was harmless to `create_feature_branch`
        //    (which cuts from `origin/<default>`), but the validate step
        //    in a subtask worktree would see `git log master..HEAD` as
        //    containing every commit upstream has merged since the user's
        //    last manual pull — "extra changes not in main". So when the
        //    ref-only fast-forward is rejected we fall back to a path
        //    that keeps the local ref in sync.
        let fetch_outcome = self
            .exec
            .run_program(
                machine_str,
                git_request(
                    repo_dir,
                    [
                        "fetch",
                        "origin",
                        &format!("+{default_branch}:{default_branch}"),
                    ],
                ),
            )
            .await;
        if fetch_outcome.is_ok() {
            return Ok(());
        }

        // Ref-only fast-forward was rejected (HEAD is on `<default>`,
        // or origin is unreachable — step 1 is best-effort). Try the
        // safe fallback that updates the local ref together with HEAD
        // and the working tree when the checkout state allows it.
        self.fast_forward_local_default_safe(machine_str, repo_dir, default_branch, &tracking)
            .await
    }

    /// Fallback fast-forward of the local `<default>` ref when the
    /// ref-only `git fetch origin +src:dst` is rejected (git refuses to
    /// fetch into a checked-out branch). Branches on the local checkout
    /// state:
    ///
    /// - **HEAD on a different branch**: `git update-ref refs/heads/<default>
    ///   refs/remotes/origin/<default>`. Ref-only, safe because the
    ///   working tree belongs to a different branch — no HEAD, index, or
    ///   working-tree file gets out of sync.
    /// - **HEAD on `<default>` with a clean working tree**: `git merge
    ///   --ff-only origin/<default>`. Fast-forwards HEAD, the index, and
    ///   the working tree in one atomic step.
    /// - **HEAD on `<default>` with a dirty working tree**: return `Err`
    ///   with a clear message — the caller surfaces it as a soft
    ///   bootstrap detail ("local main is N commits behind; please
    ///   `git pull` manually"). The feature branch is still cut
    ///   correctly from `origin/<default>` in the next phase, so the
    ///   pipeline always proceeds.
    /// - **HEAD on `<default>` but local is ahead of origin** (or
    ///   diverged): `git merge --ff-only` rejects non-fast-forwards, so
    ///   the underlying git error is surfaced verbatim.
    async fn fast_forward_local_default_safe(
        &self,
        machine_str: &str,
        repo_dir: &str,
        default_branch: &str,
        tracking: &str,
    ) -> Result<(), String> {
        let head_branch = self
            .exec
            .run_program(
                machine_str,
                git_request(repo_dir, ["rev-parse", "--abbrev-ref", "HEAD"]),
            )
            .await
            .map(|s| s.trim().to_string())
            .unwrap_or_default();

        if head_branch != default_branch {
            // HEAD is on a non-default branch (a feature branch the
            // user checked out to inspect, or a previously-cut feature
            // branch the previous run left behind). The working tree
            // doesn't claim to match `<default>`, so a ref-only update
            // is safe.
            return self
                .exec
                .run_program(
                    machine_str,
                    git_request(
                        repo_dir,
                        [
                            "update-ref",
                            &format!("refs/heads/{default_branch}"),
                            tracking,
                        ],
                    ),
                )
                .await
                .map(|_| ())
                .map_err(|e| {
                    format!(
                        "Could not fast-forward local '{}' via update-ref: {}",
                        default_branch, e
                    )
                });
        }

        // HEAD is on `<default>`. A ref-only update would leave the
        // working tree and index out of sync with the new HEAD, which
        // is a real foot-gun (the user could `git add` + `git commit`
        // files that don't match the parent). Use the proper merge path
        // so HEAD, the index, and the working tree move together.
        let status_porcelain = self
            .exec
            .run_program(
                machine_str,
                git_request(repo_dir, ["status", "--porcelain", "--untracked-files=no"]),
            )
            .await
            .map(|s| s.trim().to_string())
            .unwrap_or_default();

        if !status_porcelain.is_empty() {
            let behind = self
                .exec
                .run_program(
                    machine_str,
                    git_request(
                        repo_dir,
                        ["rev-list", "--count", &format!("HEAD..{tracking}")],
                    ),
                )
                .await
                .ok()
                .and_then(|s| s.trim().parse::<u64>().ok())
                .unwrap_or(0);
            return Err(format!(
                "Local '{}' is {} commit(s) behind origin/{} but the \
                 working tree has uncommitted changes; please `git pull` \
                 manually to keep the local default ref in sync.",
                default_branch, behind, default_branch
            ));
        }

        // Clean working tree, HEAD on `<default>` — fast-forward the
        // whole repo in one step. If the local branch has unpushed
        // commits (so origin isn't strictly ahead), `--ff-only` rejects
        // it and we surface the underlying error verbatim.
        self.exec
            .run_program(
                machine_str,
                git_request(repo_dir, ["merge", "--ff-only", tracking]),
            )
            .await
            .map(|_| ())
            .map_err(|e| {
                format!(
                    "Local '{}' could not be fast-forwarded to origin/{}: {}. \
                     If the local branch has unpushed commits, pull with \
                     `--rebase` or merge manually.",
                    default_branch, default_branch, e
                )
            })
    }

    /// Merge `origin/<base_branch>` into `feature_branch`. This is
    /// the "rebase from the user's perspective" call: it does NOT
    /// rebase (which would rewrite history) — it creates a merge
    /// commit so any in-flight reviewers see a clear fork/join in the
    /// graph. If conflicts arise, returns the list of unmerged files
    /// and leaves the working tree in the conflicted state for the
    /// caller to resolve.
    ///
    /// The `Ok` variant returns the new HEAD commit SHA (so the
    /// caller can record the merge commit in the audit trail). The
    /// `Err` variant says which of the two failures happened —
    /// [`SyncFailure::Conflict`] only when the merge itself left unmerged
    /// paths, and [`SyncFailure::Blocked`] for every stage that stopped short
    /// of that verdict, the merge's own transport and timeout failures
    /// included ([`crate::domain::sync_failure`]).
    ///
    /// `base_branch` is the run's declared base
    /// ([`diff_base::resolve`](crate::domain::diff_base::resolve)), which is
    /// the project's default branch only for a run that started there.
    ///
    /// `observer` is told the merge worktree the instant one exists, which is
    /// the only moment a caller keeping a durable row can learn it: every
    /// failure between here and the verdict returns without one, and an
    /// interrupted sync returns nothing at all.
    ///
    /// Both branches are refreshed from origin first, not only the base: the
    /// local feature ref is fast-forwarded onto `origin/<feature>` where one
    /// exists, and a divergence between them is reconciled or refused on what
    /// patch equivalence says ([`crate::domain::upstream_feature`] carries what
    /// the merge writes without that). `head_before` is therefore the tip
    /// *after* that reconcile — the base of what this sync itself did, which is
    /// the only range a review of it may use ([`SyncOutcome::head_before`]).
    ///
    /// This is the sync that answers the divergence for itself, which is every
    /// caller that has no human in it. [`Self::sync_feature_reconciling`] is
    /// the same sync with that answer supplied.
    pub async fn sync_feature_with_upstream(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
        feature_branch: &str,
        base_branch: &str,
        gate: MergeGate<'_>,
        observer: &dyn SyncWorktreeObserver,
    ) -> Result<SyncOutcome, SyncFailure> {
        self.sync_feature_reconciling(
            SyncRequest {
                machine_id,
                repo_dir,
                feature_branch,
                base_branch,
                gate,
                reconcile: None,
            },
            observer,
        )
        .await
    }

    /// [`Self::sync_feature_with_upstream`], with the divergence already
    /// answered by whoever pressed the button.
    ///
    /// `reconcile` is weighed rather than obeyed
    /// ([`divergence_move`](crate::domain::upstream_feature::divergence_move)):
    /// a merge is taken whatever the branch now looks like, and a reset only
    /// while the reading that made it safe still holds. Everything after the
    /// reconcile — the base merge, the gate, the push — is the ordinary sync,
    /// so a reconcile that conflicts is an ordinary conflicted session and the
    /// resolver reaches it the ordinary way.
    pub async fn sync_feature_reconciling(
        &self,
        req: SyncRequest<'_>,
        observer: &dyn SyncWorktreeObserver,
    ) -> Result<SyncOutcome, SyncFailure> {
        let SyncRequest {
            machine_id,
            repo_dir,
            feature_branch,
            base_branch,
            gate,
            reconcile,
        } = req;
        let machine_str = machine_id.unwrap_or(crate::domain::ids::LOCAL_MACHINE);
        let tracking = format!("origin/{}", base_branch);
        let feat_ref = format!("refs/heads/{}", feature_branch);

        // 1. Refresh remote refs. We use `git fetch <remote> <branch>`
        //    so the local `refs/remotes/origin/<base>` ref is
        //    updated to the latest upstream state. The fetch is
        //    *reported* on failure — silently swallowing it is what
        //    caused the "no conflicts detected" bug where a stale
        //    `origin/<base>` was used as the merge source.
        let fetch_outcome = self
            .exec
            .run_program(
                machine_str,
                git_request(repo_dir, ["fetch", "origin", "--", base_branch]),
            )
            .await;
        if let Err(fetch_err) = fetch_outcome {
            return Err(SyncFailure::Blocked {
                stage: SyncBlockedStage::Fetch,
                raw_error: format!(
                    "Could not fetch origin/{} from remote: {}. \
                     Check the project's remote URL and credentials.",
                    base_branch, fetch_err
                ),
                worktree_path: None,
                head_before: None,
                merge_commit_sha: None,
            });
        }

        // The other half of the refresh, and the one that decides whether the
        // branch being merged *into* is the whole branch
        // ([`crate::domain::upstream_feature`]). Best-effort where the base
        // fetch is fatal: a branch that has never been pushed is the first sync
        // of every feature, and git answers that with the same non-zero exit as
        // a real failure, so blocking on it would block every feature's first
        // sync. What survives the swallow is the ref probe below, which reads
        // whatever this did or did not update.
        let _ = self
            .exec
            .run_program(
                machine_str,
                git_request(repo_dir, ["fetch", "origin", "--", feature_branch]),
            )
            .await;

        // 2. Verify `origin/<base>` exists locally. After a
        //    successful fetch this is guaranteed for any branch the
        //    remote actually has; if the run's base doesn't match a
        //    real upstream branch we surface that as a config error
        //    rather than a silent no-op.
        if self
            .exec
            .run_program(
                machine_str,
                git_request(repo_dir, ["rev-parse", "--verify", &tracking]),
            )
            .await
            .is_err()
        {
            return Err(SyncFailure::Blocked {
                stage: SyncBlockedStage::BaseRefMissing,
                raw_error: format!(
                    "Fetched origin but {} does not exist on the remote. \
                     The branch this run is based on ('{}') may be wrong.",
                    tracking, base_branch
                ),
                worktree_path: None,
                head_before: None,
                merge_commit_sha: None,
            });
        }

        // 3. Refs-only ops (no checkout needed). Use `refs/heads/<feature>`
        //    directly instead of `HEAD` to avoid touching the shared checkout.
        let mut head_before = self
            .exec
            .run_program(machine_str, git_request(repo_dir, ["rev-parse", &feat_ref]))
            .await
            .ok()
            .map(|s| s.trim().to_string());
        let divergence = super::divergence::count_divergence(
            &*self.exec,
            machine_str,
            repo_dir,
            &feat_ref,
            &tracking,
        )
        .await;

        // Only a measured zero is a no-op. A count that did not resolve says
        // nothing about whether upstream moved, and skipping the merge on it
        // costs the user the sync they asked for — where attempting one that
        // turns out to have nothing to do costs a worktree.
        if divergence.behind == Some(0) {
            return Ok(SyncOutcome {
                merge_commit_sha: head_before.clone(),
                changed: false,
                head_before,
            });
        }

        // Reconcile the feature branch with its own upstream, so the base is
        // merged into everything that branch holds rather than into whatever
        // Demeteo last left in this clone.
        let upstream =
            super::divergence::feature_upstream(&*self.exec, machine_str, repo_dir, feature_branch)
                .await;
        let mut reconcile_with_origin = None;
        if let Some(FeatureUpstream::Diverged { ahead, behind }) = upstream {
            // The counts say the two sides disagree; only patch equivalence
            // says which disagreement this is, and the answers are different
            // moves ([`crate::domain::upstream_feature`]).
            let cherry = super::divergence::patch_equivalence(
                &*self.exec,
                machine_str,
                repo_dir,
                feature_branch,
            )
            .await;
            match crate::domain::upstream_feature::divergence_move(
                DivergedBranch {
                    feature: feature_branch,
                    base: base_branch,
                    ahead,
                    behind,
                },
                reconcile,
                cherry.as_deref(),
            ) {
                Ok(move_) => reconcile_with_origin = Some(move_),
                // Stopped before a worktree exists, because a divergence nobody
                // may act on has nothing at risk in a tree: no merge was
                // attempted, so there is nothing for the user to look at and
                // nothing to reclaim.
                Err(raw_error) => {
                    return Err(SyncFailure::Blocked {
                        stage: SyncBlockedStage::FeatureDiverged,
                        raw_error,
                        worktree_path: None,
                        head_before,
                        merge_commit_sha: None,
                    });
                }
            }
        }

        // Do the merge in a temporary worktree (not the main repo) so
        // concurrent features cannot race on the shared checkout.
        let wt_path = self
            .provision_sync_worktree(Some(machine_str), repo_dir, feature_branch, gate)
            .await
            .map_err(|e| SyncFailure::Blocked {
                stage: SyncBlockedStage::WorktreeProvision,
                raw_error: e,
                worktree_path: None,
                head_before: head_before.clone(),
                merge_commit_sha: None,
            })?;
        observer.provisioned(&wt_path);

        // The fast-forward runs here, in the checkout, rather than as a
        // `update-ref refs/heads/<feature>` in the clone: `provision_sync_worktree`
        // answers `repo_dir` itself when the feature branch is already checked
        // out there, and moving that ref out from under a checked-out tree
        // leaves the index and the working tree describing a commit that is no
        // longer HEAD. `git merge --ff-only` moves all three together wherever
        // it runs, and refuses out loud when no fast-forward exists — which is
        // the same divergence the counts above look for, read a second time
        // from git itself and reached even when `rev-list` could not answer.
        if upstream == Some(FeatureUpstream::FastForward) {
            let feat_tracking = format!("origin/{}", feature_branch);
            if let Err(e) = self
                .exec
                .run_program(
                    machine_str,
                    git_request(&wt_path, ["merge", "--ff-only", &feat_tracking]),
                )
                .await
            {
                return Err(SyncFailure::Blocked {
                    stage: SyncBlockedStage::FeatureDiverged,
                    raw_error: crate::domain::upstream_feature::unmergeable_refusal(
                        feature_branch,
                        base_branch,
                        &e,
                    ),
                    worktree_path: Some(wt_path.clone()),
                    head_before,
                    merge_commit_sha: None,
                });
            }
            head_before = reconciled_tip(&*self.exec, machine_str, &wt_path).await;
        }

        // The other reconcile, over a divergence the counts alone could not
        // settle: either a merge, over commits neither side has ever had, or
        // the reset a person pressed over a branch origin rewrote. Both run in
        // the checkout, for the reason the fast-forward above gives.
        let mut reconciled = false;
        if let Some(move_) = reconcile_with_origin {
            let feat_tracking = format!("origin/{}", feature_branch);
            let message = format!("chore(sync): reconcile {} with origin", feature_branch);
            // `--keep` and not `--hard`: `provision_sync_worktree` answers
            // `repo_dir` itself when the feature branch is checked out there,
            // so the tree this discards can be the user's own. `--keep` moves
            // the branch and refuses out loud over an edit it would destroy,
            // which is the same shape as the `--ff-only` above.
            let attempt = match move_ {
                DivergenceReconcile::ResetOntoOrigin => {
                    git_request(&wt_path, ["reset", "--keep", &feat_tracking])
                }
                DivergenceReconcile::MergeOrigin => {
                    git_request(&wt_path, ["merge", &feat_tracking, "-m", &message])
                }
            };
            if let Err(raw) = self.exec.run_program(machine_str, attempt).await {
                return Err(
                    match crate::domain::sync_failure::reconcile_failure_stage(move_, &raw) {
                        Some(stage) => SyncFailure::Blocked {
                            stage,
                            raw_error: raw,
                            worktree_path: Some(wt_path.clone()),
                            head_before,
                            merge_commit_sha: None,
                        },
                        None => SyncFailure::Conflict {
                            files: parse_unmerged_files(&*self.exec, machine_str, &wt_path).await,
                            raw_error: raw,
                            worktree_path: Some(wt_path.clone()),
                            head_before,
                            resolves_the_base_merge: false,
                        },
                    },
                );
            }
            let tip = reconciled_tip(&*self.exec, machine_str, &wt_path).await;
            // An unread tip on either side reads as moved, for the reason the
            // base merge's own `changed` gives below. The two are separate
            // because `head_before` is deliberately the *post*-reconcile tip —
            // the review diff and Discard are both about the commit this sync
            // wrote — which leaves a reconcile that committed and a base merge
            // that had nothing to do indistinguishable from a sync that did
            // nothing at all.
            reconciled = match (head_before.as_deref(), tip.as_deref()) {
                (Some(before), Some(after)) => before != after,
                _ => true,
            };
            head_before = tip;
        }

        let merge_out = self
            .exec
            .run_program(
                machine_str,
                git_request(
                    &wt_path,
                    [
                        "merge",
                        &tracking,
                        "-m",
                        &format!("chore(sync): sync feature with origin/{base_branch}"),
                    ],
                ),
            )
            .await;

        let result = match merge_out {
            Ok(_) => {
                let head_after = self
                    .exec
                    .run_program(machine_str, git_request(&wt_path, ["rev-parse", "HEAD"]))
                    .await
                    .ok()
                    .map(|s| s.trim().to_string());
                // An unread tip on either side cannot say the tip stayed put,
                // so it reads as moved. The two mistakes are not the same size:
                // a push of a ref already at that commit is a no-op, where
                // withholding one leaves a real merge on the local branch and
                // the open pull request never sees it.
                //
                // What that rule may *not* do is name the commit. An unread
                // `rev-parse HEAD` flattened to `""` was stored as this sync's
                // merge commit, and an empty sha passes every `is some` guard
                // between here and the pane: Publish is offered, the push runs,
                // and `git merge-base --is-ancestor '' …` then refuses forever,
                // so the user is told their push did not land about one that
                // did.
                let changed = reconciled
                    || match (head_before.as_deref(), head_after.as_deref()) {
                        (Some(before), Some(after)) => before != after,
                        _ => true,
                    };

                if changed {
                    // Between the commit and the push, because those are the
                    // only two points where the answer can still change
                    // anything: before the merge there is no tree to ask about,
                    // and after the push the pull request has already seen it.
                    if let Some(raw_error) = super::sync_verify::merge_gate_refusal(
                        &*self.exec,
                        machine_str,
                        gate,
                        crate::adapters::step_executor::harness_shell::harness_shell_options(
                            self.app_settings.as_ref(),
                            &wt_path,
                        ),
                    )
                    .await
                    {
                        return Err(SyncFailure::Blocked {
                            stage: SyncBlockedStage::Verify,
                            raw_error,
                            worktree_path: Some(wt_path.clone()),
                            head_before: head_before.clone(),
                            merge_commit_sha: head_after.clone(),
                        });
                    }

                    // Push the successful clean merge to origin so remote MR is updated
                    let credential = crate::adapters::git_push::credential_for_repo(
                        &*self.exec,
                        self.app_settings.as_ref(),
                        machine_str,
                        &wt_path,
                    )
                    .await;
                    if let Err(push_err) = self
                        .exec
                        .run_program(
                            machine_str,
                            crate::adapters::git_push::push_request(
                                &wt_path,
                                feature_branch,
                                false,
                                credential.as_ref(),
                            ),
                        )
                        .await
                    {
                        return Err(SyncFailure::Blocked {
                            stage: SyncBlockedStage::Push,
                            raw_error: format!(
                                "Sync merge succeeded locally but pushing to origin failed: {}",
                                crate::adapters::git_push::push_failure(
                                    &push_err,
                                    credential.as_ref()
                                )
                            ),
                            worktree_path: Some(wt_path.clone()),
                            head_before: head_before.clone(),
                            merge_commit_sha: head_after.clone(),
                        });
                    }
                }

                Ok(SyncOutcome {
                    merge_commit_sha: head_after.clone(),
                    changed,
                    head_before: head_before.clone(),
                })
            }
            Err(raw) => match crate::domain::sync_failure::merge_failure_stage(&raw) {
                Some(stage) => Err(SyncFailure::Blocked {
                    stage,
                    raw_error: raw,
                    worktree_path: Some(wt_path.clone()),
                    head_before: head_before.clone(),
                    merge_commit_sha: None,
                }),
                None => {
                    // The merge left the worktree in a conflicted state.
                    // Parse `git status` in the worktree for the unmerged files.
                    let files = parse_unmerged_files(&*self.exec, machine_str, &wt_path).await;
                    Err(SyncFailure::Conflict {
                        files,
                        raw_error: raw,
                        worktree_path: Some(wt_path.clone()),
                        head_before: head_before.clone(),
                        resolves_the_base_merge: true,
                    })
                }
            },
        };

        // If we used the main repo directly (no worktree), skip cleanup.
        // Otherwise, on success, remove the temp worktree; on conflict,
        // leave it in place for the resolution agent.
        if wt_path != repo_dir && result.is_ok() {
            let _ = self
                .exec
                .run_program(
                    machine_str,
                    git_request(repo_dir, ["worktree", "remove", "--force", &wt_path]),
                )
                .await;
            let _ = self.exec.remove_dir_all(machine_str, &wt_path).await;
            let _ = self
                .exec
                .run_program(machine_str, git_request(repo_dir, ["worktree", "prune"]))
                .await;
        }

        result
    }

    /// Provision a temporary linked worktree for a sync merge.
    /// The worktree has `<feature_branch>` checked out.
    ///
    /// If `<feature_branch>` is already the currently checked-out
    /// branch in the main repo, returns `repo_dir` directly
    /// (no worktree needed). The caller MUST skip worktree
    /// cleanup when the returned path equals `repo_dir`.
    async fn provision_sync_worktree(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
        feature_branch: &str,
        gate: MergeGate<'_>,
    ) -> Result<String, String> {
        let machine_str = machine_id.unwrap_or(crate::domain::ids::LOCAL_MACHINE);

        // If the main repo already has the feature branch checked
        // out, we can merge in place — no worktree needed.
        let current_branch = self
            .get_head_branch(Some(machine_str), repo_dir)
            .await
            .unwrap_or_default();
        if current_branch == feature_branch {
            return Ok(repo_dir.to_string());
        }

        // Clean up any stale sync worktrees checked out on this branch
        if let Ok(worktrees) = self.list_worktrees(Some(machine_str), repo_dir).await {
            for wt in worktrees {
                if wt.branch.as_deref() == Some(feature_branch) && wt.path.contains("_wt_sync") {
                    let _ = self
                        .exec
                        .run_program(
                            machine_str,
                            git_request(repo_dir, ["worktree", "remove", "--force", &wt.path]),
                        )
                        .await;
                    let _ = self.exec.remove_dir_all(machine_str, &wt.path).await;
                }
            }
            let _ = self
                .exec
                .run_program(machine_str, git_request(repo_dir, ["worktree", "prune"]))
                .await;
        }

        let wt_path = crate::paths::sync_worktree_dir(
            repo_dir,
            feature_branch,
            crate::paths::targets_windows_host(machine_str),
        );

        // Force remove any pre-existing worktree at that path to avoid collisions
        let _ = self
            .exec
            .run_program(
                machine_str,
                git_request(repo_dir, ["worktree", "remove", "--force", &wt_path]),
            )
            .await;
        let _ = self.exec.remove_dir_all(machine_str, &wt_path).await;
        let _ = self
            .exec
            .run_program(machine_str, git_request(repo_dir, ["worktree", "prune"]))
            .await;

        self.exec
            .run_program(
                machine_str,
                git_request(repo_dir, ["worktree", "add", &wt_path, feature_branch]),
            )
            .await
            .map_err(|e| {
                format!(
                    "Failed to create sync worktree for '{}': {}",
                    feature_branch, e
                )
            })?;

        // Only for a sync that is going to run something. A merge worktree is
        // throwaway and nothing else in it reads a dependency tree, so the
        // links are pure cost until a harness needs `node_modules` to resolve
        // an import — and without them every gated sync of a JS project would
        // report a red build about the install it was never given, which is
        // the false red `sync_verify` refuses to produce from the other side.
        if !gate.is_empty() {
            super::worktree::share_dependency_caches(
                self.exec.as_ref(),
                machine_str,
                repo_dir,
                &wt_path,
                &crate::paths::feature_cache_dir(repo_dir, feature_branch),
                crate::paths::targets_windows_host(machine_str),
            )
            .await;
        }

        Ok(wt_path)
    }
}

/// The feature branch's tip after a reconcile with `origin/<feature>`, read in
/// the worktree that performed it.
///
/// The tip read *before* a reconcile is no longer a base the sync may be
/// reviewed or undone against: `head_before..merge` would put origin's own
/// commits inside the diff of what this sync did, and the `reset --hard
/// head_before` behind Discard would rewind the branch past them. An unread tip
/// is therefore `None` and not the stale value — a sync that offers no review
/// is recoverable, one that offers the wrong one is acted on.
async fn reconciled_tip(
    exec: &dyn ExecutionPort,
    machine_id: &str,
    wt_path: &str,
) -> Option<String> {
    exec.run_program(machine_id, git_request(wt_path, ["rev-parse", "HEAD"]))
        .await
        .ok()
        .map(|s| s.trim().to_string())
}

/// Ask `repo_dir` which files a merge left unresolved.
///
/// Shared by the sync flow and the existing `merge_subtask` conflict path so
/// both produce the same `ConflictFile` shape — the XY-code table itself lives
/// in [`crate::domain::merge_status`], which the step executor reads too.
pub(crate) async fn parse_unmerged_files(
    exec: &dyn ExecutionPort,
    machine_id: &str,
    repo_dir: &str,
) -> Vec<crate::domain::models::ConflictFile> {
    let raw = match exec
        .run_program(
            machine_id,
            git_request(repo_dir, ["status", "--porcelain", "--untracked-files=no"]),
        )
        .await
    {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    crate::domain::merge_status::parse_unmerged(&raw)
}

#[cfg(test)]
#[path = "../../../../tests/infrastructure/worktree/git_ops/sync.rs"]
mod tests;
