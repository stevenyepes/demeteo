// The module reads `localStorage` once, at import time, so every case here
// loads its own instance through `freshModule()` after seeding storage — a
// static import would freeze one singleton for the whole file and make the
// poisoned-persistence case unwritable.

import { beforeEach, describe, expect, it, vi } from 'vitest';

const STORAGE_KEY = 'demeteo.terminal.lastSize';

async function freshModule() {
  vi.resetModules();
  return await import('./terminalViewport');
}

beforeEach(() => {
  localStorage.clear();
});

describe('isPlausibleTerminalSize', () => {
  it('accepts sizes a laid-out terminal actually fits to', async () => {
    const { isPlausibleTerminalSize } = await freshModule();
    expect(isPlausibleTerminalSize(80, 24)).toBe(true);
    expect(isPlausibleTerminalSize(120, 40)).toBe(true);
  });

  it('rejects degenerate, non-positive and non-numeric sizes', async () => {
    const { isPlausibleTerminalSize } = await freshModule();
    // 11 × 5 is what a `display:none` subtree fits to at `fontSize: 13`;
    // 2 × 1 is FitAddon's own floor.
    expect(isPlausibleTerminalSize(11, 5)).toBe(false);
    expect(isPlausibleTerminalSize(2, 1)).toBe(false);
    expect(isPlausibleTerminalSize(0, 0)).toBe(false);
    expect(isPlausibleTerminalSize(-5, 10)).toBe(false);
    expect(isPlausibleTerminalSize(Number.NaN, 24)).toBe(false);
  });
});

describe('setLastTerminalSize', () => {
  it('leaves memory and storage untouched for an implausible size', async () => {
    const { getLastTerminalSize, setLastTerminalSize } = await freshModule();

    setLastTerminalSize(120, 40);
    const persisted = localStorage.getItem(STORAGE_KEY);

    setLastTerminalSize(11, 5);

    expect(getLastTerminalSize()).toEqual({ cols: 120, rows: 40 });
    expect(localStorage.getItem(STORAGE_KEY)).toBe(persisted);
  });

  it('updates memory and storage for a plausible size', async () => {
    const { getLastTerminalSize, setLastTerminalSize } = await freshModule();

    setLastTerminalSize(120, 40);

    expect(getLastTerminalSize()).toEqual({ cols: 120, rows: 40 });
    expect(JSON.parse(localStorage.getItem(STORAGE_KEY) ?? 'null')).toEqual({
      cols: 120,
      rows: 40,
    });
  });
});

describe('persisted size', () => {
  it('is discarded when it was poisoned before the floor existed', async () => {
    localStorage.setItem(STORAGE_KEY, JSON.stringify({ cols: 11, rows: 5 }));

    const { getLastTerminalSize } = await freshModule();

    expect(getLastTerminalSize()).toBeNull();
  });

  it('is restored when it is plausible', async () => {
    localStorage.setItem(STORAGE_KEY, JSON.stringify({ cols: 200, rows: 50 }));

    const { getLastTerminalSize } = await freshModule();

    expect(getLastTerminalSize()).toEqual({ cols: 200, rows: 50 });
  });
});

describe('hasLayoutBox', () => {
  it('is false for a missing element', async () => {
    const { hasLayoutBox } = await freshModule();
    expect(hasLayoutBox(null)).toBe(false);
  });

  it('is false for an element with no box', async () => {
    const { hasLayoutBox } = await freshModule();
    // jsdom gives a detached element a null `offsetParent` and a 0 × 0 rect —
    // the same pair a `display:none` subtree reports in a real engine.
    expect(hasLayoutBox(document.createElement('div'))).toBe(false);
  });

  it('is true for an element reporting a non-empty box', async () => {
    const { hasLayoutBox } = await freshModule();
    const el = document.createElement('div');
    vi.spyOn(el, 'getBoundingClientRect').mockReturnValue({
      width: 800,
      height: 600,
    } as DOMRect);

    expect(hasLayoutBox(el)).toBe(true);
  });
});
