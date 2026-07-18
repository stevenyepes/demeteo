// Unit tests for the MachineDot presentational primitive (spec §5).
//
// Pins the local/remote colour rule copied from `machineDotColor` in
// `src/components/TerminalTab.tsx`, the `data-machine-kind` attribute
// consumers assert against, and the pulse ↔ dim class toggle.

import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { MachineDot } from './MachineDot';

describe('MachineDot', () => {
  it('renders a local (cyan) dot for the local machine id', () => {
    render(<MachineDot machineId="local" machineLabel="whatever" />);

    const dot = screen.getByTestId('machine-dot');
    expect(dot).toHaveAttribute('data-machine-kind', 'local');
    expect(dot).toHaveClass('bg-cyan-400');
    expect(dot).not.toHaveClass('bg-emerald-400');
  });

  it('treats a "local" machineLabel as local even with a non-local id', () => {
    render(<MachineDot machineId="host-7" machineLabel="Local" />);

    const dot = screen.getByTestId('machine-dot');
    expect(dot).toHaveAttribute('data-machine-kind', 'local');
    expect(dot).toHaveClass('bg-cyan-400');
  });

  it('renders a remote (emerald) dot for a remote machine', () => {
    render(<MachineDot machineId="host-7" machineLabel="build-box" />);

    const dot = screen.getByTestId('machine-dot');
    expect(dot).toHaveAttribute('data-machine-kind', 'remote');
    expect(dot).toHaveClass('bg-emerald-400');
    expect(dot).not.toHaveClass('bg-cyan-400');
  });

  it('applies the pulse-glow animation when pulse is set', () => {
    render(<MachineDot machineId="local" machineLabel="local" pulse />);

    const dot = screen.getByTestId('machine-dot');
    expect(dot).toHaveClass('animate-pulse-glow');
    expect(dot).not.toHaveClass('opacity-60');
  });

  it('dims to opacity-60 when pulse is not set', () => {
    render(<MachineDot machineId="local" machineLabel="local" />);

    const dot = screen.getByTestId('machine-dot');
    expect(dot).toHaveClass('opacity-60');
    expect(dot).not.toHaveClass('animate-pulse-glow');
  });

  it('merges a caller-supplied className', () => {
    render(<MachineDot machineId="local" machineLabel="local" className="mr-2" />);

    expect(screen.getByTestId('machine-dot')).toHaveClass('mr-2');
  });
});
