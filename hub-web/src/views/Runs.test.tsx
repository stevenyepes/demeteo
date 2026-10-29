import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { setCsrfToken } from '../lib/api';
import { FleetProvider } from '../lib/fleetContext';
import { routeHref } from '../lib/router';
import { Runs } from './Runs';

const WORKSTATION = '6f1c2b3a-0000-4000-8000-000000000001';
const LAPTOP = '6f1c2b3a-0000-4000-8000-000000000002';
const NOW = Math.floor(Date.now() / 1000);

afterEach(() => {
  vi.unstubAllGlobals();
  setCsrfToken(null);
});

/**
 * Answers `GET /v1/runs` with `runs` once and `GET /v1/fleet` with `fleet`,
 * a preview's run read from `details` (by feature id), and throws on anything else.
 */
function stubHub(runs: Response, fleet?: Response, details: Record<string, unknown> = {}) {
  const answers = new Map<string, Response | undefined>([
    ['GET /v1/runs', runs],
    ['GET /v1/fleet', fleet],
  ]);
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: unknown, init?: RequestInit): Promise<Response> => {
      const request = `${init?.method ?? 'GET'} ${String(input)}`;
      const detailRead = /^GET \/v1\/instances\/([^/]+)\/runs\/([^/]+)$/.exec(request);
      if (detailRead !== null) {
        const [, instance, feature] = detailRead.map(decodeURIComponent);
        return json(details[feature] ?? runDetail(instance, feature, { status: 'completed' }));
      }
      const answer = answers.get(request);
      if (answer === undefined) throw new Error(`fetch double was not told about ${request}`);
      answers.delete(request);
      return answer;
    }),
  );
}

function runDetail(instance_id: string, feature_id: string, overrides: Record<string, unknown> = {}) {
  return {
    instance_id,
    feature_id,
    status: 'running',
    workflow: null,
    title: null,
    project_id: null,
    project_name: null,
    steps: [],
    tickets: { completed: 0, failed: 0, running: 0 },
    pending_gates: [],
    events: [],
    ...overrides,
  };
}

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'Content-Type': 'application/json' },
  });
}

function run(instance_id: string, feature_id: string, overrides: Record<string, unknown> = {}) {
  return {
    instance_id,
    feature_id,
    status: 'running',
    started_at: NOW - 3600,
    updated_at: NOW - 60,
    cost_cents: null,
    title: null,
    project_id: null,
    project_name: null,
    step_id: null,
    step_title: null,
    tickets: { completed: 0, failed: 0, running: 0 },
    ...overrides,
  };
}

/** Deliberately out of `updated_at` order, and interleaving the two instances. */
const REPLY = {
  runs: [
    run(WORKSTATION, 'f-old', { status: 'completed', updated_at: NOW - 7200 }),
    run(LAPTOP, 'f-new', { updated_at: NOW - 30 }),
    run(WORKSTATION, 'f-mid', { status: 'failed', updated_at: NOW - 600 }),
  ],
};

function fleetInstance(id: string, label: string, needs_you: unknown[]) {
  return {
    id,
    label,
    present: true,
    os: 'linux',
    arch: 'x86_64',
    version: '1.4.2',
    scopes: ['read'],
    read_granted: true,
    last_seen: NOW,
    live_runs: 1,
    needs_you,
    harnesses: null,
    project_count: null,
  };
}

function hrefsIn(band: string): (string | null)[] {
  return within(screen.getByRole('list', { name: band }))
    .getAllByRole('link')
    .map((link) => link.getAttribute('href'));
}

function href(instanceId: string, featureId: string): string {
  return routeHref({ name: 'runs', instanceId, featureId });
}

describe('Runs', () => {
  it('shows a loading state under the Runs heading until the Hub answers', () => {
    stubHub(json(REPLY));
    render(<Runs />);

    expect(screen.getByRole('heading', { level: 1 })).toHaveTextContent(/^Runs$/);
    expect(screen.getByText(/loading/i)).toBeInTheDocument();
  });

  it('bands every run from every instance by status, newest update first, each linking to its run', async () => {
    stubHub(json(REPLY));
    render(<Runs />);

    await screen.findByRole('list', { name: 'Active' });

    expect(hrefsIn('Active')).toEqual([href(LAPTOP, 'f-new')]);
    expect(hrefsIn('Finished')).toEqual([href(WORKSTATION, 'f-mid'), href(WORKSTATION, 'f-old')]);
    expect(screen.queryByRole('list', { name: 'Needs you' })).toBeNull();
    const [failed] = within(screen.getByRole('list', { name: 'Finished' })).getAllByRole('link');
    expect(failed).toHaveTextContent('f-mid');
    expect(failed).toHaveTextContent(WORKSTATION);
    expect(failed).toHaveTextContent('Failed');
    expect(screen.getByText('3 runs · 1 active · 0 need you')).toBeInTheDocument();
  });

  it("leads each row with the run's title and names its project, step and tickets", async () => {
    stubHub(
      json({
        runs: [
          run(LAPTOP, 'f-new', {
            title: 'Index search by tenant',
            project_name: 'search-svc',
            step_title: 'Implement',
            tickets: { completed: 2, failed: 0, running: 1 },
          }),
        ],
      }),
    );
    render(<Runs />);

    const [row] = within(await screen.findByRole('list', { name: 'Active' })).getAllByRole('link');
    expect(row).toHaveTextContent('Index search by tenant');
    expect(row).not.toHaveTextContent('f-new');
    expect(row).toHaveTextContent(`${LAPTOP} · search-svc · Implement · 2 of 3 tickets done · 1h`);
  });

  it('puts a run with a waiting gate under Needs you, by its instance label and the gated step', async () => {
    const gate = {
      instance_id: LAPTOP,
      feature_id: 'f-new',
      step_id: 'review',
      step_execution_id: 'se-review',
      since: NOW - 840,
      title: null,
      project_name: null,
      step_title: 'Review gate',
    };
    stubHub(json(REPLY), json({ instances: [fleetInstance(LAPTOP, 'laptop', [gate])] }));
    render(
      <FleetProvider readKey="runs">
        <Runs />
      </FleetProvider>,
    );

    const needsYou = await screen.findByRole('list', { name: 'Needs you' });
    const [row] = within(needsYou).getAllByRole('link');
    expect(row).toHaveAttribute('href', href(LAPTOP, 'f-new'));
    expect(row).toHaveTextContent('Gate needs you');
    expect(row).toHaveTextContent('laptop · Review gate · waiting 14m');
    expect(screen.queryByRole('list', { name: 'Active' })).toBeNull();
  });

  it('says runs of instances without read are left out', async () => {
    stubHub(json(REPLY));
    render(<Runs />);

    await screen.findByRole('list', { name: 'Active' });

    expect(screen.getByText(/read off.*not listed/i)).toBeInTheDocument();
  });

  it('renders an empty reply as an empty state, not an error', async () => {
    stubHub(json({ runs: [] }));
    render(<Runs />);

    expect(await screen.findByText(/no runs reported/i)).toBeInTheDocument();
    expect(screen.queryByRole('alert')).toBeNull();
    expect(screen.queryByRole('list')).toBeNull();
    expect(screen.queryByRole('radiogroup')).toBeNull();
  });

  it('renders a refusal through CeremonyError', async () => {
    stubHub(json({ error: 'internal' }, 500));
    render(<Runs />);

    expect(await screen.findByRole('alert')).toHaveTextContent(/^internal$/);
    expect(screen.queryByRole('list')).toBeNull();
  });

  it('narrows the rows to the chosen band, and back to all', async () => {
    stubHub(json(REPLY));
    render(<Runs />);
    await screen.findByRole('list', { name: 'Active' });
    const filter = screen.getByRole('radiogroup', { name: 'Show' });

    await userEvent.click(within(filter).getByRole('radio', { name: 'Finished, 2' }));

    expect(within(filter).getByRole('radio', { name: 'Finished, 2' })).toBeChecked();
    expect(screen.queryByRole('list', { name: 'Active' })).toBeNull();
    expect(hrefsIn('Finished')).toHaveLength(2);

    await userEvent.click(within(filter).getByRole('radio', { name: /^Needs you/ }));

    expect(screen.queryByRole('list')).toBeNull();
    expect(screen.getByText('No run is waiting on you.')).toBeInTheDocument();

    await userEvent.click(within(filter).getByRole('radio', { name: 'All, 3' }));

    expect(hrefsIn('Active')).toHaveLength(1);
    expect(hrefsIn('Finished')).toHaveLength(2);
  });

  it("opens the first listed run's preview, or the one the URL names", async () => {
    stubHub(json(REPLY));
    const { unmount } = render(<Runs />);

    const first = within(await screen.findByRole('list', { name: 'Active' })).getByRole('link');
    expect(first).toHaveAttribute('aria-current', 'true');
    expect(await screen.findByRole('link', { name: 'Open the run graph' })).toHaveAttribute(
      'href',
      routeHref({ name: 'run', instanceId: LAPTOP, featureId: 'f-new' }),
    );
    unmount();

    stubHub(json(REPLY));
    render(<Runs instanceId={WORKSTATION} featureId="f-old" />);

    const [, old] = within(await screen.findByRole('list', { name: 'Finished' })).getAllByRole('link');
    expect(old).toHaveAttribute('aria-current', 'true');
    expect(within(screen.getByRole('list', { name: 'Active' })).getByRole('link')).not.toHaveAttribute(
      'aria-current',
    );
    expect(await screen.findByRole('link', { name: 'Open the run graph' })).toHaveAttribute(
      'href',
      routeHref({ name: 'run', instanceId: WORKSTATION, featureId: 'f-old' }),
    );
  });

  it('previews the pipeline by step title and offers the waiting gate beside the list', async () => {
    const gate = {
      instance_id: LAPTOP,
      feature_id: 'f-new',
      step_id: 'review',
      step_execution_id: 'se-review',
      since: NOW - 840,
      title: null,
      project_name: null,
      step_title: 'Review gate',
    };
    const laptop = { ...fleetInstance(LAPTOP, 'laptop', [gate]), scopes: ['read', 'gates'] };
    stubHub(json(REPLY), json({ instances: [laptop] }), {
      'f-new': runDetail(LAPTOP, 'f-new', {
        title: 'Index search by tenant',
        workflow: {
          schema_version: 2,
          id: 'w-standard',
          name: 'Standard pipeline',
          nodes: [
            { id: 'plan', type: 'agent', title: 'Plan' },
            { id: 'review', type: 'gate', title: 'Review gate' },
            { id: 'finalize', type: 'finalize', title: 'Finalize' },
          ],
          edges: [
            { from: 'plan', to: 'review' },
            { from: 'review', to: 'finalize' },
          ],
        },
        steps: [{ step_id: 'plan', state: 'completed', title: 'Plan' }],
        pending_gates: [
          { feature_id: 'f-new', step_execution_id: 'se-review', step_id: 'review', since: NOW - 840, step_title: 'Review gate' },
        ],
      }),
    });
    render(
      <FleetProvider readKey="runs">
        <Runs />
      </FleetProvider>,
    );

    const pipeline = await screen.findByRole('list', { name: 'Pipeline' });
    expect(within(pipeline).getAllByRole('listitem').map((item) => item.textContent)).toEqual([
      'Plan, Completed',
      'Review gate, Gate needs you',
      'Finalize',
    ]);
    expect(screen.getByRole('heading', { level: 2, name: 'Index search by tenant' })).toBeInTheDocument();
    expect(screen.getByText('Standard pipeline')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Approve' })).toBeEnabled();
  });
});
