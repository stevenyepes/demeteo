/**
 * jsdom lays nothing out, so what a test can hold here is the contract the
 * classes make together — the same limit `TicketGraph.test.tsx`'s centring
 * suite works within, and the same reason each assertion below names the one
 * class whose removal brings the bug back.
 *
 * The bug: the tile is a fixed height (the edges anchor to its bottom edge), so
 * a title long enough to wrap used to push the lane chip past it and the card
 * showed no status at all.
 */
import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { indexTickets } from '../../lib/ticketPresentation';
import type { TicketView } from '../../types';
import { TicketGraphNode } from './TicketGraphNode';

const LONG_TITLE =
  'Make the remote runner survive a dropped SSH channel without wedging the pipeline';

function ticketView(title: string): TicketView {
  return {
    ticket: {
      id: 'a',
      discovery_id: 'dsc-1',
      seq: 1,
      title,
      description: '',
      acceptance: [],
      files: [],
      blocked_by: ['b'],
      test_command: null,
      workflow_id: null,
      agent_kind: null,
      model: null,
      effort: null,
      attachments: [],
      state: 'unstarted',
      drop_reason: null,
      force_start_reason: null,
      force_started_at: null,
      feature_id: null,
      created_at: 0,
      updated_at: 0,
    },
    standing: {
      id: 'a',
      lane: 'blocked',
      startable: false,
      blockers: [{ id: 'b', reason: 'outstanding' }],
    },
    feature: null,
  };
}

function renderNode(title: string) {
  const view = ticketView(title);
  render(
    <TicketGraphNode
      view={view}
      index={indexTickets([view])}
      tone="amber"
      selected={false}
      x={0}
      y={0}
      onSelect={() => {}}
    />,
  );
  return screen.getByTestId('ticket-node');
}

describe('TicketGraphNode', () => {
  it('spends the tile from the title half and never from the status row', () => {
    const card = renderNode(LONG_TITLE);
    const [title, meta] = Array.from(card.children) as HTMLElement[];

    expect(card).toHaveClass('flex', 'flex-col', 'overflow-hidden');
    // Without both, a wrapped title grows the row it is in and the status row
    // is what leaves the tile.
    expect(title).toHaveClass('flex-1', 'min-h-0');
    expect(meta).toHaveClass('shrink-0');
  });

  it("keeps the lane note on the chip's own line", () => {
    const card = renderNode(LONG_TITLE);
    const meta = card.children[1] as HTMLElement;
    const note = meta.querySelector('.truncate') as HTMLElement | null;

    // `flex-wrap` is what sent a long note under the chip, onto a line the
    // fixed height has no room for; `min-w-0` is what lets it ellipsize
    // instead of insisting on its content width.
    expect(meta).not.toHaveClass('flex-wrap');
    expect(note).not.toBeNull();
    expect(note).toHaveClass('min-w-0');
  });

  it('offers the clipped title and note back on hover', () => {
    const card = renderNode(LONG_TITLE);
    expect(card.getAttribute('title')).toContain(LONG_TITLE);
    expect(card.getAttribute('title')).toContain('waiting on');
  });
});
