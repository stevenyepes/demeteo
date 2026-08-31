// The bubble's whole claim is that it says what the turn is doing. These pin
// the places that claim can quietly become false: an event shape the guards
// no longer recognise (so the turn looks idle or, worse, throws), a `read`
// call that is actually a fetch and gets misfiled, and a source list that
// double-counts the same URL.

import { describe, expect, it } from 'vitest';

import {
  MAX_COMMANDS,
  MAX_LEDGER,
  NO_TURN,
  commandName,
  describeTool,
  foldTurnEvent,
  formatActivitySummary,
  openTurn,
  phaseOfStatus,
  sourcesOf,
  truncateMiddle,
  type LiveTurn,
} from './askActivity';

/** Exactly what `AgentEvent::ToolCall` serialises to — `ActionKind` carries no
 *  `rename_all`, so the action is the Rust variant name verbatim. */
function toolCall(id: string, action: string, target: string) {
  return {
    kind: 'tool_call',
    tool_call_id: id,
    intercept_id: `claude-${id}`,
    action,
    target,
    preview: null,
  };
}

/** `ToolCallStatus` is internally tagged, so the status is a nested object. */
function toolCallUpdate(id: string, status: string) {
  return {
    kind: 'tool_call_update',
    tool_call_id: id,
    status: status === 'failed' ? { status, reason: 'exit 1' } : { status },
    preview: null,
  };
}

function fold(events: unknown[], from: LiveTurn = openTurn(1_000, 'working')): LiveTurn {
  return events.reduce<LiveTurn>((turn, event) => foldTurnEvent(turn, event), from);
}

describe('the activity reducer', () => {
  it('keeps a tool call in flight until its own update says otherwise', () => {
    const turn = fold([toolCall('t1', 'Read', 'src/auth.rs')]);
    expect(turn.current).not.toBeNull();
    expect(turn.current?.kind).toBe('read');
    expect(turn.current?.target).toBe('src/auth.rs');

    const done = foldTurnEvent(turn, toolCallUpdate('t1', 'completed'));
    expect(done.current).toBeNull();
    expect(done.ledger[0].done).toBe(true);
  });

  it('classifies a read call with an absolute URL target as a fetch', () => {
    const turn = fold([toolCall('t1', 'Read', 'https://example.com/docs')]);
    expect(turn.current?.kind).toBe('fetch');
    expect(turn.current?.target).toBe('https://example.com/docs');
  });

  it('leaves a web search — a read with no target at all — out of the fetches', () => {
    const turn = fold([toolCall('t1', 'Read', '')]);
    expect(turn.current?.kind).toBe('read');
    expect(sourcesOf(turn)).toEqual([]);

    const done = foldTurnEvent(turn, toolCallUpdate('t1', 'completed'));
    expect(done.activity.reads).toBe(1);
  });

  it('leaves a read call with a file-path target as a read', () => {
    const turn = fold([toolCall('t1', 'Read', 'src/auth.rs')]);
    expect(turn.current?.kind).toBe('read');
  });

  it('appends text deltas via the same capped buffer as Discovery', () => {
    const turn = fold([
      { kind: 'text', delta: 'Reading' },
      { kind: 'text', delta: ' auth.rs' },
    ]);
    expect(turn.text).toBe('Reading auth.rs');
    expect(turn.activity).toEqual(NO_TURN.activity);
  });

  it('marks the matching ledger entry done and failed on a failed update', () => {
    const turn = fold([
      toolCall('t1', 'RunBash', 'cargo test'),
      toolCallUpdate('t1', 'failed'),
    ]);
    expect(turn.current).toBeNull();
    expect(turn.ledger[0].done).toBe(true);
    expect(turn.ledger[0].failed).toBe(true);
  });

  it('hands the same snapshot back for an event whose kind it does not recognise', () => {
    const turn = fold([toolCall('t1', 'Read', 'a.rs')]);
    expect(() => foldTurnEvent(turn, { kind: 'mode_changed', mode_id: 'plan' })).not.toThrow();
    expect(foldTurnEvent(turn, { kind: 'mode_changed', mode_id: 'plan' })).toBe(turn);
    expect(foldTurnEvent(turn, { kind: 'plan_updated', plan: [] })).toBe(turn);
    expect(foldTurnEvent(turn, {})).toBe(turn);
    expect(foldTurnEvent(turn, null)).toBe(turn);
  });

  it('surfaces the newest of several calls in flight and counts the rest', () => {
    const turn = fold([
      toolCall('t1', 'Read', 'a.rs'),
      toolCall('t2', 'Read', 'b.rs'),
      toolCall('t3', 'RunBash', 'cargo metadata'),
    ]);
    expect(turn.current?.id).toBe('t3');
    expect(turn.alsoRunning).toBe(2);
  });

  it('caps the ledger without losing the counts behind it', () => {
    const events = Array.from({ length: MAX_LEDGER + 10 }, (_, i) =>
      toolCall(`t${i}`, 'Read', `src/f${i}.rs`),
    );
    const turn = fold(events);
    expect(turn.ledger).toHaveLength(MAX_LEDGER);
    expect(turn.activity.reads).toBe(MAX_LEDGER + 10);
  });

  it('folds a fetch call into the reads tally, same as a plain read', () => {
    const turn = fold([toolCall('t1', 'Read', 'https://example.com')]);
    expect(turn.activity.reads).toBe(1);
  });

  it('counts each action against its own tally', () => {
    const turn = fold([
      toolCall('t1', 'Read', 'a.rs'),
      toolCall('t2', 'Edit', 'a.rs'),
      toolCall('t3', 'Write', 'b.rs'),
      toolCall('t4', 'RunBash', 'rg discovery'),
    ]);
    expect(turn.activity).toEqual({
      reads: 1,
      edits: 1,
      writes: 1,
      ran: 1,
      commands: ['rg discovery'],
    });
  });

  it('preserves the turn start across every event', () => {
    const turn = fold([toolCall('t1', 'Read', 'a.rs'), { kind: 'text', delta: 'hi' }]);
    expect(turn.startedAt).toBe(1_000);
  });
});

describe('sourcesOf', () => {
  it('dedupes fetches to the same URL, keeping the first one\'s position', () => {
    const turn = fold([
      toolCall('t1', 'Read', 'https://example.com/a'),
      toolCall('t2', 'Read', 'https://example.com/b'),
      toolCall('t3', 'Read', 'https://example.com/a'),
    ]);
    expect(sourcesOf(turn)).toEqual([
      { url: 'https://example.com/a' },
      { url: 'https://example.com/b' },
    ]);
  });

  it('says nothing about a turn with no fetches', () => {
    const turn = fold([toolCall('t1', 'Read', 'src/a.rs')]);
    expect(sourcesOf(turn)).toEqual([]);
  });

  it('has nothing to say about a turn that never opened', () => {
    expect(sourcesOf(NO_TURN)).toEqual([]);
  });
});

describe('the turn summary', () => {
  it('says nothing about a turn that did nothing, and nothing about one never measured', () => {
    expect(formatActivitySummary(null)).toBeNull();
    expect(formatActivitySummary(NO_TURN.activity)).toBeNull();
  });

  it('names a single command rather than counting it', () => {
    expect(
      formatActivitySummary({
        reads: 6,
        edits: 0,
        writes: 0,
        ran: 1,
        commands: ['git log --oneline -20'],
      }),
    ).toBe('6 reads · ran git log');
  });

  it('marks the sample as partial once it is full', () => {
    const commands = Array.from({ length: MAX_COMMANDS }, (_, i) => `tool-${i} run`);
    expect(
      formatActivitySummary({ reads: 0, edits: 0, writes: 0, ran: 40, commands }),
    ).toContain(', …)');
  });
});

describe('naming a command', () => {
  it('keeps the subcommand that identifies the tool', () => {
    expect(commandName('git log --oneline -20')).toBe('git log');
    expect(commandName('cargo test -p demeteo-core')).toBe('cargo test');
  });

  it('has nothing to say about an empty command', () => {
    expect(commandName('   ')).toBe('');
  });
});

describe('what the activity line shows', () => {
  it('keeps both ends of a path', () => {
    const long = 'crates/demeteo-core/src/adapters/agent/claude_code/mod.rs';
    const shown = truncateMiddle(long, 30);
    expect(shown).toHaveLength(30);
    expect(shown.startsWith('crates/')).toBe(true);
    expect(shown.endsWith('mod.rs')).toBe(true);
  });

  it('shows a fetch with its own verb, distinct from a read', () => {
    expect(
      describeTool({
        id: 't',
        kind: 'fetch',
        target: 'https://example.com/docs',
        done: false,
        failed: false,
      }),
    ).toBe('Fetching https://example.com/docs');
  });

  it('shows a read as the file and a run as the command', () => {
    expect(describeTool({ id: 't', kind: 'read', target: 'src/a.rs', done: false, failed: false }))
      .toBe('Reading src/a.rs');
    expect(
      describeTool({
        id: 't',
        kind: 'run_bash',
        target: 'cargo metadata',
        done: false,
        failed: false,
      }),
    ).toBe('Running cargo metadata');
  });

  it('falls back to the verb alone when the target is empty', () => {
    expect(describeTool({ id: 't', kind: 'write', target: '', done: false, failed: false })).toBe(
      'Writing',
    );
  });
});

describe('the phase a status event puts a turn in', () => {
  it('keeps a turn live while it is being set up', () => {
    expect(phaseOfStatus('setting_up')).toBe('setting_up');
    expect(phaseOfStatus('running')).toBe('working');
  });

  it('ends it on the two that end it, and on anything it cannot read', () => {
    expect(phaseOfStatus('idle')).toBeNull();
    expect(phaseOfStatus('error')).toBeNull();
    expect(phaseOfStatus('')).toBeNull();
  });
});
