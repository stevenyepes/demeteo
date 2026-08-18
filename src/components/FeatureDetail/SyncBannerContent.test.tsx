import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { SyncBannerContent } from './SyncBannerContent';
import type { SyncOutcomeView } from '../../types';
import type { HarnessOverrides } from './useHarnessOverrides';

const GIT_SAYS = 'fatal: could not read Username for https://github.com: No such device';

function resolverOverrides(over: Partial<HarnessOverrides> = {}): HarnessOverrides {
  return {
    availableModels: [{ value: 'gpt-5-codex', name: 'GPT-5 Codex' }],
    selectedModel: '',
    setSelectedModel: vi.fn(),
    isLoadingModels: false,
    availableAgents: ['opencode', 'codex'],
    selectedAgent: '',
    selectedEffort: '',
    setSelectedEffort: vi.fn(),
    featureAgentKind: 'opencode',
    retryEffortLevels: ['low', 'high'],
    onAgentChange: vi.fn(),
    adoptFeatureModel: vi.fn(),
    probeForFeature: vi.fn(),
    ...over,
  };
}

function renderBanner(outcome: SyncOutcomeView, overrides = resolverOverrides()) {
  const onResolve = vi.fn();
  const onAbort = vi.fn();
  render(
    <SyncBannerContent
      outcome={outcome}
      onResolve={onResolve}
      resolverOverrides={overrides}
      onAbort={onAbort}
      resolving={false}
      aborting={false}
      onDismiss={vi.fn()}
    />,
  );
  return { onResolve, onAbort };
}

describe('SyncBannerContent', () => {
  /**
   * The bug this banner shipped with: every failure of the sync path — an
   * expired token, an unreachable remote, a rejected push — rendered as
   * "Merge conflict in 0 file(s)" beside a button that spawned an agent into
   * a tree with no merge in it.
   */
  it('says a blocked sync did not complete, and offers no agent to resolve it', () => {
    renderBanner({ status: 'blocked', stage: 'fetch', raw_error: GIT_SAYS });

    expect(screen.getByText(/Sync did not complete/)).toBeInTheDocument();
    expect(screen.getByText(GIT_SAYS)).toBeInTheDocument();
    expect(screen.queryByText(/Merge conflict/)).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /Resolve with agent/ })).not.toBeInTheDocument();
  });

  it('names the next move per stage rather than repeating git', () => {
    renderBanner({ status: 'blocked', stage: 'push', raw_error: GIT_SAYS });

    expect(screen.getByText(/the push to origin failed/)).toBeInTheDocument();
  });

  /**
   * `raw_error` was carried the whole way from git and dropped at the last
   * hop, so a conflict banner listed paths and never said what git objected to.
   */
  it('shows git raw words on a conflict, beside the file list', () => {
    renderBanner({
      status: 'conflict',
      conflict_files: [{ path: 'README.md', kind: 'both-modified' }],
      raw_error: 'CONFLICT (content): Merge conflict in README.md',
    });

    expect(screen.getByText('README.md')).toBeInTheDocument();
    expect(
      screen.getByText('CONFLICT (content): Merge conflict in README.md'),
    ).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /Resolve with agent/ })).toBeInTheDocument();
  });

  /**
   * A conflict whose `git status` read failed carries an empty list and a real
   * conflicted worktree. Hiding the button on it left that state with no entry
   * point in the whole UI, while the workflow's sync step spawned the resolver
   * on the identical value.
   */
  it('offers the agent on a conflict whose file list did not parse', () => {
    renderBanner({ status: 'conflict', conflict_files: [], raw_error: GIT_SAYS });

    expect(screen.getByRole('button', { name: /Resolve with agent/ })).toBeInTheDocument();
  });

  /**
   * The conflict outlives this component now — a worktree with `MERGE_HEAD`
   * set and a session row — so "not this one" has to be sayable. Without it
   * the only thing that ever cleans that tree up is the next sync
   * force-removing it.
   */
  it('lets the user abandon a conflict, and only a conflict', () => {
    const { onAbort } = renderBanner({
      status: 'conflict',
      conflict_files: [],
      raw_error: GIT_SAYS,
    });
    fireEvent.click(screen.getByRole('button', { name: /Abort sync/ }));
    expect(onAbort).toHaveBeenCalledOnce();

    cleanup();
    renderBanner({ status: 'blocked', stage: 'fetch', raw_error: GIT_SAYS });
    expect(screen.queryByRole('button', { name: /Abort sync/ })).not.toBeInTheDocument();
  });

  /**
   * The picker is only worth having if what it holds travels with the click.
   * An untouched control sends `null`, which is what makes it "inherit" rather
   * than "no harness" — the backend then walks the project's own
   * conflict-resolver setting before the run's.
   */
  it('sends the picked harness, model and effort with the resolve', () => {
    const { onResolve } = renderBanner(
      { status: 'conflict', conflict_files: [{ path: 'README.md', kind: 'both-modified' }], raw_error: GIT_SAYS },
      resolverOverrides({ selectedAgent: 'codex', selectedModel: 'gpt-5-codex', selectedEffort: 'low' }),
    );
    expect(screen.getByLabelText('Harness')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: /Resolve with agent/ }));

    expect(onResolve).toHaveBeenCalledWith(['README.md'], {
      agentKind: 'codex',
      model: 'gpt-5-codex',
      effort: 'low',
    });
  });

  it('sends nulls when the picker was left alone', () => {
    const { onResolve } = renderBanner({
      status: 'conflict',
      conflict_files: [{ path: 'README.md', kind: 'both-modified' }],
      raw_error: GIT_SAYS,
    });
    fireEvent.click(screen.getByRole('button', { name: /Resolve with agent/ }));

    expect(onResolve).toHaveBeenCalledWith(['README.md'], {
      agentKind: null,
      model: null,
      effort: null,
    });
  });

  /** The picker belongs to the one outcome that has anything to resolve. */
  it('offers no harness picker on an outcome with nothing to resolve', () => {
    renderBanner({ status: 'blocked', stage: 'fetch', raw_error: GIT_SAYS });

    expect(screen.queryByLabelText('Harness')).not.toBeInTheDocument();
  });
});
