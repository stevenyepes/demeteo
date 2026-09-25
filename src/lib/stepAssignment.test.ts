// Unit tests for `src/lib/stepAssignment.ts`.

import { describe, expect, it } from 'vitest';

import { RUN_STATUSES } from './runStatus';
import { isAssignable, showsPlanned, takesAssignment } from './stepAssignment';

describe('isAssignable', () => {
  it.each(['running', 'verifying'])("refuses '%s' — the attempt's agent is already up", (status) => {
    expect(isAssignable(status)).toBe(false);
  });

  it('assigns every other status the vocabulary knows', () => {
    const refused = RUN_STATUSES.filter((s) => !isAssignable(s));

    expect(refused).toEqual(['running', 'verifying']);
  });

  it('assigns a status this build has never heard of', () => {
    expect(isAssignable('quiescing')).toBe(true);
  });
});

describe('takesAssignment', () => {
  it.each(['agent', 'sequence', 'parallel', 'sync', 'finalize'])('offers it on %s', (kind) => {
    expect(takesAssignment(kind)).toBe(true);
  });

  it.each(['gate', 'command', 'a-kind-from-a-later-release'])('withholds it on %s', (kind) => {
    expect(takesAssignment(kind)).toBe(false);
  });
});

describe('showsPlanned', () => {
  it.each(['completed', 'skipped'])('does not call a pin on a %s node planned', (status) => {
    expect(showsPlanned(status)).toBe(false);
  });

  it.each(['pending', 'failed', 'interrupted', 'awaiting_gate', undefined])(
    'calls a pin on a %s node planned',
    (status) => {
      expect(showsPlanned(status)).toBe(true);
    },
  );
});
