import { describe, expect, it } from 'vitest';

import {
  DEFAULT_UI_ZOOM,
  IDLE_WHEEL_GESTURE,
  UI_ZOOM_LEVELS,
  WHEEL_COOLDOWN_MS,
  WHEEL_IDLE_MS,
  WHEEL_STEP_PX,
  advanceWheelGesture,
  decodeUiZoom,
  nearestUiZoom,
  nextUiZoom,
  uiZoomKeyAction,
  uiZoomPercent,
  type UiZoomAction,
  type WheelGesture,
} from './uiZoom';

function key(init: Partial<Parameters<typeof uiZoomKeyAction>[0]>) {
  return { key: '', shiftKey: false, altKey: false, metaKey: false, ctrlKey: false, ...init };
}

/** Feed `[pixels, at]` events through the gesture and collect what it earned. */
function play(events: readonly (readonly [number, number])[]): UiZoomAction[] {
  let gesture: WheelGesture = IDLE_WHEEL_GESTURE;
  const actions: UiZoomAction[] = [];
  for (const [pixels, at] of events) {
    const step = advanceWheelGesture(gesture, pixels, at);
    gesture = step.gesture;
    if (step.action) actions.push(step.action);
  }
  return actions;
}

describe('the ladder', () => {
  it('is ascending and contains the default', () => {
    expect([...UI_ZOOM_LEVELS].sort((a, b) => a - b)).toEqual(UI_ZOOM_LEVELS);
    expect(UI_ZOOM_LEVELS).toContain(DEFAULT_UI_ZOOM);
  });

  it('moves one rung per action', () => {
    expect(nextUiZoom(1, 'in')).toBe(1.1);
    expect(nextUiZoom(1, 'out')).toBe(0.9);
    expect(nextUiZoom(1.1, 'out')).toBe(1);
  });

  it('holds at both ends', () => {
    expect(nextUiZoom(2, 'in')).toBe(2);
    expect(nextUiZoom(0.5, 'out')).toBe(0.5);
  });

  it('resets from anywhere', () => {
    expect(nextUiZoom(1.75, 'reset')).toBe(1);
    expect(nextUiZoom(0.5, 'reset')).toBe(1);
  });

  it('steps to a neighbour from a value that is not a rung', () => {
    expect(nearestUiZoom(1.2)).toBe(1.25);
    expect(nextUiZoom(1.2, 'in')).toBe(1.5);
  });

  it('reads as a whole percentage', () => {
    expect(uiZoomPercent(0.67)).toBe('67%');
    expect(uiZoomPercent(1.1)).toBe('110%');
  });
});

describe('decodeUiZoom', () => {
  it('round-trips every rung', () => {
    for (const level of UI_ZOOM_LEVELS) expect(decodeUiZoom(String(level))).toBe(level);
  });

  it('snaps an off-ladder value inside the range', () => {
    expect(decodeUiZoom('1.2')).toBe(1.25);
  });

  it.each(['', '  ', 'big', 'NaN', '0', '-1', '10', 'Infinity'])('rejects %j', (raw) => {
    expect(decodeUiZoom(raw)).toBeUndefined();
  });
});

describe('uiZoomKeyAction', () => {
  it.each([
    ['Cmd+=', { key: '=', metaKey: true }, 'in'],
    ['Ctrl+=', { key: '=', ctrlKey: true }, 'in'],
    ['Cmd+Shift+=', { key: '+', metaKey: true, shiftKey: true }, 'in'],
    ['Ctrl+numpad +', { key: '+', ctrlKey: true }, 'in'],
    ['Cmd+-', { key: '-', metaKey: true }, 'out'],
    ['Ctrl+Shift+-', { key: '_', ctrlKey: true, shiftKey: true }, 'out'],
    ['Cmd+0', { key: '0', metaKey: true }, 'reset'],
  ] as const)('%s', (_label, init, action) => {
    expect(uiZoomKeyAction(key(init))).toBe(action);
  });

  it.each([
    ['a bare character', { key: '-' }],
    ['AltGr, which reports as Ctrl+Alt', { key: '-', ctrlKey: true, altKey: true }],
    ['a project-switch digit', { key: '1', metaKey: true }],
    ['another chord', { key: 'k', metaKey: true }],
  ] as const)('ignores %s', (_label, init) => {
    expect(uiZoomKeyAction(key(init))).toBeNull();
  });
});

describe('advanceWheelGesture', () => {
  it('takes one rung from one mouse notch, in either direction', () => {
    expect(play([[-100, 0]])).toEqual(['in']);
    expect(play([[100, 0]])).toEqual(['out']);
  });

  it('ignores travel short of a deliberate movement', () => {
    expect(play([[-WHEEL_STEP_PX + 1, 0]])).toEqual([]);
  });

  it('banks small pinch deltas until they amount to a rung', () => {
    const events = Array.from({ length: 10 }, (_, i) => [-5, i * 16] as const);
    expect(play(events)).toEqual(['in']);
  });

  it('cannot take a second rung inside the cooldown, however large the deltas', () => {
    const burst = Array.from({ length: 6 }, (_, i) => [-400, i * 16] as const);
    expect(burst[burst.length - 1][1]).toBeLessThan(WHEEL_COOLDOWN_MS);
    expect(play(burst)).toEqual(['in']);
  });

  it('takes the next rung once the cooldown has passed', () => {
    expect(play([[-100, 0], [-100, WHEEL_COOLDOWN_MS]])).toEqual(['in', 'in']);
  });

  it('takes nothing extra on a stray pixel once the cooldown ends', () => {
    const burst = Array.from({ length: 6 }, (_, i) => [-400, i * 16] as const);
    expect(play([...burst, [-1, WHEEL_COOLDOWN_MS]])).toEqual(['in']);
  });

  it('drops the bank when the direction reverses', () => {
    expect(play([[-40, 0], [40, 16], [-40, 32]])).toEqual([]);
  });

  it('drops the bank across a pause', () => {
    expect(play([[-40, 0], [-40, WHEEL_IDLE_MS]])).toEqual([]);
  });
});
