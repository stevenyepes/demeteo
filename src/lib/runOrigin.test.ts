import { describe, expect, it } from 'vitest';

import { runOriginArgs } from './runOrigin';

describe('runOriginArgs', () => {
  it('states nothing for a run that starts where every run used to', () => {
    const args = runOriginArgs({ base: null, diffBase: null });
    // `toEqual` passes on `{ origin: undefined }`; the launch payload's
    // identity is about the keys, so assert those.
    expect(Object.keys(args)).toEqual([]);
  });

  it('names a chosen base as the branch arm', () => {
    expect(runOriginArgs({ base: 'release/2.1', diffBase: null })).toEqual({
      origin: { kind: 'branch', base: 'release/2.1' },
    });
  });

  it('declares a diff base only when it differs from the base', () => {
    expect(runOriginArgs({ base: 'release/2.1', diffBase: 'main' })).toEqual({
      origin: { kind: 'branch', base: 'release/2.1' },
      diffBaseBranch: 'main',
    });

    const same = runOriginArgs({ base: 'release/2.1', diffBase: 'release/2.1' });
    expect(same).toEqual({ origin: { kind: 'branch', base: 'release/2.1' } });
    expect(Object.keys(same)).toEqual(['origin']);
  });

  it('diffs the default-branch start against a branch that is not it', () => {
    expect(runOriginArgs({ base: null, diffBase: 'develop' })).toEqual({
      diffBaseBranch: 'develop',
    });
  });

  it('reads a blank control as unstated', () => {
    expect(Object.keys(runOriginArgs({ base: '  ', diffBase: '' }))).toEqual([]);
  });
});
