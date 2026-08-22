import { useCallback } from 'react';
import { startFeature } from '../lib/createProjectWizard';
import { submitRemoteRun } from '../lib/remoteRuns';
import { useNavigation } from '../context';
import { useErrorBus } from '../lib/errorBus';
import { stagedAttachmentInputs } from '../lib/attachments';
import type { LaunchStageEntry } from '../components/AttachmentDropzone';
import type { EffortLevel, Feature, FeatureOrigin } from '../types';

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
  targetRepos?: string[];
  commitArtifacts?: boolean;
  loopIterations?: number;
  /** Per-run override of the per-turn dollar budget (`--max-budget-usd`).
   *  Unset = inherit the project default, then the engine default ($20). */
  maxBudgetUsd?: number;
  stepOverrides?: { step_id: string; agent_kind?: string | null; model?: string | null; effort?: EffortLevel | null }[];
  attachments?: LaunchStageEntry[];
  /** Run detached on this machine via `remote_submit_run`; unset/empty
   * means the local `start_feature` path (which is also the
   * attached-remote path — that routing is a project-level setting). */
  machineId?: string;
  unattended?: boolean;
  maxCostUsd?: number;
  maxWallClockMins?: number;
  /** Where the run's branch is cut from, and what its diff is measured
   *  against (migration V41). Composed by `src/lib/runOrigin.ts`, which omits
   *  both for a run that named neither. */
  origin?: FeatureOrigin;
  diffBaseBranch?: string;
}

/**
 * The one launch code path (ux-audit F28): every composer routes through
 * this hook, and every branch ends the same way — `navigate` to
 * `FeatureDetail` with a real feature id. The detached branch can do
 * that because `remote_submit_run` inserts an eager shadow Feature and
 * returns its id in the handle; there is no separate "remote landing".
 *
 * Returns the launched `Feature` (shadow or local) or `null` on failure
 * (already reported to the error bus) so callers can decide whether to
 * close their composer / clear staged state.
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
  const { reportError } = useErrorBus();

  return useCallback(
    async (params: LaunchRunParams): Promise<Feature | null> => {
      try {
        if (!projectId) {
          throw new Error('No active project to launch a feature in.');
        }

        // Both transports honour the batch before the agent runs: locally
        // `StepExecutor::feature_start` persists it before the driver is
        // spawned, and `remote_submit_run` spools the bytes onto the runner
        // host over SFTP before submitting. Post-launch
        // `feature_add_attachment` calls would race the first turn instead,
        // and the user sees "no image attached" for a screenshot they watched
        // themselves attach.
        const stagedAttachments = await stagedAttachmentInputs(params.attachments ?? []);

        if (params.machineId) {
          // Detached run (docs/REMOTE_EXECUTION.md M6.1): the
          // runner drives it; the laptop keeps an eager shadow Feature
          // (inserted by `remote_submit_run` before the RPC) that the
          // reconcile loop hydrates as the runner reports progress.
          const handle = await submitRemoteRun({
            machineId: params.machineId,
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
            unattended: params.unattended ?? false,
            maxCostUsd: params.maxCostUsd ?? null,
            maxWallClockSecs:
              params.maxWallClockMins != null ? params.maxWallClockMins * 60 : null,
            origin: params.origin,
            diffBaseBranch: params.diffBaseBranch,
          });
          const feature: Feature = {
            id: handle.feature_id,
            project_id: projectId,
            workflow_id: params.workflowId,
            title: params.title,
            status: handle.status || 'pending',
            total_cost: 0,
            tokens: 0,
            duration: '0s',
            created_at: Date.now(),
            agent_kind: params.agentKind,
            model: params.model,
          };
          onLaunched?.(feature);
          navigate({ kind: 'detail', featureId: feature.id, featureTitle: feature.title });
          return feature;
        }

        const res = await startFeature({
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
          origin: params.origin,
          diffBaseBranch: params.diffBaseBranch,
        });
        const feature: Feature = {
          id: res.id,
          project_id: projectId,
          workflow_id: res.workflow_id ?? undefined,
          title: res.title,
          status: res.status,
          total_cost: res.total_cost ?? 0,
          tokens: res.tokens || 0,
          duration: res.duration ?? '0s',
          created_at: res.created_at ?? Date.now(),
          agent_kind: res.agent_kind,
          model: res.model,
        };
        onLaunched?.(feature);
        navigate({ kind: 'detail', featureId: feature.id, featureTitle: feature.title });
        return feature;
      } catch (err) {
        reportError(err);
        return null;
      }
    },
    [projectId, onLaunched, navigate, reportError],
  );
}
