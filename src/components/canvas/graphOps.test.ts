/**
 * `graphOps` (P2.4): the pure downstream-cone traversal powering the replay
 * preview highlight and the modal's DAG-accurate downstream count.
 */
import { describe, expect, it } from 'vitest';

import { descendantIds, replayCone } from './graphOps';
import type { WorkflowDefinitionV2 } from './types';

// research → tickets → { validate, critic } → gate-ship (a diamond fan-in).
const diamond: WorkflowDefinitionV2 = {
  schema_version: 2,
  id: 'wf',
  name: 'diamond',
  nodes: [
    { id: 'research', type: 'agent', title: 'Research' },
    { id: 'tickets', type: 'sequence', title: 'Tickets' },
    { id: 'validate', type: 'agent', title: 'Validate' },
    { id: 'critic', type: 'agent', title: 'Critic' },
    { id: 'gate', type: 'gate', title: 'Gate' },
  ],
  edges: [
    { from: 'research', to: 'tickets' },
    { from: 'tickets', to: 'validate' },
    { from: 'tickets', to: 'critic' },
    { from: 'validate', to: 'gate' },
    { from: 'critic', to: 'gate' },
  ],
};

describe('descendantIds', () => {
  it('collects the whole downstream subgraph across a fan-out/fan-in', () => {
    expect(descendantIds(diamond, 'tickets')).toEqual(
      new Set(['validate', 'critic', 'gate']),
    );
  });

  it('excludes the node itself and returns empty for a sink', () => {
    expect(descendantIds(diamond, 'gate').size).toBe(0);
    expect(descendantIds(diamond, 'validate')).toEqual(new Set(['gate']));
  });

  it('reaches every node from the root', () => {
    expect(descendantIds(diamond, 'research').size).toBe(4);
  });
});

describe('replayCone', () => {
  it('is the node plus its descendants (what replay_from_step re-runs)', () => {
    expect(replayCone(diamond, 'validate')).toEqual(new Set(['validate', 'gate']));
    expect(replayCone(diamond, 'gate')).toEqual(new Set(['gate']));
  });
});
