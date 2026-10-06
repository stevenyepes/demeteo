import { useCallback } from 'react';
import { launchRun, reportDegradedLaunch } from '../lib/launch';
import { useNavigation, useProject, useUIState } from '../context';
import { useErrorBus } from '../lib/errorBus';
import { stagedAttachmentInputs } from '../lib/attachments';
import { invalidateRunnerCompatibility, isRunnerIncompatibleError } from '../lib/runnerCompatibility';
import type { LaunchStageEntry } from '../components/AttachmentDropzone';
import type { EffortLevel, Feature, FeatureOrigin, StepOverride } from '../types';

/** Launch parameters — the union of what `StartFeatureModal` and the
 * ProjectHome composer collect. Matches the modal's `onLaunch` shape. */
export interface LaunchRunParams {
  workflowId: string;
  title: string;
  description: string;
  agentKind?: string;
  model?: string;
  /** Feature-wide reasoning effort. Unset = inherit the project default,
   *  which bottoms out at the engine default (`high`). */
  effort?: EffortLevel;
  /** Detached-only, like `unattended` and the two caps: `launch_run` refuses
   *  any of them on a local launch, so a composer sets them only alongside a
   *  `machineId`. */
  targetRepos?: string[];
  commitArtifacts?: boolean;
  loopIterations?: number;
  /** Per-run override of the per-turn dollar budget (`--max-budget-usd`).
   *  Unset = inherit the project default, then the engine default ($20). */
  maxBudgetUsd?: number;
  stepOverrides?: StepOverride[];
  attachments?: LaunchStageEntry[];
  /** Run detached on this machine; unset/empty runs on the project's own
   * compute (which is also the attached-remote path — that routing is a
   * project-level setting). */
  machineId?: string;
  /** Unset = a detached run's default, which is unattended. */
  unattended?: boolean;
  maxCostUsd?: number;
  maxWallClockMins?: number;
  /** Where the run's branch is cut from, and what its diff is measured
   *  against (migration V41). Composed by `src/lib/runOrigin.ts`, which omits
   *  both for a run that named neither. */
  origin?: FeatureOrigin;
  diffBaseBranch?: string;
}

/** Per-call hooks for the one caller that needs more than `Feature | null`. */
export interface LaunchRunOptions {
  /** The runner refused the detached submit as version-incompatible, already
   *  reported and its cached verdict dropped — the caller's cue to re-probe. */
  onRunnerRefused?: () => void;
}

/**
 * The one launch code path (ux-audit F28): every composer routes through
 * this hook, and every launch ends the same way — `navigate` to
 * `FeatureDetail` with a real feature id. `launch_run` decides where the run
 * goes and returns its Feature either way (for a detached run, the eager
 * shadow row), so there is no separate "remote landing" and no placement test
 * here.
 *
 * Returns the launched `Feature` (shadow or local) or `null` on failure
 * (already reported to the error bus) so callers can decide whether to
 * close their composer / clear staged state. A run the runner accepted but
 * left degraded is a launch, not a failure: it is reported as a notice and
 * still lands.
 */
export function useLaunchRun(options: {
  projectId: string | null;
  /** Called with the new feature before navigation — used to pre-seed
   * feature lists (Cmd+G cycling in App, the pipeline list in
   * ProjectHome) without waiting for the next fetch. */
  onLaunched?: (feature: Feature) => void;
}) {
  const { projectId, onLaunched } = options;
  const { navigate } = useNavigation();
  const { refreshProjectActivity } = useProject();
  const { reportError } = useErrorBus();
  const { uiDispatch } = useUIState();

  return useCallback(
    async (params: LaunchRunParams, launchOptions?: LaunchRunOptions): Promise<Feature | null> => {
      try {
        if (!projectId) {
          throw new Error('No active project to launch a feature in.');
        }

        // Both transports honour the batch before the agent runs: locally
        // `StepExecutor::feature_start` persists it before the driver is
        // spawned, and a detached submit spools the bytes onto the runner
        // host over SFTP before submitting. Post-launch
        // `feature_add_attachment` calls would race the first turn instead,
        // and the user sees "no image attached" for a screenshot they watched
        // themselves attach.
        const stagedAttachments = await stagedAttachmentInputs(params.attachments ?? []);

        const feature = await launchRun({
          machineId: params.machineId ?? null,
          projectId,
          workflowId: params.workflowId,
          title: params.title,
          description: params.description,
          agentKind: params.agentKind ?? null,
          model: params.model ?? null,
          effort: params.effort ?? null,
          commitArtifacts: params.commitArtifacts ?? null,
          loopIterations: params.loopIterations ?? null,
          maxBudgetUsd: params.maxBudgetUsd ?? null,
          stepOverrides: params.stepOverrides ?? null,
          stagedAttachments,
          // A detached run clones exactly one repository — the first
          // selected repo wins; `null` keeps the project's first.
          targetRepoId: params.targetRepos?.[0] ?? null,
          unattended: params.unattended,
          maxCostUsd: params.maxCostUsd ?? null,
          maxWallClockSecs: params.maxWallClockMins != null ? params.maxWallClockMins * 60 : null,
          origin: params.origin,
          diffBaseBranch: params.diffBaseBranch,
        });
        // Inserted at `bootstrapping` with no event; the first one comes after
        // worktree, branch and preflight.
        onLaunched?.(feature);
        refreshProjectActivity();
        navigate({ kind: 'detail', featureId: feature.id, featureTitle: feature.title });
        reportDegradedLaunch(feature, reportError, () => navigate({ kind: 'remote-inbox' }));
        return feature;
      } catch (err) {
        if (params.machineId && isRunnerIncompatibleError(err)) {
          // The refusal is fresher than any cached verdict that let Start enable.
          invalidateRunnerCompatibility(params.machineId);
          reportError(err, {
            action: {
              label: 'Open machine settings',
              // The toast sits above the Start Feature modal, which stays over
              // every view until something closes it (see `StartFeatureHost`).
              // `params` is not that modal's draft unless it launched them, so
              // the modal is asked to leave and keeps its own.
              onClick: () => {
                uiDispatch({ type: 'LEAVE_START_FEATURE' });
                navigate({ kind: 'settings' });
              },
            },
          });
          launchOptions?.onRunnerRefused?.();
        } else {
          reportError(err);
        }
        return null;
      }
    },
    [projectId, onLaunched, navigate, refreshProjectActivity, reportError, uiDispatch],
  );
}
