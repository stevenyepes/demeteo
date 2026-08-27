import { useCallback } from 'react';

import type { SyncIntent } from '../../lib/syncPanel';
import type { SyncResolverSelection } from './useSyncResolverOverrides';
import type { SyncSession } from './useSyncSession';

/**
 * One intent in, one call out.
 *
 * The Sync pane emits an intent rather than a handler per button so the model
 * decides which affordances exist in one place — `lib/syncPanel.ts` — and this
 * is the only place that knows what each of them reaches for. Splitting it back
 * into props would put the "may I offer this" rule and the "what does it do"
 * rule in two files that have to agree.
 */
export function useSyncActions(input: {
  sync: SyncSession;
  resolver: SyncResolverSelection;
  refreshDrift: () => void;
  openDiffRange: (refs: { baseRef: string; headRef: string }) => void;
  /** Show the resolver's own step row and the output streaming into it. */
  showResolverStream: () => void;
}): (intent: SyncIntent) => void {
  const { sync, resolver, refreshDrift, openDiffRange, showResolverStream } = input;
  const { overrides } = resolver;

  return useCallback(
    (intent: SyncIntent) => {
      switch (intent) {
        case 'sync':
          void sync.startSync();
          return;
        case 'resolve':
          void sync.resolve(
            (sync.session?.conflict_files ?? []).map((file) => file.path),
            {
              agentKind: overrides.selectedAgent || null,
              model: overrides.selectedModel || null,
              effort: overrides.selectedEffort || null,
            },
          );
          return;
        case 'abort':
          void sync.abort();
          return;
        case 'publish':
          void sync.publish();
          return;
        case 'discard':
          void sync.discard();
          return;
        case 'reconcile':
        case 'reset_onto_origin':
          void sync.reconcile(intent);
          return;
        case 'refresh':
          void sync.refresh();
          refreshDrift();
          return;
        case 'review': {
          // `head_before..merge_commit_sha`, never `merge_commit_sha^`: a
          // resolver that added a follow-up commit makes the first parent that
          // commit's parent, and the review then omits the merge itself.
          const base = sync.session?.head_before;
          const head = sync.session?.merge_commit_sha;
          if (base && head) void openDiffRange({ baseRef: base, headRef: head });
          return;
        }
        case 'watch':
          showResolverStream();
          return;
      }
    },
    [sync, overrides, refreshDrift, openDiffRange, showResolverStream],
  );
}
