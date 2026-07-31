import { act, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import type { ComponentProps } from 'react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { createTerminalWorktree, listTerminalWorktrees } from '../lib/terminal';
import { TerminalWorktreeLocationPicker, type TerminalWorktreeLocation } from './TerminalWorktreeLocationPicker';

vi.mock('../lib/terminal', () => ({
  createTerminalWorktree: vi.fn(),
  listTerminalWorktrees: vi.fn(),
}));

const onChange = vi.fn<(location: TerminalWorktreeLocation) => void>();

function mount(props: Partial<ComponentProps<typeof TerminalWorktreeLocationPicker>> = {}) {
  return render(<TerminalWorktreeLocationPicker projectId="project-a" repositoryId="repository-a" onChange={onChange} {...props} />);
}

beforeEach(() => {
  onChange.mockReset();
  vi.mocked(listTerminalWorktrees).mockReset();
  vi.mocked(createTerminalWorktree).mockReset();
});

describe('TerminalWorktreeLocationPicker', () => {
  it('disables selection and creation actions while the lazy list is pending', async () => {
    vi.mocked(listTerminalWorktrees).mockImplementation(() => new Promise(() => {}));
    mount();

    await userEvent.click(screen.getByTestId('terminal-location-trigger'));
    expect(screen.getByTestId('terminal-location-loading')).toBeInTheDocument();
    expect(screen.getByTestId('terminal-location-main')).toBeDisabled();
    expect(screen.getByTestId('terminal-location-create')).toBeDisabled();
    expect(screen.getByLabelText('Branch name')).toBeDisabled();
  });

  it('lazily renders main and typed existing worktrees, returning the backend target', async () => {
    vi.mocked(listTerminalWorktrees).mockResolvedValue([{ path: '/repos/demo-ticket', branch: 'ticket', isLocked: true }]);
    mount();

    expect(listTerminalWorktrees).not.toHaveBeenCalled();
    await userEvent.click(screen.getByTestId('terminal-location-trigger'));
    expect(listTerminalWorktrees).toHaveBeenCalledWith('project-a', 'repository-a');
    expect(await screen.findByText('ticket')).toBeInTheDocument();
    expect(screen.getByText('locked')).toBeInTheDocument();

    await userEvent.click(screen.getByTestId('terminal-location-worktree-/repos/demo-ticket'));
    expect(onChange).toHaveBeenLastCalledWith({ kind: 'worktree', workDir: '/repos/demo-ticket', workBranch: 'ticket' });

    await userEvent.click(screen.getByTestId('terminal-location-trigger'));
    await userEvent.click(screen.getByTestId('terminal-location-main'));
    expect(onChange).toHaveBeenLastCalledWith({ kind: 'main', workDir: null, workBranch: null });
  });

  it('creates once synchronously, selects the backend result, and disables location actions while pending', async () => {
    vi.mocked(listTerminalWorktrees).mockResolvedValue([]);
    let resolveCreate: ((value: { path: string; branch: string; isLocked: boolean }) => void) | undefined;
    vi.mocked(createTerminalWorktree).mockImplementation(() => new Promise((resolve) => { resolveCreate = resolve; }));
    mount();
    await userEvent.click(screen.getByTestId('terminal-location-trigger'));
    await screen.findByText('No linked worktrees');
    await userEvent.type(screen.getByLabelText('Branch name'), 'feature/terminal');
    await userEvent.type(screen.getByLabelText('Worktree name'), 'terminal');

    const create = screen.getByTestId('terminal-location-create');
    await act(async () => {
      await userEvent.click(create);
      await userEvent.click(create);
    });
    expect(createTerminalWorktree).toHaveBeenCalledTimes(1);
    expect(createTerminalWorktree).toHaveBeenCalledWith({ projectId: 'project-a', repositoryId: 'repository-a', branch: 'feature/terminal', worktreeName: 'terminal' });
    expect(screen.getByTestId('terminal-location-main')).toBeDisabled();
    expect(screen.getByLabelText('Branch name')).toBeDisabled();

    await act(async () => { resolveCreate?.({ path: '/repos/demo-terminal', branch: 'feature/terminal', isLocked: false }); });
    expect(onChange).toHaveBeenLastCalledWith({ kind: 'worktree', workDir: '/repos/demo-terminal', workBranch: 'feature/terminal' });
  });

  it('formats failures and clears stale selection/error when its repository changes', async () => {
    vi.mocked(listTerminalWorktrees).mockResolvedValueOnce([{ path: '/repos/demo-old', branch: 'old', isLocked: false }]);
    const { rerender } = mount();
    await userEvent.click(screen.getByTestId('terminal-location-trigger'));
    await userEvent.click(await screen.findByTestId('terminal-location-worktree-/repos/demo-old'));
    expect(onChange).toHaveBeenLastCalledWith({ kind: 'worktree', workDir: '/repos/demo-old', workBranch: 'old' });

    vi.mocked(createTerminalWorktree).mockRejectedValueOnce({ kind: 'validation', message: 'repository unavailable' });
    await userEvent.click(screen.getByTestId('terminal-location-trigger'));
    await userEvent.type(screen.getByLabelText('Branch name'), 'bad');
    await userEvent.type(screen.getByLabelText('Worktree name'), 'bad');
    await userEvent.click(screen.getByTestId('terminal-location-create'));
    expect(await screen.findByTestId('terminal-location-error')).toHaveTextContent('repository unavailable');

    await act(async () => {
      rerender(<TerminalWorktreeLocationPicker projectId="project-b" repositoryId="repository-b" onChange={onChange} />);
    });
    expect(screen.queryByTestId('terminal-location-error')).not.toBeInTheDocument();
    expect(onChange).toHaveBeenLastCalledWith({ kind: 'main', workDir: null, workBranch: null });
  });

  it('formats create failures and keeps the picker available for correction', async () => {
    vi.mocked(listTerminalWorktrees).mockResolvedValue([]);
    vi.mocked(createTerminalWorktree).mockRejectedValue({ kind: 'validation', message: 'branch is invalid' });
    mount();
    await userEvent.click(screen.getByTestId('terminal-location-trigger'));
    await screen.findByText('No linked worktrees');
    await userEvent.type(screen.getByLabelText('Branch name'), 'bad branch');
    await userEvent.type(screen.getByLabelText('Worktree name'), 'ticket');
    await userEvent.click(screen.getByTestId('terminal-location-create'));
    expect(await screen.findByTestId('terminal-location-error')).toHaveTextContent('branch is invalid');
    expect(screen.getByTestId('terminal-location-create')).not.toBeDisabled();
  });

  it('discards a rejected create from a previously selected repository', async () => {
    vi.mocked(listTerminalWorktrees).mockResolvedValue([]);
    let rejectCreate: ((reason?: unknown) => void) | undefined;
    vi.mocked(createTerminalWorktree).mockImplementation(
      () => new Promise((_, reject) => { rejectCreate = reject; }),
    );
    const { rerender } = mount();
    await userEvent.click(screen.getByTestId('terminal-location-trigger'));
    await screen.findByText('No linked worktrees');
    await userEvent.type(screen.getByLabelText('Branch name'), 'old-request');
    await userEvent.type(screen.getByLabelText('Worktree name'), 'old-request');
    await userEvent.click(screen.getByTestId('terminal-location-create'));
    expect(screen.getByTestId('terminal-location-create')).toBeDisabled();

    await act(async () => {
      rerender(<TerminalWorktreeLocationPicker projectId="project-b" repositoryId="repository-b" onChange={onChange} />);
    });
    expect(screen.getByTestId('terminal-location-trigger')).not.toBeDisabled();

    await act(async () => { rejectCreate?.({ kind: 'validation', message: 'old repository failed' }); });
    await userEvent.click(screen.getByTestId('terminal-location-trigger'));
    expect(await screen.findByText('No linked worktrees')).toBeInTheDocument();
    expect(screen.queryByTestId('terminal-location-error')).not.toBeInTheDocument();
    await userEvent.type(screen.getByLabelText('Branch name'), 'new-request');
    await userEvent.type(screen.getByLabelText('Worktree name'), 'new-request');
    expect(screen.getByTestId('terminal-location-create')).not.toBeDisabled();
  });
});
