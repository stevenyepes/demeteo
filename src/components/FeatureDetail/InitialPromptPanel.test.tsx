// Collapse behaviour for the initial-prompt panel (UI_REDESIGN_PLAN §1 idea B).
//
// The unpersisted-across-remount case is an assertion about a decision, not an
// oversight — see the component's own doc for why this one does not survive.

import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';

import { InitialPromptPanel } from './InitialPromptPanel';

const PROMPT = 'Add a metric strip to the run header.\nKeep the tooltips.';

describe('InitialPromptPanel', () => {
  it('starts collapsed, with the prompt body absent', () => {
    render(<InitialPromptPanel featureDescription={PROMPT} />);

    expect(screen.queryByTestId('disclosure-body')).not.toBeInTheDocument();
    expect(screen.queryByText(PROMPT)).not.toBeInTheDocument();
    const trigger = screen.getByTestId('disclosure-trigger');
    expect(trigger).toHaveAttribute('aria-expanded', 'false');
    expect(trigger).not.toHaveAttribute('aria-controls');
  });

  it('summarises the first line while collapsed', () => {
    render(<InitialPromptPanel featureDescription={PROMPT} />);

    expect(screen.getByTestId('initial-prompt-summary')).toHaveTextContent(
      'Add a metric strip to the run header.',
    );
  });

  it('truncates a long first line instead of widening the header', () => {
    render(<InitialPromptPanel featureDescription={'x'.repeat(400)} />);

    const summary = screen.getByTestId('initial-prompt-summary');
    expect(summary.textContent ?? '').toHaveLength(72);
    expect(summary.textContent ?? '').toMatch(/…$/);
  });

  it('reveals the full prompt on activation and wires aria-controls to it', async () => {
    render(<InitialPromptPanel featureDescription={PROMPT} />);

    const trigger = screen.getByTestId('disclosure-trigger');
    await userEvent.click(trigger);

    const body = screen.getByTestId('disclosure-body');
    expect(body).toHaveTextContent('Add a metric strip to the run header.');
    expect(body).toHaveTextContent('Keep the tooltips.');
    expect(trigger).toHaveAttribute('aria-expanded', 'true');
    expect(trigger.getAttribute('aria-controls')).toBe(body.getAttribute('id'));
  });

  it('collapses again on a second activation', async () => {
    render(<InitialPromptPanel featureDescription={PROMPT} />);

    const trigger = screen.getByTestId('disclosure-trigger');
    await userEvent.click(trigger);
    await userEvent.click(trigger);

    expect(screen.queryByTestId('disclosure-body')).not.toBeInTheDocument();
    expect(trigger).toHaveAttribute('aria-expanded', 'false');
  });

  it('says so in both the summary and the body when no prompt was recorded', async () => {
    render(<InitialPromptPanel featureDescription="" />);

    expect(screen.getByTestId('initial-prompt-summary')).toHaveTextContent('No prompt recorded');

    await userEvent.click(screen.getByTestId('disclosure-trigger'));

    expect(screen.getByTestId('disclosure-body')).toHaveTextContent(
      'No initial prompt was recorded for this run.',
    );
  });

  it('does not carry the open state across a remount', async () => {
    const { unmount } = render(<InitialPromptPanel featureDescription={PROMPT} />);
    await userEvent.click(screen.getByTestId('disclosure-trigger'));
    expect(screen.getByTestId('disclosure-body')).toBeInTheDocument();
    unmount();

    render(<InitialPromptPanel featureDescription={PROMPT} />);

    expect(screen.queryByTestId('disclosure-body')).not.toBeInTheDocument();
  });
});
