import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import NewAskThreadModal from './NewAskThreadModal';
import type { AskThread } from '../../types';

const agents = [
  { kind: 'claude-code', display_label: 'Claude Code', lists_models: true, default_model: null, install_command: '' },
  { kind: 'opencode', display_label: 'opencode', lists_models: true, default_model: null, install_command: '' },
  { kind: 'hermes', display_label: 'Hermes', lists_models: true, default_model: null, install_command: '' },
];

vi.mock('../../lib/agentCatalog', () => ({
  useAgentCatalog: () => ({ agents }),
  effortLevelsFor: () => ['low', 'medium', 'high', 'xhigh', 'max'],
}));

vi.mock('../../lib/agentModels', () => ({
  getAgentModels: vi.fn().mockResolvedValue([{ value: 'sonnet-5' }]),
  modelSupportsImages: () => false,
}));

vi.mock('../../lib/machines', () => ({
  listMachines: vi.fn().mockResolvedValue([]),
}));

const createAskThread = vi.fn();
vi.mock('../../lib/ask', () => ({
  createAskThread: (...args: unknown[]) => createAskThread(...args),
}));

function thread(): AskThread {
  return {
    id: 'thread-1',
    project_id: 'project-1',
    title: 'New thread',
    status: 'open',
    agent_kind: 'claude-code',
    model: 'sonnet-5',
    effort: 'high',
    machine_id: 'local',
    worktree_path: null,
    session_id: null,
    turn_count: 0,
    cost_usd: 0,
    tokens: 0,
    network: true,
    created_at: 0,
    updated_at: 0,
  };
}

beforeEach(() => {
  createAskThread.mockReset();
});

describe('NewAskThreadModal', () => {
  it('lists agents from the catalog, not a hardcoded array', async () => {
    render(
      <NewAskThreadModal
        projectId="project-1"
        machineId="local"
        seedTitle=""
        onClose={vi.fn()}
        onCreated={vi.fn()}
      />,
    );

    expect(await screen.findByRole('radio', { name: 'claude-code' })).toBeInTheDocument();
    expect(screen.getByRole('radio', { name: 'opencode' })).toBeInTheDocument();
  });

  it('shows the supplied project name in the eyebrow', async () => {
    render(
      <NewAskThreadModal
        projectId="project-1"
        machineId="local"
        projectName="Acme API"
        seedTitle=""
        onClose={vi.fn()}
        onCreated={vi.fn()}
      />,
    );

    expect(await screen.findByText('Acme API')).toBeInTheDocument();
    expect(screen.queryByText('demeteo')).toBeNull();
  });

  it('falls back to a generic eyebrow when no project name is given', async () => {
    render(
      <NewAskThreadModal
        projectId="project-1"
        machineId="local"
        seedTitle=""
        onClose={vi.fn()}
        onCreated={vi.fn()}
      />,
    );

    expect(await screen.findByText('this project')).toBeInTheDocument();
  });

  it('creates a thread and calls onCreated on submit', async () => {
    createAskThread.mockResolvedValue(thread());
    const onCreated = vi.fn();

    render(
      <NewAskThreadModal
        projectId="project-1"
        machineId="local"
        seedTitle=""
        onClose={vi.fn()}
        onCreated={onCreated}
      />,
    );

    fireEvent.change(screen.getByPlaceholderText(/ask about the auth flow/i), {
      target: { value: 'My thread' },
    });

    await waitFor(() => expect(screen.getByRole('button', { name: /start thread/i })).toBeEnabled());
    fireEvent.click(screen.getByRole('button', { name: /start thread/i }));

    await waitFor(() => expect(onCreated).toHaveBeenCalledWith(thread()));
    expect(createAskThread).toHaveBeenCalledWith(
      expect.objectContaining({
        projectId: 'project-1',
        title: 'My thread',
        agentKind: 'claude-code',
        machineId: 'local',
        network: true,
      }),
    );
  });

  it('opens the thread with the network off when the control is toggled off', async () => {
    createAskThread.mockResolvedValue({ ...thread(), network: false });

    render(
      <NewAskThreadModal
        projectId="project-1"
        machineId="local"
        seedTitle=""
        onClose={vi.fn()}
        onCreated={vi.fn()}
      />,
    );

    fireEvent.change(screen.getByPlaceholderText(/ask about the auth flow/i), {
      target: { value: 'Offline thread' },
    });
    const toggle = screen.getByTestId('ask-new-thread-network-toggle');
    expect(toggle).toHaveAttribute('aria-checked', 'true');
    fireEvent.click(toggle);
    expect(toggle).toHaveAttribute('aria-checked', 'false');

    await waitFor(() => expect(screen.getByRole('button', { name: /start thread/i })).toBeEnabled());
    fireEvent.click(screen.getByRole('button', { name: /start thread/i }));

    await waitFor(() => expect(createAskThread).toHaveBeenCalled());
    expect(createAskThread).toHaveBeenCalledWith(
      expect.objectContaining({ title: 'Offline thread', network: false }),
    );
  });

  it('starts the name field from the seed a Try chip carried in', async () => {
    render(
      <NewAskThreadModal
        projectId="project-1"
        machineId="local"
        seedTitle="Draw the architecture of Acme API"
        onClose={vi.fn()}
        onCreated={vi.fn()}
      />,
    );

    const name = screen.getByPlaceholderText(/ask about the auth flow/i) as HTMLInputElement;
    expect(name.value).toBe('Draw the architecture of Acme API');
    await waitFor(() => expect(screen.getByRole('button', { name: /start thread/i })).toBeEnabled());
  });

  it('qualifies the network claim once hermes is picked, where enforcement is not established', async () => {
    render(
      <NewAskThreadModal
        projectId="project-1"
        machineId="local"
        seedTitle=""
        onClose={vi.fn()}
        onCreated={vi.fn()}
      />,
    );

    // claude-code is the catalog's first entry, so the modal opens on it.
    expect(await screen.findByRole('radio', { name: 'claude-code' })).toBeInTheDocument();
    expect(screen.queryByTestId('ask-network-unenforced-note')).toBeNull();

    fireEvent.click(screen.getByRole('radio', { name: 'hermes' }));

    const note = await screen.findByTestId('ask-network-unenforced-note');
    expect(note.textContent).toContain('hermes');

    fireEvent.click(screen.getByRole('radio', { name: 'opencode' }));
    await waitFor(() =>
      expect(screen.queryByTestId('ask-network-unenforced-note')).toBeNull(),
    );
  });

  // AC5: the note is copy beside the control, never a gate on it — the toggle
  // stays live for the harness it names, exactly as in the settings panel.
  it('leaves the web-access toggle live on hermes', async () => {
    render(
      <NewAskThreadModal
        projectId="project-1"
        machineId="local"
        seedTitle=""
        onClose={vi.fn()}
        onCreated={vi.fn()}
      />,
    );

    fireEvent.click(await screen.findByRole('radio', { name: 'hermes' }));
    await screen.findByTestId('ask-network-unenforced-note');

    const toggle = screen.getByTestId('ask-new-thread-network-toggle');
    expect(toggle).toBeEnabled();
    fireEvent.click(toggle);
    expect(toggle).toHaveAttribute('aria-checked', 'false');
  });

  it('does not claim network access once the control is off', async () => {
    render(
      <NewAskThreadModal
        projectId="project-1"
        machineId="local"
        seedTitle=""
        onClose={vi.fn()}
        onCreated={vi.fn()}
      />,
    );

    const capability = await screen.findByTestId('ask-new-thread-capability');
    expect(capability).toHaveTextContent(/reaches the network/i);

    fireEvent.click(screen.getByTestId('ask-new-thread-network-toggle'));

    expect(capability).not.toHaveTextContent(/reaches the network/i);
  });
});
