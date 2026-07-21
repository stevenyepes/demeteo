import { describe, expect, it } from 'vitest';

import { compileRulePacks, DEFAULT_COMPILED_PACKS } from './rulePacks';
import {
  matchesApproval,
  readBottomRows,
  recognizerTick,
  type TerminalLike,
} from './recognizer';

/** Build a minimal `TerminalLike` whose rendered viewport is `visibleRows`,
 *  optionally sitting above `scrollbackRows` of history the reader must NOT
 *  see. */
function fakeTerm(visibleRows: string[], scrollbackRows: string[] = []): TerminalLike {
  const lines = [...scrollbackRows, ...visibleRows];
  return {
    rows: visibleRows.length,
    buffer: {
      active: {
        baseY: scrollbackRows.length,
        getLine: (y: number) =>
          y >= 0 && y < lines.length
            ? { translateToString: () => lines[y] }
            : undefined,
      },
    },
  };
}

describe('readBottomRows', () => {
  it('returns the last N rendered rows, top-to-bottom', () => {
    const term = fakeTerm(['a', 'b', 'c', 'd']);
    expect(readBottomRows(term, 2)).toEqual(['c', 'd']);
  });

  it('never reads scrollback — only the on-screen viewport', () => {
    const term = fakeTerm(['visible-1', 'visible-2'], ['old-1', 'old-2', 'old-3']);
    // Ask for more rows than are visible; still clamps to the viewport.
    expect(readBottomRows(term, 10)).toEqual(['visible-1', 'visible-2']);
  });

  it('yields empty for a zero-row terminal', () => {
    expect(readBottomRows(fakeTerm([]), 5)).toEqual([]);
  });
});

describe('matchesApproval', () => {
  const codex = DEFAULT_COMPILED_PACKS.get('codex')!;

  it('matches a known approval prompt', () => {
    const rows = ['', 'Allow the agent to run this command?', '[y/n]'];
    expect(matchesApproval(rows, codex)).toBe(true);
  });

  it('never matches a blank screen (strict approval-only)', () => {
    expect(matchesApproval(['', '', ''], codex)).toBe(false);
    expect(matchesApproval([], codex)).toBe(false);
  });

  it('does not match ordinary non-blocking output', () => {
    const rows = ['Reading files...', 'Editing src/main.rs', 'Running tests'];
    expect(matchesApproval(rows, codex)).toBe(false);
  });

  it('respects a `none` false-positive guard', () => {
    const pack = compileRulePacks([
      {
        agentKind: 'demo',
        approval: [{ id: 'r', all: ['allow'], none: ['already allowed'] }],
      },
    ]).get('demo')!;
    expect(matchesApproval(['allow this?'], pack)).toBe(true);
    expect(matchesApproval(['already allowed, continuing'], pack)).toBe(false);
  });

  it('matches a pattern split across two rendered lines', () => {
    const pack = compileRulePacks([
      { agentKind: 'demo', approval: [{ id: 'r', all: ['do you want', 'proceed'] }] },
    ]).get('demo')!;
    expect(matchesApproval(['Do you want to', 'proceed with this edit?'], pack)).toBe(true);
  });
});

describe('recognizerTick', () => {
  const codex = DEFAULT_COMPILED_PACKS.get('codex')!;

  it('is false when there is no pack for the agent', () => {
    expect(recognizerTick(() => ['Allow this? [y/n]'], undefined)).toBe(false);
  });

  it('reflects the current rows when a pack is present', () => {
    let rows: string[] = ['working...'];
    const tick = () => recognizerTick(() => rows, codex);
    expect(tick()).toBe(false);
    rows = ['Allow the agent to execute? [y/n]'];
    expect(tick()).toBe(true);
    rows = ['done'];
    expect(tick()).toBe(false);
  });
});
