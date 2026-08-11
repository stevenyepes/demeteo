// Unit tests for `src/lib/pipelineCard.ts`.

import { describe, expect, it } from 'vitest';

import { pipelineCardMeta } from './pipelineCard';
import { segmentFor } from './pipelineFilter';
import { buildWorkflowById } from './workflowBadge';
import type { Feature } from '../types';

const workflowById = buildWorkflowById([
  { id: 'wf-bugfix', name: 'Bugfix Pipeline', is_starter: true },
  { id: 'wf-feature', name: 'Standard Feature Pipeline', is_starter: false },
]);

function feature(overrides: Partial<Feature> = {}): Feature {
  return {
    id: 'f-42',
    project_id: 'p-1',
    workflow_id: 'wf-feature',
    title: 'Add a retry budget to the verifier',
    description: 'Cap agent retries per step.',
    status: 'running',
    total_cost: 1.25,
    tokens: 12_500,
    duration: '2m 10s',
    created_at: 0,
    ...overrides,
  };
}

function meta(overrides: Partial<Feature> = {}, transport: Partial<{
  detached: boolean;
  computeType: string | undefined;
  remoteHost: string | null | undefined;
}> = {}) {
  return pipelineCardMeta({
    feature: feature(overrides),
    workflowById,
    detached: transport.detached ?? false,
    computeType: transport.computeType,
    remoteHost: transport.remoteHost,
  });
}

describe('scan tier', () => {
  it('carries the status meta, title and elapsed', () => {
    const card = meta();

    expect(card.scan.title).toBe('Add a retry budget to the verifier');
    expect(card.scan.elapsed).toBe('2m 10s');
    expect(card.scan.status).toEqual({ label: 'Running', tone: 'cyan', active: true });
  });

  it('resolves the display status rather than the raw one', () => {
    const card = meta({
      status: 'completed',
      mr_url: 'https://github.com/acme/repo/pull/7',
      mr_state: 'open',
    });

    expect(card.scan.status.label).toBe('Published');
    expect(card.scan.status.tone).toBe('emerald');
  });

  it('flags amber statuses as needing a human', () => {
    expect(meta({ status: 'awaiting_gate' }).scan.needsYou).toBe(true);
    expect(meta({ status: 'running' }).scan.needsYou).toBe(false);
  });

  // The card's badge and the list's ordering are read side by side, so a row the
  // filter leaves in `active` must not carry a needs-you badge. Asserted against
  // `segmentFor` itself rather than a copied status list: a second table here is
  // exactly how the two drifted apart in the first place.
  it.each(['pending', 'bootstrapping', 'running', 'verifying', 'awaiting_gate', 'parked',
    'needs_credentials', 'completed', 'failed', 'cancelled', 'interrupted'])(
    'agrees with the pipeline filter about %s',
    (status) => {
      expect(meta({ status }).scan.needsYou).toBe(segmentFor(feature({ status })) === 'needs-you');
    },
  );

  it('does not claim a still-bootstrapping run needs a decision', () => {
    expect(meta({ status: 'bootstrapping' }).scan.needsYou).toBe(false);
  });
});

describe('context tier', () => {
  it('names a known workflow and falls back on a miss', () => {
    expect(meta().context.workflow).toEqual({
      variant: 'known',
      name: 'Standard Feature Pipeline',
      is_starter: false,
    });
    expect(meta({ workflow_id: 'wf-deleted' }).context.workflow).toEqual({ variant: 'fallback' });
  });

  it('formats cost and tokens', () => {
    const card = meta({ total_cost: 1.25, tokens: 12_500 });

    expect(card.context.cost).toBe('$1.25');
    expect(card.context.tokens).toBe('12.5k');
  });

  it('reads a missing token count as zero', () => {
    expect(meta({ tokens: null }).context.tokens).toBe('0');
  });
});

describe('transport badge', () => {
  it('reports detached even when the project is local', () => {
    const badge = meta({}, { detached: true, computeType: 'local' }).context.transport;

    expect(badge.label).toBe('Detached');
    expect(badge.tone).toBe('cyan');
    expect(badge.title).toMatch(/runner/i);
  });

  it('detached wins over a remote project', () => {
    const badge = meta({}, { detached: true, computeType: 'remote', remoteHost: 'build-01' })
      .context.transport;

    expect(badge.label).toBe('Detached');
  });

  it('names the host an attached remote run executes on', () => {
    const badge = meta({}, { computeType: 'remote', remoteHost: 'build-01' }).context.transport;

    expect(badge.label).toBe('Remote · SSH');
    expect(badge.tone).toBe('cyan');
    expect(badge.title).toBe('Executes on build-01 over SSH');
  });

  it.each([
    ['null', null],
    ['undefined', undefined],
    ['blank', '   '],
  ])('falls back to a generic host when remote_host is %s', (_label, remoteHost) => {
    const badge = meta({}, { computeType: 'remote', remoteHost }).context.transport;

    expect(badge.title).toBe('Executes on the project machine over SSH');
  });

  it.each([
    ['local', 'local'],
    ['unset', undefined],
    ['blank', ''],
  ])('is local when compute_type is %s', (_label, computeType) => {
    const badge = meta({}, { computeType }).context.transport;

    expect(badge.label).toBe('Local');
    expect(badge.tone).toBe('slate');
    expect(badge.title).toBe('Executes on this machine');
  });

  // `ProjectHome` compares `compute_type === 'remote'` in the card but
  // lowercases it elsewhere, so a `Remote` row rendered as local.
  it('matches compute_type case-insensitively', () => {
    expect(meta({}, { computeType: 'Remote' }).context.transport.label).toBe('Remote · SSH');
  });
});

describe('detail tier', () => {
  it('carries the feature id and description', () => {
    const card = meta();

    expect(card.detail.featureId).toBe('f-42');
    expect(card.detail.description).toBe('Cap agent retries per step.');
  });

  it.each([
    ['undefined', undefined],
    ['empty', ''],
    ['whitespace', '  \n '],
  ])('drops a %s description instead of rendering an empty block', (_label, description) => {
    expect(meta({ description }).detail.description).toBeNull();
  });

  it('trims a padded description', () => {
    expect(meta({ description: '  padded  ' }).detail.description).toBe('padded');
  });
});
