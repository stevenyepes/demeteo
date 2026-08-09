// Behaviour tests for the run chrome's gate strip (UI_REDESIGN_PLAN §3.2).
//
// The parts worth pinning are the ones a later edit could quietly undo: the
// strip disappears completely when nothing is waiting, the CTA carries the
// *earliest* open gate's execution id, its colour comes from the shared tone
// table rather than a re-spelled chip (F27), and only the dot animates.

import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';

import { TONE_CHIP } from '../../lib/runStatus';
import type { GateStripRow } from '../../lib/gateStrip';
import { GateStrip } from './GateStrip';

function step(id: string, status: string, step_index: number, step_id = `s-${id}`): GateStripRow {
  return { id, step_id, step_index, status };
}

function strip(): HTMLElement {
  return screen.getByTestId('gate-strip');
}

describe('GateStrip', () => {
  it('renders nothing while no step is waiting on a decision', () => {
    const { container } = render(
      <GateStrip steps={[step('a', 'running', 0), step('b', 'completed', 1)]} onDecideGate={() => {}} />,
    );

    expect(container).toBeEmptyDOMElement();
  });

  it('renders nothing for a run with no steps at all', () => {
    const { container } = render(<GateStrip steps={[]} onDecideGate={() => {}} />);

    expect(container).toBeEmptyDOMElement();
  });

  it('names the step the CTA acts on, humanized from its step id', () => {
    render(
      <GateStrip
        steps={[step('a', 'awaiting_gate', 2, 's-review-plan')]}
        onDecideGate={() => {}}
      />,
    );

    expect(strip()).toHaveTextContent('Review Plan');
  });

  it('counts one waiting gate in the singular', () => {
    render(<GateStrip steps={[step('a', 'awaiting_gate', 0)]} onDecideGate={() => {}} />);

    expect(strip()).toHaveTextContent('1 gate needs you');
  });

  it('counts every open gate, not only the one it acts on', () => {
    render(
      <GateStrip
        steps={[step('a', 'awaiting_gate', 0), step('b', 'awaiting_gate', 4)]}
        onDecideGate={() => {}}
      />,
    );

    expect(strip()).toHaveTextContent('2 gates need you');
  });

  it('decides the earliest open gate, whatever order the rows arrived in', async () => {
    const onDecideGate = vi.fn();
    render(
      <GateStrip
        steps={[
          step('late', 'awaiting_gate', 6, 's-ship'),
          step('early', 'awaiting_gate', 1, 's-review-plan'),
        ]}
        onDecideGate={onDecideGate}
      />,
    );

    expect(strip()).toHaveTextContent('Review Plan');
    await userEvent.click(screen.getByRole('button', { name: /Decide Gate/ }));

    expect(onDecideGate).toHaveBeenCalledWith('early');
  });

  it('takes its amber from the shared tone table', () => {
    render(<GateStrip steps={[step('a', 'awaiting_gate', 0)]} onDecideGate={() => {}} />);

    for (const toneClass of TONE_CHIP.amber.split(' ')) {
      expect(strip()).toHaveClass(toneClass);
    }
  });

  it('announces itself when it appears', () => {
    render(<GateStrip steps={[step('a', 'awaiting_gate', 0)]} onDecideGate={() => {}} />);

    expect(screen.getByRole('status')).toBe(strip());
  });

  // The block this replaces pulsed a whole card of text; App.css records the
  // WKWebView GPU incident that makes an animated container expensive.
  it('animates a dot and nothing else', () => {
    render(<GateStrip steps={[step('a', 'awaiting_gate', 0)]} onDecideGate={() => {}} />);

    // `querySelectorAll` does not include the element it is called on, and the
    // container is exactly where the wrong animation would be put.
    const animated = strip().querySelectorAll('[class*="animate-"]');
    expect(strip().className).not.toMatch(/animate-/);
    expect(animated).toHaveLength(1);
    expect(animated[0]).toHaveClass('animate-pulse-glow-amber', 'h-2', 'w-2');
    expect(animated[0]).toBeEmptyDOMElement();
  });

  it('merges a caller-supplied className', () => {
    render(
      <GateStrip steps={[step('a', 'awaiting_gate', 0)]} onDecideGate={() => {}} className="mb-4" />,
    );

    expect(strip()).toHaveClass('mb-4');
  });
});
