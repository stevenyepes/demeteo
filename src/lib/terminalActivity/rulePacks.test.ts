import { describe, expect, it } from 'vitest';

import {
  compileRulePacks,
  DEFAULT_COMPILED_PACKS,
  DEFAULT_RULE_PACKS,
  RulePackError,
  type AgentRulePack,
} from './rulePacks';

describe('compileRulePacks — the happy path', () => {
  it('compiles a sample pack keyed by agentKind', () => {
    const packs: AgentRulePack[] = [
      { agentKind: 'demo', approval: [{ id: 'r1', all: ['allow'], any: ['run'] }] },
    ];
    const compiled = compileRulePacks(packs);
    expect(compiled.size).toBe(1);
    const demo = compiled.get('demo');
    expect(demo?.agentKind).toBe('demo');
    expect(demo?.approval).toHaveLength(1);
    expect(demo?.approval[0].id).toBe('r1');
    // Patterns compile to case-insensitive regexes.
    expect(demo?.approval[0].all[0].flags).toContain('i');
    expect(demo?.approval[0].all[0].test('ALLOW')).toBe(true);
  });

  it('tolerates a rule that sets only `any` (no `all`)', () => {
    const compiled = compileRulePacks([
      { agentKind: 'demo', approval: [{ id: 'r', any: ['\\[y/n\\]'] }] },
    ]);
    expect(compiled.get('demo')?.approval[0].any[0].test('[y/n]')).toBe(true);
    expect(compiled.get('demo')?.approval[0].all).toEqual([]);
  });
});

describe('compileRulePacks — malformed packs fail loudly (T3.1)', () => {
  it('rejects a missing agentKind', () => {
    expect(() =>
      compileRulePacks([{ approval: [{ id: 'r', all: ['x'] }] } as unknown as AgentRulePack]),
    ).toThrow(RulePackError);
  });

  it('rejects an empty approval array', () => {
    expect(() => compileRulePacks([{ agentKind: 'demo', approval: [] }])).toThrow(
      /non-empty "approval"/,
    );
  });

  it('rejects a rule with neither all nor any (matches every frame)', () => {
    expect(() =>
      compileRulePacks([{ agentKind: 'demo', approval: [{ id: 'empty' }] }]),
    ).toThrow(/at least one of "all"\/"any"/);
  });

  it('rejects an invalid regex with a helpful message', () => {
    expect(() =>
      compileRulePacks([{ agentKind: 'demo', approval: [{ id: 'bad', all: ['('] }] }]),
    ).toThrow(/not a valid regex/);
  });

  it('rejects a non-string pattern', () => {
    expect(() =>
      compileRulePacks([
        { agentKind: 'demo', approval: [{ id: 'r', all: [42] }] } as unknown as AgentRulePack,
      ]),
    ).toThrow(RulePackError);
  });

  it('rejects a rule missing an id', () => {
    expect(() =>
      compileRulePacks([
        { agentKind: 'demo', approval: [{ all: ['x'] }] } as unknown as AgentRulePack,
      ]),
    ).toThrow(/missing a non-empty "id"/);
  });

  it('rejects duplicate packs for the same agentKind', () => {
    expect(() =>
      compileRulePacks([
        { agentKind: 'demo', approval: [{ id: 'a', all: ['x'] }] },
        { agentKind: 'demo', approval: [{ id: 'b', all: ['y'] }] },
      ]),
    ).toThrow(/duplicate rule pack/);
  });
});

describe('the bundled default packs', () => {
  it('compile without error and cover the non-hooked agents', () => {
    expect(DEFAULT_COMPILED_PACKS.size).toBe(DEFAULT_RULE_PACKS.length);
    expect(DEFAULT_COMPILED_PACKS.has('codex')).toBe(true);
    expect(DEFAULT_COMPILED_PACKS.has('opencode')).toBe(true);
    // Claude self-reports via hooks — no on-screen guessing for it.
    expect(DEFAULT_COMPILED_PACKS.has('claude-code')).toBe(false);
  });
});
