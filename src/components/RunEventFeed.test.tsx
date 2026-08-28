/**
 * `RunEventFeed` (P2.6): the renderer for the unified run-event log, shared by
 * both transports so a local run and a detached one produce identical rows.
 * These pin the compact per-kind rendering (`describeEvent`) it depends on.
 */
import { render, screen, cleanup } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';

import { RunEventFeed, describeEvent } from './RunEventFeed';
import type { RunEvent } from '../types';

const evt = (over: Partial<RunEvent>): RunEvent => ({
  offset: 0,
  run_id: 'f1',
  kind: 'submitted',
  payload_json: null,
  created_at: 0,
  ...over,
});

afterEach(cleanup);

describe('describeEvent', () => {
  it('renders an agent spawn with the canonical effective-effort label', () => {
    const d = describeEvent(
      'agent_spawned',
      JSON.stringify({
        step_execution_id: 'execution-1',
        agent_kind: 'codex',
        effort: 'xhigh',
      }),
    );

    expect(d.label).toBe('Agent spawned');
    expect(d.detail).toBe('Agent codex · Effective effort Extra high');
  });

  it('renders an explicit null effort without substituting a default', () => {
    const d = describeEvent(
      'agent_spawned',
      JSON.stringify({
        step_execution_id: 'execution-1',
        agent_kind: 'hermes',
        effort: null,
      }),
    );

    expect(d.detail).toBe('Agent hermes · Effective effort No injected effort');
  });

  it('accepts unknown fields on an otherwise valid agent spawn', () => {
    const d = describeEvent(
      'agent_spawned',
      JSON.stringify({
        step_execution_id: 'execution-1',
        agent_kind: 'opencode',
        effort: 'medium',
        added_by_a_newer_runner: true,
      }),
    );

    expect(d.detail).toBe('Agent opencode · Effective effort Medium');
  });

  it.each([
    ['malformed JSON', '{'],
    [
      'version-skewed effort',
      JSON.stringify({
        step_execution_id: 'execution-1',
        agent_kind: 'future-agent',
        effort: 'ultra',
      }),
    ],
  ])('falls back safely for a %s agent spawn payload', (_case, payloadJson) => {
    const d = describeEvent('agent_spawned', payloadJson);

    expect(d.label).toBe('Agent spawned');
    expect(d.detail).toBe(payloadJson === '{' ? '{' : JSON.stringify(JSON.parse(payloadJson)));
  });

  it('renders a step_progress row with status, tokens and cost', () => {
    const d = describeEvent(
      'step_progress',
      JSON.stringify({ step_id: 's-impl', status: 'completed', tokens: 12000, cost_usd: 0.04 }),
    );
    expect(d.tone).toBe('success');
    expect(d.detail).toContain('s-impl done');
    expect(d.detail).toContain('12k tok');
    expect(d.detail).toContain('$0.04');
  });

  it('renders a retry_decision row with class and rule', () => {
    const d = describeEvent(
      'retry_decision',
      JSON.stringify({ step_id: 's-impl', error_class: 'agent_failure', rule_id: 'agent_failure.in_place', attempt: 2, max: 3 }),
    );
    expect(d.detail).toContain('agent_failure');
    expect(d.detail).toContain('agent_failure.in_place');
    expect(d.detail).toContain('attempt 2/3');
  });

  it('falls back to a raw string payload for unknown kinds', () => {
    const d = describeEvent('pushed', JSON.stringify('feature/x'));
    expect(d.label).toBe('Branch pushed');
    expect(d.detail).toBe('feature/x');
  });
});

describe('RunEventFeed', () => {
  it('renders one row per event, newest included', () => {
    render(
      <RunEventFeed
        events={[
          evt({ offset: 1, kind: 'submitted' }),
          evt({ offset: 2, kind: 'pr_opened', payload_json: JSON.stringify('https://example/pr/1') }),
        ]}
      />,
    );
    expect(screen.getByText('Submitted')).toBeInTheDocument();
    expect(screen.getByText('PR opened')).toBeInTheDocument();
    expect(screen.getByText('https://example/pr/1')).toBeInTheDocument();
  });

  it('shows the empty hint when there are no events', () => {
    render(<RunEventFeed events={[]} emptyHint="Nothing yet" />);
    expect(screen.getByText('Nothing yet')).toBeInTheDocument();
  });
});
