// NewDiscoveryModal — base-branch choice and the two-call `start()` sequence
// (createDiscovery, then setDiscoveryBase when a named branch was chosen).

import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';

const createDiscovery = vi.fn();
const setDiscoveryBase = vi.fn();
const getRepositoriesForProject = vi.fn();
const getProposedStrategy = vi.fn();
const listTerminalBranches = vi.fn();

vi.mock('../../lib/discovery', () => ({
  createDiscovery: (...args: unknown[]) => createDiscovery(...args),
  setDiscoveryBase: (...args: unknown[]) => setDiscoveryBase(...args),
}));

vi.mock('../../lib/project', () => ({
  getRepositoriesForProject: (...args: unknown[]) => getRepositoriesForProject(...args),
  getProposedStrategy: (...args: unknown[]) => getProposedStrategy(...args),
}));

vi.mock('../../lib/terminal', () => ({
  listTerminalBranches: (...args: unknown[]) => listTerminalBranches(...args),
}));

vi.mock('../../lib/agentCatalog', () => ({
  useAgentCatalog: () => ({
    agents: [{ kind: 'claude-code', display_label: 'claude-code', lists_models: false }],
    loading: false,
  }),
  effortLevelsFor: () => [],
}));

vi.mock('../../lib/agentModels', () => ({
  getAgentModels: () => Promise.resolve([]),
  modelSupportsImages: () => true,
}));

vi.mock('../../lib/machines', () => ({
  listMachines: () => Promise.resolve([]),
}));

vi.mock('../AttachmentDropzone', () => ({
  AttachmentDropzone: () => null,
}));

import { NewDiscoveryModal } from './NewDiscoveryModal';
import type { Discovery } from '../../types';

function discovery(overrides: Partial<Discovery> = {}): Discovery {
  return {
    id: 'disc-1',
    project_id: 'proj-1',
    title: 'A discovery',
    status: 'open',
    machine_id: 'local',
    agent_kind: 'claude-code',
    model: null,
    effort: null,
    resume_session_id: null,
    worktree_path: null,
    base_branch: null,
    integration_mr_url: null,
    integration_mr_state: null,
    attachments: [],
    total_cost: 0,
    tokens: 0,
    created_at: 0,
    updated_at: 0,
    ...overrides,
  };
}

function mount() {
  const onCreated = vi.fn();
  const onClose = vi.fn();
  render(
    <NewDiscoveryModal
      projectId="proj-1"
      machineId="local"
      seedTitle="My discovery"
      onClose={onClose}
      onCreated={onCreated}
    />,
  );
  return { onCreated, onClose };
}

describe('NewDiscoveryModal base branch', () => {
  it('prefills the named-branch name from the title until the user edits it', async () => {
    const user = userEvent.setup();
    getRepositoriesForProject.mockResolvedValue([{ id: 'repo-1', repo_path: '/x', provider_id: 'git' }]);
    getProposedStrategy.mockResolvedValue({
      worktree_strategy: { branch_prefix: 'demeteo/features/' },
    });
    listTerminalBranches.mockResolvedValue({ defaultBranch: 'main', branches: [] });

    mount();
    await user.click(screen.getByRole('radio', { name: 'Named branch' }));

    const input = await screen.findByLabelText('Base branch name');
    await waitFor(() =>
      expect((input as HTMLInputElement).value).toBe('demeteo/features/my-discovery'),
    );

    await user.clear(input);
    await user.type(input, 'custom-branch');
    expect((input as HTMLInputElement).value).toBe('custom-branch');

    // A later re-render of the seed effect (e.g. title unchanged, prefix
    // already resolved) must not clobber what the user just typed.
    await waitFor(() => expect((input as HTMLInputElement).value).toBe('custom-branch'));
  });

  it('does not re-create the Discovery on retry after a setDiscoveryBase failure, and never calls onCreated', async () => {
    const user = userEvent.setup();
    getRepositoriesForProject.mockResolvedValue([]);
    createDiscovery.mockResolvedValue(discovery());
    setDiscoveryBase.mockRejectedValueOnce(new Error('branch not found'));

    const { onCreated } = mount();

    await user.click(screen.getByRole('radio', { name: 'Named branch' }));
    const input = await screen.findByLabelText('Base branch name');
    await user.clear(input);
    await user.type(input, 'feat/x');

    await user.click(screen.getByRole('button', { name: 'Start discovery' }));

    const alert = await screen.findByRole('alert');
    expect(alert.textContent).toContain('Discovery created, but its base branch could not be set');
    expect(createDiscovery).toHaveBeenCalledTimes(1);
    expect(onCreated).not.toHaveBeenCalled();

    expect(screen.getByRole('button', { name: 'Retry base branch' })).toBeInTheDocument();

    setDiscoveryBase.mockResolvedValueOnce(discovery({ base_branch: 'feat/x' }));
    await user.click(screen.getByRole('button', { name: 'Retry base branch' }));

    await waitFor(() => expect(onCreated).toHaveBeenCalledTimes(1));
    expect(createDiscovery).toHaveBeenCalledTimes(1);
    expect(setDiscoveryBase).toHaveBeenCalledTimes(2);
  });

  it('surfaces the already-created Discovery via onCreated when Cancel is clicked after a setDiscoveryBase failure', async () => {
    const user = userEvent.setup();
    getRepositoriesForProject.mockResolvedValue([]);
    const created = discovery();
    createDiscovery.mockResolvedValue(created);
    setDiscoveryBase.mockRejectedValueOnce(new Error('branch not found'));

    const { onCreated, onClose } = mount();

    await user.click(screen.getByRole('radio', { name: 'Named branch' }));
    const input = await screen.findByLabelText('Base branch name');
    await user.clear(input);
    await user.type(input, 'feat/x');
    await user.click(screen.getByRole('button', { name: 'Start discovery' }));

    await screen.findByRole('alert');

    expect(screen.queryByRole('button', { name: 'Cancel' })).toBeNull();
    await user.click(screen.getByRole('button', { name: 'Close' }));

    expect(onCreated).toHaveBeenCalledWith(created);
    expect(onClose).not.toHaveBeenCalled();
  });

  it('leaves Cancel a true no-op before any Discovery has been created', async () => {
    const user = userEvent.setup();
    const { onCreated, onClose } = mount();

    await user.click(screen.getByRole('button', { name: 'Cancel' }));

    expect(onClose).toHaveBeenCalledTimes(1);
    expect(onCreated).not.toHaveBeenCalled();
    expect(createDiscovery).not.toHaveBeenCalled();
  });
});
