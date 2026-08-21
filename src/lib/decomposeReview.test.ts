// `DISCOVERY_UI_SPEC.md` §4.4 starts every change checked and §4.2's footer
// recomputes as they toggle, so the accept set and the label it drives are the
// modal's whole state. §4.9's refused case is pinned here too: a subset of a
// valid proposal is not itself valid, and the refusal has to land on the
// checkboxes that caused it rather than as a sentence pointing at nothing.

import { describe, expect, it } from 'vitest';

import {
  applyLabel,
  groupChanges,
  initialAccepted,
  lockedCount,
  passEyebrow,
  refusedChangeIds,
  renumberNote,
  toggleAccepted,
  validationState,
  violationFor,
} from './decomposeReview';
import type { ChangeKind, DecomposeProposal, ProposedChange } from '../types';

function change(id: string, kind: ChangeKind, extra: Partial<ProposedChange> = {}): ProposedChange {
  return {
    id,
    kind,
    seq: null,
    title: `ticket ${id}`,
    why: null,
    workflow_name: null,
    agent_kind: null,
    blocked_by: [],
    fields: [],
    ...extra,
  };
}

function proposal(extra: Partial<DecomposeProposal> = {}): DecomposeProposal {
  return {
    discovery_id: 'dsc-1',
    first_pass: false,
    tickets: [],
    changes: [],
    locked: [],
    refused: [],
    refusal: null,
    violations: [],
    cost_usd: 0,
    tokens: 0,
    ...extra,
  };
}

const CHANGES = [
  change('revoke', 'added'),
  change('lease', 'added'),
  change('fair-share', 'revised', { seq: 5 }),
  change('bench', 'removed', { seq: 8 }),
];

describe('the accept set', () => {
  it('starts with every change checked', () => {
    expect([...initialAccepted(CHANGES)]).toEqual(['revoke', 'lease', 'fair-share', 'bench']);
  });

  it('toggles one without touching the rest', () => {
    const once = toggleAccepted(initialAccepted(CHANGES), 'lease');

    expect(once.has('lease')).toBe(false);
    expect(once.size).toBe(3);
    expect(toggleAccepted(once, 'lease').has('lease')).toBe(true);
  });
});

describe('the apply label', () => {
  it('counts the diff, not the plan', () => {
    expect(applyLabel(initialAccepted(CHANGES).size, CHANGES.length)).toBe('Apply 4 of 4 changes');
  });

  it('decrements as changes are deselected', () => {
    const accepted = toggleAccepted(toggleAccepted(initialAccepted(CHANGES), 'bench'), 'lease');

    expect(applyLabel(accepted.size, CHANGES.length)).toBe('Apply 2 of 4 changes');
  });
});

describe('the groups', () => {
  it('come in the order §4.3 lists them, with their counts', () => {
    expect(groupChanges(CHANGES).map((group) => [group.label, group.count])).toEqual([
      ['Added', '2 new tickets'],
      ['Revised', '1 unstarted ticket'],
      ['Removed', '1 unstarted ticket'],
    ]);
  });

  it('drop an empty group rather than heading nothing', () => {
    expect(groupChanges([change('one', 'added')]).map((group) => group.label)).toEqual(['Added']);
  });

  it('say how many tickets are locked', () => {
    expect(lockedCount(1)).toBe('1 ticket has a feature');
    expect(lockedCount(2)).toBe('2 tickets have a feature');
  });
});

describe('the eyebrow', () => {
  it('is derived from the pass, never counted', () => {
    expect(passEyebrow(true)).toBe('First pass');
    expect(passEyebrow(false)).toBe('Second pass');
  });
});

describe('the validation bar', () => {
  it('is valid and quiet when nothing was refused', () => {
    const state = validationState(proposal());

    expect(state.chip).toBe('Schema valid');
    expect(state.fatal).toBe(false);
    expect(state.details).toEqual([]);
  });

  it('stays valid over a refusal the interviewer then answered', () => {
    const state = validationState(proposal({ refused: ["ticket 'a' is on a dependency cycle"] }));

    expect(state.chip).toBe('Schema valid');
    expect(state.fatal).toBe(false);
    expect(state.sentence).toContain('Nothing invalid reaches a ticket row.');
    expect(state.details).toEqual(["ticket 'a' is on a dependency cycle"]);
  });

  it('goes ruby and blocks the apply once the last attempt was refused too', () => {
    const state = validationState(proposal({ refusal: 'it kept re-authoring the same graph' }));

    expect(state.chip).toBe('Schema refused');
    expect(state.tone).toBe('ruby');
    expect(state.fatal).toBe(true);
  });

  it('is fatal on an immutable violation even with no refusal', () => {
    const state = validationState(
      proposal({ violations: [{ id: 't1', change: 'revised', reason: 'it has a feature' }] }),
    );

    expect(state.fatal).toBe(true);
  });
});

describe('a refusal of the chosen subset', () => {
  // The backend names the tickets in single quotes, in proposal space — the
  // same space the checkboxes are keyed in.
  const MESSAGE =
    "these changes cannot be applied together: ticket 'lease' is blocked_by 'revoke', which is " +
    'not a ticket in this plan. Either accept the change that would have carried it, or leave ' +
    'the one that names it unchecked.';

  it('marks the checkboxes it names', () => {
    expect([...refusedChangeIds(MESSAGE, CHANGES)].sort()).toEqual(['lease', 'revoke']);
  });

  it('marks nothing when the refusal names no change on screen', () => {
    expect(refusedChangeIds("ticket 'stranger' is unknown", CHANGES).size).toBe(0);
  });
});

describe('the footer', () => {
  it('names the stored range it will not renumber', () => {
    expect(
      renumberNote(
        proposal({
          changes: CHANGES,
          locked: [{ id: 'l1', seq: 1, title: 'registry', lane: 'landed' }],
        }),
      ),
    ).toBe('Ticket ids are stable. Applying this never renumbers DSC-1 through DSC-8.');
  });

  it('names no range when the pass has nothing stored to renumber', () => {
    expect(renumberNote(proposal({ changes: [change('one', 'added')] }))).toBe(
      'Ticket ids are stable. Applying this renumbers nothing.',
    );
  });
});

describe('an immutable violation', () => {
  it('is attached to the locked card it names', () => {
    const locked = { id: 't1', seq: 1, title: 'registry', lane: 'landed' as const };
    const violations = [{ id: 't1', change: 'removed' as const, reason: 'it has a feature' }];

    expect(violationFor(locked, violations)?.reason).toBe('it has a feature');
    expect(violationFor({ ...locked, id: 't2' }, violations)).toBeNull();
  });
});
