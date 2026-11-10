import { afterEach, describe, expect, it, vi } from 'vitest';

import { HubApiError } from './api';
import { listWorkflows } from './workflows';

function stubWorkflows(body: unknown) {
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: unknown): Promise<Response> => {
      if (input !== '/v1/workflows') throw new Error(`fetch double was not told about ${String(input)}`);
      return new Response(JSON.stringify(body), { status: 200, headers: { 'Content-Type': 'application/json' } });
    }),
  );
}

const WIRE = {
  workflow_id: 'wf-starter-feature',
  version: 123_456,
  name: 'Feature',
  description: 'Plan, build and review',
  graph: {
    nodes: [{ id: 's-plan', type: 'agent', title: 'Plan', position: { x: 0, y: 0 } }],
    edges: [],
  },
};

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('listWorkflows', () => {
  it('reads each starter with its description and graph', async () => {
    stubWorkflows({ workflows: [WIRE] });

    await expect(listWorkflows()).resolves.toEqual([
      {
        workflowId: 'wf-starter-feature',
        version: 123_456,
        name: 'Feature',
        description: 'Plan, build and review',
        graph: {
          nodes: [{ id: 's-plan', type: 'agent', title: 'Plan', position: { x: 0, y: 0 } }],
          edges: [],
        },
      },
    ]);
  });

  it.each([
    ['no description', { description: undefined }],
    ['a malformed graph', { graph: { nodes: 'x', edges: [] } }],
    ['a string version', { version: '1' }],
  ])('refuses a workflow with %s', async (_case, change) => {
    stubWorkflows({ workflows: [{ ...WIRE, ...change }] });

    await expect(listWorkflows()).rejects.toBeInstanceOf(HubApiError);
  });
});
