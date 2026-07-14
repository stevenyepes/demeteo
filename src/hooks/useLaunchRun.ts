import { useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useNavigation } from '../context';
import { useErrorBus } from '../lib/errorBus';
import type { LaunchStageEntry } from '../components/AttachmentDropzone';
import type { EffortLevel, Feature } from '../types';

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
  stepOverrides?: { step_id: string; agent_kind?: string | null; model?: string | null; effort?: EffortLevel | null }[];
  attachments?: LaunchStageEntry[];
  /** Run detached on this machine via `remote_submit_run`; unset/empty
   * means the local `start_feature` path (which is also the
   * attached-remote path — that routing is a project-level setting). */
  machineId?: string;
  unattended?: boolean;
  maxCostUsd?: number;
  maxWallClockMins?: number;
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

        // Convert staged attachments (which carry a browser File handle
        // or an absolute drag-drop path) into the Rust wire shape. On
        // the local path the orchestrator persists them BEFORE the
        // driver is spawned (see `StepExecutor::feature_start`); on the
        // detached path `remote_submit_run` spools the bytes onto the
        // runner host over SFTP before submitting. Without this batch,
        // the agent's first turn races the post-launch
        // `feature_add_attachment` calls and the user sees "no image
        // attached" responses from a freshly-attached screenshot.
        const stagedAttachments = await Promise.all(
          (params.attachments ?? []).map(async (a) => ({
            source_path: a.sourcePath ?? '',
            mime: a.mime ?? null,
            source_filename: a.source_filename ?? null,
            bytes: a.file ? Array.from(new Uint8Array(await a.file.arrayBuffer())) : null,
          })),
        );

        if (params.machineId) {
          // Detached run (docs/REMOTE_EXECUTION_PLAN.md M6.1): the
          // runner drives it; the laptop keeps an eager shadow Feature
          // (inserted by `remote_submit_run` before the RPC) that the
          // reconcile loop hydrates as the runner reports progress.
          const handle = await invoke<{
            run_id: string;
            machine_id: string;
            status: string;
            feature_id: string;
          }>('remote_submit_run', {
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
            stepOverrides: params.stepOverrides ?? null,
            stagedAttachments,
            // A detached run clones exactly one repository — the first
            // selected repo wins; `null` keeps the project's first.
            targetRepoId: params.targetRepos?.[0] ?? null,
            unattended: params.unattended ?? false,
            maxCostUsd: params.maxCostUsd ?? null,
            maxWallClockSecs:
              params.maxWallClockMins != null ? params.maxWallClockMins * 60 : null,
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

        const res: any = await invoke('start_feature', {
          projectId,
          workflowId: params.workflowId,
          title: params.title,
          description: params.description,
          agentKind: params.agentKind ?? null,
          model: params.model ?? null,
          effort: params.effort ?? null,
          commitArtifacts: params.commitArtifacts ?? null,
          loopIterations: params.loopIterations ?? null,
          stepOverrides: params.stepOverrides ?? null,
          stagedAttachments,
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
