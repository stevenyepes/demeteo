/**
 * The containment note has to reach the pane the user actually opens, which is
 * two hand-offs away from the component that renders it: `useHarnessOverrides`
 * has to keep the containment field on the rows it stores, and this component
 * has to hand those rows on. Both are invisible to
 * `HarnessContainmentNote.test.tsx`, which drives the component directly, and
 * to `sync/SyncPanel.test.tsx`, whose fixture passes `machineAgents: []` —
 * under which a correctly wired note also renders nothing.
 *
 * So this mounts the real pane over the real hooks and lets only the backend be
 * a stub, the way `review/CodeReviewView.test.tsx` gates the sibling note.
 */
import { render, screen, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';

import { describeSyncPanel } from '../../lib/syncPanel';
import type { SyncSessionView } from '../../types';
import { SyncPanel } from './sync/SyncPanel';
import { useSyncResolverOverrides } from './useSyncResolverOverrides';

const SESSION: SyncSessionView = {
  feature_id: 'f-1',
  machine_id: 'local',
  repo_dir: '/repos/demeteo',
  feature_branch: 'feature/f-1',
  base_branch: 'origin/master',
  status: 'conflicted',
  worktree_path: '/repos/demeteo_wt_sync_feature-f-1',
  head_before: 'aaaaaaa1111',
  merge_commit_sha: null,
  conflict_files: [{ path: 'src/lib.rs', kind: 'both-modified' }],
  raw_error: 'CONFLICT (content): Merge conflict in src/lib.rs',
  blocked_stage: null,
  pushed_at: null,
  user_may_intervene: true,
  attempts: 1,
  created_at: 0,
  updated_at: 0,
};

/** The four commands the pane reads before it can state what the resolver's
 *  harness is held to, and nothing else — a stub that answered everything
 *  would pass against a pane that asked the wrong one. */
function backend() {
  vi.mocked(invoke).mockImplementation((cmd: string) => {
    if (cmd === 'feature_sync_resolver') {
      return Promise.resolve({ agent_kind: 'codex', model: null, effort: 'medium' });
    }
    if (cmd === 'get_project_by_id') return Promise.resolve({ id: 'p-1', remote_host: null });
    if (cmd === 'get_agent_models') return Promise.resolve([]);
    if (cmd === 'get_agent_configs') {
      return Promise.resolve([
        {
          kind: 'codex',
          enabled: true,
          available: true,
          path_containment: { reads: 'none', writes: 'os', shell: 'os' },
        },
        {
          kind: 'opencode',
          enabled: true,
          available: true,
          path_containment: { reads: 'harness', writes: 'harness', shell: 'harness-partial' },
        },
      ]);
    }
    if (cmd === 'list_agents') {
      return Promise.resolve([
        {
          kind: 'codex',
          display_label: 'Codex',
          lists_models: false,
          default_model: null,
          install_command: '',
        },
      ]);
    }
    return Promise.reject(new Error(`unexpected command: ${cmd}`));
  });
}

function Pane() {
  const resolverSelection = useSyncResolverOverrides({
    featureId: 'f-1',
    projectId: 'p-1',
    conflicted: true,
  });
  return (
    <SyncPanel
      model={describeSyncPanel({
        session: SESSION,
        drift: null,
        divergence: null,
        canSync: true,
        pending: null,
      })}
      session={SESSION}
      drift={null}
      resolverStep={null}
      pending={null}
      resolverSelection={resolverSelection}
      onAction={() => {}}
      onOpenPath={() => {}}
    />
  );
}

describe('SyncResolverOptions', () => {
  it('says what the inherited harness is held to, without being asked', async () => {
    backend();
    render(<Pane />);

    const note = await screen.findByTestId('harness-containment');
    // Codex's own row on this machine, not opencode's beside it and not a
    // default: reads unfenced, writes and shell held by the kernel.
    const line = (dimension: string) =>
      note.querySelector(`[data-dimension="${dimension}"]`);
    await waitFor(() => expect(line('reads')).toHaveAttribute('data-enforcement', 'none'));
    expect(line('reads')).toHaveTextContent('nothing stops Codex reading any file your account can');
    expect(line('writes')).toHaveAttribute('data-enforcement', 'os');
    expect(line('shell')).toHaveAttribute('data-enforcement', 'os');
  });
});
