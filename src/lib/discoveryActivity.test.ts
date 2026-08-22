// The bubble's whole claim is that it says what the turn is doing. These pin
// the three places that claim can quietly become false: an event shape the
// guards no longer recognise (so the turn looks idle), a summary that counts
// what it did not see, and a truncation that hides the half a reader needs.

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
  truncateMiddle,
  type LiveTurn,
} from './discoveryActivity';

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

function fold(events: unknown[], from: LiveTurn = openTurn(1_000)): LiveTurn {
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

  it('surfaces the newest of several calls in flight and counts the rest', () => {
    const turn = fold([
      toolCall('t1', 'Read', 'a.rs'),
      toolCall('t2', 'Read', 'b.rs'),
      toolCall('t3', 'RunBash', 'cargo metadata'),
    ]);
    expect(turn.current?.id).toBe('t3');
    expect(turn.alsoRunning).toBe(2);

    const after = foldTurnEvent(turn, toolCallUpdate('t3', 'completed'));
    expect(after.current?.id).toBe('t2');
    expect(after.alsoRunning).toBe(1);
  });

  it('records a failed call as run, and as failed', () => {
    const turn = fold([
      toolCall('t1', 'RunBash', 'cargo test'),
      toolCallUpdate('t1', 'failed'),
    ]);
    expect(turn.current).toBeNull();
    expect(turn.ledger[0].failed).toBe(true);
    expect(turn.activity.ran).toBe(1);
  });

  it('appends text deltas and leaves everything else alone', () => {
    const turn = fold([
      { kind: 'text', delta: 'Reading' },
      { kind: 'text', delta: ' auth.rs' },
    ]);
    expect(turn.text).toBe('Reading auth.rs');
    expect(turn.activity).toEqual(NO_TURN.activity);
  });

  it('hands the same snapshot back when an event changes nothing', () => {
    const turn = fold([toolCall('t1', 'Read', 'a.rs')]);
    expect(foldTurnEvent(turn, { kind: 'mode_changed', mode_id: 'plan' })).toBe(turn);
    expect(foldTurnEvent(turn, { kind: 'text', delta: '' })).toBe(turn);
    expect(foldTurnEvent(turn, toolCallUpdate('unknown-id', 'completed'))).toBe(turn);
  });

  it('caps the ledger without losing the counts behind it', () => {
    const events = Array.from({ length: MAX_LEDGER + 10 }, (_, i) =>
      toolCall(`t${i}`, 'Read', `src/f${i}.rs`),
    );
    const turn = fold(events);
    expect(turn.ledger).toHaveLength(MAX_LEDGER);
    expect(turn.activity.reads).toBe(MAX_LEDGER + 10);
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

  it('accepts the snake_case spelling of an action as well as the variant name', () => {
    const turn = fold([toolCall('t1', 'run_bash', 'ls')]);
    expect(turn.activity.ran).toBe(1);
  });

  it('preserves the turn start across every event', () => {
    const turn = fold([toolCall('t1', 'Read', 'a.rs'), { kind: 'text', delta: 'hi' }]);
    expect(turn.startedAt).toBe(1_000);
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

  it('counts the runs it cannot name individually', () => {
    expect(
      formatActivitySummary({
        reads: 1,
        edits: 0,
        writes: 0,
        ran: 9,
        commands: ['git log --oneline', 'rg discovery'],
      }),
    ).toBe('1 read · ran 9 commands (git log, rg)');
  });

  it('marks the sample as partial once it is full', () => {
    const commands = Array.from({ length: MAX_COMMANDS }, (_, i) => `tool-${i} run`);
    expect(
      formatActivitySummary({ reads: 0, edits: 0, writes: 0, ran: 40, commands }),
    ).toContain(', …)');
  });

  it('still reports runs whose commands were all blank', () => {
    expect(formatActivitySummary({ reads: 0, edits: 0, writes: 0, ran: 3, commands: [] })).toBe(
      'ran 3 commands',
    );
  });

  it('reads the same from a live turn as from the stored one it becomes', () => {
    const live = fold([
      toolCall('t1', 'Read', 'src/a.rs'),
      toolCall('t2', 'Read', 'src/b.rs'),
      toolCall('t3', 'RunBash', 'git log --oneline'),
      toolCall('t4', 'RunBash', 'rg discovery'),
    ]);
    // What the backend persists is this same shape, off the same events.
    expect(formatActivitySummary(live.activity)).toBe('2 reads · ran 2 commands (git log, rg)');
  });
});

describe('naming a command', () => {
  it('keeps the subcommand that identifies the tool', () => {
    expect(commandName('git log --oneline -20')).toBe('git log');
    expect(commandName('cargo test -p demeteo-core')).toBe('cargo test');
    expect(commandName('npm run checks')).toBe('npm run');
  });

  it('drops flags where there is no subcommand to keep', () => {
    expect(commandName('git --version')).toBe('git');
    expect(commandName('rg -n discovery src/')).toBe('rg');
  });

  it('reads through a path and a leading environment assignment', () => {
    expect(commandName('/usr/bin/git status')).toBe('git status');
    expect(commandName('RUST_LOG=debug cargo build')).toBe('cargo build');
  });

  it('takes only the first line of a script', () => {
    expect(commandName('set -e\nrm -rf /tmp/x')).toBe('set');
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
    expect(shown).toContain('…');
  });

  it('leaves a path that already fits untouched', () => {
    expect(truncateMiddle('src/auth.rs', 30)).toBe('src/auth.rs');
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

  it('shows only the first line of a script-shaped command', () => {
    const shown = describeTool({
      id: 't',
      kind: 'run_bash',
      target: 'set -euo pipefail\nfor f in *; do echo "$f"; done',
      done: false,
      failed: false,
    });
    expect(shown).toBe('Running set -euo pipefail');
    expect(shown).not.toContain('\n');
  });

  it('falls back to the verb alone when the target is empty', () => {
    expect(describeTool({ id: 't', kind: 'write', target: '', done: false, failed: false })).toBe(
      'Writing',
    );
  });
});
