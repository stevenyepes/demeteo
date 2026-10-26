import { act, render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { WorkflowCanvasProps } from '../../../src/components/canvas/WorkflowCanvas';
import { setCsrfToken } from '../lib/api';
import { Dispatch } from './Dispatch';

/** What each render handed the canvas; the real one lays out through an elk worker jsdom cannot run. */
const canvas = vi.hoisted(() => ({ renders: [] as Pick<WorkflowCanvasProps, 'definition' | 'highlightedNodeIds'>[] }));

vi.mock('../../../src/components/canvas/WorkflowCanvas', () => ({
  WorkflowCanvas: ({ definition, highlightedNodeIds, onNodeActivate }: WorkflowCanvasProps) => {
    canvas.renders.push({ definition, highlightedNodeIds });
    return (
      <div>
        {definition.nodes.map((node) => (
          <button key={node.id} type="button" onClick={() => onNodeActivate?.(node.id)}>
            {`Node ${node.id}`}
          </button>
        ))}
      </div>
    );
  },
}));

const ID = '6f1c2b3a-0000-4000-8000-000000000001';
const PATH = `/v1/instances/${ID}`;
const RUNS = `${PATH}/runs`;
const NOW = Math.floor(Date.now() / 1000);

type Scripted = { request: string; answer: () => Response | Promise<Response> };

/** Every request the double saw, scripted or not, in order. */
let asked: string[] = [];
/** The parsed JSON body of each `POST`, in order. */
let posted: unknown[] = [];
/** What each request was sent with, in the order of `asked`. */
let sent: (RequestInit | undefined)[] = [];

beforeEach(() => {
  setCsrfToken('csrf-1');
});

afterEach(() => {
  vi.unstubAllGlobals();
  setCsrfToken(null);
  asked = [];
  posted = [];
  sent = [];
  canvas.renders = [];
});

/** Answers each scripted `METHOD path` once, in any order, and throws on anything else. */
function stubFetch(script: Scripted[]) {
  const pending = [...script];
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: unknown, init?: RequestInit): Promise<Response> => {
      const request = `${init?.method ?? 'GET'} ${String(input)}`;
      asked.push(request);
      sent.push(init);
      if (init?.method === 'POST' && typeof init.body === 'string') posted.push(JSON.parse(init.body));
      const index = pending.findIndex((entry) => entry.request === request);
      if (index === -1) throw new Error(`fetch double was not told about ${request}`);
      const [entry] = pending.splice(index, 1);
      return entry.answer();
    }),
  );
}

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } });
}

function detail(overrides: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    id: ID,
    present: true,
    connected_at: NOW - 3600,
    last_seen: NOW - 120,
    install_id: 'install-9',
    os: 'linux',
    arch: 'x86_64',
    version: '1.4.2',
    scopes: ['read', 'spend', 'gates'],
    read_granted: true,
    projects: [
      { id: 'proj-a', name: 'demeteo', remote_url: null },
      { id: 'proj-b', name: 'notes', remote_url: null },
    ],
    live_runs: [],
    harnesses: null,
    workflows: [
      { workflow_id: 'wf-feature', version: 3, name: 'Feature', graph: null },
      { workflow_id: 'wf-hotfix', version: 1, name: 'Hotfix', graph: null },
    ],
    run_shape: [],
    ...overrides,
  };
}

function fleetRow(overrides: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    id: ID,
    label: 'laptop',
    present: true,
    os: 'linux',
    arch: 'x86_64',
    version: '1.4.2',
    scopes: ['read', 'spend'],
    read_granted: true,
    last_seen: NOW - 60,
    live_runs: 0,
    needs_you: [],
    ...overrides,
  };
}

const ENQUEUED = { request_id: 'req-1', expires_at: NOW + 900 };
const STATUS = `${PATH}/requests/${ENQUEUED.request_id}`;

function requestStatus(fields: Record<string, unknown>): Response {
  return json({
    request_id: ENQUEUED.request_id,
    kind: 'start_feature',
    deliveries: 1,
    expires_at: ENQUEUED.expires_at,
    answered_at: null,
    ...fields,
  });
}

async function renderLoaded(body: Record<string, unknown>, more: Scripted[] = []) {
  stubFetch([{ request: `GET ${PATH}`, answer: () => json(body) }, ...more]);
  render(<Dispatch instanceId={ID} />);
  await screen.findByRole('button', { name: /feature$/ });
}

function optionsOf(label: string): { value: string; text: string | null }[] {
  const select = screen.getByLabelText(label);
  return Array.from(select.querySelectorAll('option'), (option) => ({
    value: option.value,
    text: option.textContent,
  }));
}

async function fillValid() {
  await userEvent.type(screen.getByLabelText('Title'), 'Add dark mode');
  await userEvent.type(screen.getByLabelText('Description'), 'Make the app dark.');
  await userEvent.type(screen.getByLabelText('Max cost (USD)'), '12.50');
}

function posts(): string[] {
  return asked.filter((request) => request.startsWith('POST '));
}

describe('Dispatch', () => {
  it("offers exactly the instance's projects and workflows, and no free-text workflow", async () => {
    await renderLoaded(detail());

    expect(optionsOf('Project')).toEqual([
      { value: 'proj-a', text: 'demeteo' },
      { value: 'proj-b', text: 'notes' },
    ]);
    expect(optionsOf('Workflow')).toEqual([
      { value: 'wf-feature', text: 'Feature (v3)' },
      { value: 'wf-hotfix', text: 'Hotfix (v1)' },
    ]);
    expect(screen.getByLabelText('Workflow').tagName).toBe('SELECT');
    expect(screen.getByLabelText('Project').tagName).toBe('SELECT');
  });

  it('explains that read is off, with no select and no POST, when the lists are withheld', async () => {
    stubFetch([
      {
        request: `GET ${PATH}`,
        answer: () => json(detail({ scopes: ['spend'], read_granted: false, projects: null, workflows: null, run_shape: null })),
      },
    ]);
    render(<Dispatch instanceId={ID} />);

    expect(await screen.findByText(/read is off on this instance/i)).toBeVisible();
    expect(screen.queryByRole('combobox')).toBeNull();
    expect(screen.queryByRole('button')).toBeNull();
    expect(asked).toEqual([`GET ${PATH}`]);
  });

  it('says the instance has reported nothing yet when its snapshot is empty', async () => {
    stubFetch([{ request: `GET ${PATH}`, answer: () => json(detail({ projects: [], workflows: [] })) }]);
    render(<Dispatch instanceId={ID} />);

    expect(await screen.findByText(/reported no projects or workflows yet/i)).toBeVisible();
    expect(screen.queryByRole('combobox')).toBeNull();
  });

  it('blocks a submit with no max cost: zero POSTs, a field error and the ceiling help text', async () => {
    await renderLoaded(detail());
    await userEvent.type(screen.getByLabelText('Title'), 'Add dark mode');
    await userEvent.type(screen.getByLabelText('Description'), 'Make the app dark.');

    await userEvent.click(screen.getByRole('button', { name: 'Start feature' }));

    expect(posts()).toEqual([]);
    const cost = screen.getByLabelText('Max cost (USD)');
    expect(cost).toHaveAttribute('aria-invalid', 'true');
    expect(cost).toHaveAccessibleDescription(/a max cost is required/i);
    expect(cost).toHaveAccessibleDescription(/no cap, or with a cap above its own local ceiling/);
    expect(screen.getByText(/^A max cost is required/)).toBeVisible();
  });

  it('disables the submit, says spend has not been granted, and posts nothing without spend', async () => {
    await renderLoaded(detail({ scopes: ['read', 'gates'] }));
    await fillValid();

    const submit = screen.getByRole('button', { name: 'Start feature' });
    expect(submit).toBeDisabled();
    expect(screen.getByText(/spend has not been granted on this instance/i)).toBeVisible();
    await userEvent.click(submit);
    expect(posts()).toEqual([]);
  });

  it('posts exactly the DispatchBody keys, then shows the queued status', async () => {
    await renderLoaded(detail(), [{ request: `POST ${RUNS}`, answer: () => json(ENQUEUED, 202) }]);
    await userEvent.selectOptions(screen.getByLabelText('Project'), 'proj-b');
    await userEvent.selectOptions(screen.getByLabelText('Workflow'), 'wf-hotfix');
    await fillValid();
    await userEvent.selectOptions(screen.getByLabelText('Default agent'), 'codex');
    await userEvent.type(screen.getByLabelText('Model'), 'gpt-5');
    await userEvent.selectOptions(screen.getByLabelText('Effort'), 'low');
    await userEvent.type(screen.getByLabelText('Max wall-clock (min)'), '90');

    await userEvent.click(screen.getByRole('button', { name: 'Start feature' }));

    expect(await screen.findByRole('status')).toHaveTextContent(/queued/i);
    expect(posts()).toEqual([`POST ${RUNS}`]);
    expect(posted).toEqual([
      {
        project_id: 'proj-b',
        workflow_id: 'wf-hotfix',
        title: 'Add dark mode',
        description: 'Make the app dark.',
        agent_kind: 'codex',
        model: 'gpt-5',
        effort: 'low',
        step_overrides: [],
        max_budget_cents: 1250,
        max_wall_clock_secs: 5400,
        attachment_ids: [],
      },
    ]);
  });

  it('leaves unset optional fields out of the body', async () => {
    await renderLoaded(detail(), [{ request: `POST ${RUNS}`, answer: () => json(ENQUEUED, 202) }]);
    await fillValid();

    await userEvent.click(screen.getByRole('button', { name: 'Start feature' }));

    await screen.findByRole('status');
    expect(posted).toEqual([
      {
        project_id: 'proj-a',
        workflow_id: 'wf-feature',
        title: 'Add dark mode',
        description: 'Make the app dark.',
        step_overrides: [],
        max_budget_cents: 1250,
        attachment_ids: [],
      },
    ]);
  });

  it('changing the agent clears the model and drops an effort it cannot run', async () => {
    await renderLoaded(detail());
    await userEvent.selectOptions(screen.getByLabelText('Default agent'), 'codex');
    await userEvent.type(screen.getByLabelText('Model'), 'gpt-5');
    await userEvent.selectOptions(screen.getByLabelText('Effort'), 'low');

    await userEvent.selectOptions(screen.getByLabelText('Default agent'), 'hermes');

    expect(screen.getByLabelText('Model')).toHaveValue('');
    expect(screen.getByLabelText('Effort')).toHaveValue('');
    expect(screen.getByLabelText('Effort')).toBeDisabled();
  });

  it('sends one POST for a double click while the dispatch is in flight', async () => {
    let release: () => void = () => {};
    const held = new Promise<Response>((resolve) => {
      release = () => resolve(json(ENQUEUED, 202));
    });
    await renderLoaded(detail(), [
      { request: `POST ${RUNS}`, answer: () => held },
      { request: `POST ${RUNS}`, answer: () => json(ENQUEUED, 202) },
    ]);
    await fillValid();

    await userEvent.dblClick(screen.getByRole('button', { name: 'Start feature' }));

    expect(screen.getByRole('button', { name: 'Start feature' })).toBeDisabled();
    expect(posts()).toEqual([`POST ${RUNS}`]);
    release();
    await screen.findByRole('status');
    expect(posts()).toEqual([`POST ${RUNS}`]);
  });

  it('shows a refusal of the dispatch as its sentence, and keeps the form', async () => {
    await renderLoaded(detail(), [
      {
        request: `POST ${RUNS}`,
        answer: () => json({ error: 'scope_not_granted', scope: 'spend' }, 409),
      },
    ]);
    await fillValid();

    await userEvent.click(screen.getByRole('button', { name: 'Start feature' }));

    expect(await screen.findByRole('alert')).toHaveTextContent(/^Spend is off on this instance\.$/);
    expect(screen.getByRole('button', { name: 'Start feature' })).toBeEnabled();
  });

  it('shows a refusal with no sentence as its code', async () => {
    await renderLoaded(detail(), [
      { request: `POST ${RUNS}`, answer: () => json({ error: 'rate_limited' }, 429) },
    ]);
    await fillValid();

    await userEvent.click(screen.getByRole('button', { name: 'Start feature' }));

    expect(await screen.findByRole('alert')).toHaveTextContent(/^rate_limited$/);
  });

  it('without an instance id, lists the fleet and loads the chosen instance', async () => {
    stubFetch([
      { request: 'GET /v1/fleet', answer: () => json({ instances: [fleetRow()] }) },
      { request: `GET ${PATH}`, answer: () => json(detail()) },
    ]);
    render(<Dispatch />);

    const list = await screen.findByRole('region', { name: 'Where to run' });
    await userEvent.click(within(list).getByRole('button', { name: 'laptop' }));

    expect(await screen.findByLabelText('Project')).toBeVisible();
    expect(asked).toEqual(['GET /v1/fleet', `GET ${PATH}`]);
  });

  it('with an instance id, loads that instance directly and lists no fleet to choose from', async () => {
    await renderLoaded(detail());

    expect(screen.queryByRole('region', { name: 'Where to run' })).toBeNull();
    expect(screen.getByLabelText('Project')).toHaveValue('proj-a');
    expect(asked).toEqual([`GET ${PATH}`]);
  });
});

function runShape(fields: Record<string, unknown>): Record<string, unknown> {
  return {
    project_id: 'proj-b',
    default_agent_kind: 'codex',
    default_model: 'gpt-5',
    default_effort: 'low',
    default_workflow_id: 'wf-hotfix',
    default_max_budget_cents: 1205,
    ...fields,
  };
}

function prefilled(): Record<string, unknown> {
  return {
    agent: (screen.getByLabelText('Default agent') as HTMLSelectElement).value,
    model: (screen.getByLabelText('Model') as HTMLInputElement).value,
    effort: (screen.getByLabelText('Effort') as HTMLSelectElement).value,
    workflow: (screen.getByLabelText('Workflow') as HTMLSelectElement).value,
    maxCost: (screen.getByLabelText('Max cost (USD)') as HTMLInputElement).value,
  };
}

const BLANK = { agent: '', model: '', effort: '', workflow: 'wf-feature', maxCost: '' };

describe('Dispatch run-shape defaults', () => {
  it("pre-fills agent, model, effort, workflow and max cost from the chosen project's row", async () => {
    await renderLoaded(detail({ run_shape: [runShape({})] }), [
      { request: `POST ${RUNS}`, answer: () => json(ENQUEUED, 202) },
    ]);
    expect(prefilled()).toEqual(BLANK);

    await userEvent.selectOptions(screen.getByLabelText('Project'), 'proj-b');

    expect(prefilled()).toEqual({ agent: 'codex', model: 'gpt-5', effort: 'low', workflow: 'wf-hotfix', maxCost: '12.05' });
    await userEvent.type(screen.getByLabelText('Title'), 'Add dark mode');
    await userEvent.click(screen.getByRole('button', { name: 'Start feature' }));
    await screen.findByRole('status');
    expect(posted[0]).toMatchObject({ project_id: 'proj-b', workflow_id: 'wf-hotfix', max_budget_cents: 1205 });
  });

  it('pre-fills the project selected on load', async () => {
    await renderLoaded(detail({ run_shape: [runShape({ project_id: 'proj-a', default_max_budget_cents: 5 })] }));

    expect(prefilled()).toEqual({ agent: 'codex', model: 'gpt-5', effort: 'low', workflow: 'wf-hotfix', maxCost: '0.05' });
  });

  it('does not select a default workflow the instance did not report', async () => {
    await renderLoaded(detail({ run_shape: [runShape({ default_workflow_id: 'wf-gone' })] }), [
      { request: `POST ${RUNS}`, answer: () => json(ENQUEUED, 202) },
    ]);

    await userEvent.selectOptions(screen.getByLabelText('Project'), 'proj-b');

    expect(prefilled()).toEqual({ agent: 'codex', model: 'gpt-5', effort: 'low', workflow: 'wf-feature', maxCost: '12.05' });
    // A select whose value matches no option still shows its first one, so only the body tells.
    await userEvent.type(screen.getByLabelText('Title'), 'Add dark mode');
    await userEvent.click(screen.getByRole('button', { name: 'Start feature' }));
    await screen.findByRole('status');
    expect(posted[0]).toMatchObject({ workflow_id: 'wf-feature' });
  });

  it('leaves a field the user already edited as they left it', async () => {
    await renderLoaded(detail({ run_shape: [runShape({})] }));
    await userEvent.selectOptions(screen.getByLabelText('Default agent'), 'claude-code');
    await userEvent.type(screen.getByLabelText('Max cost (USD)'), '3');

    await userEvent.selectOptions(screen.getByLabelText('Project'), 'proj-b');

    // The default model belongs to the default agent, which the user replaced.
    expect(prefilled()).toEqual({ agent: 'claude-code', model: '', effort: 'low', workflow: 'wf-hotfix', maxCost: '3' });
  });

  it('blanks the pre-filled fields for a project with no row, leaving max cost required', async () => {
    await renderLoaded(detail({ run_shape: [runShape({ project_id: 'proj-a' })] }));

    await userEvent.selectOptions(screen.getByLabelText('Project'), 'proj-b');
    expect(prefilled()).toEqual(BLANK);
    await userEvent.type(screen.getByLabelText('Title'), 'Add dark mode');
    await userEvent.click(screen.getByRole('button', { name: 'Start feature' }));

    expect(posts()).toEqual([]);
    expect(screen.getByLabelText('Max cost (USD)')).toHaveAccessibleDescription(/a max cost is required/i);
  });
});

const GRAPH = {
  nodes: [
    { id: 'plan', type: 'agent', title: 'Plan', position: { x: 0, y: 0 } },
    { id: 'build', type: 'agent', title: 'Build', position: null },
    { id: 'review', type: 'gate', title: 'Review', position: null },
    { id: 'ship', type: 'finalize', title: 'Ship', position: null },
  ],
  edges: [
    { from: 'plan', to: 'build' },
    { from: 'build', to: 'review' },
    { from: 'review', to: 'ship' },
  ],
};

function withGraph(): Record<string, unknown> {
  return detail({
    workflows: [
      { workflow_id: 'wf-feature', version: 3, name: 'Feature', graph: GRAPH },
      { workflow_id: 'wf-hotfix', version: 1, name: 'Hotfix', graph: null },
    ],
  });
}

async function activate(nodeId: string) {
  await userEvent.click(screen.getByRole('button', { name: `Node ${nodeId}` }));
  return screen.getByRole('region', { name: 'Step override' });
}

describe('Dispatch per-step overrides', () => {
  it("draws the canvas from the instance's graph, one definition for every render", async () => {
    await renderLoaded(withGraph());
    const panel = await activate('plan');
    await userEvent.type(within(panel).getByLabelText('Model'), 'gpt-5');

    const last = canvas.renders[canvas.renders.length - 1];
    expect(last?.definition).toEqual({
      schema_version: 2,
      id: 'wf-feature',
      name: 'Feature',
      nodes: GRAPH.nodes,
      edges: GRAPH.edges,
    });
    expect(new Set(canvas.renders.map((render) => render.definition)).size).toBe(1);
    expect(last?.highlightedNodeIds).toEqual(new Set(['plan']));
  });

  it('sends the step pinned through the canvas, and nothing for a step reset to inherit', async () => {
    await renderLoaded(withGraph(), [{ request: `POST ${RUNS}`, answer: () => json(ENQUEUED, 202) }]);
    await fillValid();
    const plan = await activate('plan');
    await userEvent.selectOptions(within(plan).getByLabelText('Agent'), 'codex');
    await userEvent.type(within(plan).getByLabelText('Model'), 'gpt-5');
    await userEvent.selectOptions(within(plan).getByLabelText('Effort'), 'high');
    const build = await activate('build');
    await userEvent.type(within(build).getByLabelText('Model'), 'sonnet');
    await userEvent.click(within(build).getByRole('button', { name: 'Inherit all' }));

    await userEvent.click(screen.getByRole('button', { name: 'Start feature' }));

    await screen.findByRole('status');
    expect(posted).toHaveLength(1);
    expect(posted[0]).toMatchObject({
      workflow_id: 'wf-feature',
      step_overrides: [{ step_id: 'plan', agent_kind: 'codex', model: 'gpt-5', effort: 'high' }],
    });
  });

  it.each(['review', 'ship'])('offers no override controls on the %s step', async (nodeId) => {
    await renderLoaded(withGraph());

    const panel = await activate(nodeId);

    expect(within(panel).queryByRole('combobox')).toBeNull();
    expect(within(panel).queryByRole('textbox')).toBeNull();
    expect(within(panel).getByText(/runs no agent/)).toBeVisible();
  });

  it('says overrides are unavailable for a workflow with no graph, and draws nothing', async () => {
    await renderLoaded(detail());

    expect(screen.getByText(/per-step overrides are unavailable for this workflow/i)).toBeVisible();
    expect(canvas.renders).toEqual([]);
    expect(screen.queryByRole('region', { name: 'Step override' })).toBeNull();
  });

  it('drops the overrides when the workflow changes', async () => {
    await renderLoaded(withGraph(), [{ request: `POST ${RUNS}`, answer: () => json(ENQUEUED, 202) }]);
    await fillValid();
    const plan = await activate('plan');
    await userEvent.type(within(plan).getByLabelText('Model'), 'gpt-5');

    await userEvent.selectOptions(screen.getByLabelText('Workflow'), 'wf-hotfix');
    await userEvent.selectOptions(screen.getByLabelText('Workflow'), 'wf-feature');
    expect(screen.queryByRole('region', { name: 'Step override' })).toBeNull();
    await userEvent.click(screen.getByRole('button', { name: 'Start feature' }));

    await screen.findByRole('status');
    expect(posted[0]).toMatchObject({ workflow_id: 'wf-feature', step_overrides: [] });
  });
});

describe('Dispatch after the 202', () => {
  /** `RequestStatusCard` first polls 2 s after it mounts; the clock still runs so `findBy*` can wait. */
  beforeEach(() => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  async function dispatchThenPoll(body: Record<string, unknown>, status: Record<string, unknown>, submit: string) {
    await renderLoaded(body, [
      { request: `POST ${RUNS}`, answer: () => json(ENQUEUED, 202) },
      { request: `GET ${STATUS}`, answer: () => requestStatus(status) },
    ]);
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    await user.type(screen.getByLabelText('Title'), 'Add dark mode');
    await user.type(screen.getByLabelText('Max cost (USD)'), '12.50');
    expect(screen.getAllByRole('button', { name: /feature$/ }).map((button) => button.textContent)).toEqual([submit]);
    expect(screen.getByRole('button', { name: submit })).toBeEnabled();
    await user.click(screen.getByRole('button', { name: submit }));
    await screen.findByRole('status');
    await act(() => vi.advanceTimersByTimeAsync(2_000));
    expect(asked).toEqual([`GET ${PATH}`, `POST ${RUNS}`, `GET ${STATUS}`]);
    return screen.getByRole('status');
  }

  it('shows an over_run_ceiling refusal in ruby, with the per-run-ceiling sentence and the raw code', async () => {
    const card = await dispatchThenPoll(
      detail(),
      { status: 'refused', reason: 'over_run_ceiling', answered_at: NOW + 5 },
      'Start feature',
    );

    expect(card).toHaveClass('text-ruby-300', 'border-ruby-500/30', 'bg-ruby-500/10');
    expect(card).toHaveTextContent(/Refused by the instance/);
    expect(within(card).getByText("The max cost is above this instance's per-run ceiling.")).toBeVisible();
    expect(within(card).getByText('over_run_ceiling')).toHaveClass('font-mono');
  });

  it('shows a scope_off refusal as spend being off on the instance', async () => {
    const card = await dispatchThenPoll(detail(), { status: 'refused', reason: 'scope_off' }, 'Start feature');

    expect(card).toHaveClass('text-ruby-300');
    expect(within(card).getByText(/^Spend is off on this instance/)).toBeVisible();
    expect(within(card).getByText('scope_off')).toBeVisible();
  });

  it('shows a reason it has no sentence for as the raw code alone', async () => {
    const card = await dispatchThenPoll(detail(), { status: 'refused', reason: 'brand_new_reason' }, 'Start feature');

    expect(card).toHaveTextContent(/^Refused by the instancebrand_new_reason$/);
  });

  it('queues to an offline instance with spend, then says it waits for the reconnect until the answered expiry', async () => {
    const card = await dispatchThenPoll(detail({ present: false }), { status: 'queued', deliveries: 0 }, 'Queue feature');

    expect(card).toHaveTextContent(/Queued until the instance reconnects/);
    const expiry = within(card).getByText(new Date(ENQUEUED.expires_at * 1000).toLocaleTimeString());
    expect(expiry).toHaveAttribute('datetime', new Date(ENQUEUED.expires_at * 1000).toISOString());
  });

  it('shows a 409 scope_not_granted on queueing as spend being off, not as the code', async () => {
    await renderLoaded(detail({ present: false }), [
      { request: `POST ${RUNS}`, answer: () => json({ error: 'scope_not_granted', scope: 'spend' }, 409) },
    ]);
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    await user.type(screen.getByLabelText('Title'), 'Add dark mode');
    await user.type(screen.getByLabelText('Max cost (USD)'), '12.50');

    await user.click(screen.getByRole('button', { name: 'Queue feature' }));

    expect(await screen.findByRole('alert')).toHaveTextContent(/^Spend is off on this instance\.$/);
    expect(screen.queryByText(/scope_not_granted/)).toBeNull();
    expect(screen.queryByRole('status')).toBeNull();
  });
});

const UPLOADS = `${PATH}/attachments`;

function staged(attachmentId: string, size: number): Response {
  return json({ attachment_id: attachmentId, size, sha256: 'ab'.repeat(32), expires_at: NOW + 3600 }, 201);
}

const notes = () => new File(['first file'], 'notes.txt', { type: 'text/plain' });
const shot = () => new File([new Uint8Array([137, 80, 78, 71])], 'shot.png', { type: 'image/png' });

async function attach(...files: File[]) {
  await userEvent.upload(screen.getByLabelText('Add files'), files);
}

describe('Dispatch with attachments', () => {
  it('uploads each file as raw bytes, in order, then dispatches once with the answered ids', async () => {
    await renderLoaded(detail(), [
      { request: `POST ${UPLOADS}`, answer: () => staged('att-1', 10) },
      { request: `POST ${UPLOADS}`, answer: () => staged('att-2', 4) },
      { request: `POST ${RUNS}`, answer: () => json(ENQUEUED, 202) },
    ]);
    await fillValid();
    const files = [notes(), shot()];
    await attach(...files);

    await userEvent.click(screen.getByRole('button', { name: 'Start feature' }));

    await screen.findByRole('status');
    expect(posts()).toEqual([`POST ${UPLOADS}`, `POST ${UPLOADS}`, `POST ${RUNS}`]);
    const uploads = sent.filter((_, index) => asked[index] === `POST ${UPLOADS}`);
    const bodies = uploads.map((init) => init?.body);
    expect(bodies).toHaveLength(2);
    for (const [index, body] of bodies.entries()) expect(body).toBe(files[index]);
    const bytes = await Promise.all(
      bodies.map(async (body) => (body instanceof Blob ? Array.from(new Uint8Array(await body.arrayBuffer())) : null)),
    );
    expect(bytes).toEqual([Array.from(new TextEncoder().encode('first file')), [137, 80, 78, 71]]);
    for (const init of uploads) expect(init?.headers).toMatchObject({ 'X-CSRF-Token': 'csrf-1' });
    expect(posted).toEqual([expect.objectContaining({ attachment_ids: ['att-1', 'att-2'] })]);
  });

  it('dispatches nothing when an upload is over the size limit, and says so', async () => {
    await renderLoaded(detail(), [
      { request: `POST ${UPLOADS}`, answer: () => staged('att-1', 10) },
      { request: `POST ${UPLOADS}`, answer: () => json({ error: 'attachment_too_large' }, 413) },
    ]);
    await fillValid();
    await attach(notes(), shot());

    await userEvent.click(screen.getByRole('button', { name: 'Start feature' }));

    expect(await screen.findByRole('alert')).toHaveTextContent(/size limit/);
    expect(posts()).toEqual([`POST ${UPLOADS}`, `POST ${UPLOADS}`]);
    expect(screen.getByRole('button', { name: 'Start feature' })).toBeEnabled();
  });

  it('shows a withheld spend scope on an upload as spend being off, and dispatches nothing', async () => {
    await renderLoaded(detail(), [
      { request: `POST ${UPLOADS}`, answer: () => json({ error: 'scope_not_granted', scope: 'spend' }, 409) },
    ]);
    await fillValid();
    await attach(notes());

    await userEvent.click(screen.getByRole('button', { name: 'Start feature' }));

    expect(await screen.findByRole('alert')).toHaveTextContent(/^Spend is off on this instance\.$/);
    expect(posts()).toEqual([`POST ${UPLOADS}`]);
  });

  it('re-uses the staged ids when a dispatch is retried after it failed', async () => {
    await renderLoaded(detail(), [
      { request: `POST ${UPLOADS}`, answer: () => staged('att-1', 10) },
      { request: `POST ${UPLOADS}`, answer: () => staged('att-2', 4) },
      { request: `POST ${RUNS}`, answer: () => json({ error: 'key_service_unavailable' }, 503) },
      { request: `POST ${RUNS}`, answer: () => json(ENQUEUED, 202) },
    ]);
    await fillValid();
    await attach(notes(), shot());
    await userEvent.click(screen.getByRole('button', { name: 'Start feature' }));
    await screen.findByRole('alert');

    await userEvent.click(screen.getByRole('button', { name: 'Start feature' }));

    await screen.findByRole('status');
    expect(posts()).toEqual([`POST ${UPLOADS}`, `POST ${UPLOADS}`, `POST ${RUNS}`, `POST ${RUNS}`]);
    expect(posted).toEqual([
      expect.objectContaining({ attachment_ids: ['att-1', 'att-2'] }),
      expect.objectContaining({ attachment_ids: ['att-1', 'att-2'] }),
    ]);
  });

  it('re-uploads after the dispatch answers attachment_unavailable', async () => {
    await renderLoaded(detail(), [
      { request: `POST ${UPLOADS}`, answer: () => staged('att-1', 10) },
      { request: `POST ${RUNS}`, answer: () => json({ error: 'attachment_unavailable' }, 409) },
      { request: `POST ${UPLOADS}`, answer: () => staged('att-2', 10) },
      { request: `POST ${RUNS}`, answer: () => json(ENQUEUED, 202) },
    ]);
    await fillValid();
    await attach(notes());
    await userEvent.click(screen.getByRole('button', { name: 'Start feature' }));
    await screen.findByRole('alert');

    await userEvent.click(screen.getByRole('button', { name: 'Start feature' }));

    await screen.findByRole('status');
    expect(posts()).toEqual([`POST ${UPLOADS}`, `POST ${RUNS}`, `POST ${UPLOADS}`, `POST ${RUNS}`]);
    expect(posted).toEqual([
      expect.objectContaining({ attachment_ids: ['att-1'] }),
      expect.objectContaining({ attachment_ids: ['att-2'] }),
    ]);
  });

  it('drops the staged id of a removed file, so adding it back uploads it again', async () => {
    await renderLoaded(detail(), [
      { request: `POST ${UPLOADS}`, answer: () => staged('att-1', 10) },
      { request: `POST ${UPLOADS}`, answer: () => staged('att-2', 4) },
      { request: `POST ${RUNS}`, answer: () => json({ error: 'key_service_unavailable' }, 503) },
      { request: `POST ${UPLOADS}`, answer: () => staged('att-3', 10) },
      { request: `POST ${RUNS}`, answer: () => json(ENQUEUED, 202) },
    ]);
    await fillValid();
    const first = notes();
    await attach(first, shot());
    await userEvent.click(screen.getByRole('button', { name: 'Start feature' }));
    await screen.findByRole('alert');

    await userEvent.click(screen.getByRole('button', { name: 'Remove notes.txt' }));
    await attach(first);
    await userEvent.click(screen.getByRole('button', { name: 'Start feature' }));

    await screen.findByRole('status');
    expect(posts()).toEqual([`POST ${UPLOADS}`, `POST ${UPLOADS}`, `POST ${RUNS}`, `POST ${UPLOADS}`, `POST ${RUNS}`]);
    expect(posted[1]).toMatchObject({ attachment_ids: ['att-2', 'att-3'] });
  });

  it('holds the submit and the picker still while the uploads are in flight', async () => {
    let release: () => void = () => {};
    const held = new Promise<Response>((resolve) => {
      release = () => resolve(staged('att-1', 10));
    });
    await renderLoaded(detail(), [
      { request: `POST ${UPLOADS}`, answer: () => held },
      { request: `POST ${RUNS}`, answer: () => json(ENQUEUED, 202) },
    ]);
    await fillValid();
    await attach(notes());

    await userEvent.dblClick(screen.getByRole('button', { name: 'Start feature' }));

    expect(screen.getByRole('button', { name: 'Start feature' })).toBeDisabled();
    expect(screen.getByLabelText('Add files')).toBeDisabled();
    expect(screen.getByRole('button', { name: 'Remove notes.txt' })).toBeDisabled();
    expect(posts()).toEqual([`POST ${UPLOADS}`]);
    release();
    await screen.findByRole('status');
    expect(posts()).toEqual([`POST ${UPLOADS}`, `POST ${RUNS}`]);
  });
});
