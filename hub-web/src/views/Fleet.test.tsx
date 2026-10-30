import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { HubApiError } from '../lib/api';
import type { FleetInstance, NeedsYou } from '../lib/fleet';
import { FleetProvider } from '../lib/fleetContext';
import { routeHref } from '../lib/router';
import type { RunSummary, RunsFilter } from '../lib/runs';
import { Fleet } from './Fleet';

const fleet = vi.hoisted(() => ({
  getFleet: vi.fn<() => Promise<FleetInstance[]>>(),
}));

const runs = vi.hoisted(() => ({
  listRuns: vi.fn<(filter?: RunsFilter) => Promise<RunSummary[]>>(),
}));

vi.mock('../lib/fleet', () => fleet);
vi.mock('../lib/runs', () => runs);

const NOW = Math.floor(Date.now() / 1000);

function instance(id: string, overrides: Partial<FleetInstance>): FleetInstance {
  return {
    id,
    label: null,
    present: true,
    os: 'linux',
    arch: 'x86_64',
    version: '1.4.2',
    scopes: ['read'],
    kind: 'desktop',
    readGranted: true,
    lastSeen: NOW - 60,
    liveRuns: 0,
    needsYou: [],
    harnesses: null,
    projectCount: null,
    ...overrides,
  };
}

function gate(instanceId: string, featureId: string, overrides: Partial<NeedsYou> = {}): NeedsYou {
  return {
    instanceId,
    featureId,
    stepId: 's-review',
    stepExecutionId: `se-${featureId}`,
    since: NOW - 300,
    title: null,
    projectName: null,
    stepTitle: null,
    ...overrides,
  };
}

const WORKSTATION = instance('6f1c2b3a-0000-4000-8000-000000000001', {
  label: 'workstation',
  liveRuns: 2,
  harnesses: ['claude-code', 'opencode'],
});

const LAPTOP = instance('6f1c2b3a-0000-4000-8000-000000000002', {
  label: 'laptop',
  present: false,
  os: 'macos',
  arch: 'aarch64',
  version: '1.3.0',
});

const LOCKED = instance('6f1c2b3a-0000-4000-8000-000000000003', {
  label: 'locked-box',
  present: false,
  os: 'windows',
  scopes: [],
  readGranted: false,
  liveRuns: null,
  needsYou: null,
});

const SSO_RUN: RunSummary = {
  instanceId: WORKSTATION.id,
  featureId: 'f-sso',
  status: 'running',
  startedAt: NOW - 12 * 60,
  updatedAt: NOW - 10,
  costCents: null,
  title: 'Add SSO to the admin console',
  projectId: 'p-admin',
  projectName: 'admin-web',
  stepId: 's-implement',
  stepTitle: 'Implement',
  tickets: { completed: 3, failed: 0, running: 1 },
};

function renderFleet() {
  return render(
    <FleetProvider readKey="fleet">
      <Fleet />
    </FleetProvider>,
  );
}

/** The card an instance's label links from: each assertion is held to its own card. */
function cardOf(label: string): HTMLElement {
  const card = screen.getByRole('link', { name: label }).closest('article');
  if (card === null) throw new Error(`${label} has no card`);
  return card;
}

function cardLabels(): string[] {
  return Array.from(document.querySelectorAll('article'), (card) => card.querySelector('a')?.textContent ?? '');
}

beforeEach(() => {
  fleet.getFleet.mockRejectedValue(new Error('getFleet was not scripted'));
  runs.listRuns.mockResolvedValue([]);
});

describe('Fleet', () => {
  it('shows a loading state until the fleet arrives', () => {
    fleet.getFleet.mockReturnValueOnce(new Promise<never>(() => {}));
    renderFleet();

    expect(screen.getByText(/loading/i)).toBeInTheDocument();
    expect(screen.queryByRole('alert')).toBeNull();
  });

  it('empty fleet: shows the empty state and the Add instance link, and no instance card', async () => {
    fleet.getFleet.mockResolvedValueOnce([]);
    const { container } = renderFleet();

    expect(await screen.findByText(/no instances are paired/i)).toBeInTheDocument();
    expect(screen.getByRole('link', { name: 'Add instance' })).toHaveAttribute('href', '#/add-instance');
    expect(container.querySelector('article')).toBeNull();
    expect(screen.queryByRole('heading', { name: 'Needs you' })).toBeNull();
    expect(fleet.getFleet).toHaveBeenCalledOnce();
  });

  it('summarises the fleet in the header, and counts no read-off instance', async () => {
    fleet.getFleet.mockResolvedValueOnce([WORKSTATION, LAPTOP, LOCKED]);
    renderFleet();

    const summary = await screen.findByRole('region', { name: 'Fleet summary' });
    expect(summary).toHaveTextContent('3 instances · 1 online · 2 runs active · 0 need you');
    expect(summary).toHaveTextContent(/1 instance has read off/i);
    expect(screen.getByText('Personal fleet')).toBeInTheDocument();
    expect(screen.queryByRole('heading', { name: 'Needs you' })).toBeNull();
  });

  it('reads only running runs, and puts each on its own instance card', async () => {
    fleet.getFleet.mockResolvedValueOnce([WORKSTATION, LAPTOP, LOCKED]);
    runs.listRuns.mockResolvedValueOnce([SSO_RUN]);
    renderFleet();

    const row = await within(await screen.findByRole('list', { name: 'Runs on workstation' })).findByRole(
      'listitem',
    );
    expect(row).toHaveTextContent('Add SSO to the admin console');
    expect(row).toHaveTextContent('admin-web · Implement · 3 tickets landed');
    expect(runs.listRuns.mock.calls).toEqual([[{ status: 'running' }]]);

    expect(cardOf('workstation')).toHaveTextContent('claude-code');
    expect(cardOf('laptop')).toHaveTextContent('Idle — nothing running');
    expect(cardOf('locked-box')).toHaveTextContent(/read is off/i);
  });

  it('still shows every card when the run list cannot be read', async () => {
    fleet.getFleet.mockResolvedValueOnce([WORKSTATION]);
    runs.listRuns.mockRejectedValueOnce(new Error('runs unavailable'));
    renderFleet();

    expect(await screen.findByText('2 live runs')).toBeInTheDocument();
    expect(screen.queryByRole('alert')).toBeNull();
  });

  it('filters the cards by status, counting each segment', async () => {
    const gated = instance('6f1c2b3a-0000-4000-8000-000000000004', {
      label: 'build-box',
      needsYou: [gate('6f1c2b3a-0000-4000-8000-000000000004', 'f-abc')],
    });
    fleet.getFleet.mockResolvedValueOnce([WORKSTATION, LAPTOP, gated]);
    renderFleet();
    await screen.findByRole('link', { name: 'workstation' });
    const status = screen.getByRole('radiogroup', { name: 'Status' });

    expect(within(status).getByRole('radio', { name: 'All, 3' })).toBeChecked();
    await userEvent.click(within(status).getByRole('radio', { name: 'Offline, 1' }));
    expect(cardLabels()).toEqual(['laptop']);
    await userEvent.click(within(status).getByRole('radio', { name: 'Attention, 1' }));
    expect(cardLabels()).toEqual(['build-box']);
    await userEvent.click(within(status).getByRole('radio', { name: 'Online, 2' }));
    expect(cardLabels()).toEqual(['workstation', 'build-box']);
  });

  it('searches by label, os, harness and the projects of its runs', async () => {
    fleet.getFleet.mockResolvedValueOnce([WORKSTATION, LAPTOP, LOCKED]);
    runs.listRuns.mockResolvedValueOnce([SSO_RUN]);
    renderFleet();
    await screen.findByRole('list', { name: 'Runs on workstation' });
    const search = screen.getByRole('searchbox', { name: 'Search instances' });

    await userEvent.type(search, 'LAP');
    expect(cardLabels()).toEqual(['laptop']);
    await userEvent.clear(search);
    await userEvent.type(search, 'windows');
    expect(cardLabels()).toEqual(['locked-box']);
    await userEvent.clear(search);
    await userEvent.type(search, 'opencode');
    expect(cardLabels()).toEqual(['workstation']);
    await userEvent.clear(search);
    await userEvent.type(search, 'admin-web');
    expect(cardLabels()).toEqual(['workstation']);
    await userEvent.clear(search);
    await userEvent.type(search, 'nothing-matches');
    expect(cardLabels()).toEqual([]);
    expect(screen.getByText('No instance matches.')).toBeInTheDocument();
  });

  it('instance with a pending gate: the gate appears in the Needs you lane, linking to its run', async () => {
    const gated = instance('6f1c2b3a-0000-4000-8000-000000000004', {
      label: 'build-box',
      needsYou: [
        gate('6f1c2b3a-0000-4000-8000-000000000004', 'f-abc', {
          title: 'Retry failed webhooks',
          projectName: 'checkout-api',
          stepTitle: 'Review gate',
        }),
      ],
    });
    fleet.getFleet.mockResolvedValueOnce([WORKSTATION, gated]);
    renderFleet();

    const heading = await screen.findByRole('heading', { name: 'Needs you' });
    const lane = heading.closest('section');
    if (lane === null) throw new Error('the Needs you heading heads no section');
    const items = within(lane).getAllByRole('listitem');
    expect(items).toHaveLength(1);
    expect(items[0]).toHaveTextContent('build-box');
    expect(items[0]).toHaveTextContent('Retry failed webhooks');
    expect(items[0]).toHaveTextContent('checkout-api · Review gate · waiting 5m');
    expect(within(items[0]).getByRole('link')).toHaveAttribute(
      'href',
      routeHref({ name: 'run', instanceId: gated.id, featureId: 'f-abc' }),
    );
    expect(within(lane).queryByText('workstation')).toBeNull();
    expect(screen.getByRole('region', { name: 'Fleet summary' })).toHaveTextContent('1 need you');
  });

  it('renders untrusted labels and feature ids as text', async () => {
    const marked = instance('6f1c2b3a-0000-4000-8000-000000000005', {
      label: '<b>x</b>',
      needsYou: [gate('6f1c2b3a-0000-4000-8000-000000000005', '<b>x</b>', { stepId: null })],
    });
    fleet.getFleet.mockResolvedValueOnce([marked]);
    const { container } = renderFleet();

    await screen.findByRole('heading', { name: 'Needs you' });
    expect(screen.getAllByText('<b>x</b>').length).toBeGreaterThan(0);
    expect(container.querySelector('b')).toBeNull();
  });

  it('offers Add instance and Run a pipeline from a populated fleet', async () => {
    fleet.getFleet.mockResolvedValueOnce([WORKSTATION]);
    renderFleet();

    expect(await screen.findByRole('link', { name: 'Add instance' })).toHaveAttribute(
      'href',
      routeHref({ name: 'add-instance' }),
    );
    expect(screen.getByRole('link', { name: 'Run a pipeline' })).toHaveAttribute(
      'href',
      routeHref({ name: 'dispatch' }),
    );
  });

  it('failed load: a Hub 500 renders an alert', async () => {
    fleet.getFleet.mockRejectedValueOnce(
      new HubApiError('/v1/fleet answered HTTP 500', '/v1/fleet', 500, 'internal'),
    );
    const { container } = renderFleet();

    expect(await screen.findByRole('alert')).toHaveTextContent(/^internal$/);
    expect(container.querySelector('article')).toBeNull();
    expect(screen.queryByText(/loading/i)).toBeNull();
  });
});
