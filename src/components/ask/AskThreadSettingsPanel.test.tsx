import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { AskThreadSettingsPanel } from './AskThreadSettingsPanel';
import type { AskThread } from '../../types';

const agents = [
  { kind: 'claude-code', display_label: 'Claude Code', lists_models: true, default_model: null, install_command: '' },
  { kind: 'opencode', display_label: 'opencode', lists_models: true, default_model: null, install_command: '' },
  { kind: 'hermes', display_label: 'Hermes', lists_models: false, default_model: null, install_command: '' },
];

vi.mock('../../lib/agentCatalog', () => ({
  useAgentCatalog: () => ({ agents }),
  effortLevelsFor: () => ['low', 'medium', 'high'],
}));

vi.mock('../../lib/agentModels', () => ({
  getAgentModels: vi.fn().mockResolvedValue([{ value: 'sonnet-5' }, { value: 'opus-5' }]),
}));

const updateAskThreadSettings = vi.fn();
vi.mock('../../lib/ask', () => ({
  updateAskThreadSettings: (...args: unknown[]) => updateAskThreadSettings(...args),
}));

function thread(overrides: Partial<AskThread> = {}): AskThread {
  return {
    id: 'thread-1',
    project_id: 'project-1',
    title: 'Ask thread',
    status: 'open',
    agent_kind: 'claude-code',
    model: 'sonnet-5',
    effort: 'high',
    machine_id: 'local',
    worktree_path: null,
    session_id: null,
    turn_count: 3,
    cost_usd: 0.42,
    tokens: 1000,
    network: true,
    created_at: 0,
    updated_at: 0,
    ...overrides,
  };
}

beforeEach(() => {
  updateAskThreadSettings.mockReset();
  updateAskThreadSettings.mockResolvedValue(thread());
});

describe('AskThreadSettingsPanel', () => {
  it('renders the agent as a read-only label, not a selectable control', async () => {
    render(
      <AskThreadSettingsPanel thread={thread()} onClose={() => {}} onSaved={() => {}} />,
    );
    await waitFor(() => expect(screen.getByTestId('ask-settings-agent')).toBeInTheDocument());

    const agentField = screen.getByTestId('ask-settings-agent');
    expect(agentField.textContent).toContain('claude-code');
    expect(agentField.querySelector('button')).toBeNull();
    expect(agentField.querySelector('[role="radio"]')).toBeNull();
  });

  it.each(agents.map((a) => a.kind))(
    'renders the web-access toggle enabled for agent_kind=%s',
    async (kind) => {
      render(
        <AskThreadSettingsPanel
          thread={thread({ agent_kind: kind })}
          onClose={() => {}}
          onSaved={() => {}}
        />,
      );
      const toggle = await screen.findByTestId('ask-settings-network-toggle');
      expect(toggle).not.toBeDisabled();
      expect(toggle).toHaveAttribute('aria-checked', 'true');
    },
  );

  it('saves only the fields that changed', async () => {
    render(
      <AskThreadSettingsPanel thread={thread()} onClose={() => {}} onSaved={() => {}} />,
    );

    const toggle = await screen.findByTestId('ask-settings-network-toggle');
    fireEvent.click(toggle);

    fireEvent.click(screen.getByText('Save for this thread'));

    await waitFor(() => expect(updateAskThreadSettings).toHaveBeenCalledTimes(1));
    expect(updateAskThreadSettings).toHaveBeenCalledWith('thread-1', { network: false });
  });

  it('calls onSaved and onClose with the updated thread', async () => {
    const updated = thread({ network: false });
    updateAskThreadSettings.mockResolvedValue(updated);
    const onSaved = vi.fn();
    const onClose = vi.fn();

    render(<AskThreadSettingsPanel thread={thread()} onClose={onClose} onSaved={onSaved} />);

    const toggle = await screen.findByTestId('ask-settings-network-toggle');
    fireEvent.click(toggle);
    fireEvent.click(screen.getByText('Save for this thread'));

    await waitFor(() => expect(onSaved).toHaveBeenCalledWith(updated));
    expect(onClose).toHaveBeenCalled();
  });

  it('lets model be changed via the same OptionPill radiogroup pattern', async () => {
    render(
      <AskThreadSettingsPanel thread={thread()} onClose={() => {}} onSaved={() => {}} />,
    );

    const opusPill = await screen.findByText('opus-5');
    fireEvent.click(opusPill);
    fireEvent.click(screen.getByText('Save for this thread'));

    await waitFor(() => expect(updateAskThreadSettings).toHaveBeenCalledTimes(1));
    expect(updateAskThreadSettings).toHaveBeenCalledWith('thread-1', { model: 'opus-5' });
  });

  it('qualifies the network claim on a hermes thread, where enforcement is not established', async () => {
    render(
      <AskThreadSettingsPanel
        thread={thread({ agent_kind: 'hermes' })}
        onClose={() => {}}
        onSaved={() => {}}
      />,
    );

    const note = await screen.findByTestId('ask-network-unenforced-note');
    expect(note.textContent).toContain('hermes');
  });

  it('leaves the network claim unqualified on claude-code, where enforcement is established', async () => {
    render(
      <AskThreadSettingsPanel thread={thread()} onClose={() => {}} onSaved={() => {}} />,
    );
    await waitFor(() => expect(screen.getByTestId('ask-settings-agent')).toBeInTheDocument());

    expect(screen.queryByTestId('ask-network-unenforced-note')).toBeNull();
  });

  // AC5: the note is copy beside the control, never a gate on it. The effort
  // list is `useAgentCatalog()`'s, the model list is the machine probe's, and
  // neither narrows for the harness the note names.
  it.each(agents.map((a) => a.kind))(
    'gates nothing on agent_kind=%s — toggle and both pickers stay whole',
    async (kind) => {
      render(
        <AskThreadSettingsPanel
          thread={thread({ agent_kind: kind })}
          onClose={() => {}}
          onSaved={() => {}}
        />,
      );

      const toggle = await screen.findByTestId('ask-settings-network-toggle');
      expect(toggle).not.toHaveAttribute('disabled');
      fireEvent.click(toggle);
      expect(toggle).toHaveAttribute('aria-checked', 'false');

      const models = await screen.findByRole('radiogroup', { name: 'Model' });
      expect([...models.children].map((pill) => pill.textContent)).toEqual(['sonnet-5', 'opus-5']);
      const efforts = screen.getByRole('radiogroup', { name: 'Effort' });
      expect([...efforts.children].map((pill) => pill.textContent)).toEqual(['Low', 'Medium', 'High']);
    },
  );

  it('describes the Sources list as the live answer renders it', async () => {
    render(
      <AskThreadSettingsPanel thread={thread()} onClose={() => {}} onSaved={() => {}} />,
    );
    await waitFor(() => expect(screen.getByTestId('ask-settings-agent')).toBeInTheDocument());

    expect(screen.getByTestId('ask-settings-network-copy').textContent).toContain(
      'while the turn runs',
    );
  });

  it('does not promise Sources lists a search the agent only counted', async () => {
    render(
      <AskThreadSettingsPanel thread={thread()} onClose={() => {}} onSaved={() => {}} />,
    );
    await waitFor(() => expect(screen.getByTestId('ask-settings-agent')).toBeInTheDocument());

    const copy = screen.getByTestId('ask-settings-network-copy').textContent ?? '';
    expect(copy).not.toContain('Each distinct URL');
    expect(copy).toContain('search');
    expect(copy).toContain('not listed');
  });

  it('drops the fetch sentence once the toggle reads network: Deny', async () => {
    render(
      <AskThreadSettingsPanel thread={thread()} onClose={() => {}} onSaved={() => {}} />,
    );

    fireEvent.click(await screen.findByTestId('ask-settings-network-toggle'));

    const copy = screen.getByTestId('ask-settings-network-copy').textContent ?? '';
    expect(copy).not.toContain('may fetch');
    expect(copy).not.toContain('Sources in the answer while the turn runs');
    expect(copy).toContain('A fetch is refused');
  });
});
