// The claim this file defends: `TicketOverlayPanel` reuses `OverlayPortal` and
// `Modal.tsx`'s Escape idiom rather than a hand-rolled third copy (AGENTS.md
// §6 Constraints), but — unlike `Modal.tsx` — deliberately has no click-to-dismiss
// backdrop, because the wrapper is `pointer-events-none` so it never obscures
// or blocks `InterviewColumn`/`TicketColumn` underneath (DISCOVERY_UI_SPEC.md
// §3.2.1). A portal-target assertion, the pointer-events split, and Escape
// coverage stand in for re-testing `OverlayPortal` itself.

import { fireEvent, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';

import { TicketOverlayPanel } from './TicketOverlayPanel';

describe('TicketOverlayPanel', () => {
  it('portals children onto document.body, outside the local render container', () => {
    const { container } = render(
      <TicketOverlayPanel widthPx={760} onClose={() => {}} label="Ticket editor">
        <div data-testid="panel-content">content</div>
      </TicketOverlayPanel>,
    );

    const content = screen.getByTestId('panel-content');
    expect(document.body.contains(content)).toBe(true);
    expect(container.contains(content)).toBe(false);
  });

  it('calls onClose on Escape', () => {
    const onClose = vi.fn();
    render(
      <TicketOverlayPanel widthPx={760} onClose={onClose} label="Ticket editor">
        <div data-testid="panel-content">content</div>
      </TicketOverlayPanel>,
    );

    fireEvent.keyDown(window, { key: 'Escape' });

    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('does not call onClose on a backdrop click — there is no click-to-dismiss', () => {
    const onClose = vi.fn();
    render(
      <TicketOverlayPanel widthPx={760} onClose={onClose} label="Ticket editor">
        <div data-testid="panel-content">content</div>
      </TicketOverlayPanel>,
    );

    fireEvent.click(screen.getByLabelText('Ticket editor'));

    expect(onClose).not.toHaveBeenCalled();
  });

  it('does not call onClose on a click inside the panel content', async () => {
    const onClose = vi.fn();
    render(
      <TicketOverlayPanel widthPx={760} onClose={onClose} label="Ticket editor">
        <div data-testid="panel-content">content</div>
      </TicketOverlayPanel>,
    );

    await userEvent.click(screen.getByTestId('panel-content'));

    expect(onClose).not.toHaveBeenCalled();
  });

  it('keeps the full-viewport wrapper non-interactive and re-enables pointer events only on the docked panel', () => {
    render(
      <TicketOverlayPanel widthPx={760} onClose={() => {}} label="Ticket editor">
        <div data-testid="panel-content">content</div>
      </TicketOverlayPanel>,
    );

    const wrapper = screen.getByLabelText('Ticket editor');
    expect(wrapper).toHaveClass('pointer-events-none');

    const dockedPanel = screen.getByTestId('panel-content').parentElement;
    expect(dockedPanel).toHaveClass('pointer-events-auto');
  });
});
