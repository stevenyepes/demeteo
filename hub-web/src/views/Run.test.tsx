import { act, fireEvent, render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import type { ElkNode } from 'elkjs/lib/elk-api.js';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';

import type { WorkflowDefinitionV2 } from '../../../src/components/canvas/types';
import { runStatusMeta } from '../../../src/lib/runStatus';
import { setCsrfToken } from '../lib/api';
import { Run } from './Run';

/** jsdom has no `Worker`; this answers with a position for exactly the children it was given. */
vi.mock('elkjs/lib/elk-api.js', () => ({
  default: class {
    async layout(graph: ElkNode): Promise<ElkNode> {
      return {
        ...graph,
        children: (graph.children ?? []).map((c, i) => ({ ...c, x: i * 400, y: 0 })),
      };
    }
    terminateWorker() {}
  },
}));

const INSTANCE = '6f1c2b3a-0000-4000-8000-000000000001';
const FEATURE = 'f-42';
const PATH = `/v1/instances/${INSTANCE}/runs/${FEATURE}`;
const INSTANCE_PATH = `/v1/instances/${INSTANCE}`;

const WORKFLOW: WorkflowDefinitionV2 = {
  schema_version: 2,
  id: 'wf-run',
  name: 'Run fixture',
  nodes: [
    { id: 'plan', type: 'agent', title: 'Plan work' },
    { id: 'implement', type: 'agent', title: 'Implement ticket' },
    { id: 'verify', type: 'command', title: 'Verify build' },
    { id: 'approve', type: 'gate', title: 'Approve merge' },
    { id: 'ship', type: 'finalize', title: 'Ship it' },
  ],
  edges: [
    { from: 'plan', to: 'implement' },
    { from: 'implement', to: 'verify' },
    { from: 'verify', to: 'approve' },
    { from: 'approve', to: 'ship' },
  ],
};

/** Every request the double saw, scripted or not. */
let asked: string[] = [];

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
  setCsrfToken(null);
  asked = [];
});

beforeAll(() => {
  // React Flow reaches for these; jsdom implements neither.
  class DOMMatrixStub {
    m22 = 1;
  }
  vi.stubGlobal('DOMMatrixReadOnly', DOMMatrixStub);
  vi.stubGlobal('DOMMatrix', DOMMatrixStub);
});

/**
 * Answers `GET PATH` once per scripted reply, in order, `GET INSTANCE_PATH`
 * with `instance`, and throws on anything else.
 */
function stubFetch(...answers: (() => Response)[]) {
  stubHub(json(instanceDetail()), ...answers);
}

function stubHub(instance: () => Response, ...answers: (() => Response)[]) {
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: unknown, init?: RequestInit): Promise<Response> => {
      const request = `${init?.method ?? 'GET'} ${String(input)}`;
      asked.push(request);
      if (request === `GET ${INSTANCE_PATH}`) return instance();
      const answer = answers.shift();
      if (answer === undefined || request !== `GET ${PATH}`) {
        throw new Error(`fetch double was not told about ${request}`);
      }
      return answer();
    }),
  );
}

function instanceDetail(): Record<string, unknown> {
  return {
    id: INSTANCE,
    present: true,
    connected_at: 1_700_000_000,
    last_seen: 1_700_000_000,
    install_id: 'install-9',
    os: 'linux',
    arch: 'x86_64',
    version: '1.4.2',
    scopes: ['read', 'spend', 'gates'],
    read_granted: true,
    projects: [],
    live_runs: [],
    harnesses: null,
  };
}

function json(body: unknown, status = 200): () => Response {
  return () =>
    new Response(JSON.stringify(body), {
      status,
      headers: { 'Content-Type': 'application/json' },
    });
}

function detail(overrides: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    instance_id: INSTANCE,
    feature_id: FEATURE,
    status: 'running',
    workflow: WORKFLOW,
    steps: [
      { step_id: 'plan', state: 'completed' },
      { step_id: 'implement', state: 'running' },
      { step_id: 'verify', state: 'failed' },
      { step_id: 'approve', state: 'running' },
    ],
    tickets: { completed: 0, failed: 0, running: 0 },
    pending_gates: [{ feature_id: FEATURE, step_execution_id: 'se-approve', step_id: 'approve', since: 1_700_000_000 }],
    events: [],
    ...overrides,
  };
}

/** `findBy*` never settles under Vitest's fake timers; `vi.waitFor` advances them itself. */
async function landed() {
  await vi.waitFor(() => screen.getByText(/this instance has not reported the run's graph/i));
}

function nodeCard(container: HTMLElement, nodeId: string): HTMLElement {
  const card = container.querySelector<HTMLElement>(`.react-flow__node[data-id="${nodeId}"]`);
  if (card === null) throw new Error(`the canvas drew no node ${nodeId}`);
  return card;
}

/**
 * The `workflow-canvas` wrapper lands a commit before React Flow's store takes the nodes and
 * draws them, so a lookup straight after `findByTestId('workflow-canvas')` can find none.
 */
async function canvasDrawn(container: HTMLElement) {
  await vi.waitFor(() => {
    for (const node of WORKFLOW.nodes) nodeCard(container, node.id);
  });
}

describe('Run', () => {
  it('draws the reported graph with each node in the state the mirror reported', async () => {
    stubFetch(json(detail()));
    const { container } = render(<Run instanceId={INSTANCE} featureId={FEATURE} />);

    await canvasDrawn(container);
    for (const node of WORKFLOW.nodes) {
      expect(within(nodeCard(container, node.id)).getByText(node.title)).toBeInTheDocument();
    }
    expect(within(nodeCard(container, 'implement')).getByText(runStatusMeta('running').label)).toBeInTheDocument();
    expect(within(nodeCard(container, 'verify')).getByText(runStatusMeta('failed').label)).toBeInTheDocument();
    expect(
      within(nodeCard(container, 'approve')).getByText(runStatusMeta('awaiting_gate').label),
    ).toBeInTheDocument();
    expect(within(nodeCard(container, 'ship')).queryByText(runStatusMeta('running').label)).toBeNull();
    expect(asked).toEqual([`GET ${PATH}`, `GET ${INSTANCE_PATH}`]);
  });

  it('says the graph was not reported, lists the reported steps, and draws no canvas', async () => {
    stubFetch(json(detail({ workflow: null })));
    render(<Run instanceId={INSTANCE} featureId={FEATURE} />);

    expect(await screen.findByText(/this instance has not reported the run's graph/i)).toBeInTheDocument();
    const steps = screen.getByRole('list', { name: 'Reported steps' });
    expect(
      within(steps)
        .getAllByRole('listitem')
        .map((item) => item.textContent),
    ).toEqual([
      `plan${runStatusMeta('completed').label}`,
      `implement${runStatusMeta('running').label}`,
      `verify${runStatusMeta('failed').label}`,
      `approve${runStatusMeta('awaiting_gate').label}`,
    ]);
    expect(screen.queryByTestId('workflow-canvas')).toBeNull();
    expect(screen.queryByRole('application')).toBeNull();
  });

  it.each([
    ['the canvas', WORKFLOW],
    ['the not-reported state', null],
  ])('shows ticket progress and the mirrored events below %s', async (_, workflow) => {
    stubFetch(
      json(
        detail({
          workflow,
          tickets: { completed: 3, failed: 1, running: 2 },
          events: [{ offset: 7, kind: 'ticket_started', occurred_at: 1_700_000_000, step_id: null, step_execution_id: null, ticket_id: 't-9' }],
        }),
      ),
    );
    render(<Run instanceId={INSTANCE} featureId={FEATURE} />);

    const tickets = await screen.findByRole('region', { name: 'Tickets' });
    expect(within(tickets).getByText('Running', { selector: 'dt' }).nextElementSibling).toHaveTextContent('2');
    expect(within(tickets).getByRole('list', { name: 'Tickets' })).toHaveTextContent('t-9ticket_started');
    const events = screen.getByRole('region', { name: 'Events' });
    expect(within(events).getByRole('listitem')).toHaveTextContent('#7ticket_startedticket t-9');
    expect(screen.queryByTestId('workflow-canvas') === null).toBe(workflow === null);
  });

  it('shows the run status in the header', async () => {
    stubFetch(json(detail({ workflow: null, status: 'cancelled' })));
    render(<Run instanceId={INSTANCE} featureId={FEATURE} />);

    expect(await screen.findByText(runStatusMeta('cancelled').label)).toBeInTheDocument();
  });

  it('does not ask again for a run that has finished', async () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
    stubFetch(json(detail({ workflow: null, status: 'completed' })));
    render(<Run instanceId={INSTANCE} featureId={FEATURE} />);
    await landed();

    await act(() => vi.advanceTimersByTimeAsync(60_000));

    expect(asked).toEqual([`GET ${PATH}`, `GET ${INSTANCE_PATH}`]);
  });

  it('asks again while the run is running, and stops when the session is gone', async () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
    stubFetch(
      json(detail({ workflow: null })),
      json(detail({ workflow: null, steps: [{ step_id: 'plan', state: 'failed' }], pending_gates: [] })),
      json({ error: 'unauthenticated' }, 401),
    );
    render(<Run instanceId={INSTANCE} featureId={FEATURE} />);
    await landed();

    await act(() => vi.advanceTimersByTimeAsync(60_000));

    expect(asked).toEqual([`GET ${PATH}`, `GET ${INSTANCE_PATH}`, `GET ${PATH}`, `GET ${PATH}`]);
    expect(screen.getByRole('alert')).toHaveTextContent('unauthenticated');
  });

  it('stops asking once unmounted', async () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
    stubFetch(json(detail({ workflow: null })));
    const { unmount } = render(<Run instanceId={INSTANCE} featureId={FEATURE} />);
    await landed();

    unmount();
    await act(() => vi.advanceTimersByTimeAsync(60_000));

    expect(asked).toEqual([`GET ${PATH}`, `GET ${INSTANCE_PATH}`]);
  });

  it('renders a run the Hub does not know as not found', async () => {
    stubFetch(json({ error: 'not_found' }, 404));
    render(<Run instanceId={INSTANCE} featureId={FEATURE} />);

    expect(await screen.findByRole('alert')).toHaveTextContent(/was not found or not yours/);
    expect(screen.queryByTestId('workflow-canvas')).toBeNull();
  });

  it.each([
    ['withdrawn', 404, 'not_found', /was not found or not yours/],
    ['no longer readable', 409, 'scope_not_granted', /does not grant read/],
  ])('stops asking once the run is %s, and says so in place of the last answer', async (_, status, code, said) => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'Date'] });
    stubFetch(json(detail({ workflow: null })), json({ error: code }, status));
    render(<Run instanceId={INSTANCE} featureId={FEATURE} />);
    await landed();

    await act(() => vi.advanceTimersByTimeAsync(5000));
    expect(screen.getByRole('alert')).toHaveTextContent(said);
    expect(screen.queryByText(/this instance has not reported the run's graph/i)).toBeNull();

    await act(() => vi.advanceTimersByTimeAsync(60_000));
    expect(asked).toEqual([`GET ${PATH}`, `GET ${INSTANCE_PATH}`, `GET ${PATH}`]);
  });

  it('marks the last answer stale while a refresh fails, keeps asking, and clears the mark on success', async () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'Date'] });
    vi.setSystemTime(1_700_000_000_000);
    stubFetch(
      json(detail({ workflow: null })),
      json({ error: 'internal' }, 500),
      json(detail({ workflow: null })),
    );
    render(<Run instanceId={INSTANCE} featureId={FEATURE} />);
    await landed();
    expect(screen.queryByText(/could not refresh/)).toBeNull();

    await act(() => vi.advanceTimersByTimeAsync(5000));
    const notice = screen.getByText(/could not refresh/);
    expect(notice).toHaveTextContent(`Last updated at ${new Date(1_700_000_000_000).toLocaleTimeString()}`);
    expect(screen.getByText(/this instance has not reported the run's graph/i)).toBeInTheDocument();

    await act(() => vi.advanceTimersByTimeAsync(5000));
    expect(asked).toEqual([`GET ${PATH}`, `GET ${INSTANCE_PATH}`, `GET ${PATH}`, `GET ${PATH}`]);
    expect(screen.queryByText(/could not refresh/)).toBeNull();
  });

  describe('step inspector', () => {
    it('opens on a step chosen from the not-reported list with its id and state, and closes', async () => {
      stubFetch(json(detail({ workflow: null })));
      const user = userEvent.setup();
      render(<Run instanceId={INSTANCE} featureId={FEATURE} />);
      const steps = await screen.findByRole('list', { name: 'Reported steps' });
      expect(screen.queryByRole('region', { name: 'Step' })).toBeNull();

      await user.click(within(steps).getByRole('button', { name: /^verify/ }));

      const inspector = screen.getByRole('region', { name: 'Step' });
      expect(field(inspector, 'Id')).toHaveTextContent(/^verify$/);
      expect(field(inspector, 'State')).toHaveTextContent(runStatusMeta('failed').label);
      expect(within(inspector).queryByText('Pending gate')).toBeNull();

      await user.click(within(inspector).getByRole('button', { name: 'Close' }));

      expect(screen.queryByRole('region', { name: 'Step' })).toBeNull();
    });

    it("shows a pending gate's step execution id", async () => {
      stubFetch(json(detail({ workflow: null })));
      const user = userEvent.setup();
      render(<Run instanceId={INSTANCE} featureId={FEATURE} />);
      const steps = await screen.findByRole('list', { name: 'Reported steps' });

      await user.click(within(steps).getByRole('button', { name: /^approve/ }));

      const inspector = screen.getByRole('region', { name: 'Step' });
      expect(field(inspector, 'Pending gate')).toHaveTextContent(/^se-approve$/);
      expect(field(inspector, 'State')).toHaveTextContent(runStatusMeta('running').label);
    });

    it('lists a gated step that reported no event, and says its state was not reported', async () => {
      stubFetch(
        json(
          detail({
            workflow: null,
            steps: [{ step_id: 'plan', state: 'completed' }],
            pending_gates: [
              { feature_id: FEATURE, step_execution_id: 'se-review', step_id: 'review', since: 1_700_000_000 },
            ],
          }),
        ),
      );
      const user = userEvent.setup();
      render(<Run instanceId={INSTANCE} featureId={FEATURE} />);
      const steps = await screen.findByRole('list', { name: 'Reported steps' });

      await user.click(within(steps).getByRole('button', { name: /^review/ }));

      const inspector = screen.getByRole('region', { name: 'Step' });
      expect(field(inspector, 'State')).toHaveTextContent(/^not reported$/);
      expect(field(inspector, 'Pending gate')).toHaveTextContent(/^se-review$/);
    });

    // `user.click` presses the mouse first, and d3-zoom's mousedown handler needs a real view.
    it('opens on a node chosen on the canvas with its kind and title, and clears the highlight on close', async () => {
      stubFetch(json(detail()));
      const { container } = render(<Run instanceId={INSTANCE} featureId={FEATURE} />);
      await canvasDrawn(container);

      fireEvent.click(nodeCard(container, 'ship'));

      const inspector = screen.getByRole('region', { name: 'Step' });
      expect(field(inspector, 'Id')).toHaveTextContent(/^ship$/);
      expect(field(inspector, 'State')).toHaveTextContent(/^not reported$/);
      expect(field(inspector, 'Kind')).toHaveTextContent(/^Finalize$/);
      expect(field(inspector, 'Title')).toHaveTextContent(/^Ship it$/);
      expect(nodeCard(container, 'ship')).toHaveClass('selected');

      fireEvent.click(within(inspector).getByRole('button', { name: 'Close' }));

      expect(screen.queryByRole('region', { name: 'Step' })).toBeNull();
      expect(nodeCard(container, 'ship')).not.toHaveClass('selected');
    });

    it('opens the gate panel for a gate on a step the reported graph has no node for', async () => {
      stubFetch(
        json(
          detail({
            pending_gates: [
              { feature_id: FEATURE, step_execution_id: 'se-hotfix', step_id: 'hotfix-review', since: 1_700_000_000 },
            ],
          }),
        ),
      );
      const user = userEvent.setup();
      render(<Run instanceId={INSTANCE} featureId={FEATURE} />);
      const gates = await screen.findByRole('list', { name: 'Gates off the graph' });

      await user.click(within(gates).getByRole('button', { name: /^hotfix-review/ }));

      const inspector = screen.getByRole('region', { name: 'Step' });
      expect(field(inspector, 'Pending gate')).toHaveTextContent(/^se-hotfix$/);
      expect(within(inspector).getByRole('region', { name: 'Gate' })).toBeInTheDocument();
      expect(within(inspector).getByRole('button', { name: 'Approve' })).toBeEnabled();
    });

    it('lists no off-graph gates when every gate is on the graph', async () => {
      stubFetch(json(detail()));
      render(<Run instanceId={INSTANCE} featureId={FEATURE} />);
      await screen.findByTestId('workflow-canvas');

      expect(screen.queryByRole('list', { name: 'Gates off the graph' })).toBeNull();
    });

    it('still opens when the instance read fails', async () => {
      stubHub(json({ error: 'not_found' }, 404), json(detail({ workflow: null })));
      const user = userEvent.setup();
      render(<Run instanceId={INSTANCE} featureId={FEATURE} />);
      const steps = await screen.findByRole('list', { name: 'Reported steps' });

      await user.click(within(steps).getByRole('button', { name: /^plan/ }));

      expect(field(screen.getByRole('region', { name: 'Step' }), 'Id')).toHaveTextContent(/^plan$/);
      expect(asked).toEqual([`GET ${PATH}`, `GET ${INSTANCE_PATH}`]);
    });

    it('offers nothing to open beyond the mirror and the assignment', async () => {
      stubFetch(json(detail({ workflow: null })));
      const user = userEvent.setup();
      render(<Run instanceId={INSTANCE} featureId={FEATURE} />);
      const steps = await screen.findByRole('list', { name: 'Reported steps' });

      await user.click(within(steps).getByRole('button', { name: /^implement/ }));

      const inspector = screen.getByRole('region', { name: 'Step' });
      expect(within(inspector).getAllByRole('button').map((button) => button.textContent)).toEqual([
        'Close',
        'Change assignment',
      ]);
      expect(within(inspector).queryAllByRole('link')).toEqual([]);
      expect(inspector).not.toHaveTextContent(/transcript|log|diff|output|artifact/i);
    });
  });
});

/**
 * Every path hub-web may ask a Run view's Hub for. There is no transcript,
 * log, output, diff or artifact route to add here: the Hub holds none
 * (docs/HUB.md §6.4). `/v1/session` is `postInSession`'s re-read after a CSRF
 * refusal.
 */
const ALLOWED_PATHS: readonly RegExp[] = [
  /^\/v1\/runs(\?[^#]*)?$/,
  /^\/v1\/instances\/[^/?#]+$/,
  /^\/v1\/instances\/[^/?#]+\/runs\/[^/?#]+$/,
  /^\/v1\/instances\/[^/?#]+\/runs\/[^/?#]+\/steps\/[^/?#]+\/assignment$/,
  /^\/v1\/instances\/[^/?#]+\/gates\/(challenge|decision)$/,
  /^\/v1\/instances\/[^/?#]+\/requests\/[^/?#]+$/,
  /^\/v1\/session$/,
];

const ASSIGNMENT_PATH = `${PATH}/steps/plan/assignment`;
const CHALLENGE_PATH = `${INSTANCE_PATH}/gates/challenge`;
const DECISION_PATH = `${INSTANCE_PATH}/gates/decision`;
const ASSIGN_REQUEST = 'rq-assign';
const GATE_REQUEST = 'rq-gate';
const EXPIRES_AT = 4_000_000_000;

/** Not one this build names: a refusal is shown as the instance worded it. */
const REFUSAL = 'gate_superseded_by_operator';

type Sent = { path: string; body: unknown };

/**
 * Answers each `"METHOD path"` with its replies in order, repeating the last
 * one for the polls that follow, and throws on anything it was not told about.
 */
function stubJourney(script: Record<string, (() => Response)[]>) {
  const sent: Sent[] = [];
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: unknown, init?: RequestInit): Promise<Response> => {
      const path = String(input);
      const request = `${init?.method ?? 'GET'} ${path}`;
      asked.push(request);
      sent.push({ path, body: typeof init?.body === 'string' ? JSON.parse(init.body) : undefined });
      const replies = script[request];
      const reply = replies !== undefined && replies.length > 1 ? replies.shift() : replies?.[0];
      if (reply === undefined) throw new Error(`fetch double was not told about ${request}`);
      return reply();
    }),
  );
  return { sent };
}

function requestState(requestId: string, kind: string, outcome: Record<string, unknown>): () => Response {
  return json({
    request_id: requestId,
    kind,
    deliveries: 1,
    expires_at: EXPIRES_AT,
    answered_at: null,
    ...outcome,
  });
}

function buffer(...octets: number[]): ArrayBuffer {
  return new Uint8Array(octets).buffer;
}

/** Answers `get` with one assertion and refuses `create`, which no gate decision reaches. */
function stubWebAuthn() {
  const credentials = {
    create: vi.fn(async () => {
      throw new Error('navigator.credentials.create double was not told to answer');
    }),
    get: vi.fn(async (_options?: CredentialRequestOptions) => ({
      rawId: buffer(0xfb, 0xff),
      type: 'public-key',
      response: {
        clientDataJSON: buffer(1, 2, 3),
        authenticatorData: buffer(4, 5, 6, 7),
        signature: buffer(8),
        userHandle: null,
      },
    })),
  };
  vi.stubGlobal('navigator', { credentials });
  vi.stubGlobal('PublicKeyCredential', class {});
  return credentials;
}

function journeyScript(): Record<string, (() => Response)[]> {
  return {
    [`GET ${PATH}`]: [json(detail())],
    [`GET ${INSTANCE_PATH}`]: [json(instanceDetail())],
    [`POST ${ASSIGNMENT_PATH}`]: [json({ request_id: ASSIGN_REQUEST, expires_at: EXPIRES_AT }, 202)],
    [`GET ${INSTANCE_PATH}/requests/${ASSIGN_REQUEST}`]: [
      requestState(ASSIGN_REQUEST, 'set_step_assignment', { status: 'accepted' }),
    ],
    [`POST ${CHALLENGE_PATH}`]: [
      json({
        ceremony_id: 'cer-9',
        request_id: GATE_REQUEST,
        expires_at: EXPIRES_AT,
        gate_challenge: 'AAECAw',
        public_key: { challenge: '-_-_', rpId: 'hub.example', userVerification: 'discouraged' },
      }),
    ],
    [`POST ${DECISION_PATH}`]: [json({ request_id: GATE_REQUEST, expires_at: EXPIRES_AT }, 202)],
    [`GET ${INSTANCE_PATH}/requests/${GATE_REQUEST}`]: [
      requestState(GATE_REQUEST, 'gate_decision', { status: 'refused', reason: REFUSAL }),
    ],
  };
}

describe('Run, end to end on the real wrappers', () => {
  it('loads, selects a step on the canvas, assigns it, and decides its gate over a verified passkey', async () => {
    setCsrfToken('csrf-1');
    const { sent } = stubJourney(journeyScript());
    const credentials = stubWebAuthn();
    const { container } = render(<Run instanceId={INSTANCE} featureId={FEATURE} />);
    await canvasDrawn(container);

    fireEvent.click(nodeCard(container, 'plan'));
    const planInspector = screen.getByRole('region', { name: 'Step' });
    expect(field(planInspector, 'Id')).toHaveTextContent(/^plan$/);
    const assignment = within(planInspector).getByRole('region', { name: 'Assignment' });
    await vi.waitFor(() => expect(within(assignment).getByRole('group')).toBeEnabled());
    fireEvent.change(within(assignment).getByLabelText('Agent'), { target: { value: 'codex' } });
    fireEvent.change(within(assignment).getByLabelText('Model'), { target: { value: ' gpt-5-codex ' } });
    fireEvent.click(within(assignment).getByRole('button', { name: 'Change assignment' }));

    await vi.waitFor(() =>
      expect(within(assignment).getByRole('status', { name: 'Request status' })).toHaveTextContent(
        'Accepted by the instance',
      ),
    );
    expect(sent.find((request) => request.path === ASSIGNMENT_PATH)?.body).toEqual({
      agent_kind: 'codex',
      model: 'gpt-5-codex',
    });

    fireEvent.click(nodeCard(container, 'approve'));
    const gateInspector = screen.getByRole('region', { name: 'Step' });
    expect(field(gateInspector, 'Id')).toHaveTextContent(/^approve$/);
    const gate = within(gateInspector).getByRole('region', { name: 'Gate' });
    fireEvent.click(within(gate).getByRole('button', { name: 'Approve' }));

    const status = await vi.waitFor(() => {
      const decision = within(gateInspector).getByRole('region', { name: 'Gate decision' });
      const shown = within(decision).getByRole('status', { name: 'Request status' });
      expect(shown).toHaveTextContent('Refused by the instance');
      return shown;
    });
    expect(within(status).getByText(REFUSAL).textContent).toBe(REFUSAL);
    expect(credentials.get).toHaveBeenCalledTimes(1);
    expect(credentials.get.mock.calls[0]?.[0]?.publicKey?.userVerification).toBe('required');
    expect(sent.find((request) => request.path === CHALLENGE_PATH)?.body).toEqual({
      feature_id: FEATURE,
      step_execution_id: 'se-approve',
      decision: 'approve',
    });

    const paths = asked.map((request) => request.slice(request.indexOf(' ') + 1));
    expect(paths.filter((path) => !ALLOWED_PATHS.some((allowed) => allowed.test(path)))).toEqual([]);
    expect(paths.filter((path) => /transcript|log|output|diff|artifact/i.test(path))).toEqual([]);
    expect(new Set(asked)).toEqual(new Set(Object.keys(journeyScript())));
  });

  it('keeps showing a gate decision until the instance accepts it, after the gate has left the mirror', async () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'Date'] });
    vi.setSystemTime(1_700_000_000_000);
    setCsrfToken('csrf-1');
    const queued = requestState(GATE_REQUEST, 'gate_decision', { status: 'queued' });
    stubJourney({
      ...journeyScript(),
      [`GET ${PATH}`]: [json(detail({ workflow: null })), json(detail({ workflow: null, pending_gates: [] }))],
      // Reads at 0, 1, 3 and 7 s: the instance answers after the 5 s run poll has dropped the gate.
      [`GET ${INSTANCE_PATH}/requests/${GATE_REQUEST}`]: [
        queued,
        queued,
        queued,
        requestState(GATE_REQUEST, 'gate_decision', { status: 'accepted' }),
      ],
    });
    const credentials = stubWebAuthn();
    render(<Run instanceId={INSTANCE} featureId={FEATURE} />);
    await landed();
    fireEvent.click(
      within(screen.getByRole('list', { name: 'Reported steps' })).getByRole('button', { name: /^approve/ }),
    );
    const inspector = screen.getByRole('region', { name: 'Step' });
    fireEvent.click(within(within(inspector).getByRole('region', { name: 'Gate' })).getByRole('button', { name: 'Approve' }));
    await vi.waitFor(() => expect(within(inspector).getByRole('status', { name: 'Request status' })).toBeInTheDocument());
    expect(credentials.get.mock.calls[0]?.[0]?.publicKey?.userVerification).toBe('required');

    await act(() => vi.advanceTimersByTimeAsync(5000));
    expect(within(inspector).queryByRole('region', { name: 'Gate' })).toBeNull();

    await act(() => vi.advanceTimersByTimeAsync(5000));
    const decision = within(inspector).getByRole('region', { name: 'Gate decision' });
    expect(within(decision).getByRole('status', { name: 'Request status' })).toHaveTextContent(
      'Accepted by the instance',
    );
    expect(asked.filter((request) => request === `GET ${INSTANCE_PATH}/requests/${GATE_REQUEST}`)).toHaveLength(4);
  });

  it('holds the gate actions while its decision is queued, and offers them again once it is refused', async () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'Date'] });
    vi.setSystemTime(1_700_000_000_000);
    setCsrfToken('csrf-1');
    stubJourney({
      ...journeyScript(),
      [`GET ${PATH}`]: [json(detail({ workflow: null }))],
      [`GET ${INSTANCE_PATH}/requests/${GATE_REQUEST}`]: [
        requestState(GATE_REQUEST, 'gate_decision', { status: 'queued' }),
        requestState(GATE_REQUEST, 'gate_decision', { status: 'refused', reason: REFUSAL }),
      ],
    });
    const credentials = stubWebAuthn();
    render(<Run instanceId={INSTANCE} featureId={FEATURE} />);
    await landed();
    fireEvent.click(
      within(screen.getByRole('list', { name: 'Reported steps' })).getByRole('button', { name: /^approve/ }),
    );
    const inspector = screen.getByRole('region', { name: 'Step' });
    const gate = within(inspector).getByRole('region', { name: 'Gate' });
    fireEvent.click(within(gate).getByRole('button', { name: 'Approve' }));
    await vi.waitFor(() =>
      expect(within(inspector).getByRole('status', { name: 'Request status' })).toHaveTextContent('Queued at the Hub'),
    );

    expect(within(gate).getByRole('button', { name: 'Approve' })).toBeDisabled();
    expect(within(gate).getByRole('button', { name: 'Abort feature' })).toBeDisabled();
    expect(within(gate).getByRole('group')).toHaveAccessibleDescription(/waiting for the instance/i);
    fireEvent.click(within(gate).getByRole('button', { name: 'Abort feature' }));
    await act(() => vi.advanceTimersByTimeAsync(0));
    expect(credentials.get).toHaveBeenCalledTimes(1);

    await act(() => vi.advanceTimersByTimeAsync(1000));
    const status = within(inspector).getByRole('status', { name: 'Request status' });
    expect(status).toHaveTextContent('Refused by the instance');
    expect(within(status).getByText(REFUSAL)).toBeInTheDocument();
    expect(within(gate).getByRole('button', { name: 'Approve' })).toBeEnabled();
    expect(within(gate).getByRole('button', { name: 'Abort feature' })).toBeEnabled();
  });

  it('offers the gate actions again once a decision runs out at expiry without an answer', async () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'Date'] });
    vi.setSystemTime(1_700_000_000_000);
    setCsrfToken('csrf-1');
    const expiresAt = 1_700_000_020;
    const queued = json({
      request_id: GATE_REQUEST,
      kind: 'gate_decision',
      deliveries: 0,
      expires_at: expiresAt,
      answered_at: null,
      status: 'queued',
    });
    stubJourney({
      ...journeyScript(),
      [`GET ${PATH}`]: [json(detail({ workflow: null }))],
      [`POST ${DECISION_PATH}`]: [json({ request_id: GATE_REQUEST, expires_at: expiresAt }, 202)],
      [`GET ${INSTANCE_PATH}/requests/${GATE_REQUEST}`]: [queued],
    });
    stubWebAuthn();
    render(<Run instanceId={INSTANCE} featureId={FEATURE} />);
    await landed();
    fireEvent.click(
      within(screen.getByRole('list', { name: 'Reported steps' })).getByRole('button', { name: /^approve/ }),
    );
    const inspector = screen.getByRole('region', { name: 'Step' });
    const gate = within(inspector).getByRole('region', { name: 'Gate' });
    fireEvent.click(within(gate).getByRole('button', { name: 'Approve' }));
    await vi.waitFor(() => expect(within(gate).getByRole('button', { name: 'Approve' })).toBeDisabled());

    await act(() => vi.advanceTimersByTimeAsync(30_000));

    expect(within(inspector).getByRole('status', { name: 'Request status' })).toHaveTextContent(
      'No answer from the instance before expiry',
    );
    expect(within(gate).getByRole('button', { name: 'Approve' })).toBeEnabled();
  });

  it("does not show an earlier decision's status against a fresh gate on the same step", async () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'Date'] });
    vi.setSystemTime(1_700_000_000_000);
    setCsrfToken('csrf-1');
    stubJourney({
      ...journeyScript(),
      [`GET ${PATH}`]: [
        json(detail({ workflow: null })),
        json(
          detail({
            workflow: null,
            pending_gates: [{ feature_id: FEATURE, step_execution_id: 'se-approve-2', step_id: 'approve', since: 1_700_000_005 }],
          }),
        ),
      ],
      [`GET ${INSTANCE_PATH}/requests/${GATE_REQUEST}`]: [
        requestState(GATE_REQUEST, 'gate_decision', { status: 'accepted' }),
      ],
    });
    stubWebAuthn();
    render(<Run instanceId={INSTANCE} featureId={FEATURE} />);
    await landed();
    fireEvent.click(
      within(screen.getByRole('list', { name: 'Reported steps' })).getByRole('button', { name: /^approve/ }),
    );
    const inspector = screen.getByRole('region', { name: 'Step' });
    fireEvent.click(within(within(inspector).getByRole('region', { name: 'Gate' })).getByRole('button', { name: 'Approve' }));
    await vi.waitFor(() => expect(within(inspector).getByRole('region', { name: 'Gate decision' })).toBeInTheDocument());

    await act(() => vi.advanceTimersByTimeAsync(5000));

    expect(field(inspector, 'Pending gate')).toHaveTextContent(/^se-approve-2$/);
    expect(within(inspector).queryByRole('region', { name: 'Gate decision' })).toBeNull();
  });

  it('asks nothing more once unmounted while the run and a request are both being polled', async () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'Date'] });
    vi.setSystemTime(1_700_000_000_000);
    stubJourney({
      ...journeyScript(),
      [`GET ${PATH}`]: [json(detail({ workflow: null }))],
      [`GET ${INSTANCE_PATH}/requests/${ASSIGN_REQUEST}`]: [
        requestState(ASSIGN_REQUEST, 'set_step_assignment', { status: 'delivered' }),
      ],
    });
    const { unmount } = render(<Run instanceId={INSTANCE} featureId={FEATURE} />);
    await landed();
    fireEvent.click(within(screen.getByRole('list', { name: 'Reported steps' })).getByRole('button', { name: /^plan/ }));
    const assignment = screen.getByRole('region', { name: 'Assignment' });
    await vi.waitFor(() => expect(within(assignment).getByRole('group')).toBeEnabled());
    fireEvent.change(within(assignment).getByLabelText('Agent'), { target: { value: 'codex' } });
    fireEvent.click(within(assignment).getByRole('button', { name: 'Change assignment' }));
    await vi.waitFor(() =>
      expect(within(assignment).getByRole('status', { name: 'Request status' })).toHaveTextContent(
        'Delivered, waiting for the instance',
      ),
    );
    await act(() => vi.advanceTimersByTimeAsync(1000));
    const polled = asked.length;
    expect(asked.filter((request) => request.startsWith(`GET ${INSTANCE_PATH}/requests/`)).length).toBe(2);

    unmount();
    await act(() => vi.advanceTimersByTimeAsync(60_000));

    expect(asked).toHaveLength(polled);
  });
});

/** The `dd` the inspector shows under the term `term`. */
function field(inspector: HTMLElement, term: string): Element | null {
  return within(inspector).getByText(term, { selector: 'dt' }).nextElementSibling;
}
