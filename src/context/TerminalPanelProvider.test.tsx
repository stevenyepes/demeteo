// Unit tests for the terminal-panel reducer's activity wiring (T1.5/T1.10).
//
// The reducer is pure, so `SET_ACTIVITY` is exercised directly — no provider
// mount needed. We pin the two promises the plan makes: the reducer is
// idempotent (an unchanged report is a no-op, same state reference), and the
// backend event `state` map turns `"exit"` into `null` while passing the live
// states straight through. The nav attention count is the same predicate
// ProjectRail derives, checked here against injected descriptors.

import { describe, expect, it } from 'vitest';

import {
  activityFromEventState,
  terminalPanelReducer,
} from './TerminalPanelProvider';
import type {
  TerminalActivity,
  TerminalPanelState,
  TerminalTabDescriptor,
} from '../types';

function tab(
  overrides: Partial<TerminalTabDescriptor> & { sessionId: string },
): TerminalTabDescriptor {
  return {
    tabId: `tab-${overrides.sessionId}`,
    machineId: 'local',
    machineLabel: 'local',
    title: 'shell',
    phase: 'running',
    createdAt: 0,
    ...overrides,
  };
}

function stateWith(tabs: TerminalTabDescriptor[]): TerminalPanelState {
  return { tabs, activeTabId: tabs[0]?.tabId ?? null };
}

// The count ProjectRail renders on the Terminals nav item.
function attentionCount(state: TerminalPanelState): number {
  return state.tabs.filter((t) => t.activity === 'awaiting_approval').length;
}

describe('terminalPanelReducer — SET_ACTIVITY', () => {
  it('applies a new activity to the matching session', () => {
    const before = stateWith([tab({ sessionId: 'sess_1' })]);

    const after = terminalPanelReducer(before, {
      type: 'SET_ACTIVITY',
      sessionId: 'sess_1',
      activity: 'working',
    });

    expect(after.tabs[0].activity).toBe('working');
  });

  it('is idempotent: reporting the same activity twice is a no-op (same state reference)', () => {
    const working = terminalPanelReducer(
      stateWith([tab({ sessionId: 'sess_1' })]),
      { type: 'SET_ACTIVITY', sessionId: 'sess_1', activity: 'working' },
    );

    const again = terminalPanelReducer(working, {
      type: 'SET_ACTIVITY',
      sessionId: 'sess_1',
      activity: 'working',
    });

    // Unchanged report must not mint a new state object (no re-render).
    expect(again).toBe(working);
  });

  it('ignores an activity report for an unknown session (same state reference)', () => {
    const before = stateWith([tab({ sessionId: 'sess_1' })]);

    const after = terminalPanelReducer(before, {
      type: 'SET_ACTIVITY',
      sessionId: 'sess_missing',
      activity: 'awaiting_approval',
    });

    expect(after).toBe(before);
  });

  it('clears the activity when set back to null (exit)', () => {
    const working = terminalPanelReducer(
      stateWith([tab({ sessionId: 'sess_1' })]),
      { type: 'SET_ACTIVITY', sessionId: 'sess_1', activity: 'working' },
    );

    const cleared = terminalPanelReducer(working, {
      type: 'SET_ACTIVITY',
      sessionId: 'sess_1',
      activity: null,
    });

    expect(cleared).not.toBe(working);
    expect(cleared.tabs[0].activity).toBeNull();
  });
});

describe('activityFromEventState — backend event mapping', () => {
  it('maps "exit" to null', () => {
    expect(activityFromEventState('exit')).toBeNull();
  });

  it('passes the live states straight through', () => {
    const cases: TerminalActivity[] = [
      'working',
      'awaiting_input',
      'awaiting_approval',
    ];
    for (const state of cases) {
      expect(activityFromEventState(state as string)).toBe(state);
    }
  });
});

describe('nav attention count derivation', () => {
  it('is 0 when no session is awaiting approval', () => {
    const state = stateWith([
      tab({ sessionId: 'sess_1', activity: 'working' }),
      tab({ sessionId: 'sess_2', activity: 'awaiting_input' }),
    ]);

    expect(attentionCount(state)).toBe(0);
  });

  it('increments when an awaiting_approval descriptor is injected', () => {
    const before = stateWith([tab({ sessionId: 'sess_1', activity: 'working' })]);
    expect(attentionCount(before)).toBe(0);

    const after = terminalPanelReducer(before, {
      type: 'SET_ACTIVITY',
      sessionId: 'sess_1',
      activity: 'awaiting_approval',
    });

    expect(attentionCount(after)).toBe(1);
  });
});
