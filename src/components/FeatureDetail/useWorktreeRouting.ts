import { useCallback } from 'react';
import type { AppView, RemoteRunMirror } from '../../types';
import { useErrorBus } from '../../lib/errorBus';
import { useTerminalPanel } from '../../context';
import { getFeatureWorktree, getRemoteWorktree } from '../../lib/featureDetail';

/**
 * Routing out of the run view into the feature's own code: the read-only
 * editor and the global terminal panel.
 */
export function useWorktreeRouting(input: {
  featureId: string;
  featureTitle: string;
  projectId: string | undefined;
  remoteRun: RemoteRunMirror | null;
  navigate: (view: AppView) => void;
}) {
  const { featureId, featureTitle, projectId, remoteRun, navigate } = input;
  const { reportError } = useErrorBus();
  /** The panel hosts the live PTY (spec §3 (c)); this only routes the
   *  request — it does NOT own session teardown. */
  const { open: openTerminalTab } = useTerminalPanel();

  const handleOpenTerminalTab = async () => {
    try {
      const info = await getFeatureWorktree(featureId);
      // Pass the absolute worktree path as `workDir` so the panel
      // bypasses `resolve_repo_dir` and the shell actually starts
      // inside the feature worktree, not a basename-derived clone.
      void openTerminalTab({
        machineId: info.machine_id,
        machineLabel: info.machine_id,
        projectId,
        workDir: info.worktree_path,
        workBranch: info.branch,
      });
      // The live terminal surface renders on the full-page Terminals
      // view, not here — route to it so the session the user just asked
      // for is actually on screen instead of only pulsing on the rail.
      navigate({ kind: 'terminals' });
    } catch (err) {
      reportError(err);
    }
  };

  // Resolve the feature's worktree path + branch for Browse Code. A detached
  // (runner) run's code lives in the *runner's* workspace, not where
  // `feature_get_worktree` would compute from the shadow's re-homed local
  // project — so route those through `remote_get_worktree`, which asks the
  // runner for its real path and re-homes `machine_id` onto the mirror's box
  // (reachable over the SSH the laptop already holds). Local/SSH runs keep
  // the direct path.
  const resolveWorktreeInfo = useCallback(async () => {
    if (remoteRun) {
      return getRemoteWorktree({
        machineId: remoteRun.machine_id,
        runId: remoteRun.run_id,
      });
    }
    return getFeatureWorktree(featureId);
  }, [remoteRun, featureId]);

  const openEditor = async () => {
    try {
      const info = await resolveWorktreeInfo();
      navigate({
        kind: 'editor',
        editorContext: {
          machineId: info.machine_id,
          worktreePath: info.worktree_path,
          branch: info.branch,
          defaultBranch: info.default_branch,
        },
        featureId,
        featureTitle,
      });
    } catch (err) {
      reportError(err);
    }
  };

  // Open a worktree-ref artifact in the code editor — the same path the
  // timeline's `ArtifactViewer` uses, shared with the drill-down panel's Output
  // tab (P2.3). Declared after `resolveWorktreeInfo` so it isn't in its TDZ.
  const openEditorForPath = useCallback(
    async (filePath: string) => {
      try {
        const info = await resolveWorktreeInfo();
        navigate({
          kind: 'editor',
          editorContext: {
            machineId: info.machine_id,
            worktreePath: info.worktree_path,
            branch: info.branch,
            defaultBranch: info.default_branch,
            initialFile: filePath,
          },
          featureId,
          featureTitle,
        });
      } catch (err) {
        reportError(err);
      }
    },
    [resolveWorktreeInfo, navigate, featureId, featureTitle, reportError],
  );

  return { handleOpenTerminalTab, openEditor, openEditorForPath };
}
