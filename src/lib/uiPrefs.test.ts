/**
 * The interesting part of these wrappers is not the IPC — it is what they
 * refuse to do: write before the read that arms them, write once per event in a
 * burst, and let either a corrupt row or a dead transport reach the caller. Each
 * has its own case below, because each is a rule a plausible-looking
 * implementation drops silently.
 *
 * Every stored value in `CASES` differs from that preference's default, so a
 * decode that always answered the default would fail the round trip rather than
 * pass it.
 */

import { invoke } from '@tauri-apps/api/core';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { DEFAULT_DENSITY } from './density';
import { DEFAULT_PIPELINE_FILTER } from './pipelineFilter';
import {
  definePref,
  densityPref,
  inspectorWidthPref,
  pipelineSegmentPref,
  pipelineSortPref,
  runViewModePref,
  UI_PREF_WRITE_DEBOUNCE_MS,
  type UiPref,
} from './uiPrefs';

interface PrefCase {
  name: string;
  pref: UiPref<unknown>;
  stored: string;
  value: unknown;
  fallback: unknown;
}

const CASES: readonly PrefCase[] = [
  {
    name: 'density',
    pref: densityPref,
    stored: 'compact',
    value: 'compact',
    fallback: DEFAULT_DENSITY,
  },
  {
    name: 'inspector width',
    pref: inspectorWidthPref,
    stored: '412',
    value: 412,
    fallback: null,
  },
  {
    name: 'run view mode',
    pref: runViewModePref,
    stored: 'timeline',
    value: 'timeline',
    fallback: 'graph',
  },
  {
    name: 'pipeline segment',
    pref: pipelineSegmentPref,
    stored: 'needs-you',
    value: 'needs-you',
    fallback: DEFAULT_PIPELINE_FILTER.segment,
  },
  {
    name: 'pipeline sort',
    pref: pipelineSortPref,
    stored: 'oldest',
    value: 'oldest',
    fallback: DEFAULT_PIPELINE_FILTER.sort,
  },
];

function storeReturning(raw: unknown): void {
  vi.mocked(invoke).mockImplementation(((cmd: string) =>
    cmd === 'get_app_session'
      ? Promise.resolve(raw)
      : Promise.resolve(undefined)) as unknown as typeof invoke);
}

function writes(): { key: string; value: string }[] {
  return vi
    .mocked(invoke)
    .mock.calls.filter(([cmd]) => cmd === 'set_app_session')
    .map(([, args]) => args as unknown as { key: string; value: string });
}

const settle = () => vi.advanceTimersByTimeAsync(UI_PREF_WRITE_DEBOUNCE_MS);

beforeEach(() => {
  vi.mocked(invoke).mockReset();
  vi.useFakeTimers();
});

afterEach(() => {
  vi.useRealTimers();
});

describe('the persisted preferences', () => {
  it('give every preference its own namespaced row', () => {
    const keys = CASES.map((c) => c.pref.key);
    expect(new Set(keys).size).toBe(keys.length);
    for (const key of keys) expect(key.startsWith('ui.')).toBe(true);
  });

  /* Spelled out rather than derived from the objects: a row name is a
     compatibility surface, so renaming one discards the choice every existing
     user already made, silently and for all of them at once. `ui.density` is
     the one shared row — the run timeline and the project view's pipeline list
     both resolve their toggle through `densityPref`, so a second key here is
     also how the two surfaces would start disagreeing about one choice. */
  it('keeps the row names it already wrote into installed stores', () => {
    expect(densityPref.key).toBe('ui.density');
    expect(inspectorWidthPref.key).toBe('ui.inspector_width');
    expect(runViewModePref.key).toBe('ui.run_view_mode');
    expect(pipelineSegmentPref.key).toBe('ui.pipeline_segment');
    expect(pipelineSortPref.key).toBe('ui.pipeline_sort');
  });

  for (const { name, pref, stored, value } of CASES) {
    it(`round-trips ${name} through its own key`, async () => {
      storeReturning(stored);

      expect(await pref.read()).toEqual(value);
      expect(invoke).toHaveBeenCalledWith('get_app_session', { key: pref.key });

      pref.write(value);
      await settle();
      expect(writes()).toEqual([{ key: pref.key, value: stored }]);
    });
  }

  for (const { name, pref, fallback } of CASES) {
    it(`answers the default for an unusable ${name} row`, async () => {
      storeReturning(null);
      expect(await pref.read()).toEqual(fallback);

      storeReturning('not-a-stored-value');
      expect(await pref.read()).toEqual(fallback);

      // A value the union does not hold but `Object.prototype` does: the guards
      // must answer on the table's own keys, not on the chain behind them.
      storeReturning('toString');
      expect(await pref.read()).toEqual(fallback);

      // Nothing forces the row to be a string — an older build or a partial
      // write can put anything on the wire.
      storeReturning(42);
      expect(await pref.read()).toEqual(fallback);
    });
  }

  it('keeps "never dragged" distinct from a width, both ways', async () => {
    storeReturning(null);
    await inspectorWidthPref.read();

    inspectorWidthPref.write(null);
    await settle();
    expect(writes()).toEqual([{ key: inspectorWidthPref.key, value: '' }]);

    storeReturning('');
    expect(await inspectorWidthPref.read()).toBeNull();
    storeReturning('0');
    expect(await inspectorWidthPref.read()).toBeNull();
  });
});

describe('a store that cannot be reached', () => {
  it('reads as the default instead of rejecting', async () => {
    vi.mocked(invoke).mockRejectedValue(new Error('ipc down'));

    expect(await densityPref.read()).toBe(DEFAULT_DENSITY);
  });

  it('still takes the write it drops, so a failed read does not disarm', async () => {
    vi.mocked(invoke).mockRejectedValue(new Error('ipc down'));
    await densityPref.read();

    expect(() => densityPref.write('compact')).not.toThrow();
    await settle();
    expect(writes()).toEqual([{ key: densityPref.key, value: 'compact' }]);
  });
});

describe('the write debounce', () => {
  function numberPref(key: string): UiPref<number> {
    return definePref<number>({
      key,
      fallback: 0,
      decode: (raw) => (raw === '' ? undefined : Number(raw)),
      encode: String,
    });
  }

  it('coalesces a burst into one write of the last value', async () => {
    storeReturning(null);
    const pref = numberPref('ui.test_width');
    await pref.read();

    for (const width of [320, 361, 402]) pref.write(width);
    await vi.advanceTimersByTimeAsync(UI_PREF_WRITE_DEBOUNCE_MS - 1);
    expect(writes()).toEqual([]);

    // A second value inside the window restarts it — a fixed window would have
    // stored 402 here and lost 480.
    pref.write(480);
    await vi.advanceTimersByTimeAsync(UI_PREF_WRITE_DEBOUNCE_MS - 1);
    expect(writes()).toEqual([]);

    await vi.advanceTimersByTimeAsync(1);
    expect(writes()).toEqual([{ key: 'ui.test_width', value: '480' }]);
  });

  it('reads back a choice the debounce has not stored yet', async () => {
    // The store is one mount behind for 400 ms, and `ui.density` is read by two
    // surfaces: choose Compact in the project view, open a feature inside the
    // window, and a read that went past the pending value would hand the run
    // timeline the choice the user had just replaced — then keep it for the
    // life of that mount, with the toggle agreeing and the store disagreeing.
    storeReturning('512');
    const pref = numberPref('ui.test_pending');
    expect(await pref.read()).toBe(512);

    pref.write(320);
    expect(await pref.read()).toBe(320);
    expect(writes()).toEqual([]);

    await settle();
    expect(writes()).toEqual([{ key: 'ui.test_pending', value: '320' }]);
  });

  it('drops a write issued before the read that arms it', async () => {
    storeReturning('512');
    const pref = numberPref('ui.test_armed');

    pref.write(320);
    await settle();
    expect(writes()).toEqual([]);

    expect(await pref.read()).toBe(512);
    pref.write(320);
    await settle();
    expect(writes()).toEqual([{ key: 'ui.test_armed', value: '320' }]);
  });
});
