import { useEffect, useState } from 'react';
import { confirm as confirmDialog, message as messageDialog } from '@tauri-apps/plugin-dialog';
import type { AppView, MrState } from '../../types';
import { formatError } from '../../lib/errors';
import { useErrorBus } from '../../lib/errorBus';
import { fetchMrState, getFeature } from '../../lib/featureSync';
import { cleanupFeature, publishMr } from '../../lib/featureDetail';

/**
 * The feature's pull request: publish it, track its state, and apply the
 * project's lifecycle policy.
 *
 * The branch's own state — sync, conflict, resolution — left here for
 * `useSyncSession`, which reads the durable row instead of remembering.
 */
export function useFeatureMr(input: {
  featureId: string;
  projectId: string | undefined;
  status: string;
  reload: () => void;
  navigate: (view: AppView) => void;
}) {
  const { featureId, projectId, status, reload, navigate } = input;
  const { reportError } = useErrorBus();
  const [publishing, setPublishing] = useState(false);
  const [mrState, setMrState] = useState<MrState | null>(null);
  const [mrUrl, setMrUrl] = useState<string | null>(null);

  /**
   * Refresh the MR state from the provider. The badge updates
   * inline so the user always knows whether their PR is in
   * review, merged, or closed.
   */
  const refreshMrState = async () => {
    if (!projectId || !mrUrl) return;
    try {
      const fresh = await fetchMrState(projectId, mrUrl);
      setMrState(fresh as MrState);
    } catch (err) {
      // Best-effort: fall back to the cached state from the row.
      console.warn('Failed to refresh MR state', err);
    }
  };

  /**
   * Read the latest feature row and pick up the MR url/state.
   * Keyed on `status` so the badge stays in sync with any backend
   * change (publish, cleanup, manual update).
   */
  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const f = await getFeature(featureId);
        if (cancelled) return;
        setMrUrl(f?.mr_url ?? null);
        setMrState((f?.mr_state ?? 'none') as MrState);
      } catch (err) {
        reportError(err, { kind: "internal" });
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [featureId, status]);

  /** Publish, without asking for a title.
   *
   *  The `finalize` step normally does all of this by itself at the end of the
   *  run: it squashes the branch, writes the commit message and PR title/body
   *  in the repo's own style, and the PR opens automatically. This button is
   *  the fallback for the cases where that didn't happen — a feature on an
   *  older workflow with no finalize step, or a publish that failed (the
   *  provider was down, credentials were missing). Either way there is nothing
   *  to type: the backend uses the summary the agent authored when there is
   *  one, and its own default title when there isn't. */
  const handlePublishClick = async () => {
    if (!projectId) {
      await messageDialog('No project is associated with this feature.', {
        title: 'Cannot publish',
        kind: 'error',
      });
      return;
    }
    setPublishing(true);
    try {
      const result = await publishMr({ projectId, featureId, draft: false });
      const url = result?.url ?? '(unknown)';
      const state = result?.state ?? 'open';
      await messageDialog(
        `MR/PR opened (state: ${state}).\n\n${url}`,
        { title: 'Published', kind: 'info' },
      );
      reload();
    } catch (err) {
      await messageDialog(formatError(err), { title: 'Publish failed', kind: 'error' });
    } finally {
      setPublishing(false);
    }
  };

  /** Apply the project's `feature_lifecycle` policy (R6 decision 26).
   *  `archive` → soft-delete; `auto_delete` → git branch -D +
   *  soft-delete; `keep` → no-op. */
  const handleCleanup = async (force = false) => {
    try {
      const result = await cleanupFeature({ featureId, force });
      let msg = `Cleanup (${result.policy}): ${result.action}`;
      if (result.warnings?.length) {
        msg += `\n\nWarnings:\n${result.warnings.join('\n')}`;
      }
      await messageDialog(msg, { title: 'Lifecycle applied', kind: 'info' });
      navigate({ kind: 'home' });
    } catch (err) {
      const msg = formatError(err);
      if (msg.includes('Auto-delete requires the MR to be merged')) {
        const ok = await confirmDialog(
          'The branch has not been merged yet. Force delete anyway?',
          { title: 'Force delete branch?', kind: 'warning', okLabel: 'Force Delete', cancelLabel: 'Cancel' },
        );
        if (ok) handleCleanup(true);
      } else {
        await messageDialog(msg, { title: 'Cleanup failed', kind: 'error' });
      }
    }
  };

  return {
    publishing,
    mrState,
    mrUrl,
    refreshMrState,
    handlePublishClick,
    handleCleanup,
  };
}
