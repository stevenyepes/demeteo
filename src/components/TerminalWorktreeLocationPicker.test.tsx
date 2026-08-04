import { act, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { StrictMode, type ComponentProps } from 'react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { CreatedTerminalWorktree, TerminalLocations, TerminalWorktree } from '../types';
import {
  createTerminalWorktree,
  listTerminalBranches,
  listTerminalLocations,
  removeTerminalWorktree,
} from '../lib/terminal';
import { TerminalWorktreeLocationPicker, type TerminalWorktreeLocation } from './TerminalWorktreeLocationPicker';

vi.mock('../lib/terminal', () => ({
  createTerminalWorktree: vi.fn(),
  listTerminalBranches: vi.fn(),
  listTerminalLocations: vi.fn(),
  removeTerminalWorktree: vi.fn(),
}));

const onChange = vi.fn<(location: TerminalWorktreeLocation) => void>();

/** A listing whose main checkout is on a branch nobody here picked — which is
 *  the state the annotation exists to report. */
function locations(worktrees: TerminalWorktree[], mainBranch: string | null = 'chore/left-here'): TerminalLocations {
  return { mainBranch, worktrees };
}

function mount(props: Partial<ComponentProps<typeof TerminalWorktreeLocationPicker>> = {}) {
  return render(<TerminalWorktreeLocationPicker projectId="project-a" repositoryId="repository-a" onChange={onChange} {...props} />);
}

/** Open the menu and expand the create form, which is where branches load. */
async function openCreateForm() {
  await userEvent.click(screen.getByTestId('terminal-location-trigger'));
  await userEvent.click(await screen.findByTestId('terminal-location-new'));
}

beforeEach(() => {
  onChange.mockReset();
  vi.mocked(listTerminalLocations).mockReset();
  vi.mocked(listTerminalBranches).mockReset();
  vi.mocked(createTerminalWorktree).mockReset();
  vi.mocked(removeTerminalWorktree).mockReset();
  vi.mocked(listTerminalBranches).mockResolvedValue({
    defaultBranch: 'main',
    branches: [
      { name: 'main', hasLocal: true, hasRemote: true },
      { name: 'scratch', hasLocal: true, hasRemote: false },
    ],
  });
});

describe('TerminalWorktreeLocationPicker', () => {
  it('disables selection and creation actions while the lazy list is pending', async () => {
    vi.mocked(listTerminalLocations).mockImplementation(() => new Promise(() => {}));
    mount();

    await userEvent.click(screen.getByTestId('terminal-location-trigger'));
    expect(screen.getByTestId('terminal-location-loading')).toBeInTheDocument();
    expect(screen.getByTestId('terminal-location-main')).toBeDisabled();
    expect(screen.getByTestId('terminal-location-new')).toBeDisabled();
  });

  it('lazily renders main and typed existing worktrees, returning the backend target', async () => {
    vi.mocked(listTerminalLocations).mockResolvedValue(locations([{ path: '/repos/demo-ticket', branch: 'ticket', isLocked: true }]));
    mount();

    expect(listTerminalLocations).not.toHaveBeenCalled();
    await userEvent.click(screen.getByTestId('terminal-location-trigger'));
    expect(listTerminalLocations).toHaveBeenCalledWith('project-a', 'repository-a');
    expect(await screen.findByText('ticket')).toBeInTheDocument();
    expect(screen.getByText('locked')).toBeInTheDocument();

    await userEvent.click(screen.getByTestId('terminal-location-worktree-/repos/demo-ticket'));
    expect(onChange).toHaveBeenLastCalledWith({ kind: 'worktree', workDir: '/repos/demo-ticket', workBranch: 'ticket' });

    await userEvent.click(screen.getByTestId('terminal-location-trigger'));
    await userEvent.click(screen.getByTestId('terminal-location-main'));
    expect(onChange).toHaveBeenLastCalledWith({ kind: 'main', workDir: null, workBranch: null });
  });

  it('names the branch the main checkout is on, in the menu and on the field', async () => {
    vi.mocked(listTerminalLocations).mockResolvedValue(locations([], 'chore/left-here-yesterday'));
    mount();

    // Nothing has been read yet, so there is nothing truthful to annotate with.
    expect(screen.getByTestId('terminal-location-trigger')).toHaveTextContent('Main checkout');
    expect(screen.getByTestId('terminal-location-trigger')).not.toHaveTextContent('chore/left-here-yesterday');

    await userEvent.click(screen.getByTestId('terminal-location-trigger'));

    expect(await screen.findByTestId('terminal-location-main-branch')).toHaveTextContent(
      'on chore/left-here-yesterday',
    );
    // Selecting it still asks for no branch: the annotation reports where the
    // session lands, it does not check anything out in the shared checkout.
    await userEvent.click(screen.getByTestId('terminal-location-main'));
    expect(onChange).toHaveBeenLastCalledWith({ kind: 'main', workDir: null, workBranch: null });
    expect(screen.getByTestId('terminal-location-trigger')).toHaveTextContent(
      'Main checkout · chore/left-here-yesterday',
    );
  });

  it('says nothing about the branch when the main checkout is detached', async () => {
    vi.mocked(listTerminalLocations).mockResolvedValue(locations([], null));
    mount();

    await userEvent.click(screen.getByTestId('terminal-location-trigger'));

    expect(await screen.findByTestId('terminal-location-main-branch')).toBeEmptyDOMElement();
    expect(screen.getByTestId('terminal-location-trigger')).toHaveTextContent('Main checkout');
  });

  it('derives the folder from the branch and cuts from the default base', async () => {
    vi.mocked(listTerminalLocations).mockResolvedValue(locations([]));
    vi.mocked(createTerminalWorktree).mockResolvedValue({
      worktree: { path: '/repos/demo-feature-x', branch: 'feature/x', isLocked: false },
      baseRef: 'origin/main',
    } satisfies CreatedTerminalWorktree);
    mount();
    await openCreateForm();

    await userEvent.type(screen.getByLabelText('Branch name'), 'feature/x');
    expect(screen.getByTestId('terminal-location-folder')).toHaveTextContent('feature-x');
    expect(screen.getByLabelText('Base branch')).toHaveValue('main');

    await userEvent.click(screen.getByTestId('terminal-location-create'));

    expect(createTerminalWorktree).toHaveBeenCalledWith({
      projectId: 'project-a',
      repositoryId: 'repository-a',
      branch: 'feature/x',
      baseBranch: 'main',
      worktreeName: 'feature-x',
    });
    expect(onChange).toHaveBeenLastCalledWith({ kind: 'worktree', workDir: '/repos/demo-feature-x', workBranch: 'feature/x' });
    // The ref git was actually given, not the one that was asked for.
    expect(await screen.findByTestId('terminal-location-notice')).toHaveTextContent('from origin/main');
  });

  it('states whether the chosen base will be refreshed from origin', async () => {
    vi.mocked(listTerminalLocations).mockResolvedValue(locations([]));
    mount();
    await openCreateForm();

    expect(await screen.findByTestId('terminal-location-base-note')).toHaveTextContent('origin/main');

    await userEvent.selectOptions(screen.getByLabelText('Base branch'), 'scratch');
    expect(screen.getByTestId('terminal-location-base-note')).toHaveTextContent('No origin copy');
  });

  it('creates once synchronously and disables location actions while pending', async () => {
    vi.mocked(listTerminalLocations).mockResolvedValue(locations([]));
    let resolveCreate: ((value: CreatedTerminalWorktree) => void) | undefined;
    vi.mocked(createTerminalWorktree).mockImplementation(() => new Promise((resolve) => { resolveCreate = resolve; }));
    mount();
    await openCreateForm();
    await userEvent.type(screen.getByLabelText('Branch name'), 'terminal');

    const create = screen.getByTestId('terminal-location-create');
    await act(async () => {
      await userEvent.click(create);
      await userEvent.click(create);
    });
    expect(createTerminalWorktree).toHaveBeenCalledTimes(1);
    expect(screen.getByTestId('terminal-location-main')).toBeDisabled();
    expect(screen.getByLabelText('Branch name')).toBeDisabled();

    await act(async () => {
      resolveCreate?.({
        worktree: { path: '/repos/demo-terminal', branch: 'terminal', isLocked: false },
        baseRef: 'main',
      });
    });
    expect(onChange).toHaveBeenLastCalledWith({ kind: 'worktree', workDir: '/repos/demo-terminal', workBranch: 'terminal' });
  });

  it('removes a worktree behind a confirmation and drops it from the list', async () => {
    vi.mocked(listTerminalLocations).mockResolvedValue(locations([{ path: '/repos/demo-done', branch: 'done', isLocked: false }]));
    vi.mocked(removeTerminalWorktree).mockResolvedValue(undefined);
    mount();

    await userEvent.click(screen.getByTestId('terminal-location-trigger'));
    await userEvent.click(await screen.findByTestId('terminal-location-remove-/repos/demo-done'));
    expect(removeTerminalWorktree).not.toHaveBeenCalled();

    await userEvent.click(screen.getByTestId('terminal-location-remove-confirm-/repos/demo-done'));

    expect(removeTerminalWorktree).toHaveBeenCalledWith('project-a', 'repository-a', '/repos/demo-done', false);
    expect(screen.queryByTestId('terminal-location-worktree-/repos/demo-done')).not.toBeInTheDocument();
  });

  it('offers force only after git has refused, never on the first attempt', async () => {
    vi.mocked(listTerminalLocations).mockResolvedValue(locations([{ path: '/repos/demo-dirty', branch: 'dirty', isLocked: false }]));
    vi.mocked(removeTerminalWorktree)
      .mockRejectedValueOnce({ kind: 'validation', message: 'contains modified or untracked files' })
      .mockResolvedValueOnce(undefined);
    mount();

    await userEvent.click(screen.getByTestId('terminal-location-trigger'));
    await userEvent.click(await screen.findByTestId('terminal-location-remove-/repos/demo-dirty'));
    await userEvent.click(screen.getByTestId('terminal-location-remove-confirm-/repos/demo-dirty'));

    expect(await screen.findByTestId('terminal-location-remove-error-/repos/demo-dirty')).toHaveTextContent(
      'contains modified or untracked files',
    );
    expect(vi.mocked(removeTerminalWorktree).mock.calls[0][3]).toBe(false);
    // Still listed, still confirming: a refusal leaves the row exactly where it
    // was rather than optimistically dropping it.
    expect(screen.getByTestId('terminal-location-confirm-/repos/demo-dirty')).toBeInTheDocument();

    await userEvent.click(screen.getByTestId('terminal-location-remove-confirm-/repos/demo-dirty'));
    expect(vi.mocked(removeTerminalWorktree).mock.calls[1][3]).toBe(true);
    expect(screen.queryByTestId('terminal-location-worktree-/repos/demo-dirty')).not.toBeInTheDocument();
  });

  it('falls back to the main checkout when the selected worktree is removed', async () => {
    vi.mocked(listTerminalLocations).mockResolvedValue(locations([{ path: '/repos/demo-picked', branch: 'picked', isLocked: false }]));
    vi.mocked(removeTerminalWorktree).mockResolvedValue(undefined);
    mount();

    await userEvent.click(screen.getByTestId('terminal-location-trigger'));
    await userEvent.click(await screen.findByTestId('terminal-location-worktree-/repos/demo-picked'));
    expect(onChange).toHaveBeenLastCalledWith({ kind: 'worktree', workDir: '/repos/demo-picked', workBranch: 'picked' });

    await userEvent.click(screen.getByTestId('terminal-location-trigger'));
    await userEvent.click(await screen.findByTestId('terminal-location-remove-/repos/demo-picked'));
    await userEvent.click(screen.getByTestId('terminal-location-remove-confirm-/repos/demo-picked'));

    expect(onChange).toHaveBeenLastCalledWith({ kind: 'main', workDir: null, workBranch: null });
  });

  it('formats failures and clears stale selection/error when its repository changes', async () => {
    vi.mocked(listTerminalLocations).mockResolvedValue(locations([{ path: '/repos/demo-old', branch: 'old', isLocked: false }]));
    const { rerender } = mount();
    await userEvent.click(screen.getByTestId('terminal-location-trigger'));
    await userEvent.click(await screen.findByTestId('terminal-location-worktree-/repos/demo-old'));
    expect(onChange).toHaveBeenLastCalledWith({ kind: 'worktree', workDir: '/repos/demo-old', workBranch: 'old' });

    vi.mocked(createTerminalWorktree).mockRejectedValueOnce({ kind: 'validation', message: 'repository unavailable' });
    await openCreateForm();
    await userEvent.type(screen.getByLabelText('Branch name'), 'bad');
    await userEvent.click(screen.getByTestId('terminal-location-create'));
    expect(await screen.findByTestId('terminal-location-error')).toHaveTextContent('repository unavailable');

    await act(async () => {
      rerender(<TerminalWorktreeLocationPicker projectId="project-b" repositoryId="repository-b" onChange={onChange} />);
    });
    expect(screen.queryByTestId('terminal-location-error')).not.toBeInTheDocument();
    expect(onChange).toHaveBeenLastCalledWith({ kind: 'main', workDir: null, workBranch: null });
  });

  it('formats create failures and keeps the form available for correction', async () => {
    vi.mocked(listTerminalLocations).mockResolvedValue(locations([]));
    vi.mocked(createTerminalWorktree).mockRejectedValue({ kind: 'validation', message: 'branch is invalid' });
    mount();
    await openCreateForm();
    await userEvent.type(screen.getByLabelText('Branch name'), 'bad');
    await userEvent.click(screen.getByTestId('terminal-location-create'));

    expect(await screen.findByTestId('terminal-location-error')).toHaveTextContent('branch is invalid');
    expect(screen.getByTestId('terminal-location-create')).not.toBeDisabled();
  });

  it('offers the unscoped machine home only when the caller allows it', async () => {
    vi.mocked(listTerminalLocations).mockResolvedValue(locations([]));
    const { unmount } = mount();
    await userEvent.click(screen.getByTestId('terminal-location-trigger'));
    expect(screen.getByTestId('terminal-location-main')).toBeInTheDocument();
    expect(screen.queryByTestId('terminal-location-home')).not.toBeInTheDocument();
    unmount();

    mount({ allowHome: true });
    await userEvent.click(screen.getByTestId('terminal-location-trigger'));
    await userEvent.click(screen.getByTestId('terminal-location-home'));
    expect(onChange).toHaveBeenLastCalledWith({ kind: 'home', workDir: null, workBranch: null });
    expect(screen.getByTestId('terminal-location-trigger')).toHaveTextContent('Machine home');
  });

  it('lists once per menu open even when StrictMode double-invokes state updaters', async () => {
    vi.mocked(listTerminalLocations).mockResolvedValue(locations([]));
    render(
      <StrictMode>
        <TerminalWorktreeLocationPicker projectId="project-a" repositoryId="repository-a" onChange={onChange} />
      </StrictMode>,
    );

    await userEvent.click(screen.getByTestId('terminal-location-trigger'));
    expect(listTerminalLocations).toHaveBeenCalledTimes(1);
  });

  it('reads branches only when the create form is opened', async () => {
    vi.mocked(listTerminalLocations).mockResolvedValue(locations([]));
    mount();

    await userEvent.click(screen.getByTestId('terminal-location-trigger'));
    expect(listTerminalBranches).not.toHaveBeenCalled();

    await userEvent.click(await screen.findByTestId('terminal-location-new'));
    expect(listTerminalBranches).toHaveBeenCalledWith('project-a', 'repository-a');
  });

  it('refetches on every open so a worktree removed while closed stops being offered', async () => {
    vi.mocked(listTerminalLocations)
      .mockResolvedValueOnce(locations([{ path: '/repos/demo-gone', branch: 'gone', isLocked: false }]))
      .mockResolvedValueOnce(locations([]));
    mount();

    await userEvent.click(screen.getByTestId('terminal-location-trigger'));
    expect(await screen.findByTestId('terminal-location-worktree-/repos/demo-gone')).toBeInTheDocument();

    await userEvent.click(screen.getByTestId('terminal-location-trigger'));
    await userEvent.click(screen.getByTestId('terminal-location-trigger'));

    expect(listTerminalLocations).toHaveBeenCalledTimes(2);
    expect(screen.queryByTestId('terminal-location-worktree-/repos/demo-gone')).not.toBeInTheDocument();
  });

  it('keeps typed create input when the caller passes a fresh onChange identity', async () => {
    vi.mocked(listTerminalLocations).mockResolvedValue(locations([]));
    const { rerender } = render(
      <TerminalWorktreeLocationPicker projectId="project-a" repositoryId="repository-a" onChange={(location) => onChange(location)} />,
    );
    await openCreateForm();
    await userEvent.type(screen.getByLabelText('Branch name'), 'feature/keep');

    await act(async () => {
      rerender(
        <TerminalWorktreeLocationPicker projectId="project-a" repositoryId="repository-a" onChange={(location) => onChange(location)} />,
      );
    });

    expect(screen.getByLabelText('Branch name')).toHaveValue('feature/keep');
    expect(screen.getByTestId('terminal-location-menu')).toBeInTheDocument();
  });

  it('discards a rejected create from a previously selected repository', async () => {
    vi.mocked(listTerminalLocations).mockResolvedValue(locations([]));
    let rejectCreate: ((reason?: unknown) => void) | undefined;
    vi.mocked(createTerminalWorktree).mockImplementation(
      () => new Promise((_, reject) => { rejectCreate = reject; }),
    );
    const { rerender } = mount();
    await openCreateForm();
    await userEvent.type(screen.getByLabelText('Branch name'), 'old-request');
    await userEvent.click(screen.getByTestId('terminal-location-create'));
    expect(screen.getByTestId('terminal-location-create')).toBeDisabled();

    await act(async () => {
      rerender(<TerminalWorktreeLocationPicker projectId="project-b" repositoryId="repository-b" onChange={onChange} />);
    });
    expect(screen.getByTestId('terminal-location-trigger')).not.toBeDisabled();

    await act(async () => { rejectCreate?.({ kind: 'validation', message: 'old repository failed' }); });
    await openCreateForm();
    expect(screen.queryByTestId('terminal-location-error')).not.toBeInTheDocument();
    await userEvent.type(screen.getByLabelText('Branch name'), 'new-request');
    expect(screen.getByTestId('terminal-location-create')).not.toBeDisabled();
  });
});
