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

  // A shell in a checkout the caller names.
  //
  // Deliberately not routed through `list_terminal_locations`, whose exclusion
  // of pipeline-owned worktrees stands: that picker is repo-scoped and offers
  // checkouts for casual browsing, and a sync worktree is neither — it belongs
  // to one feature, exists only while a conflict does, and is worth opening for
  // exactly one reason. `start_terminal_session` takes an explicit `workDir`,
  // so naming it here needs nothing of the picker or its rule.
  const openWorktreeTerminal = useCallback(
    (at: { machineId: string; workDir: string; workBranch: string }) => {
      void openTerminalTab({
        machineId: at.machineId,
        machineLabel: at.machineId,
        projectId,
        workDir: at.workDir,
        workBranch: at.workBranch,
      });
      navigate({ kind: 'terminals' });
    },
    [openTerminalTab, projectId, navigate],
  );

  // Open a file in a checkout the caller names, rather than the one
  // `resolveWorktreeInfo` finds.
  //
  // The sync worktree is the case this exists for. It is a separate checkout
  // from the feature's, and it is the only one where the conflict markers are —
  // so routing a conflicted path through `openEditorForPath` opened the
  // feature worktree's copy of that same path: clean, marker-free, and with
  // nothing on screen to say it was a different file.
  const openEditorInWorktree = useCallback(
    (
      at: { machineId: string; worktreePath: string; branch: string; defaultBranch: string },
      filePath: string,
    ) => {
      navigate({
        kind: 'editor',
        editorContext: {
          machineId: at.machineId,
          worktreePath: at.worktreePath,
          branch: at.branch,
          defaultBranch: at.defaultBranch,
          initialFile: filePath,
        },
        featureId,
        featureTitle,
      });
    },
    [navigate, featureId, featureTitle],
  );

  // Open the editor's Changes tab on an explicit pair of refs. Same
  // `resolveWorktreeInfo` as `openEditor`, so a detached run still resolves the
  // runner's own path; declared after it for the same TDZ reason
  // `openEditorForPath` is.
  const openDiffRange = useCallback(
    async ({ baseRef, headRef }: { baseRef: string; headRef: string }) => {
      try {
        const info = await resolveWorktreeInfo();
        navigate({
          kind: 'editor',
          editorContext: {
            machineId: info.machine_id,
            worktreePath: info.worktree_path,
            branch: info.branch,
            defaultBranch: info.default_branch,
            baseRef,
            headRef,
            initialTab: 'changes',
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

  return {
    handleOpenTerminalTab,
    openEditor,
    openEditorForPath,
    openEditorInWorktree,
    openWorktreeTerminal,
    openDiffRange,
  };
}
