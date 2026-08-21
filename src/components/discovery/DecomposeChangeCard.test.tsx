// `DISCOVERY_UI_SPEC.md` §4.6 draws a revision as two stacked lines per field:
// the old value struck through in ruby over the new one in emerald. Nothing in
// tsc, biome or the class gate can tell those two apart — both are strings in
// a `className` — so the pairing is pinned here.
//
// §4.4's "the whole card is the click target" is pinned too: the 18 px box is
// not the affordance, the paragraph is.

import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { DecomposeChangeCard } from './DecomposeChangeCard';
import type { ChangeKind, ProposedChange } from '../../types';

afterEach(cleanup);

function change(kind: ChangeKind, extra: Partial<ProposedChange> = {}): ProposedChange {
  return {
    id: 'fair-share',
    kind,
    seq: 5,
    title: 'Fair-share scheduling across clients',
    why: 'Leases decide what fair-share is scheduling over, so it has to land first.',
    workflow_name: null,
    agent_kind: null,
    blocked_by: [],
    fields: [],
    ...extra,
  };
}

function renderCard(subject: ProposedChange, overrides: { accepted?: boolean; refused?: boolean } = {}) {
  const onToggle = vi.fn();
  render(
    <DecomposeChangeCard
      change={subject}
      accepted={overrides.accepted ?? true}
      refused={overrides.refused ?? false}
      disabled={false}
      onToggle={onToggle}
      seqOf={(id) => (id === 'lease' ? 10 : null)}
    />,
  );
  return { onToggle };
}

describe('a revised card', () => {
  const REVISED = change('revised', {
    fields: [
      { field: 'blocked by', was: 'DSC-3', now: 'DSC-3, DSC-10' },
      { field: 'test command', was: 'cargo test -p demeteo-runner', now: 'npm run checks:code' },
    ],
  });

  it('strikes the old value through in ruby and leaves the new one emerald', () => {
    renderCard(REVISED);

    const was = screen.getByText('cargo test -p demeteo-runner');
    const now = screen.getByText('npm run checks:code');

    expect(was.className).toContain('line-through');
    expect(was.className).toContain('text-ruby-400');
    expect(now.className).toContain('text-emerald-400');
    expect(now.className).not.toContain('line-through');
  });

  it('names each field and keeps its two lines together', () => {
    renderCard(REVISED);

    expect(screen.getByText('blocked by')).toBeTruthy();
    expect(screen.getByText('DSC-3').className).toContain('line-through');
    expect(screen.getByText('DSC-3, DSC-10').className).toContain('text-emerald-400');
  });

  it('renders an emptied field as a dash rather than as nothing', () => {
    renderCard(change('revised', { fields: [{ field: 'model', was: 'opus', now: '' }] }));

    expect(screen.getByText('—').className).toContain('text-emerald-400');
  });

  it('carries no chip row — §4.6 gives a revision its diff and its why, and nothing else', () => {
    renderCard(change('revised', { workflow_name: 'Standard Feature', agent_kind: 'claude-code' }));

    expect(screen.queryByText('Standard Feature')).toBeNull();
  });
});

describe('an added card', () => {
  it('names its prerequisites in amber and their absence in slate', () => {
    renderCard(change('added', { seq: null, blocked_by: ['lease'] }));

    expect(screen.getByText('blocked by DSC-10')).toBeTruthy();
    expect(screen.getByTestId('chip').getAttribute('data-tone')).toBe('amber');
  });

  it('says so when nothing gates it', () => {
    renderCard(change('added', { seq: null }));

    expect(screen.getByText('no prerequisites')).toBeTruthy();
  });

  it('shows `new` where a stored ticket shows its number', () => {
    renderCard(change('added', { seq: null }));

    expect(screen.getByText('new')).toBeTruthy();
  });
});

describe('the checkbox', () => {
  it('takes the click anywhere on the card', () => {
    const { onToggle } = renderCard(change('added', { seq: null }));

    fireEvent.click(screen.getByText('Fair-share scheduling across clients'));

    expect(onToggle).toHaveBeenCalledTimes(1);
  });

  it('dims a deselected change without hiding it', () => {
    renderCard(change('added', { seq: null }), { accepted: false });

    const card = screen.getByTestId('decompose-change');
    expect(card.getAttribute('aria-checked')).toBe('false');
    expect(card.className).toContain('opacity-45');
  });

  it('goes ruby when the backend refused the combination it is part of', () => {
    renderCard(change('added', { seq: null }), { refused: true });

    expect(screen.getByTestId('decompose-change').className).toContain('border-ruby-500/40');
  });
});
