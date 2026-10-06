import { afterEach, describe, expect, it, vi } from 'vitest';

import { installMotionPause } from './motionPause';

const root = () => document.documentElement;

function setFocus(focused: boolean) {
  vi.spyOn(document, 'hasFocus').mockReturnValue(focused);
}

function setHidden(hidden: boolean) {
  Object.defineProperty(document, 'hidden', { configurable: true, get: () => hidden });
}

describe('installMotionPause', () => {
  let uninstall: (() => void) | null = null;

  afterEach(() => {
    uninstall?.();
    uninstall = null;
    vi.restoreAllMocks();
    setHidden(false);
  });

  it('runs animations while the window is focused and visible', () => {
    setFocus(true);
    uninstall = installMotionPause();
    expect(root().dataset.motion).toBeUndefined();
  });

  it('pauses on blur and resumes on focus', () => {
    setFocus(true);
    uninstall = installMotionPause();

    setFocus(false);
    window.dispatchEvent(new Event('blur'));
    expect(root().dataset.motion).toBe('paused');

    setFocus(true);
    window.dispatchEvent(new Event('focus'));
    expect(root().dataset.motion).toBeUndefined();
  });

  it('pauses a hidden document even when it reports focus', () => {
    setFocus(true);
    uninstall = installMotionPause();

    setHidden(true);
    document.dispatchEvent(new Event('visibilitychange'));
    expect(root().dataset.motion).toBe('paused');
  });

  it('starts paused when installed into an unfocused window', () => {
    setFocus(false);
    uninstall = installMotionPause();
    expect(root().dataset.motion).toBe('paused');
  });

  it('stops listening and clears the flag on uninstall', () => {
    setFocus(false);
    uninstall = installMotionPause();
    uninstall();
    uninstall = null;
    expect(root().dataset.motion).toBeUndefined();

    window.dispatchEvent(new Event('blur'));
    expect(root().dataset.motion).toBeUndefined();
  });
});
