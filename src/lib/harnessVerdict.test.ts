// HB7 — the two engine-authored strings this UI reads its "now" side out of.
//
// The claims these rest on:
//
//   1. A terminal environment failure is *told apart* from a verdict. The two
//      have opposite meanings — the machine's fault vs. the feature's — and the
//      whole point of the panel is that they no longer look alike.
//
//   2. The remediation survives the parse whole. It is the entire payload of an
//      environment failure and the 127 one is several lines long; a
//      single-line read would truncate exactly the part worth pasting.
//
//   3. An excluded gate is recovered by name. `build_exclusion_note` is the
//      only place the engine persists which failures it subtracted, so if this
//      misses them the subtraction is invisible and unauditable.
//
//   4. Absent never becomes green. A gate with no measurement is `not-measured`
//      and a gate nothing reported is `not-reported` — the inversion decision
//      44 exists to prevent.

import { describe, expect, it } from 'vitest';

import {
  buildGateRows,
  isBaselineEnvironmentFailure,
  parseEnvironmentFailure,
  parseHarnessVerdict,
  readHarnessBaseline,
  readHarnessEvidence,
} from './harnessVerdict';
import type { HarnessBaseline, StepExecution } from '../types';

/** Byte-for-byte what `build_environment_message` composes. */
function environmentMessage(opts: {
  reason: string;
  remediation: string;
  cmd: string;
  machine: string;
}): string {
  const reproduce =
    opts.machine === '' || opts.machine === 'local'
      ? `  cd /wt && ${opts.cmd}`
      : `  ssh ${opts.machine}\n  cd /wt && ${opts.cmd}`;
  const remediationLine =
    opts.remediation.trim() === '' ? '' : `\nRemediation: ${opts.remediation.trim()}\n`;
  return (
    'Environment not ready — this failure is not something editing the code can fix.\n\n' +
    `${opts.reason.trim()}\n${remediationLine}\n` +
    `Failing command: ${opts.cmd}\n` +
    `Machine: ${opts.machine === '' ? 'local' : opts.machine}\n` +
    `Reproduce:\n${reproduce}\n`
  );
}

/** Byte-for-byte what `build_failure_reason` + `build_exclusion_note` compose
 *  for one attributable gate beside one excluded one. */
const VERDICT_MESSAGE =
  "'unit' — command 'cargo test' exited with failure:\n" +
  'test tokens::budget ... FAILED\n\n' +
  "\n\nAlso red, but NOT part of this verdict: 'lint'. That gate was already failing " +
  'identically before this feature started, so it is excluded — do not try to fix it.';

function step(over: Partial<StepExecution>): StepExecution {
  return {
    id: 'se-1',
    feature_id: 'f-1',
    step_id: 's-validate',
    step_index: 0,
    step_kind: 'agent',
    status: 'failed',
    artifact_paths: [],
    created_at: 0,
    updated_at: 0,
    ...over,
  };
}

const BASELINE: HarnessBaseline = {
  base_sha: 'abcdef0123456789abcdef',
  harnesses: [
    {
      name: 'lint',
      command: 'npm run lint',
      exit_ok: false,
      fingerprint: 'fp-lint',
      measured_at: 1_700_000_000,
      producer: 'node',
    },
    {
      name: 'unit',
      command: 'cargo test',
      exit_ok: true,
      fingerprint: '',
      measured_at: 1_700_000_000,
      producer: 'node',
    },
  ],
};

describe('parseEnvironmentFailure', () => {
  it('splits the engine message into remediation, reason, command, machine and reproduce', () => {
    const parsed = parseEnvironmentFailure(
      environmentMessage({
        reason: 'The build needs the gdk-3.0 development headers, which are not installed.',
        remediation: 'install libgtk-3-dev\nthen re-run the gate',
        cmd: 'cargo test',
        machine: 'runner-01',
      }),
    );
    expect(parsed).not.toBeNull();
    expect(parsed?.reason).toBe(
      'The build needs the gdk-3.0 development headers, which are not installed.',
    );
    // Multi-line remediation survives whole — the 127 remediation embeds a
    // `bash -l -i -c` check on its own line.
    expect(parsed?.remediation).toBe('install libgtk-3-dev\nthen re-run the gate');
    expect(parsed?.command).toBe('cargo test');
    expect(parsed?.machine).toBe('runner-01');
    expect(parsed?.reproduce).toBe('  ssh runner-01\n  cd /wt && cargo test');
  });

  it('reports an empty remediation rather than inventing one', () => {
    const parsed = parseEnvironmentFailure(
      environmentMessage({
        reason: 'The service the suite talks to is not running.',
        remediation: '',
        cmd: 'npm test',
        machine: 'local',
      }),
    );
    expect(parsed?.remediation).toBe('');
    expect(parsed?.reason).toBe('The service the suite talks to is not running.');
  });

  it('is null for a verdict failure, an agent failure and an empty message', () => {
    expect(parseEnvironmentFailure(VERDICT_MESSAGE)).toBeNull();
    expect(parseEnvironmentFailure("thread 'main' panicked at src/main.rs:4:5")).toBeNull();
    expect(parseEnvironmentFailure(null)).toBeNull();
    expect(parseEnvironmentFailure('')).toBeNull();
  });
});

describe('parseHarnessVerdict', () => {
  it('names the failing gate and the excluded one separately', () => {
    const { failing, excluded } = parseHarnessVerdict(VERDICT_MESSAGE);
    expect(failing).toEqual([{ name: 'unit', command: 'cargo test' }]);
    expect(excluded).toEqual(['lint']);
  });

  it('recovers every gate of a multi-gate verdict', () => {
    const many =
      "2 of this step's harnesses failed — all of them must pass.\n\n" +
      "'lint' — command 'npm run lint' exited with failure:\n3 problems\n\n" +
      "'unit' — command 'cargo test' exited with failure:\n1 failed\n";
    expect(parseHarnessVerdict(many).failing).toEqual([
      { name: 'lint', command: 'npm run lint' },
      { name: 'unit', command: 'cargo test' },
    ]);
  });

  it('finds nothing in a message that names no gate', () => {
    expect(parseHarnessVerdict('the agent produced no report')).toEqual({
      failing: [],
      excluded: [],
    });
  });
});

describe('readHarnessEvidence', () => {
  it('reads the last step that reported a gate, not the first', () => {
    const evidence = readHarnessEvidence([
      step({ id: 'a', step_index: 0, error_message: "'lint' — command 'npm run lint' exited with failure:\nold" }),
      step({ id: 'b', step_index: 1, step_id: 's-validate-2', error_message: VERDICT_MESSAGE }),
    ]);
    expect(evidence?.stepId).toBe('s-validate-2');
    expect(evidence?.failing).toEqual([{ name: 'unit', command: 'cargo test' }]);
  });

  it('is null when no step said anything about a gate', () => {
    expect(readHarnessEvidence([step({ status: 'completed', error_message: null })])).toBeNull();
  });
});

describe('buildGateRows', () => {
  it('pairs each measured gate with what this run said about it', () => {
    const rows = buildGateRows(
      BASELINE,
      readHarnessEvidence([step({ error_message: VERDICT_MESSAGE })]),
    );
    expect(rows.map(r => [r.name, r.baseline, r.now])).toEqual([
      ['lint', 'failed', 'excluded'],
      ['unit', 'passed', 'failed'],
    ]);
  });

  it('reports a gate with no measurement as not-measured, never as passed', () => {
    const rows = buildGateRows(null, readHarnessEvidence([step({ error_message: VERDICT_MESSAGE })]));
    expect(rows).toHaveLength(1);
    expect(rows[0].name).toBe('unit');
    expect(rows[0].baseline).toBe('not-measured');
    expect(rows[0].measuredAt).toBeNull();
  });

  it('reports a measured gate no step named as not-reported, never as passed', () => {
    const rows = buildGateRows(BASELINE, null);
    expect(rows.map(r => r.now)).toEqual(['not-reported', 'not-reported']);
  });

  it('marks a gate the baseline classified environmental as unrunnable, not merely failed', () => {
    const rows = buildGateRows(
      {
        base_sha: 'abc',
        harnesses: [
          {
            name: 'unit',
            command: 'cargo test',
            exit_ok: false,
            fingerprint: 'fp',
            environment: { reason: 'gdk-3.0 is missing', remediation: 'install libgtk-3-dev' },
            measured_at: 1,
            producer: 'node',
          },
        ],
      },
      null,
    );
    expect(rows[0].baseline).toBe('unrunnable');
    expect(rows[0].baselineReason).toBe('gdk-3.0 is missing');
  });

  it('joins the terminal environment failure onto the gate by command', () => {
    const evidence = readHarnessEvidence([
      step({
        error_message: environmentMessage({
          reason: 'gdk-3.0 is missing',
          remediation: 'install libgtk-3-dev',
          cmd: 'cargo test',
          machine: 'local',
        }),
      }),
    ]);
    const rows = buildGateRows(BASELINE, evidence);
    expect(rows.find(r => r.name === 'unit')?.now).toBe('unrunnable');
    expect(rows.find(r => r.name === 'lint')?.now).toBe('not-reported');
  });
});

describe('isBaselineEnvironmentFailure', () => {
  const failure = {
    reason: 'gdk-3.0 is missing',
    remediation: 'install libgtk-3-dev',
    command: 'cargo test',
    machine: 'local',
    reproduce: '  cd /wt && cargo test',
  };

  it('is true only when the record positively classified that gate environmental', () => {
    expect(
      isBaselineEnvironmentFailure(failure, {
        base_sha: 'abc',
        harnesses: [
          {
            name: 'unit',
            command: 'cargo test',
            exit_ok: false,
            environment: { reason: 'gdk-3.0 is missing', remediation: '' },
            measured_at: 1,
            producer: 'node',
          },
        ],
      }),
    ).toBe(true);
  });

  it('is false for a red gate with no classification — the fail-safe direction', () => {
    expect(isBaselineEnvironmentFailure(failure, BASELINE)).toBe(false);
    expect(isBaselineEnvironmentFailure(failure, null)).toBe(false);
  });
});

describe('readHarnessBaseline', () => {
  it('reads the record off a feature payload', () => {
    expect(readHarnessBaseline({ id: 'f-1', harness_baseline: BASELINE })).toEqual(BASELINE);
  });

  it('degrades every shape it does not understand to no baseline', () => {
    expect(readHarnessBaseline({ id: 'f-1' })).toBeNull();
    expect(readHarnessBaseline({ id: 'f-1', harness_baseline: null })).toBeNull();
    expect(readHarnessBaseline({ harness_baseline: { harnesses: [] } })).toBeNull();
    expect(readHarnessBaseline(null)).toBeNull();
  });

  it('drops a gate entry it cannot read rather than inventing its fields', () => {
    const read = readHarnessBaseline({
      harness_baseline: {
        base_sha: 'abc',
        harnesses: [{ name: 'lint' }, BASELINE.harnesses![1]],
      },
    });
    expect(read?.harnesses).toEqual([BASELINE.harnesses![1]]);
  });
});
