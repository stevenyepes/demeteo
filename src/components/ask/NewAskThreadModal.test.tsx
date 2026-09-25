import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import NewAskThreadModal from './NewAskThreadModal';
import { getAgentModels } from '../../lib/agentModels';
import { getAgentConfigs, listMachines } from '../../lib/machines';
import type { AskThread, Machine } from '../../types';

const agents = [
  { kind: 'claude-code', display_label: 'Claude Code', lists_models: true, default_model: null, install_command: '' },
  { kind: 'opencode', display_label: 'opencode', lists_models: true, default_model: null, install_command: '' },
  { kind: 'hermes', display_label: 'Hermes', lists_models: true, default_model: null, install_command: '' },
  { kind: 'codex', display_label: 'Codex', lists_models: true, default_model: null, install_command: '' },
];

vi.mock('../../lib/agentCatalog', () => ({
  useAgentCatalog: () => ({ agents }),
  effortLevelsFor: (_agents: unknown, kind: string) => {
    if (kind === 'hermes') return [];
    if (kind === 'codex') return ['low', 'medium', 'high', 'xhigh'];
    return ['low', 'medium', 'high', 'xhigh', 'max'];
  },
}));

vi.mock('../../lib/agentModels', () => ({
  getAgentModels: vi.fn(),
  modelSupportsImages: () => false,
}));

vi.mock('../../lib/machines', () => ({
  listMachines: vi.fn().mockResolvedValue([]),
  getAgentConfigs: vi.fn(),
}));

function config(kind: string, enabled = true, available = true) {
  return { kind, enabled, available, install_command: '', display_label: kind };
}

const MODELS = [
  { value: 'sonnet-5', name: 'Sonnet 5' },
  { value: 'opus-5-5', name: 'Opus 5.5' },
];

/** Model probes the declared configs do not justify. The modal swallows a
 *  rejected probe into an empty list, so a rejection alone would pass silently;
 *  `afterEach` fails the test on any entry here instead. */
let undeclaredModelProbes: string[] = [];

/** Configs per machine id; any id a test did not configure rejects, so a probe
 *  against the wrong machine fails loudly instead of answering a default. The
 *  model probe answers only for a harness that machine offers. */
function configsByMachine(byMachine: Record<string, ReturnType<typeof config>[]>) {
  vi.mocked(getAgentConfigs).mockImplementation(async (machineId: string) => {
    const configs = byMachine[machineId];
    if (!configs) throw new Error(`no agent configs for machine ${machineId}`);
    return configs;
  });
  vi.mocked(getAgentModels).mockImplementation(async (machineId: string, kind: string) => {
    const offered = (byMachine[machineId] ?? []).some(
      (c) => c.kind === kind && c.enabled && c.available,
    );
    if (!offered) {
      undeclaredModelProbes.push(`${machineId}/${kind}`);
      throw new Error(`${machineId} does not offer ${kind}`);
    }
    return MODELS;
  });
}

function renderModal(props: Partial<React.ComponentProps<typeof NewAskThreadModal>> = {}) {
  return render(
    <NewAskThreadModal
      projectId="project-1"
      machineId="local"
      seedTitle=""
      onClose={vi.fn()}
      onCreated={vi.fn()}
      {...props}
    />,
  );
}

async function harnessOffers(kind: string): Promise<HTMLSelectElement> {
  const harness = screen.getByRole('combobox', { name: 'Harness' }) as HTMLSelectElement;
  await waitFor(() => expect(harness.value).toBe(kind));
  return harness;
}

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
  vi.mocked(getAgentConfigs).mockReset();
  vi.mocked(getAgentModels).mockReset();
  undeclaredModelProbes = [];
  configsByMachine({
    local: [config('claude-code'), config('opencode'), config('hermes')],
  });
});

afterEach(() => {
  cleanup();
  expect(undeclaredModelProbes).toEqual([]);
});

describe('NewAskThreadModal', () => {
  // The shared picker, not the pill radiogroups it replaced.
  it('renders harness, model and effort as the shared picker comboboxes', async () => {
    renderModal();

    await harnessOffers('claude-code');
    expect(screen.getByRole('combobox', { name: 'Effort' })).toBeInTheDocument();
    expect(screen.queryAllByRole('radio')).toHaveLength(0);

    const model = await screen.findByRole('combobox', { name: 'Model' });
    await screen.findByRole('option', { name: 'Sonnet 5' });
    const values = Array.from((model as HTMLSelectElement).options).map((o) => o.value);
    expect(values).toEqual(['', 'sonnet-5', 'opus-5-5']);
  });

  // Only harnesses the machine both has and allows are offered.
  it('offers only enabled and available harnesses from the chosen machine', async () => {
    configsByMachine({
      'host-7': [
        config('claude-code', true, true),
        config('opencode', true, false),
        config('hermes', false, true),
      ],
    });

    renderModal({ machineId: 'host-7' });

    const harness = await harnessOffers('claude-code');
    const kinds = Array.from(harness.options)
      .map((o) => o.value)
      .filter(Boolean);
    expect(kinds).toEqual(['claude-code']);
    expect(getAgentConfigs).toHaveBeenCalledWith('host-7', false);
    expect(getAgentConfigs).not.toHaveBeenCalledWith(expect.anything(), true);
  });

  // The harness list belongs to the machine, so moving it re-asks.
  it('re-checks agents on a machine change and replaces a pick it cannot run', async () => {
    vi.mocked(listMachines).mockResolvedValueOnce([
      { id: 'host-2', name: 'Build box' } as Machine,
    ]);
    configsByMachine({
      local: [config('claude-code'), config('opencode')],
      'host-2': [config('claude-code', true, false), config('hermes'), config('opencode')],
    });

    renderModal();

    await harnessOffers('claude-code');
    await screen.findByRole('option', { name: 'Build box' });
    fireEvent.change(screen.getByRole('combobox', { name: 'Machine' }), {
      target: { value: 'host-2' },
    });

    const harness = await harnessOffers('hermes');
    expect(getAgentConfigs).toHaveBeenCalledWith('host-2', false);
    const kinds = Array.from(harness.options)
      .map((o) => o.value)
      .filter(Boolean);
    expect(kinds).toEqual(['hermes', 'opencode']);
    await waitFor(() => expect(getAgentModels).toHaveBeenCalledWith('host-2', 'hermes'));
    expect(getAgentModels).not.toHaveBeenCalledWith('host-2', 'claude-code');
  });

  // Nothing offerable is said out loud, not papered over by the catalog.
  it('shows an empty state and blocks start when no agent is offerable', async () => {
    configsByMachine({ local: [config('claude-code', false, true), config('opencode', true, false)] });

    renderModal({ seedTitle: 'Nothing to run' });

    expect(await screen.findByText(/no coding agent is available on local/i)).toBeVisible();
    expect(screen.getByRole('button', { name: /start thread/i })).toBeDisabled();
    const harness = screen.getByRole('combobox', { name: 'Harness' }) as HTMLSelectElement;
    expect(Array.from(harness.options).map((o) => o.value).filter(Boolean)).toEqual([]);
  });

  // A failed check is not an empty machine, but it offers nothing either.
  it('shows the same state and blocks start when the agent check fails', async () => {
    vi.mocked(getAgentConfigs).mockRejectedValue(new Error('ssh: connection refused'));

    renderModal({ seedTitle: 'Unreachable' });

    expect(await screen.findByText(/no coding agent is available on local/i)).toBeVisible();
    expect(screen.getByRole('button', { name: /start thread/i })).toBeDisabled();
    const harness = screen.getByRole('combobox', { name: 'Harness' }) as HTMLSelectElement;
    expect(Array.from(harness.options).map((o) => o.value).filter(Boolean)).toEqual([]);
  });

  it('names the machine by its label, not its id, in the no-agent note', async () => {
    vi.mocked(listMachines).mockResolvedValueOnce([
      { id: 'host-9', name: 'Build box' } as Machine,
    ]);
    configsByMachine({ 'host-9': [config('claude-code', true, false)] });

    renderModal({ machineId: 'host-9' });

    const note = await screen.findByTestId('ask-new-thread-no-agent');
    await waitFor(() => expect(note).toHaveTextContent(/available on Build box/));
    expect(note).not.toHaveTextContent('host-9');
    expect(note).toHaveTextContent(/could not be reached/i);
  });

  it('re-checks the same machine and offers a harness that has since appeared', async () => {
    let offered = [config('claude-code', true, false)];
    vi.mocked(getAgentConfigs).mockImplementation(async () => offered);

    renderModal();

    await screen.findByTestId('ask-new-thread-no-agent');
    offered = [config('claude-code')];
    fireEvent.click(screen.getByRole('button', { name: 'Re-check' }));

    await harnessOffers('claude-code');
    expect(getAgentConfigs).toHaveBeenCalledTimes(2);
    expect(getAgentConfigs).toHaveBeenLastCalledWith('local', false);
    expect(screen.queryByTestId('ask-new-thread-no-agent')).toBeNull();
  });

  it('offers a re-check when the agent check itself failed', async () => {
    vi.mocked(getAgentConfigs)
      .mockRejectedValueOnce(new Error('ssh: connection refused'))
      .mockResolvedValueOnce([config('opencode')]);

    renderModal();

    await screen.findByText(/could not be checked/i);
    fireEvent.click(screen.getByRole('button', { name: 'Re-check' }));

    await harnessOffers('opencode');
  });

  it('lists no harness from the previous machine while the next one is checked', async () => {
    vi.mocked(listMachines).mockResolvedValueOnce([
      { id: 'host-2', name: 'Build box' } as Machine,
    ]);
    configsByMachine({ local: [config('claude-code'), config('opencode')], 'host-2': [config('codex')] });
    let answerHost2: (configs: ReturnType<typeof config>[]) => void = () => {};
    vi.mocked(getAgentConfigs).mockImplementation((machineId: string) =>
      machineId === 'local'
        ? Promise.resolve([config('claude-code'), config('opencode')])
        : new Promise((resolve) => {
            answerHost2 = resolve;
          }),
    );

    renderModal();

    await harnessOffers('claude-code');
    await screen.findByRole('option', { name: 'Build box' });
    fireEvent.change(screen.getByRole('combobox', { name: 'Machine' }), {
      target: { value: 'host-2' },
    });

    const harness = screen.getByRole('combobox', { name: 'Harness' }) as HTMLSelectElement;
    await waitFor(() => expect(getAgentConfigs).toHaveBeenCalledWith('host-2', false));
    expect(Array.from(harness.options).map((o) => o.value).filter(Boolean)).toEqual([]);

    answerHost2([config('codex')]);
    await harnessOffers('codex');
  });

  it('ignores a previous machine\'s configs that answer after the switch', async () => {
    vi.mocked(listMachines).mockResolvedValueOnce([
      { id: 'host-2', name: 'Build box' } as Machine,
    ]);
    configsByMachine({ local: [config('claude-code')], 'host-2': [config('codex')] });
    let answerLocal: (configs: ReturnType<typeof config>[]) => void = () => {};
    vi.mocked(getAgentConfigs).mockImplementation((machineId: string) =>
      machineId === 'local'
        ? new Promise((resolve) => {
            answerLocal = resolve;
          })
        : Promise.resolve([config('codex')]),
    );

    renderModal();

    await screen.findByRole('option', { name: 'Build box' });
    fireEvent.change(screen.getByRole('combobox', { name: 'Machine' }), {
      target: { value: 'host-2' },
    });
    const harness = await harnessOffers('codex');

    answerLocal([config('claude-code')]);
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(harness.value).toBe('codex');
    expect(Array.from(harness.options).map((o) => o.value).filter(Boolean)).toEqual(['codex']);
  });

  it('labels the empty effort option as the agent default', async () => {
    renderModal();

    await harnessOffers('claude-code');
    const effort = screen.getByRole('combobox', { name: 'Effort' }) as HTMLSelectElement;
    expect(effort.options[0]).toHaveValue('');
    expect(effort.options[0]).toHaveTextContent('Agent default');
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
        model: null,
        machineId: 'local',
        network: true,
      }),
    );
  });

  // The picker's choices reach `createAskThread` unchanged.
  it('sends the chosen harness, model and effort', async () => {
    createAskThread.mockResolvedValue(thread());
    renderModal();

    const harness = await harnessOffers('claude-code');
    fireEvent.change(harness, { target: { value: 'opencode' } });
    await screen.findByRole('option', { name: 'Opus 5.5' });
    fireEvent.change(screen.getByRole('combobox', { name: 'Model' }), {
      target: { value: 'opus-5-5' },
    });
    fireEvent.change(screen.getByRole('combobox', { name: 'Effort' }), {
      target: { value: 'max' },
    });
    fireEvent.change(screen.getByPlaceholderText(/ask about the auth flow/i), {
      target: { value: 'Chosen thread' },
    });

    fireEvent.click(screen.getByRole('button', { name: /start thread/i }));

    await waitFor(() => expect(createAskThread).toHaveBeenCalled());
    expect(createAskThread).toHaveBeenCalledWith(
      expect.objectContaining({
        agentKind: 'opencode',
        model: 'opus-5-5',
        effort: 'max',
        machineId: 'local',
      }),
    );
  });

  it('clamps effort when the user picks a harness with a narrower ladder', async () => {
    configsByMachine({ local: [config('opencode'), config('codex')] });
    renderModal();

    await harnessOffers('opencode');
    const effort = screen.getByRole('combobox', { name: 'Effort' }) as HTMLSelectElement;
    fireEvent.change(effort, { target: { value: 'max' } });
    fireEvent.change(screen.getByRole('combobox', { name: 'Harness' }), {
      target: { value: 'codex' },
    });

    await waitFor(() => expect(effort.value).toBe('xhigh'));
  });

  it('clamps effort when a machine change seeds a harness with a narrower ladder', async () => {
    createAskThread.mockResolvedValue(thread());
    vi.mocked(listMachines).mockResolvedValueOnce([
      { id: 'host-2', name: 'Build box' } as Machine,
    ]);
    configsByMachine({
      local: [config('claude-code'), config('opencode')],
      'host-2': [config('codex')],
    });
    renderModal({ seedTitle: 'Codex thread' });

    fireEvent.change(await harnessOffers('claude-code'), { target: { value: 'opencode' } });
    fireEvent.change(screen.getByRole('combobox', { name: 'Effort' }), {
      target: { value: 'max' },
    });
    await screen.findByRole('option', { name: 'Build box' });
    fireEvent.change(screen.getByRole('combobox', { name: 'Machine' }), {
      target: { value: 'host-2' },
    });

    await harnessOffers('codex');
    const effort = screen.getByRole('combobox', { name: 'Effort' }) as HTMLSelectElement;
    await waitFor(() => expect(effort.value).toBe('xhigh'));

    fireEvent.click(screen.getByRole('button', { name: /start thread/i }));

    await waitFor(() => expect(createAskThread).toHaveBeenCalled());
    expect(createAskThread).toHaveBeenCalledWith(
      expect.objectContaining({ agentKind: 'codex', effort: 'xhigh', machineId: 'host-2' }),
    );
  });

  it('sends no model when the agent default entry is left selected', async () => {
    createAskThread.mockResolvedValue(thread());
    renderModal({ seedTitle: 'Default model thread' });

    await harnessOffers('claude-code');
    await screen.findByRole('option', { name: 'Sonnet 5' });
    fireEvent.change(screen.getByRole('combobox', { name: 'Model' }), {
      target: { value: 'sonnet-5' },
    });
    fireEvent.change(screen.getByRole('combobox', { name: 'Model' }), {
      target: { value: '' },
    });

    fireEvent.click(screen.getByRole('button', { name: /start thread/i }));

    await waitFor(() => expect(createAskThread).toHaveBeenCalled());
    expect(createAskThread).toHaveBeenCalledWith(
      expect.objectContaining({ agentKind: 'claude-code', model: null }),
    );
  });

  it('disables effort on hermes and sends none', async () => {
    createAskThread.mockResolvedValue(thread());
    renderModal({ seedTitle: 'Hermes thread' });

    fireEvent.change(await harnessOffers('claude-code'), { target: { value: 'hermes' } });

    expect(await screen.findByTestId('ask-network-unenforced-note')).toBeInTheDocument();
    expect(screen.getByRole('combobox', { name: 'Effort' })).toBeDisabled();
    expect(screen.getByTestId('ask-new-thread-network-toggle')).toBeEnabled();

    await screen.findByRole('option', { name: 'Sonnet 5' });
    fireEvent.click(screen.getByRole('button', { name: /start thread/i }));

    await waitFor(() => expect(createAskThread).toHaveBeenCalled());
    expect(createAskThread).toHaveBeenCalledWith(
      expect.objectContaining({ agentKind: 'hermes', effort: null }),
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

    // claude-code is the first offerable harness, so the modal opens on it.
    const harness = await harnessOffers('claude-code');
    expect(screen.queryByTestId('ask-network-unenforced-note')).toBeNull();

    fireEvent.change(harness, { target: { value: 'hermes' } });

    const note = await screen.findByTestId('ask-network-unenforced-note');
    expect(note.textContent).toContain('hermes');

    fireEvent.change(harness, { target: { value: 'opencode' } });
    await waitFor(() =>
      expect(screen.queryByTestId('ask-network-unenforced-note')).toBeNull(),
    );
  });

  // The note is copy beside the control, never a gate on it — the toggle
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

    fireEvent.change(await harnessOffers('claude-code'), { target: { value: 'hermes' } });
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
