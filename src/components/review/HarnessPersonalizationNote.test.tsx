// The three states have to read as three different situations — one of them a
// real downgrade — and the fourth, "nobody has said yet", has to read as
// nothing at all. A catalog that has not loaded rendering the `native` copy
// would be the app stating a fact about the user's setup it does not have.

import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { HarnessPersonalizationNote } from './HarnessPersonalizationNote';
import type { AgentCatalogEntry, PersonalizationSupport } from '../../lib/agentCatalog';

function entry(
  kind: string,
  display_label: string,
  personalization?: PersonalizationSupport,
): AgentCatalogEntry {
  return {
    kind,
    display_label,
    lists_models: false,
    default_model: null,
    install_command: '',
    personalization,
  };
}

const CATALOG: AgentCatalogEntry[] = [
  entry('claude-code', 'Claude Code', 'loaded'),
  entry('pi', 'Pi', 'suppressed'),
  entry('codex', 'Codex', 'native'),
  entry('hermes', 'Hermes'),
];

describe('HarnessPersonalizationNote', () => {
  it('says a loaded setup still applies, as furniture rather than a warning', () => {
    render(<HarnessPersonalizationNote agents={CATALOG} kind="claude-code" stepKeepsPersonalization={false} />);

    const note = screen.getByTestId('harness-personalization');
    expect(note).toHaveAttribute('data-support', 'loaded');
    expect(note).toHaveTextContent('Demeteo starts Claude Code with your own setup loaded');
    expect(note).toHaveTextContent('applies on top of your conventions');
    expect(note).toHaveClass('text-slate-500');
    expect(note).not.toHaveAttribute('role', 'alert');
  });

  it('gives suppression the weight of a downgrade, in amber and not in ruby', () => {
    render(<HarnessPersonalizationNote agents={CATALOG} kind="pi" stepKeepsPersonalization={false} />);

    const note = screen.getByTestId('harness-personalization');
    expect(note).toHaveAttribute('data-support', 'suppressed');
    expect(note).toHaveTextContent('own skills and prompt templates switched off');
    expect(note).toHaveTextContent('Pick another harness');
    expect(note).toHaveClass('text-amber-200/90');
    // Nothing has failed and the run is fully launchable: ruby here would say
    // the opposite, and `role="alert"` would interrupt for a standing fact.
    expect(note.className).not.toContain('ruby');
    expect(note).not.toHaveAttribute('role', 'alert');
  });

  it('claims nothing either way for a harness Demeteo passes no flags to', () => {
    render(<HarnessPersonalizationNote agents={CATALOG} kind="codex" stepKeepsPersonalization={false} />);

    const note = screen.getByTestId('harness-personalization');
    expect(note).toHaveAttribute('data-support', 'native');
    expect(note).toHaveTextContent('Codex starts with whatever it normally loads on this machine');
    expect(note).toHaveTextContent('no personalization flags either way');
  });

  it('reports what the step will really do, not what the catalog declares', () => {
    // The review step keeps the harness's personalization, so the flags that
    // earn Pi its `suppressed` declaration are never emitted for it. Rendering
    // the declared value here would warn about a downgrade this run does not
    // perform.
    const { rerender } = render(
      <HarnessPersonalizationNote agents={CATALOG} kind="pi" stepKeepsPersonalization={true} />,
    );
    const note = screen.getByTestId('harness-personalization');
    expect(note).toHaveAttribute('data-support', 'loaded');
    expect(note).toHaveTextContent('Demeteo starts Pi with your own setup loaded');

    // A harness Demeteo passes no such flag to has nothing for a step to keep.
    rerender(
      <HarnessPersonalizationNote agents={CATALOG} kind="codex" stepKeepsPersonalization={true} />,
    );
    expect(screen.getByTestId('harness-personalization')).toHaveAttribute('data-support', 'native');
  });

  it('renders nothing until something has actually declared an answer', () => {
    const { rerender } = render(<HarnessPersonalizationNote agents={[]} kind="pi" stepKeepsPersonalization={false} />);
    expect(screen.queryByTestId('harness-personalization')).not.toBeInTheDocument();

    // No harness chosen yet.
    rerender(<HarnessPersonalizationNote agents={CATALOG} kind="" stepKeepsPersonalization={false} />);
    expect(screen.queryByTestId('harness-personalization')).not.toBeInTheDocument();

    // A backend that predates the field. Absent is not `native`.
    rerender(<HarnessPersonalizationNote agents={CATALOG} kind="hermes" stepKeepsPersonalization={false} />);
    expect(screen.queryByTestId('harness-personalization')).not.toBeInTheDocument();
  });
});
