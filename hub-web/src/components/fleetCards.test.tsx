import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import type { FleetInstance } from '../lib/fleet';
import type { NeedsYouItem } from '../lib/fleetSummary';
import { routeHref } from '../lib/router';
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
  liveRuns: 3,
  needsYou: [],
};

const ITEM: NeedsYouItem = {
  instanceId: INSTANCE.id,
  instanceLabel: 'workstation',
  featureId: 'f-abc',
  stepId: 's-review',
  stepExecutionId: 'se-1',
  since: NOW - 14 * 60,
};

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
  it('shows presence, platform, run count and last seen', () => {
    const { container } = render(<InstanceCard instance={INSTANCE} now={NOW} />);

    expect(screen.getByRole('link', { name: 'workstation' })).toHaveAttribute(
      'href',
      routeHref({ name: 'instance', instanceId: INSTANCE.id }),
    );
    expect(screen.getAllByText('Online').length).toBeGreaterThan(0);
    expect(container.querySelector('.font-mono')?.textContent).toBe('linux · x86_64 · 1.4.2');
    expect(container).toHaveTextContent('3 live runs');
    expect(container).toHaveTextContent('2m ago');
  });

  it('says read is off, never 0, when live runs are withheld', () => {
    const { container } = render(
      <InstanceCard instance={{ ...INSTANCE, readGranted: false, liveRuns: null, needsYou: null }} now={NOW} />,
    );

    expect(container).toHaveTextContent(/read is off/i);
    expect(container.textContent).not.toMatch(/\b0\b/);
  });

  it('falls back to the id and shows an offline dot without a pulse', () => {
    const { container } = render(<InstanceCard instance={{ ...INSTANCE, label: null, present: false }} now={NOW} />);

    expect(screen.getByRole('link', { name: INSTANCE.id })).toBeInTheDocument();
    expect(screen.getAllByText('Offline').length).toBeGreaterThan(0);
    expect(container.querySelector('.animate-pulse-glow')).toBeNull();
  });

  it('carries a finite amber accent only while a gate needs you', () => {
    const { container, rerender } = render(<InstanceCard instance={INSTANCE} now={NOW} />);
    expect(container.firstElementChild).not.toHaveClass('border-amber-500/30');

    rerender(<InstanceCard instance={{ ...INSTANCE, needsYou: [ITEM] }} now={NOW} />);
    expect(container.firstElementChild).toHaveClass('border-amber-500/30');
    expect(container.querySelectorAll('.animate-pulse-glow')).toHaveLength(1);
  });

  it('renders a markup label as literal text', () => {
    const { container } = render(<InstanceCard instance={{ ...INSTANCE, label: '<b>x</b>' }} now={NOW} />);

    expect(screen.getByRole('link', { name: '<b>x</b>' })).toBeInTheDocument();
    expect(container.querySelector('b')).toBeNull();
  });
});

describe('NeedsYouCard', () => {
  it('links to the run and shows how long the gate has waited', () => {
    const { container } = render(<NeedsYouCard item={ITEM} now={NOW} />);

    expect(screen.getByRole('link', { name: /review/i })).toHaveAttribute(
      'href',
      routeHref({ name: 'run', instanceId: ITEM.instanceId, featureId: ITEM.featureId }),
    );
    expect(screen.getByText('Gate needs you')).toBeInTheDocument();
    expect(container).toHaveTextContent('workstation');
    expect(container).toHaveTextContent('waiting 14m');
    expect(screen.getByText('f-abc')).toHaveClass('font-mono');
    expect(screen.getByText('s-review')).toHaveClass('font-mono');
    expect(container.querySelector('.animate-pulse-glow-amber')).not.toBeNull();
    expect(container.querySelector('.animate-pulse-glow')).toBeNull();
  });

  it('omits the step when it is null', () => {
    const { container } = render(<NeedsYouCard item={{ ...ITEM, stepId: null }} now={NOW} />);

    expect(container.querySelectorAll('.font-mono')).toHaveLength(1);
  });

  it('renders a markup label as literal text', () => {
    const { container } = render(<NeedsYouCard item={{ ...ITEM, instanceLabel: '<b>x</b>' }} now={NOW} />);

    expect(container).toHaveTextContent('<b>x</b>');
    expect(container.querySelector('b')).toBeNull();
  });
});
