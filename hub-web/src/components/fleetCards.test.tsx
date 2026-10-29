import { render, screen, within } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import type { FleetInstance, NeedsYou } from '../lib/fleet';
import type { NeedsYouItem } from '../lib/fleetSummary';
import { routeHref } from '../lib/router';
import type { RunSummary } from '../lib/runs';
import { InstanceCard } from './InstanceCard';
import { NeedsYouCard } from './NeedsYouCard';
import { PresenceDot } from './PresenceDot';

const NOW = 1_000_000;

const INSTANCE: FleetInstance = {
  id: '6f1c2b3a-0000-4000-8000-000000000001',
  label: 'workstation',
  present: true,
  os: 'linux',
  arch: 'x86_64',
  version: '1.4.2',
  scopes: ['read'],
  readGranted: true,
  lastSeen: NOW - 120,
  liveRuns: 1,
  needsYou: [],
  harnesses: ['claude-code', 'opencode'],
  projectCount: 3,
};

const GATE: NeedsYou = {
  instanceId: INSTANCE.id,
  featureId: 'f-abc',
  stepId: 's-review',
  stepExecutionId: 'se-1',
  since: NOW - 14 * 60,
  title: 'Retry failed webhooks',
  projectName: 'checkout-api',
  stepTitle: 'Review gate',
};

const ITEM: NeedsYouItem = { ...GATE, instanceLabel: 'workstation' };

function run(featureId: string, overrides: Partial<RunSummary> = {}): RunSummary {
  return {
    instanceId: INSTANCE.id,
    featureId,
    status: 'running',
    startedAt: NOW - 12 * 60,
    updatedAt: NOW - 30,
    costCents: null,
    title: 'Add SSO to the admin console',
    projectId: 'p-1',
    projectName: 'admin-web',
    stepId: 's-implement',
    stepTitle: 'Implement',
    tickets: { completed: 3, failed: 0, running: 1 },
    ...overrides,
  };
}

describe('PresenceDot', () => {
  it('pulses only when present', () => {
    const { container, rerender } = render(<PresenceDot present />);
    expect(container.firstElementChild).toHaveClass('animate-pulse-glow');
    expect(screen.getByText('Online')).toBeInTheDocument();

    rerender(<PresenceDot present={false} />);
    expect(container.querySelector('.animate-pulse-glow')).toBeNull();
    expect(screen.getByText('Offline')).toBeInTheDocument();
  });
});

describe('InstanceCard', () => {
  it('shows presence, platform, harnesses, projects and when it was seen', () => {
    const { container } = render(<InstanceCard instance={INSTANCE} runs={[]} now={NOW} />);

    expect(screen.getByRole('link', { name: 'workstation' })).toHaveAttribute(
      'href',
      routeHref({ name: 'instance', instanceId: INSTANCE.id }),
    );
    expect(screen.getAllByText('Online').length).toBeGreaterThan(0);
    expect(container).toHaveTextContent('linux · x86_64 · 1.4.2');
    expect(screen.getByText('claude-code')).toBeInTheDocument();
    expect(screen.getByText('opencode')).toBeInTheDocument();
    expect(container).toHaveTextContent('3 projects · seen now');
  });

  it('lists each live run with its labels, elapsed time and ticket count', () => {
    render(<InstanceCard instance={INSTANCE} runs={[run('f-1')]} now={NOW} />);

    const rows = within(screen.getByRole('list', { name: 'Runs on workstation' })).getAllByRole('listitem');
    expect(rows).toHaveLength(1);
    expect(within(rows[0]).getByRole('link', { name: 'Add SSO to the admin console' })).toHaveAttribute(
      'href',
      routeHref({ name: 'run', instanceId: INSTANCE.id, featureId: 'f-1' }),
    );
    expect(rows[0]).toHaveTextContent('12m');
    expect(rows[0]).toHaveTextContent('admin-web · Implement · 3 tickets landed');
  });

  it('falls back to the feature id and step id, and leaves out labels the instance did not report', () => {
    const bare = run('f-bare', {
      title: null,
      projectName: null,
      stepTitle: null,
      stepId: 's-plan',
      tickets: { completed: 0, failed: 0, running: 0 },
    });
    render(<InstanceCard instance={INSTANCE} runs={[bare]} now={NOW} />);

    const row = within(screen.getByRole('list', { name: 'Runs on workstation' })).getByRole('listitem');
    expect(within(row).getByRole('link', { name: 'f-bare' })).toHaveClass('font-mono');
    expect(row).toHaveTextContent(/^f-bare12ms-plan$/);
  });

  it('marks the run a gate is waiting on, and gives a gate with no live run a row of its own', () => {
    const orphan: NeedsYou = { ...GATE, featureId: 'f-orphan', title: null, stepTitle: null };
    render(
      <InstanceCard
        instance={{ ...INSTANCE, needsYou: [GATE, orphan] }}
        runs={[run('f-abc', { title: 'Retry failed webhooks' })]}
        now={NOW}
      />,
    );

    const rows = within(screen.getByRole('list', { name: 'Runs on workstation' })).getAllByRole('listitem');
    expect(rows).toHaveLength(2);
    expect(within(rows[0]).getByText('Gate needs you')).toBeInTheDocument();
    expect(rows[0]).toHaveTextContent('admin-web · Implement · waiting 14m');
    expect(rows[0]).not.toHaveTextContent('tickets landed');
    expect(within(rows[1]).getByRole('link', { name: 'f-orphan' })).toBeInTheDocument();
    expect(rows[1]).toHaveTextContent('checkout-api · s-review · waiting 14m');
  });

  it('offers Run here on an idle online instance, and only there', () => {
    const { rerender } = render(<InstanceCard instance={{ ...INSTANCE, liveRuns: 0 }} runs={[]} now={NOW} />);

    expect(screen.getByText('Idle — nothing running')).toBeInTheDocument();
    expect(screen.getByRole('link', { name: 'Run here' })).toHaveAttribute(
      'href',
      routeHref({ name: 'dispatch', instanceId: INSTANCE.id }),
    );

    rerender(<InstanceCard instance={{ ...INSTANCE, liveRuns: 0, present: false }} runs={[]} now={NOW} />);
    expect(screen.queryByRole('link', { name: 'Run here' })).toBeNull();
    expect(screen.getByText(/last seen 2m ago/)).toBeInTheDocument();
  });

  it('falls back to the fleet count when the run list could not be read', () => {
    render(<InstanceCard instance={{ ...INSTANCE, liveRuns: 2 }} runs={null} now={NOW} />);

    expect(screen.getByText('2 live runs')).toBeInTheDocument();
    expect(screen.queryByText('Idle — nothing running')).toBeNull();
  });

  it('says read is off, never idle, when runs and gates are withheld', () => {
    const { container } = render(
      <InstanceCard
        instance={{ ...INSTANCE, readGranted: false, liveRuns: null, needsYou: null, harnesses: null, projectCount: null }}
        runs={[]}
        now={NOW}
      />,
    );

    expect(container).toHaveTextContent(/read is off/i);
    expect(container).not.toHaveTextContent(/idle/i);
    expect(container).not.toHaveTextContent(/project/);
  });

  it('falls back to the id and shows an offline dot without a pulse', () => {
    const { container } = render(
      <InstanceCard instance={{ ...INSTANCE, label: null, present: false }} runs={[]} now={NOW} />,
    );

    expect(screen.getByRole('link', { name: INSTANCE.id })).toBeInTheDocument();
    expect(screen.getAllByText('Offline').length).toBeGreaterThan(0);
    expect(container.querySelector('.animate-pulse-glow')).toBeNull();
  });

  it('renders a markup label as literal text', () => {
    const { container } = render(<InstanceCard instance={{ ...INSTANCE, label: '<b>x</b>' }} runs={[]} now={NOW} />);

    expect(screen.getByRole('link', { name: '<b>x</b>' })).toBeInTheDocument();
    expect(container.querySelector('b')).toBeNull();
  });
});

describe('NeedsYouCard', () => {
  it('links to the run and shows its title, project, step and how long the gate has waited', () => {
    const { container } = render(<NeedsYouCard item={ITEM} now={NOW} />);

    expect(screen.getByRole('link', { name: 'Review Retry failed webhooks on workstation' })).toHaveAttribute(
      'href',
      routeHref({ name: 'run', instanceId: ITEM.instanceId, featureId: ITEM.featureId }),
    );
    expect(screen.getByText('Gate needs you')).toBeInTheDocument();
    expect(screen.getByText('Retry failed webhooks')).toHaveClass('font-heading');
    expect(container).toHaveTextContent('checkout-api · Review gate · waiting 14m');
    expect(container.querySelector('[class*="animate-"]')).toBeNull();
  });

  it('falls back to the feature id and step id, and omits what was not reported', () => {
    const { container } = render(
      <NeedsYouCard item={{ ...ITEM, title: null, projectName: null, stepTitle: null }} now={NOW} />,
    );

    expect(screen.getByText('f-abc')).toHaveClass('font-mono');
    expect(container).toHaveTextContent(/s-review · waiting 14m/);
    expect(container).not.toHaveTextContent(/null/);
  });

  it('renders a markup label as literal text', () => {
    const { container } = render(<NeedsYouCard item={{ ...ITEM, instanceLabel: '<b>x</b>' }} now={NOW} />);

    expect(container).toHaveTextContent('<b>x</b>');
    expect(container.querySelector('b')).toBeNull();
  });
});
