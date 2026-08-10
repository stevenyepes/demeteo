// Behaviour tests for the Disclosure primitive (UI_REDESIGN_PLAN §5.1).
//
// Three of these guard decisions the surfaces that adopt it depend on and that
// a refactor could quietly undo: the body must be *absent* when closed (the
// panels it wraps poll and parse markdown, so hiding them with CSS keeps that
// work running), the open state must live with the caller (which is what lets
// one caller store it), and the meta slot must sit outside the trigger button
// so an interactive affordance there is not nested inside a button.

import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { Radio } from 'lucide-react';
import { describe, expect, it, vi } from 'vitest';

import { Disclosure } from './Disclosure';

describe('Disclosure', () => {
  it('renders the title and an optional icon on the trigger', () => {
    render(
      <Disclosure title="Activity" icon={<Radio data-testid="icon" />} open={false} onOpenChange={() => {}}>
        body
      </Disclosure>,
    );

    expect(screen.getByRole('button', { name: /Activity/ })).toBeInTheDocument();
    expect(screen.getByTestId('icon')).toBeInTheDocument();
  });

  it('reports collapsed state through aria-expanded and omits aria-controls', () => {
    render(
      <Disclosure title="Activity" open={false} onOpenChange={() => {}}>
        body
      </Disclosure>,
    );

    const trigger = screen.getByTestId('disclosure-trigger');
    expect(trigger).toHaveAttribute('aria-expanded', 'false');
    expect(trigger).not.toHaveAttribute('aria-controls');
  });

  it('points aria-controls at the rendered body and labels it by the trigger', () => {
    render(
      <Disclosure title="Activity" open onOpenChange={() => {}}>
        body
      </Disclosure>,
    );

    const trigger = screen.getByTestId('disclosure-trigger');
    const body = screen.getByTestId('disclosure-body');

    expect(trigger).toHaveAttribute('aria-expanded', 'true');
    expect(trigger.getAttribute('aria-controls')).toBe(body.getAttribute('id'));
    expect(body.getAttribute('id')).toBeTruthy();
    expect(body).toHaveAttribute('role', 'region');
    expect(body.getAttribute('aria-labelledby')).toBe(trigger.getAttribute('id'));
  });

  it('gives each instance its own body id', () => {
    render(
      <>
        <Disclosure title="One" open onOpenChange={() => {}}>
          one
        </Disclosure>
        <Disclosure title="Two" open onOpenChange={() => {}}>
          two
        </Disclosure>
      </>,
    );

    const [first, second] = screen.getAllByTestId('disclosure-body');
    expect(first.getAttribute('id')).not.toBe(second.getAttribute('id'));
  });

  it('unmounts the body when closed instead of hiding it', () => {
    const mounted = vi.fn();
    function Expensive() {
      mounted();
      return <p>log lines</p>;
    }

    const { rerender } = render(
      <Disclosure title="Activity" open onOpenChange={() => {}}>
        <Expensive />
      </Disclosure>,
    );
    expect(mounted).toHaveBeenCalledTimes(1);
    expect(screen.getByText('log lines')).toBeInTheDocument();

    rerender(
      <Disclosure title="Activity" open={false} onOpenChange={() => {}}>
        <Expensive />
      </Disclosure>,
    );

    expect(screen.queryByText('log lines')).not.toBeInTheDocument();
    expect(screen.queryByTestId('disclosure-body')).not.toBeInTheDocument();
    expect(mounted).toHaveBeenCalledTimes(1);
  });

  it('asks the parent to toggle rather than owning the state', async () => {
    const onOpenChange = vi.fn();
    render(
      <Disclosure title="Activity" open={false} onOpenChange={onOpenChange}>
        body
      </Disclosure>,
    );

    await userEvent.click(screen.getByTestId('disclosure-trigger'));

    expect(onOpenChange).toHaveBeenCalledWith(true);
    expect(screen.queryByTestId('disclosure-body')).not.toBeInTheDocument();
  });

  it('reports the closing transition when open', async () => {
    const onOpenChange = vi.fn();
    render(
      <Disclosure title="Activity" open onOpenChange={onOpenChange}>
        body
      </Disclosure>,
    );

    await userEvent.click(screen.getByTestId('disclosure-trigger'));

    expect(onOpenChange).toHaveBeenCalledWith(false);
  });

  it('toggles from the keyboard', async () => {
    const onOpenChange = vi.fn();
    render(
      <Disclosure title="Activity" open={false} onOpenChange={onOpenChange}>
        body
      </Disclosure>,
    );

    await userEvent.tab();
    expect(screen.getByTestId('disclosure-trigger')).toHaveFocus();

    await userEvent.keyboard('{Enter}');
    await userEvent.keyboard(' ');

    expect(onOpenChange).toHaveBeenCalledTimes(2);
    expect(onOpenChange).toHaveBeenNthCalledWith(1, true);
    expect(onOpenChange).toHaveBeenNthCalledWith(2, true);
  });

  it('keeps the meta slot outside the trigger so its controls stay usable', async () => {
    const onOpenChange = vi.fn();
    const onSync = vi.fn();
    render(
      <Disclosure
        title="Activity"
        open={false}
        onOpenChange={onOpenChange}
        meta={<button type="button" onClick={onSync}>Refresh</button>}
      >
        body
      </Disclosure>,
    );

    const trigger = screen.getByTestId('disclosure-trigger');
    const sync = screen.getByRole('button', { name: 'Refresh' });
    expect(trigger.contains(sync)).toBe(false);

    await userEvent.click(sync);
    expect(onSync).toHaveBeenCalledTimes(1);
    expect(onOpenChange).not.toHaveBeenCalled();
  });

  it('enters the body with the app-wide fade, never an animated height', () => {
    render(
      <Disclosure title="Activity" open onOpenChange={() => {}}>
        body
      </Disclosure>,
    );

    const body = screen.getByTestId('disclosure-body');
    expect(body).toHaveClass('animate-fade-in');
    expect(body.className).not.toMatch(/max-h-\[|transition-\[max-height\]/);
  });

  it('merges caller classNames onto the root and the body', () => {
    render(
      <Disclosure
        title="Activity"
        open
        onOpenChange={() => {}}
        className="mt-8"
        bodyClassName="max-h-64 overflow-y-auto"
      >
        body
      </Disclosure>,
    );

    expect(screen.getByTestId('disclosure')).toHaveClass('mt-8');
    const body = screen.getByTestId('disclosure-body');
    expect(body).toHaveClass('overflow-y-auto');
    expect(body).toHaveClass('animate-fade-in');
  });
});
