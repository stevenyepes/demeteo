import { render, screen, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { setCsrfToken } from '../lib/api';
import { FleetProvider } from '../lib/fleetContext';
import { routeHref } from '../lib/router';
import { Instance } from './Instance';

const ID = '6f1c2b3a-0000-4000-8000-000000000001';
const PATH = `/v1/instances/${ID}`;
const NOW = Math.floor(Date.now() / 1000);

/** Every request the double saw, scripted or not: the `'..'` test is held to there being none. */
let asked: string[] = [];

afterEach(() => {
  vi.unstubAllGlobals();
  setCsrfToken(null);
  asked = [];
});

/** Answers `GET path` with `answer` once, and throws on anything else. */
function stubFetch(path: string, answer: Response) {
  let answered = false;
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: unknown, init?: RequestInit): Promise<Response> => {
      const request = `${init?.method ?? 'GET'} ${String(input)}`;
      asked.push(request);
      if (answered || request !== `GET ${path}`) throw new Error(`fetch double was not told about ${request}`);
      answered = true;
      return answer;
    }),
  );
}

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'Content-Type': 'application/json' },
  });
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
    scopes: ['read', 'gates'],
    read_granted: true,
    projects: [],
    live_runs: [],
    harnesses: null,
    workflows: [],
    run_shape: [],
    ...overrides,
  };
}

function run(feature_id: string, overrides: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    instance_id: ID,
    feature_id,
    status: 'running',
    started_at: NOW - 600,
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

async function renderLoaded(body: Record<string, unknown>) {
  stubFetch(PATH, json(body));
  const rendered = render(<Instance instanceId={ID} />);
  await screen.findByRole('link', { name: 'Run here' });
  return rendered;
}

describe('Instance', () => {
  it('shows the id in monospace while the instance loads', () => {
    stubFetch(PATH, json(detail()));
    const { container } = render(<Instance instanceId={ID} />);

    expect(screen.getByRole('heading', { level: 1 })).toHaveTextContent(/^Instance$/);
    expect(Array.from(container.querySelectorAll('.font-mono'), (element) => element.textContent)).toEqual([ID]);
    expect(screen.getByText(/loading/i)).toBeInTheDocument();
  });

  it('shows projects by name and remote as text, and never a local path or a remote href', async () => {
    const { container } = await renderLoaded(
      detail({
        projects: [
          { id: 'p-1', name: 'demeteo', remote_url: 'https://github.com/acme/demeteo.git', path: '/home/u/secret' },
          { id: 'p-2', name: 'notes', remote_url: 'git@github.com:acme/notes.git', path: '/home/u/secret' },
        ],
      }),
    );

    const projects = screen.getByRole('region', { name: 'Projects' });
    for (const text of ['demeteo', 'https://github.com/acme/demeteo.git', 'notes', 'git@github.com:acme/notes.git']) {
      expect(within(projects).getByText(text)).toBeInTheDocument();
    }
    expect(projects).toHaveTextContent(/local paths stay on the machine/i);
    expect(container.textContent).not.toContain('/home/u/secret');
    expect(container.innerHTML).not.toContain('/home/u/secret');
    const hrefs = Array.from(container.querySelectorAll('a'), (link) => link.getAttribute('href'));
    expect(hrefs).not.toContain('https://github.com/acme/demeteo.git');
    expect(hrefs).not.toContain('git@github.com:acme/notes.git');
  });

  it('redacts userinfo from a remote URL', async () => {
    const { container } = await renderLoaded(
      detail({ projects: [{ id: 'p-1', name: 'demeteo', remote_url: 'https://user:tok@github.com/acme/demeteo.git' }] }),
    );

    expect(screen.getByText('https://github.com/acme/demeteo.git')).toHaveClass('font-mono');
    expect(container.textContent).not.toContain('tok');
  });

  it('renders no cost when none is reported, and says harnesses were not reported rather than none', async () => {
    const { container } = await renderLoaded(detail({ live_runs: [run('feat-1', { cost_cents: null })] }));

    expect(within(screen.getByRole('region', { name: 'Running now' })).getByText('feat-1')).toBeInTheDocument();
    expect(container.textContent).not.toContain('$');
    expect(screen.getByRole('region', { name: 'Harnesses' })).toHaveTextContent(/has not reported its harnesses/);
  });

  it('lists the harness kinds the instance reports', async () => {
    await renderLoaded(detail({ harnesses: ['claude-code', 'opencode'] }));

    const harnesses = screen.getByRole('region', { name: 'Harnesses' });
    expect(within(harnesses).getAllByRole('listitem').map((item) => item.textContent)).toEqual([
      'claude-code',
      'opencode',
    ]);
  });

  it('leads a run with its title, then its project, step and tickets', async () => {
    await renderLoaded(
      detail({
        live_runs: [
          run('feat-1', {
            title: 'Retry failed webhooks',
            project_name: 'checkout-api',
            step_id: 's-implement',
            step_title: 'Implement',
            tickets: { completed: 2, failed: 0, running: 1 },
          }),
        ],
      }),
    );

    const link = within(screen.getByRole('region', { name: 'Running now' })).getByRole('link');
    expect(within(link).getByText('Retry failed webhooks')).toHaveClass('font-heading');
    expect(link).toHaveTextContent('feat-1');
    expect(link).toHaveTextContent('checkout-api');
    expect(link).toHaveTextContent('Implement · 2 tickets landed, 1 running');
    expect(link).not.toHaveTextContent('s-implement');
  });

  it('shows the default workflow of each project by name and newest version', async () => {
    await renderLoaded(
      detail({
        projects: [
          { id: 'p-1', name: 'demeteo', remote_url: null },
          { id: 'p-2', name: 'notes', remote_url: null },
        ],
        workflows: [
          { workflow_id: 'w-1', version: 6, name: 'feature-standard', graph: null },
          { workflow_id: 'w-1', version: 7, name: 'feature-standard', graph: null },
        ],
        run_shape: [
          {
            project_id: 'p-1',
            default_agent_kind: null,
            default_model: null,
            default_effort: null,
            default_workflow_id: 'w-1',
            default_max_budget_cents: null,
          },
        ],
      }),
    );

    const rows = within(screen.getByRole('region', { name: 'Projects' })).getAllByRole('row');
    expect(rows[1]).toHaveTextContent('feature-standard v7');
    expect(rows[2]).toHaveTextContent('None set');
  });

  it('shows the cost of a run only when it is reported', async () => {
    await renderLoaded(detail({ live_runs: [run('feat-1', { cost_cents: 1234 })] }));

    expect(screen.getByRole('region', { name: 'Running now' })).toHaveTextContent('$12.34');
  });

  it('links each live run to its Run view, with its status and times', async () => {
    await renderLoaded(detail({ live_runs: [run('feat-1'), run('feat-2', { status: 'failed', started_at: null })] }));

    const running = screen.getByRole('region', { name: 'Running now' });
    const links = within(running).getAllByRole('link');
    expect(links.map((link) => link.getAttribute('href'))).toEqual([
      routeHref({ name: 'run', instanceId: ID, featureId: 'feat-1' }),
      routeHref({ name: 'run', instanceId: ID, featureId: 'feat-2' }),
    ]);
    expect(links[0]).toHaveTextContent('Running');
    expect(links[0]).toHaveTextContent('Started 10m ago');
    expect(links[0]).toHaveTextContent('Updated 1m ago');
    expect(links[1]).toHaveTextContent('Failed');
    expect(links[1]).not.toHaveTextContent('Started');
  });

  it('says nothing is running for an empty run list, and no project for an empty project list', async () => {
    await renderLoaded(detail({ live_runs: [], projects: [] }));

    expect(screen.getByRole('region', { name: 'Running now' })).toHaveTextContent('Nothing running');
    expect(screen.getByRole('region', { name: 'Projects' })).toHaveTextContent(/no projects/i);
    expect(screen.queryByText(/withheld/i)).toBeNull();
  });

  it('shows runs and projects as withheld, not empty, when read is off', async () => {
    await renderLoaded(detail({ read_granted: false, scopes: [], projects: null, live_runs: null, workflows: null, run_shape: null }));

    const running = screen.getByRole('region', { name: 'Running now' });
    const projects = screen.getByRole('region', { name: 'Projects' });
    const harnesses = screen.getByRole('region', { name: 'Harnesses' });
    for (const section of [running, projects, harnesses]) {
      expect(section).toHaveTextContent(/withheld/i);
      expect(section).toHaveTextContent(/turn on read/i);
    }
    expect(running).not.toHaveTextContent('Nothing running');
    expect(projects).not.toHaveTextContent(/no projects/i);
  });

  it('links Run here to Dispatch on this instance', async () => {
    await renderLoaded(detail());

    expect(screen.getByRole('link', { name: 'Run here' })).toHaveAttribute('href', `#/instances/${ID}/dispatch`);
  });

  it('shows presence, platform and connection facts', async () => {
    await renderLoaded(detail());

    expect(screen.getByText('linux · x86_64 · 1.4.2')).toBeInTheDocument();
    const connection = screen.getByRole('region', { name: 'Connection' });
    expect(connection).toHaveTextContent('Present');
    expect(connection).toHaveTextContent('Connected since1h ago');
    expect(connection).toHaveTextContent('Last seen2m ago');
    expect(within(connection).getByText('install-9')).toHaveClass('font-mono');
  });

  it('lists read, spend and gates as allowed or not, and no logs scope', async () => {
    await renderLoaded(detail({ scopes: ['read', 'gates'] }));

    const scopes = screen.getByRole('region', { name: 'What this instance allows' });
    const items = within(scopes).getAllByRole('listitem');
    expect(items.map((item) => item.textContent)).toEqual([
      expect.stringMatching(/^read.*Allowed$/),
      expect.stringMatching(/^spend.*Not allowed$/),
      expect.stringMatching(/^gates.*Allowed$/),
    ]);
    expect(scopes).not.toHaveTextContent(/logs/i);
  });

  it('renders a feature id that looks like markup as text', async () => {
    const { container } = await renderLoaded(detail({ live_runs: [run('<b>x</b>')] }));

    expect(screen.getByText('<b>x</b>')).toBeInTheDocument();
    expect(container.querySelector('b')).toBeNull();
  });

  it('marks a run whose gate waits on you amber, from the fleet the shell read', async () => {
    const gate = {
      instance_id: ID,
      feature_id: 'feat-2',
      step_id: 's-gate',
      step_execution_id: 'se-1',
      since: NOW - 300,
      title: null,
      project_name: null,
      step_title: 'Review',
    };
    const answers: Record<string, unknown> = {
      [`GET ${PATH}`]: detail({ live_runs: [run('feat-1'), run('feat-2')] }),
      'GET /v1/fleet': {
        instances: [
          {
            id: ID,
            label: 'workstation',
            present: true,
            os: 'linux',
            arch: 'x86_64',
            version: '1.4.2',
            scopes: ['read'],
            read_granted: true,
            last_seen: NOW,
            live_runs: 2,
            needs_you: [gate],
            harnesses: null,
            project_count: 0,
          },
        ],
      },
    };
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: unknown) => {
        const body = answers[`GET ${String(input)}`];
        if (body === undefined) throw new Error(`fetch double was not told about ${String(input)}`);
        return json(body);
      }),
    );
    render(
      <FleetProvider readKey="instance">
        <Instance instanceId={ID} />
      </FleetProvider>,
    );

    expect(await screen.findByRole('heading', { level: 1, name: 'workstation' })).toBeInTheDocument();
    const [plain, gated] = within(screen.getByRole('region', { name: 'Running now' })).getAllByRole('link');
    expect(gated).toHaveTextContent('Gate needs you');
    expect(gated).toHaveTextContent('waiting 5m');
    expect(plain).not.toHaveTextContent('Gate needs you');
  });

  it('renders a 404 as not found', async () => {
    stubFetch(PATH, json({ error: 'not_found' }, 404));
    const { container } = render(<Instance instanceId={ID} />);

    expect(await screen.findByRole('alert')).toHaveTextContent(/not found or not yours/i);
    expect(Array.from(container.querySelectorAll('.font-mono'), (element) => element.textContent)).toEqual([ID]);
    expect(asked).toEqual([`GET ${PATH}`]);
  });

  it('renders an id that is not an instance id as not found, without asking the Hub', async () => {
    stubFetch(PATH, json(detail()));
    render(<Instance instanceId=".." />);

    expect(await screen.findByRole('alert')).toHaveTextContent(/not found or not yours/i);
    expect(asked).toEqual([]);
  });

  it('renders any other failure as an error', async () => {
    stubFetch(PATH, json({ error: 'internal' }, 500));
    render(<Instance instanceId={ID} />);

    expect(await screen.findByRole('alert')).toHaveTextContent(/^internal$/);
  });
});
