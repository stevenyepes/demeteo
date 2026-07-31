import { useCallback, useState } from 'react';
import { setMachineSecret } from '../../lib/machines';
import {
  bootstrapProject, createProject, providerCreateRepo, wizardError,
  type CreateRepoRequest, type CreatedRepo,
} from '../../lib/createProjectWizard';
import { saveProjectSettings } from '../../lib/project';
import { formatError } from '../../lib/errors';
import type { EffortLevel, WorktreeStrategy } from '../../types';
import type { BootstrapPhase, BootstrapPhaseState } from './CreateZeroBootstrapPanel';

/** Initial phase list — every phase starts in `pending`. The hook
 *  flips each entry to `running` when it begins and to `done` when it
 *  finishes. A failure short-circuits the rest. */
function initialPhases(): BootstrapPhaseState[] {
  const order: BootstrapPhase[] = ['create_repo', 'create_project', 'bootstrap', 'save_settings'];
  return order.map((id) => ({
    id,
    label: id === 'create_repo'
      ? 'Creating repository on provider'
      : id === 'create_project'
        ? 'Registering project record'
        : id === 'bootstrap'
          ? 'Cloning repository & detecting strategy'
          : 'Persisting project settings',
    status: 'pending',
  }));
}

export interface BootstrapInput {
  projectName: string;
  providerId: string;
  /** Host captured on the Provider step; forwarded as `provider_host`
   *  to sub-1's HTTP adapter so it can route to a self-hosted
   *  enterprise host (e.g. `https://gh.corp.example.com`). When
   *  omitted the backend falls back to the provider's configured
   *  default. */
  providerHost?: string;
  namespaceId: string;
  repoSlug: string;
  repoPrivate: boolean;
  machineKind: 'local' | 'remote';
  machineId: string;
  keyPassphrase: string;
  agentKind: string;
  model: string;
  /** Project-wide default reasoning effort. `null` = no project default,
   *  which resolves to the engine default (`high`) at run time. */
  effort?: EffortLevel | null;
}

export interface BootstrapSuccess {
  projectId: string;
  repo: CreatedRepo;
  strategy: WorktreeStrategy;
}

export interface UseCreateZeroBootstrapApi {
  phases: BootstrapPhaseState[];
  logs: ReadonlyArray<string>;
  error: string | null;
  /** Whether a bootstrap run is currently in flight. */
  running: boolean;
  /** Run the bootstrap pipeline. Resolves with the success payload on
   *  completion, or `null` on failure (the error is stored on the
   *  hook's state and rendered by the panel). */
  run: (
    input: BootstrapInput,
    onSuccess: (result: BootstrapSuccess) => void,
  ) => Promise<void>;
}

/**
 * Drives the four-stage bootstrap pipeline that backs the
 * Create-From-Zero wizard's "Create & bootstrap" CTA:
 *
 *   create_repo → create_project → bootstrap → save_settings
 *
 * Mirrors the create-project → bootstrap-project → save-settings
 * chain from `NewProjectView.tsx` so the wizard reuses the same
 * backend pipeline. Returns pure React state — the wizard owns the
 * navigation transitions and the global project dispatch.
 */
export function useCreateZeroBootstrap(): UseCreateZeroBootstrapApi {
  const [phases, setPhases] = useState<BootstrapPhaseState[]>(initialPhases);
  const [logs, setLogs] = useState<string[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [running, setRunning] = useState(false);

  const appendLog = useCallback((line: string) => {
    setLogs((prev) => (prev.length > 200 ? [...prev.slice(-200), line] : [...prev, line]));
  }, []);

  const setPhaseStatus = useCallback((id: BootstrapPhase, status: BootstrapPhaseState['status']) => {
    setPhases((prev) => prev.map((p) => (p.id === id ? { ...p, status } : p)));
  }, []);

  const run = useCallback(async (
    input: BootstrapInput,
    onSuccess: (result: BootstrapSuccess) => void,
  ) => {
    setError(null);
    setLogs([]);
    setPhases(initialPhases());
    setRunning(true);
    try {
      // 1. write passphrase to keyring BEFORE the clone runs, as
      // NewProjectView does. Skip silently for password-auth machines; the
      // wizard never stores secrets in component state.
      if (input.machineKind === 'remote' && input.keyPassphrase.trim().length > 0 && input.machineId) {
        await setMachineSecret(input.machineId, input.keyPassphrase);
      }

      // 2. create the repo on the provider
      setPhaseStatus('create_repo', 'running');
      appendLog(`POST ${input.providerId} → ${input.repoSlug} (private=${input.repoPrivate}${input.providerHost ? `, host=${input.providerHost}` : ''})`);
      const createRepo: CreateRepoRequest = {
        providerId: input.providerId,
        namespaceId: input.namespaceId,
        name: input.repoSlug,
        private: input.repoPrivate,
        providerHost: input.providerHost,
      };
      const repo = await providerCreateRepo(createRepo);
      appendLog(`✓ repo created: ${repo.full_name} (default branch: ${repo.default_branch})`);
      setPhaseStatus('create_repo', 'done');

      // 3. insert the project row (status=bootstrapping)
      setPhaseStatus('create_project', 'running');
      appendLog('registering project record…');
      const projResp = await createProject({
        name: input.projectName,
        compute_type: input.machineKind,
        remote_host: input.machineKind === 'remote' ? (input.machineId || null) : null,
        repos: [{ repo_path: repo.full_name, provider_id: input.providerId }],
      });
      if (!projResp.success) throw new Error('Project row insert failed');
      appendLog(`✓ project ${projResp.id} registered (status=bootstrapping)`);
      setPhaseStatus('create_project', 'done');

      // 4. clone + worktree-strategy detection
      setPhaseStatus('bootstrap', 'running');
      appendLog('cloning + detecting worktree strategy…');
      const strategy = await bootstrapProject(projResp.id);
      appendLog(`✓ strategy detected (default branch=${strategy.default_branch})`);
      setPhaseStatus('bootstrap', 'done');

      // 5. persist project settings (read-merges via saveProjectSettings).
      setPhaseStatus('save_settings', 'running');
      appendLog('persisting project settings…');
      await saveProjectSettings(projResp.id, {
        default_branch: strategy.default_branch,
        branch_prefix: 'demeteo/features/',
        test_command: null,
        pr_template: strategy.pr_template ?? null,
        conflict_policy: 'always_gate',
        feature_lifecycle: 'archive',
        default_agent_kind: input.agentKind || null,
        default_model: input.model || null,
        default_effort: input.effort ?? null,
      });
      appendLog('✓ settings persisted');
      setPhaseStatus('save_settings', 'done');

      appendLog('→ advancing to strategy review');
      setRunning(false);
      onSuccess({ projectId: projResp.id, repo, strategy });
    } catch (err) {
      const wizErr = wizardError(err);
      const msg = wizErr?.message ?? formatError(err);
      setError(msg);
      appendLog(`✗ ${msg}`);
      setPhases((prev) => prev.map((p) => (p.status === 'running' ? { ...p, status: 'error' } : p)));
      setRunning(false);
    }
  }, [appendLog, setPhaseStatus]);

  return { phases, logs, error, running, run };
}
