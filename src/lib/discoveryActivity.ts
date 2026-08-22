import type { TurnActivity } from '../types';
import { appendCapped } from './streamBuffer';

/**
 * What an interview turn is *doing*, folded out of the events already on the
 * wire.
 *
 * The turn's prose is the last thing to arrive and the least of what it did:
 * Claude's `thinking` blocks are dropped by the parser
 * (`adapters/agent/claude_code/mod.rs`), so a turn that spends two minutes
 * reasoning and grepping emits no `text` at all. The `tool_call` events it
 * *does* emit are what stands between the user and a bubble that looks hung.
 *
 * Everything here is pure and synchronous. The store in
 * `components/discovery/useDiscoveryStream.ts` owns the buffers and the wake;
 * this module owns what an event means, so both halves of the surface — the
 * live bubble and the settled one, which reads
 * {@link TurnActivity} back off the message row — are rendered by the same
 * code and cannot disagree about the same turn.
 */

export type ActivityKind = 'read' | 'edit' | 'write' | 'run_bash';

/** One tool call, as the bubble names it. */
export interface ToolActivity {
  /** The agent's own `tool_call_id`; `tool_call_update` addresses it. */
  id: string;
  kind: ActivityKind;
  target: string;
  done: boolean;
  failed: boolean;
}

export interface LiveTurn {
  /** Milliseconds since the epoch. `0` before a turn has been opened. */
  startedAt: number;
  text: string;
  /** The most recent call still outstanding — an agent may have several. */
  current: ToolActivity | null;
  /** How many others are outstanding behind {@link LiveTurn.current}. */
  alsoRunning: number;
  /** Capped, newest last. What the counters are derived from is
   *  {@link LiveTurn.activity}, which the cap never touches. */
  ledger: readonly ToolActivity[];
  activity: TurnActivity;
}

/**
 * How many calls the ledger keeps. A turn that greps in a loop is exactly the
 * one worth watching and exactly the one that would grow without bound; the
 * counters survive the cap, so nothing the surface renders is lost with the
 * dropped entries.
 */
export const MAX_LEDGER = 60;

/**
 * Mirrors `TurnActivity::MAX_COMMANDS`. The live half caps itself the same way
 * the persisted half does, so a bubble does not lose a name the moment it
 * settles.
 */
export const MAX_COMMANDS = 6;

/** Mirrors `TurnActivity::MAX_COMMAND_CHARS`. */
export const MAX_COMMAND_CHARS = 120;

const NOTHING: readonly never[] = Object.freeze([]);

export const EMPTY_ACTIVITY: TurnActivity = Object.freeze({
  reads: 0,
  edits: 0,
  writes: 0,
  ran: 0,
  commands: NOTHING,
});

/**
 * The resting snapshot — what a discovery with no turn open reads as.
 *
 * One shared value rather than a fresh object per read: `useSyncExternalStore`
 * compares snapshots with `Object.is`, so a store that assembled one per call
 * would re-render forever. Frozen through, because every turn that opens
 * starts as a copy of it and a fold that mutated in place would reach back
 * into this one.
 */
export const NO_TURN: LiveTurn = Object.freeze({
  startedAt: 0,
  text: '',
  current: null,
  alsoRunning: 0,
  ledger: NOTHING,
  activity: EMPTY_ACTIVITY,
});

export function openTurn(startedAt: number): LiveTurn {
  return { ...NO_TURN, startedAt };
}

/**
 * `ActionKind` as serde writes it — the variant names verbatim, because
 * `crates/demeteo-core/src/domain/action.rs` carries no `rename_all`. The
 * snake_case spellings are what `ActionKind::from_str` reads and are accepted
 * here too, so a future rename on either side degrades to a missed icon rather
 * than a dropped event.
 */
const ACTION_KINDS: Record<string, ActivityKind> = {
  Read: 'read',
  Edit: 'edit',
  Write: 'write',
  RunBash: 'run_bash',
  read: 'read',
  edit: 'edit',
  write: 'write',
  run_bash: 'run_bash',
};

function field(event: unknown, key: string): unknown {
  if (typeof event !== 'object' || event === null) return undefined;
  return (event as Record<string, unknown>)[key];
}

function str(value: unknown): string | null {
  return typeof value === 'string' ? value : null;
}

/** `AgentEvent::ToolCall`, or `null` for anything else. */
export function asToolCall(event: unknown): ToolActivity | null {
  if (field(event, 'kind') !== 'tool_call') return null;
  const id = str(field(event, 'tool_call_id'));
  const action = str(field(event, 'action'));
  if (id === null || action === null) return null;
  const kind = ACTION_KINDS[action];
  if (kind === undefined) return null;
  return { id, kind, target: str(field(event, 'target')) ?? '', done: false, failed: false };
}

/**
 * `AgentEvent::ToolCallUpdate`, or `null` for anything else.
 *
 * `ToolCallStatus` is serde-tagged internally, so the status arrives as a
 * nested object rather than a bare string: `{"status":{"status":"failed",…}}`.
 */
export function asToolCallUpdate(
  event: unknown,
): { id: string; done: boolean; failed: boolean } | null {
  if (field(event, 'kind') !== 'tool_call_update') return null;
  const id = str(field(event, 'tool_call_id'));
  if (id === null) return null;
  const status = str(field(field(event, 'status'), 'status'));
  return { id, done: status === 'completed' || status === 'failed', failed: status === 'failed' };
}

/** `AgentEvent::Text`'s delta, or `null` for anything else. */
export function asTextDelta(event: unknown): string | null {
  return field(event, 'kind') === 'text' ? str(field(event, 'delta')) : null;
}

/**
 * Fold one streamed event into the turn.
 *
 * Returns `turn` itself when the event changed nothing, so the store can hand
 * the same snapshot back and skip the wake.
 */
export function foldTurnEvent(turn: LiveTurn, event: unknown): LiveTurn {
  const delta = asTextDelta(event);
  if (delta !== null) {
    const text = appendCapped(turn.text, delta);
    return text === turn.text ? turn : { ...turn, text };
  }

  const call = asToolCall(event);
  if (call !== null) {
    const ledger = [...turn.ledger, call].slice(-MAX_LEDGER);
    return { ...turn, ledger, activity: counted(turn.activity, call), ...outstanding(ledger) };
  }

  const update = asToolCallUpdate(event);
  if (update === null || !update.done) return turn;
  const index = turn.ledger.findIndex((entry) => entry.id === update.id);
  if (index === -1 || turn.ledger[index].done) return turn;
  const ledger = [...turn.ledger];
  ledger[index] = { ...ledger[index], done: true, failed: update.failed };
  return { ...turn, ledger, ...outstanding(ledger) };
}

function outstanding(ledger: readonly ToolActivity[]): Pick<LiveTurn, 'current' | 'alsoRunning'> {
  let current: ToolActivity | null = null;
  let alsoRunning = 0;
  for (let i = ledger.length - 1; i >= 0; i -= 1) {
    if (ledger[i].done) continue;
    if (current === null) current = ledger[i];
    else alsoRunning += 1;
  }
  return { current, alsoRunning };
}

/** The TypeScript half of `TurnActivity::observe`. */
function counted(activity: TurnActivity, call: ToolActivity): TurnActivity {
  if (call.kind === 'read') return { ...activity, reads: activity.reads + 1 };
  if (call.kind === 'edit') return { ...activity, edits: activity.edits + 1 };
  if (call.kind === 'write') return { ...activity, writes: activity.writes + 1 };
  const sample = firstLine(call.target, MAX_COMMAND_CHARS);
  const keep =
    sample.length > 0 &&
    activity.commands.length < MAX_COMMANDS &&
    !activity.commands.includes(sample);
  return {
    ...activity,
    ran: activity.ran + 1,
    commands: keep ? [...activity.commands, sample] : activity.commands,
  };
}

/**
 * A command's first line, capped. A `run_bash` target is whatever the harness
 * put in the tool's `command` input, which for a heredoc or an inline script
 * is the whole program.
 */
export function firstLine(text: string, max: number): string {
  const line = text.split('\n', 1)[0].trim();
  return line.length > max ? line.slice(0, max) : line;
}

/**
 * Keep both ends of a path. The tail is what identifies a file and the head is
 * what places it, so an end-truncated path is the one shape that tells the
 * reader nothing.
 */
export function truncateMiddle(text: string, max: number): string {
  if (text.length <= max) return text;
  if (max <= 1) return '…';
  const head = Math.ceil((max - 1) / 2);
  const tail = max - 1 - head;
  return `${text.slice(0, head)}…${tail > 0 ? text.slice(text.length - tail) : ''}`;
}

function truncateEnd(text: string, max: number): string {
  return text.length <= max ? text : `${text.slice(0, Math.max(0, max - 1))}…`;
}

/** Tools whose first argument is the half a reader recognises. */
const SUBCOMMAND_TOOLS = new Set([
  'git',
  'cargo',
  'npm',
  'pnpm',
  'yarn',
  'go',
  'docker',
  'gh',
  'kubectl',
  'rustup',
]);

/**
 * What to call a command in a summary — `git log`, `rg`, `cargo test`.
 *
 * Made here rather than stored, so the settled meta line and the live one name
 * the same command the same way.
 */
export function commandName(command: string): string {
  const tokens = firstLine(command, MAX_COMMAND_CHARS)
    .split(/\s+/)
    .filter((token) => token.length > 0);
  // A leading `FOO=bar` is the environment the program runs in, not the program.
  const head = tokens.findIndex((token) => !token.includes('='));
  if (head === -1) return '';
  const name = tokens[head].split('/').filter((part) => part.length > 0).pop() ?? tokens[head];
  if (!SUBCOMMAND_TOOLS.has(name)) return name;
  const sub = tokens[head + 1];
  return sub !== undefined && !sub.startsWith('-') ? `${name} ${sub}` : name;
}

const VERBS: Record<ActivityKind, string> = {
  read: 'Reading',
  edit: 'Editing',
  write: 'Writing',
  run_bash: 'Running',
};

/** The in-flight call in human words. */
export function describeTool(tool: ToolActivity, width = 52): string {
  if (tool.target.length === 0) return VERBS[tool.kind];
  const target =
    tool.kind === 'run_bash'
      ? truncateEnd(firstLine(tool.target, MAX_COMMAND_CHARS), width)
      : truncateMiddle(tool.target, width);
  return `${VERBS[tool.kind]} ${target}`;
}

/**
 * What the turn has done so far, as the meta line renders it
 * (`docs/DISCOVERY_UI_SPEC.md` §3.4.3). `null` when it has done nothing worth
 * a line — which is not the same as a turn whose summary was never collected,
 * a distinction the caller keeps by passing `null`.
 */
export function formatActivitySummary(activity: TurnActivity | null): string | null {
  if (activity === null) return null;
  const parts: string[] = [];
  if (activity.reads > 0) parts.push(plural(activity.reads, 'read'));
  if (activity.edits > 0) parts.push(plural(activity.edits, 'edit'));
  if (activity.writes > 0) parts.push(plural(activity.writes, 'write'));
  if (activity.ran > 0) parts.push(ranSummary(activity));
  return parts.length > 0 ? parts.join(' · ') : null;
}

function ranSummary(activity: TurnActivity): string {
  const names: string[] = [];
  for (const command of activity.commands) {
    const name = commandName(command);
    if (name.length > 0 && !names.includes(name)) names.push(name);
  }
  if (names.length === 0) return `ran ${plural(activity.ran, 'command')}`;
  if (activity.ran === 1) return `ran ${names[0]}`;
  // The sample is bounded and the count is not, so a turn that ran more
  // commands than it can name says how many rather than implying these were
  // all of them.
  const more = activity.commands.length >= MAX_COMMANDS ? ', …' : '';
  return `ran ${activity.ran} commands (${names.join(', ')}${more})`;
}

function plural(count: number, noun: string): string {
  return `${count} ${noun}${count === 1 ? '' : 's'}`;
}
