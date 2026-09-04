// The claim this file defends: `widthMode`/`hidden` are additive-only —
// omitting them renders exactly today's fixed-560px column, and `hidden`
// hides via class + `aria-hidden` without unmounting (AGENTS.md §3, "no
// unmounting" in this ticket's own spec).

import { cleanup, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';

import { NO_TURN } from '../../lib/discoveryActivity';
import type { Discovery } from '../../types';
import { InterviewColumn } from './InterviewColumn';
import type { DiscoveryStreamStore } from './useDiscoveryStream';

afterEach(cleanup);

const discovery: Discovery = {
  id: 'd-1',
  project_id: 'p-1',
  title: 'multi-client runner',
  status: 'open',
  machine_id: 'local',
  agent_kind: 'claude-code',
  model: null,
  effort: null,
  resume_session_id: null,
  worktree_path: null,
  attachments: [],
  total_cost: 0,
  tokens: 0,
  created_at: 0,
  updated_at: 0,
};

const store: DiscoveryStreamStore = {
  subscribe: () => () => {},
  read: () => NO_TURN,
};

function renderColumn(props: Partial<React.ComponentProps<typeof InterviewColumn>> = {}) {
  return render(
    <InterviewColumn
      discovery={discovery}
      messages={[]}
      blocks={[]}
      machineLabel="local"
      pending={false}
      store={store}
      onSend={() => {}}
      onRefresh={() => {}}
      {...props}
    />,
  );
}

function rootDiv(container: HTMLElement): HTMLElement {
  return container.firstElementChild as HTMLElement;
}

describe('InterviewColumn', () => {
  it('renders the fixed 560px column when widthMode/hidden are omitted', () => {
    const { container } = renderColumn();

    const root = rootDiv(container);
    expect(root.className).toContain('w-[560px]');
    expect(root.className).toContain('shrink-0');
    expect(root.className).toContain('border-r');
    expect(root.className).toContain('border-white/5');
    expect(root.className).not.toContain('w-full');
    expect(root.classList.contains('hidden')).toBe(false);
    expect(root.getAttribute('aria-hidden')).toBeNull();
  });

  it('swaps to full-width classes when widthMode is "full"', () => {
    const { container } = renderColumn({ widthMode: 'full' });

    const root = rootDiv(container);
    expect(root.className).toContain('w-full');
    expect(root.className).toContain('min-w-0');
    expect(root.className).toContain('flex-1');
    expect(root.className).not.toContain('w-[560px]');
  });

  it('drops the right border in "full" widthMode, since it is the sole visible pane', () => {
    const { container } = renderColumn({ widthMode: 'full' });

    const root = rootDiv(container);
    expect(root.className).not.toContain('border-r');
    expect(root.className).not.toContain('border-white/5');
  });

  it('hides via class and aria-hidden without unmounting the composer', () => {
    const { container } = renderColumn({ hidden: true });

    const root = rootDiv(container);
    expect(root.classList.contains('hidden')).toBe(true);
    expect(root.getAttribute('aria-hidden')).toBe('true');
    expect(screen.getByTestId('interview-attach')).toBeTruthy();
  });
});
