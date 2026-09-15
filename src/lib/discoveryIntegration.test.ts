import { describe, expect, it } from 'vitest';

import { discoveryIntegrationActions } from './discoveryIntegration';
import type { Discovery, TicketFeatureView, TicketLane, TicketView } from '../types';

function ticket(
  seq: number,
  lane: TicketLane,
  extra: {
    startable?: boolean;
    blockedBy?: string[];
    unmet?: string[];
    feature?: TicketFeatureView | null;
  } = {},
): TicketView {
  const id = `t${seq}`;
  return {
    ticket: {
      id,
      discovery_id: 'dsc-1',
      seq,
      title: `ticket ${seq}`,
      description: '',
      acceptance: [],
      files: [],
      blocked_by: extra.blockedBy ?? [],
      test_command: null,
      workflow_id: null,
      agent_kind: null,
      model: null,
      effort: null,
      attachments: [],
      state: lane === 'dropped' ? 'dropped' : lane === 'blocked' || lane === 'ready' ? 'unstarted' : 'started',
      drop_reason: null,
      force_start_reason: null,
      force_started_at: null,
      feature_id: extra.feature?.id ?? null,
      created_at: 0,
      updated_at: 0,
    },
    standing: {
      id,
      lane,
      startable: extra.startable ?? lane === 'ready',
      blockers: (extra.unmet ?? []).map((b) => ({ id: b, reason: 'outstanding' as const })),
    },
    feature: extra.feature ?? null,
  };
}

function discovery(overrides: Partial<Discovery> = {}): Discovery {
  return {
    id: 'dsc-1',
    project_id: 'p1',
    title: 'discovery',
    status: 'open',
    machine_id: 'm1',
    agent_kind: 'claude-code',
    model: null,
    effort: null,
    resume_session_id: null,
    worktree_path: null,
    base_branch: 'main',
    integration_mr_url: null,
    integration_mr_state: null,
    attachments: [],
    total_cost: 0,
    tokens: 0,
    created_at: 0,
    updated_at: 0,
    ...overrides,
  };
}

describe('discoveryIntegrationActions', () => {
  describe('showControls', () => {
    it('is false when base_branch is null, independent of ticket state', () => {
      const merged = ticket(1, 'landed', {
        feature: { id: 'f1', status: 'completed', mr_state: 'merged', mr_url: 'https://x/pull/1' },
      });
      const result = discoveryIntegrationActions(discovery({ base_branch: null }), [merged]);
      expect(result.showControls).toBe(false);
    });

    it('is true when base_branch is set, independent of ticket state', () => {
      const result = discoveryIntegrationActions(discovery({ base_branch: 'main' }), []);
      expect(result.showControls).toBe(true);
    });
  });

  describe('sync', () => {
    it('is enabled with zero tickets', () => {
      const result = discoveryIntegrationActions(discovery(), []);
      expect(result.sync.enabled).toBe(true);
      expect(result.sync.reason).toBeNull();
    });

    it('is enabled with some unmerged tickets', () => {
      const tickets = [ticket(1, 'in_flight', { feature: { id: 'f1', status: 'running', mr_state: 'open', mr_url: 'https://x/pull/1' } })];
      const result = discoveryIntegrationActions(discovery(), tickets);
      expect(result.sync.enabled).toBe(true);
      expect(result.sync.reason).toBeNull();
    });

    it('is enabled with some merged tickets', () => {
      const tickets = [ticket(1, 'landed', { feature: { id: 'f1', status: 'completed', mr_state: 'merged', mr_url: 'https://x/pull/1' } })];
      const result = discoveryIntegrationActions(discovery(), tickets);
      expect(result.sync.enabled).toBe(true);
      expect(result.sync.reason).toBeNull();
    });
  });

  describe('publish', () => {
    it('is disabled with a reason when tickets is empty', () => {
      const result = discoveryIntegrationActions(discovery(), []);
      expect(result.publish.enabled).toBe(false);
      expect(result.publish.reason).not.toBeNull();
    });

    it('is disabled with a reason when every ticket has no feature or an unmerged one', () => {
      const tickets = [
        ticket(1, 'ready'),
        ticket(2, 'in_flight', { feature: { id: 'f2', status: 'running', mr_state: 'open', mr_url: 'https://x/pull/2' } }),
        ticket(3, 'in_flight', { feature: { id: 'f3', status: 'running', mr_state: null, mr_url: null } }),
      ];
      const result = discoveryIntegrationActions(discovery(), tickets);
      expect(result.publish.enabled).toBe(false);
      expect(result.publish.reason).not.toBeNull();
    });

    it('is enabled with a null reason when at least one ticket has a merged feature, even with others unmerged or dropped', () => {
      const tickets = [
        ticket(1, 'landed', { feature: { id: 'f1', status: 'completed', mr_state: 'merged', mr_url: 'https://x/pull/1' } }),
        ticket(2, 'in_flight', { feature: { id: 'f2', status: 'running', mr_state: 'open', mr_url: 'https://x/pull/2' } }),
        ticket(3, 'dropped'),
      ];
      const result = discoveryIntegrationActions(discovery(), tickets);
      expect(result.publish.enabled).toBe(true);
      expect(result.publish.reason).toBeNull();
    });

    // Regression: fails if the merged-ticket condition is replaced with an
    // unconditional `enabled: true` for publish.
    it('is disabled for a non-empty, all-unmerged board', () => {
      const tickets = [
        ticket(1, 'in_flight', { feature: { id: 'f1', status: 'running', mr_state: 'open', mr_url: 'https://x/pull/1' } }),
        ticket(2, 'ready'),
        ticket(3, 'blocked', { blockedBy: ['t2'], unmet: ['t2'] }),
      ];
      const result = discoveryIntegrationActions(discovery(), tickets);
      expect(result.publish.enabled).toBe(false);
    });
  });
});
