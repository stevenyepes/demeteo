// The claim this file defends: `TicketOverlayPanel` renders where its caller
// puts it — inside the workspace row — rather than portalling to the window,
// so the inspector sits below the workspace header in every layout mode
// (DISCOVERY_UI_SPEC.md §3.2.1), and it dismisses on Escape only.

import { fireEvent, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';

import { TicketOverlayPanel } from './TicketOverlayPanel';

describe('TicketOverlayPanel', () => {
  it('renders in place, inside its caller, not portalled onto document.body', () => {
    const { container } = render(
      <TicketOverlayPanel onClose={() => {}} label="Ticket inspector">
        <div data-testid="panel-content">content</div>
      </TicketOverlayPanel>,
    );

    expect(container.contains(screen.getByTestId('panel-content'))).toBe(true);
  });

  it('calls onClose on Escape', () => {
    const onClose = vi.fn();
    render(
      <TicketOverlayPanel onClose={onClose} label="Ticket inspector">
        <div data-testid="panel-content">content</div>
      </TicketOverlayPanel>,
    );

    fireEvent.keyDown(window, { key: 'Escape' });

    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('does not call onClose on a click inside the panel content', async () => {
    const onClose = vi.fn();
    render(
      <TicketOverlayPanel onClose={onClose} label="Ticket inspector">
        <div data-testid="panel-content">content</div>
      </TicketOverlayPanel>,
    );

    await userEvent.click(screen.getByTestId('panel-content'));

    expect(onClose).not.toHaveBeenCalled();
  });
});
