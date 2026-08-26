import { describe, expect, it } from 'vitest';

import { readableOutput, stripAnsi } from './rawOutput';

const ESC = '\u001b';

/** The shape `scripts/checks.sh` writes for every stage banner, which is what
 *  reached the pane as literal `^[[1;34m` text. */
const banner = `${ESC}[1;34m==>${ESC}[0m Toolchain preflight`;

const transcript = (bodyLines: number): string =>
  [
    "The merge is committed on this branch and the project's checks failed in it, so it was not pushed.",
    '',
    '$ npm run checks:code',
    'Command failed (exit code: Some(101)): ',
    ...Array.from({ length: bodyLines }, (_, i) => `line ${i + 1}`),
    'error: could not compile `demeteo-core` (lib) due to 4 previous errors',
  ].join('\n');

describe('stripAnsi', () => {
  it('removes the colour a harness wrote for a terminal', () => {
    expect(stripAnsi(banner)).toBe('==> Toolchain preflight');
  });

  it('removes cursor and title sequences too', () => {
    expect(stripAnsi(`a${ESC}[2K${ESC}[1Gb${ESC}]0;titlec`)).toBe('abc');
  });

  it('leaves text that carries no escapes exactly as it is', () => {
    const plain = 'CONFLICT (content): Merge conflict in src/lib.rs\n[master 1a2b3c] merge\n';
    expect(stripAnsi(plain)).toBe(plain);
  });

  it('keeps the bracket text an escape only looks like', () => {
    expect(stripAnsi('warning: unused import [dead_code]')).toBe(
      'warning: unused import [dead_code]',
    );
  });

  /** The boundary, pinned. Caret notation is what a `sqlite3` dump shows and
   *  never what reaches this pane, and an arm matching it takes the head off a
   *  regex character class — `^[[a-z]+$` in a failing test's output becomes
   *  `-z]+$`, silently, in the one place the user went to read that output. */
  it('leaves a regex character class alone where an escape only looks like one', () => {
    for (const pattern of ['^[[a-z]+$', '^[[:alpha:]]+', 'matches ^[[A-Z] here', '^[[0-9]+$']) {
      expect(stripAnsi(pattern), pattern).toBe(pattern);
    }
  });
});

describe('readableOutput', () => {
  it('hands back a short transcript whole, with nothing to expand', () => {
    const short = 'fatal: could not read from remote repository.';
    expect(readableOutput(short)).toEqual({ kind: 'whole', text: short });
  });

  /** The press has to be worth its own row: hiding four lines behind a toggle
   *  costs a click to save a scroll that was never needed. */
  it('hands back a transcript barely over the window whole', () => {
    expect(readableOutput(transcript(40)).kind).toBe('whole');
  });

  it('elides the middle of a long transcript and keeps the verdict', () => {
    const result = readableOutput(transcript(2000));
    if (result.kind !== 'elided') throw new Error(`expected an elision, got ${result.kind}`);

    expect(result.head.split('\n')).toHaveLength(6);
    expect(result.head).toContain('$ npm run checks:code');
    expect(result.head).toContain('Command failed (exit code: Some(101))');
    expect(result.tail.split('\n')).toHaveLength(40);
    expect(result.tail).toContain(
      'error: could not compile `demeteo-core` (lib) due to 4 previous errors',
    );
    expect(result.head).not.toContain('line 1000');
    expect(result.tail).not.toContain('line 1000');
    expect(result.hiddenLines).toBe(result.totalLines - 46);
    expect(result.full).toContain('line 1000');
  });

  it('strips the escapes before it counts or splits', () => {
    const result = readableOutput([banner, transcript(2000)].join('\n'));
    if (result.kind !== 'elided') throw new Error(`expected an elision, got ${result.kind}`);

    expect(result.head).toContain('==> Toolchain preflight');
    expect(result.head).not.toContain(ESC);
    expect(result.full).not.toContain(ESC);
  });

  /** A harness that ends with a blank line would otherwise spend the tail's
   *  last rows on nothing, which is where the verdict is looked for. */
  it('does not spend the tail on trailing blank lines', () => {
    const result = readableOutput(`${transcript(2000)}\n\n\n`);
    if (result.kind !== 'elided') throw new Error(`expected an elision, got ${result.kind}`);

    expect(result.tail.endsWith('4 previous errors')).toBe(true);
  });
});
