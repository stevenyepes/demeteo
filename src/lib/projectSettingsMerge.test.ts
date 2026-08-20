/**
 * The claim: a save that says nothing about the conflict resolver leaves it
 * exactly as it was.
 *
 * `save_project_settings` writes the whole record with `INSERT OR REPLACE`, so
 * `saveProjectSettings` reads the stored row and overlays the caller's partial
 * input — and the carry-across is hand-written per field. Four of the five
 * callers pass a handful of fields and depend on it entirely, so a field added
 * to `ProjectSettingsInput` and not to the merge is NULLed by every one of them
 * with no type error anywhere.
 */
import { invoke } from '@tauri-apps/api/core';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { saveProjectSettings } from './project';

const mockedInvoke = vi.mocked(invoke);

const STORED = {
  project_id: 'p-1',
  worktree_strategy: { default_branch: 'main', branch_prefix: 'demeteo/features/' },
  conflict_policy: 'always_gate',
  feature_lifecycle: 'archive',
  default_agent_kind: 'opencode',
  default_model: 'sonnet',
  default_effort: 'high',
  review_entrypoint: '/code-review',
  sync_resolver_agent_kind: 'codex',
  sync_resolver_model: 'gpt-5-codex',
  sync_resolver_effort: 'low',
};

function written(): Record<string, unknown> {
  const call = mockedInvoke.mock.calls.find(([name]) => name === 'save_project_settings');
  if (!call) throw new Error('save_project_settings was never called');
  return (call[1] as { settings: Record<string, unknown> }).settings;
}

beforeEach(() => {
  mockedInvoke.mockReset();
  mockedInvoke.mockImplementation((async (cmd: string) => {
    if (cmd === 'get_proposed_strategy') return STORED;
    if (cmd === 'save_project_settings') return undefined;
    throw new Error(`unscripted invoke('${cmd}')`);
  }) as typeof invoke);
});

describe('saveProjectSettings', () => {
  it('carries the stored conflict resolver across a save that never mentions it', async () => {
    await saveProjectSettings('p-1', { default_branch: 'trunk' });

    expect(written()).toMatchObject({
      worktree_strategy: expect.objectContaining({ default_branch: 'trunk' }),
      sync_resolver_agent_kind: 'codex',
      sync_resolver_model: 'gpt-5-codex',
      sync_resolver_effort: 'low',
    });
  });

  it('writes an explicit null through rather than treating it as absent', async () => {
    await saveProjectSettings('p-1', {
      sync_resolver_agent_kind: null,
      sync_resolver_model: null,
      sync_resolver_effort: null,
    });

    expect(written()).toMatchObject({
      sync_resolver_agent_kind: null,
      sync_resolver_model: null,
      sync_resolver_effort: null,
    });
  });
});
