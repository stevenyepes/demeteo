import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { setCsrfToken } from '../lib/api';
import { routeHref } from '../lib/router';
import { Runs } from './Runs';

const WORKSTATION = '6f1c2b3a-0000-4000-8000-000000000001';
const LAPTOP = '6f1c2b3a-0000-4000-8000-000000000002';
const NOW = Math.floor(Date.now() / 1000);

afterEach(() => {
  vi.unstubAllGlobals();
  setCsrfToken(null);
});

/** Answers `GET /v1/runs` with `answer` once, and throws on anything else. */
function stubRuns(answer: Response) {
  let answered = false;
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: unknown, init?: RequestInit): Promise<Response> => {
      const request = `${init?.method ?? 'GET'} ${String(input)}`;
      if (answered || request !== 'GET /v1/runs') throw new Error(`fetch double was not told about ${request}`);
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

function run(instance_id: string, feature_id: string, overrides: Record<string, unknown> = {}) {
  return {
    instance_id,
    feature_id,
    status: 'running',
    started_at: NOW - 3600,
    updated_at: NOW - 60,
    cost_cents: null,
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

function rowHrefs(): (string | null)[] {
  return within(screen.getByRole('list')).getAllByRole('link').map((link) => link.getAttribute('href'));
}

describe('Runs', () => {
  it('shows a loading state under the Runs heading until the Hub answers', () => {
    stubRuns(json(REPLY));
    render(<Runs />);

    expect(screen.getByRole('heading', { level: 1 })).toHaveTextContent(/^Runs$/);
    expect(screen.getByText(/loading/i)).toBeInTheDocument();
  });

  it('lists every run from every instance, newest update first, each linking to its run', async () => {
    stubRuns(json(REPLY));
    render(<Runs />);

    await screen.findByRole('list');

    expect(rowHrefs()).toEqual([
      routeHref({ name: 'run', instanceId: LAPTOP, featureId: 'f-new' }),
      routeHref({ name: 'run', instanceId: WORKSTATION, featureId: 'f-mid' }),
      routeHref({ name: 'run', instanceId: WORKSTATION, featureId: 'f-old' }),
    ]);
    const rows = within(screen.getByRole('list')).getAllByRole('link');
    expect(rows[0]).toHaveTextContent('f-new');
    expect(rows[0]).toHaveTextContent(LAPTOP);
    expect(rows[1]).toHaveTextContent('failed');
  });

  it('says runs of instances without read are left out', async () => {
    stubRuns(json(REPLY));
    render(<Runs />);

    await screen.findByRole('list');

    expect(screen.getByText(/read off.*not listed/i)).toBeInTheDocument();
  });

  it('renders an empty reply as an empty state, not an error', async () => {
    stubRuns(json({ runs: [] }));
    render(<Runs />);

    expect(await screen.findByText(/no runs reported/i)).toBeInTheDocument();
    expect(screen.queryByRole('alert')).toBeNull();
    expect(screen.queryByRole('list')).toBeNull();
  });

  it('renders a refusal through CeremonyError', async () => {
    stubRuns(json({ error: 'internal' }, 500));
    render(<Runs />);

    expect(await screen.findByRole('alert')).toHaveTextContent(/^internal$/);
    expect(screen.queryByRole('list')).toBeNull();
  });

  it('narrows the rows to the chosen status, and back to all', async () => {
    stubRuns(json(REPLY));
    render(<Runs />);
    await screen.findByRole('list');
    const filter = screen.getByRole('group', { name: 'Status' });

    await userEvent.click(within(filter).getByRole('button', { name: 'Failed' }));

    expect(within(filter).getByRole('button', { name: 'Failed' })).toHaveAttribute('aria-pressed', 'true');
    expect(rowHrefs()).toEqual([routeHref({ name: 'run', instanceId: WORKSTATION, featureId: 'f-mid' })]);

    await userEvent.click(within(filter).getByRole('button', { name: 'Cancelled' }));

    expect(screen.queryByRole('list')).toBeNull();
    expect(screen.getByText(/no cancelled runs/i)).toBeInTheDocument();

    await userEvent.click(within(filter).getByRole('button', { name: 'All' }));

    expect(rowHrefs()).toHaveLength(3);
  });
});
