import { render, screen, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { HubApiError } from '../lib/api';
import type { FleetInstance, InstanceDetail } from '../lib/fleet';
import { routeHref } from '../lib/router';
import { Fleet } from './Fleet';

const fleet = vi.hoisted(() => ({
  getFleet: vi.fn<() => Promise<FleetInstance[]>>(),
  getInstance: vi.fn<(instanceId: string) => Promise<InstanceDetail>>(),
}));

vi.mock('../lib/fleet', () => fleet);

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
    readGranted: true,
    lastSeen: NOW - 60,
    liveRuns: 0,
    needsYou: [],
    ...overrides,
  };
}

const WORKSTATION = instance('6f1c2b3a-0000-4000-8000-000000000001', {
  label: 'workstation',
  present: true,
  os: 'linux',
  arch: 'x86_64',
  version: '1.4.2',
  liveRuns: 2,
});

const LAPTOP = instance('6f1c2b3a-0000-4000-8000-000000000002', {
  label: 'laptop',
  present: false,
  os: 'macos',
  arch: 'aarch64',
  version: '1.3.0',
  liveRuns: 0,
});

const LOCKED = instance('6f1c2b3a-0000-4000-8000-000000000003', {
  label: 'locked-box',
  present: false,
  os: 'windows',
  arch: 'x86_64',
  version: '1.4.0',
  scopes: [],
  readGranted: false,
  liveRuns: null,
  needsYou: null,
});

/** The card an instance's label links from: each assertion is held to its own card. */
function cardOf(label: string): HTMLElement {
  const card = screen.getByRole('link', { name: label }).closest('article');
  if (card === null) throw new Error(`${label} has no card`);
  return card;
}

beforeEach(() => {
  fleet.getFleet.mockRejectedValue(new Error('getFleet was not scripted'));
  fleet.getInstance.mockRejectedValue(new Error('getInstance was not scripted'));
});

describe('Fleet', () => {
  it('shows a loading state until the fleet arrives', () => {
    fleet.getFleet.mockReturnValueOnce(new Promise<never>(() => {}));
    render(<Fleet />);

    expect(screen.getByText(/loading/i)).toBeInTheDocument();
    expect(screen.queryByRole('alert')).toBeNull();
  });

  it('empty fleet: shows the empty state and the Add instance link, and no instance card', async () => {
    fleet.getFleet.mockResolvedValueOnce([]);
    const { container } = render(<Fleet />);

    expect(await screen.findByText(/no instances are paired/i)).toBeInTheDocument();
    expect(screen.getByRole('link', { name: 'Add instance' })).toHaveAttribute('href', '#/add-instance');
    expect(container.querySelector('article')).toBeNull();
    expect(screen.queryByRole('heading', { name: 'Needs you' })).toBeNull();
    expect(fleet.getFleet).toHaveBeenCalledOnce();
  });

  it('mixed online/offline: each card shows its own presence and platform, and read-off instead of a count', async () => {
    fleet.getFleet.mockResolvedValueOnce([WORKSTATION, LAPTOP, LOCKED]);
    render(<Fleet />);

    const workstation = await screen.findByRole('link', { name: 'workstation' });
    expect(workstation).toHaveAttribute('href', routeHref({ name: 'instance', instanceId: WORKSTATION.id }));

    const online = cardOf('workstation');
    expect(online).toHaveTextContent(/Online/);
    expect(online).not.toHaveTextContent(/Offline/);
    expect(online).toHaveTextContent('linux · x86_64 · 1.4.2');
    expect(online).toHaveTextContent('2 live runs');
    expect(online.querySelectorAll('.animate-pulse-glow')).toHaveLength(1);

    const offline = cardOf('laptop');
    expect(offline).toHaveTextContent(/Offline/);
    expect(offline).not.toHaveTextContent(/Online/);
    expect(offline).toHaveTextContent('macos · aarch64 · 1.3.0');
    expect(offline).toHaveTextContent('0 live runs');
    expect(offline.querySelector('.animate-pulse-glow')).toBeNull();

    const locked = cardOf('locked-box');
    expect(locked).toHaveTextContent(/read is off/i);
    expect(locked).not.toHaveTextContent(/live run/);
    expect(locked).toHaveTextContent('windows · x86_64 · 1.4.0');
    expect(locked.querySelector('.animate-pulse-glow')).toBeNull();

    const summary = screen.getByRole('region', { name: 'Fleet summary' });
    expect(within(summary).getByText('Instances').nextElementSibling).toHaveTextContent(/^3$/);
    expect(within(summary).getByText('Online').nextElementSibling).toHaveTextContent(/^1$/);
    expect(within(summary).getByText('Runs active').nextElementSibling).toHaveTextContent(/^2$/);
    expect(within(summary).getByText('Need you').nextElementSibling).toHaveTextContent(/^0$/);
    expect(summary).toHaveTextContent(/1 instance has read off/i);
    expect(screen.queryByRole('heading', { name: 'Needs you' })).toBeNull();
  });

  it('instance with a pending gate: the gate appears in the Needs you lane, linking to its run', async () => {
    const gated = instance('6f1c2b3a-0000-4000-8000-000000000004', {
      label: 'build-box',
      needsYou: [
        {
          instanceId: '6f1c2b3a-0000-4000-8000-000000000004',
          featureId: 'f-abc',
          stepId: 's-review',
          stepExecutionId: 'se-1',
          since: NOW - 300,
        },
      ],
    });
    fleet.getFleet.mockResolvedValueOnce([WORKSTATION, gated]);
    render(<Fleet />);

    const heading = await screen.findByRole('heading', { name: 'Needs you' });
    const lane = heading.closest('section');
    if (lane === null) throw new Error('the Needs you heading heads no section');
    const items = within(lane).getAllByRole('listitem');
    expect(items).toHaveLength(1);
    expect(items[0]).toHaveTextContent('build-box');
    expect(items[0]).toHaveTextContent('f-abc');
    expect(within(items[0]).getByRole('link')).toHaveAttribute(
      'href',
      routeHref({ name: 'run', instanceId: gated.id, featureId: 'f-abc' }),
    );
    expect(within(lane).queryByText('workstation')).toBeNull();

    const summary = screen.getByRole('region', { name: 'Fleet summary' });
    expect(within(summary).getByText('Need you').nextElementSibling).toHaveTextContent(/^1$/);
  });

  it('renders untrusted labels and feature ids as text', async () => {
    const marked = instance('6f1c2b3a-0000-4000-8000-000000000005', {
      label: '<b>x</b>',
      needsYou: [
        {
          instanceId: '6f1c2b3a-0000-4000-8000-000000000005',
          featureId: '<b>x</b>',
          stepId: null,
          stepExecutionId: 'se-2',
          since: NOW - 30,
        },
      ],
    });
    fleet.getFleet.mockResolvedValueOnce([marked]);
    const { container } = render(<Fleet />);

    await screen.findByRole('heading', { name: 'Needs you' });
    expect(screen.getAllByText('<b>x</b>').length).toBeGreaterThan(0);
    expect(container.querySelector('b')).toBeNull();
  });

  it('offers Add instance and Run a pipeline from a populated fleet', async () => {
    fleet.getFleet.mockResolvedValueOnce([WORKSTATION]);
    render(<Fleet />);

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
    const { container } = render(<Fleet />);

    expect(await screen.findByRole('alert')).toHaveTextContent(/^internal$/);
    expect(container.querySelector('article')).toBeNull();
    expect(screen.queryByText(/loading/i)).toBeNull();
  });
});
