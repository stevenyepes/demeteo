// Unit tests for `src/lib/errors.ts`.
//
// The bug this guards: catch blocks across the frontend were coercing
// an `AppError`-shaped value via `String(err)` and rendering the
// literal text `[object Object]` to the user. `formatError` exists to
// guarantee that never happens again.

import { describe, it, expect } from 'vitest';
import { formatError } from './errors';

describe('formatError', () => {
  it('surfaces an AppError-shaped value’s `message` field, not the object literal', () => {
    expect(formatError({ kind: 'ssh', message: 'host unreachable' })).toBe('host unreachable');
  });

  it('surfaces a native Error’s `.message`', () => {
    expect(formatError(new Error('disk full'))).toBe('disk full');
  });

  it('round-trips a plain string error verbatim', () => {
    expect(formatError('legacy string error')).toBe('legacy string error');
  });

  // An `AppError`-shaped object without a usable `message` (e.g. a partially
  // constructed value, or a future backend variant) must still produce a
  // non-empty fallback. The pre-existing `[object Object]` failure mode is
  // exactly what this regression net is here to catch.
  it('never collapses a `kind`-only object to "[object Object]"', () => {
    const fallback = formatError({ kind: 'ssh' });
    expect(fallback).not.toBe('[object Object]');
    expect(fallback.length).toBeGreaterThan(0);
    expect(fallback).toBe('ssh');
  });

  it('normalises `null` and `undefined` to "Unknown error"', () => {
    expect(formatError(null)).toBe('Unknown error');
    expect(formatError(undefined)).toBe('Unknown error');
  });
});
