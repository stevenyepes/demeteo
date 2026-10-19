import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { BootstrapStepper, type BootstrapPhaseView } from './BootstrapStepper';

function dotOf(label: string): HTMLElement {
  const row = screen.getByText(label).closest('div.flex.items-start');
  const dot = row?.querySelector('span.rounded-full');
  if (!(dot instanceof HTMLElement)) throw new Error(`no dot for ${label}`);
  return dot;
}

function renderPhase(phase: BootstrapPhaseView) {
  render(<BootstrapStepper phases={[phase]} />);
}

describe('BootstrapStepper', () => {
  it('renders a warning phase with a dot and detail distinct from completed', () => {
    const detail = 'fetch origin failed: could not resolve host';
    renderPhase({
      id: 'syncing_origin',
      label: 'Syncing origin',
      status: 'warning',
      detail,
    });
    const warningDot = dotOf('Syncing origin').className;
    const warningDetail = screen.getByText(detail).className;
    expect(warningDot).toContain('bg-amber-500');
    expect(warningDetail).toContain('text-amber-500');
    expect(screen.queryByText('Bootstrap failed')).not.toBeInTheDocument();
  });

  it('renders completed without the warning styling', () => {
    const detail = 'up to date';
    renderPhase({
      id: 'syncing_origin',
      label: 'Syncing origin',
      status: 'completed',
      detail,
    });
    expect(dotOf('Syncing origin').className).toContain('bg-emerald-400');
    expect(dotOf('Syncing origin').className).not.toContain('bg-amber-500');
    expect(screen.getByText(detail).className).not.toContain('text-amber-500');
  });
});
