import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { AskWorkspaceHeader } from './AskWorkspaceHeader';
import type { AskThread } from '../../types';
import { NavigationProvider } from '../../context';

// These surfaces carry the shared Back control, which reads the navigation
// stack, so every render needs a provider around it.
function renderWithNav(ui: Parameters<typeof render>[0]) {
  return render(ui, { wrapper: NavigationProvider });
}

afterEach(cleanup);

function metric(label: string): HTMLElement {
  const found = screen.getByTestId('metric-strip').querySelector(`[data-metric="${label}"]`);
  if (found === null) throw new Error(`no metric ${label}`);
  return found as HTMLElement;
}

function thread(overrides: Partial<AskThread> = {}): AskThread {
  return {
    id: 't1',
    project_id: 'p1',
    title: 'How a Step reaches the feature branch',
    status: 'open',
    agent_kind: 'claude-code',
    model: null,
    effort: null,
    machine_id: 'local',
    worktree_path: null,
    session_id: null,
    turn_count: 4,
    cost_usd: 0.42,
    tokens: 48200,
    network: true,
    created_at: 0,
    updated_at: 0,
    ...overrides,
  };
}

function renderHeader(overrides: Partial<Parameters<typeof AskWorkspaceHeader>[0]> = {}) {
  const props = {
    thread: thread(),
    onNewThread: vi.fn(),
    onOpenSettings: vi.fn(),
    onToggleOpen: vi.fn(),
    onDelete: vi.fn(),
    busy: false,
    ...overrides,
  };
  renderWithNav(<AskWorkspaceHeader {...props} />);
  return props;
}

describe('AskWorkspaceHeader', () => {
  it("renders title, kind chip, and Turns/Spend/Tokens straight off the thread row", () => {
    renderHeader();

    expect(screen.getByText('How a Step reaches the feature branch')).toBeInTheDocument();
    expect(screen.getByText('claude-code')).toBeInTheDocument();

    expect(metric('Turns')).toHaveTextContent('4');
    expect(metric('Spend')).toHaveTextContent('$0.420');
    expect(metric('Tokens')).toHaveTextContent('48.2k');
  });

  it('calls onNewThread when New thread is clicked', () => {
    const { onNewThread } = renderHeader();

    fireEvent.click(screen.getByTestId('ask-new-thread'));

    expect(onNewThread).toHaveBeenCalledTimes(1);
  });

  it('offers Close on an open thread and Reopen on a closed one', () => {
    const { onToggleOpen } = renderHeader();
    expect(screen.getByTestId('ask-toggle-open')).toHaveTextContent('Close thread');
    fireEvent.click(screen.getByTestId('ask-toggle-open'));
    expect(onToggleOpen).toHaveBeenCalledTimes(1);

    cleanup();
    renderHeader({ thread: thread({ status: 'closed' }) });
    expect(screen.getByTestId('ask-toggle-open')).toHaveTextContent('Reopen thread');
    expect(screen.getByText('Closed')).toBeInTheDocument();
  });

  it('asks the caller to delete, and disables both while one is in flight', () => {
    const { onDelete } = renderHeader();
    fireEvent.click(screen.getByTestId('ask-delete-thread'));
    expect(onDelete).toHaveBeenCalledTimes(1);

    cleanup();
    renderHeader({ busy: true });
    expect(screen.getByTestId('ask-delete-thread')).toBeDisabled();
    expect(screen.getByTestId('ask-toggle-open')).toBeDisabled();
  });
});
