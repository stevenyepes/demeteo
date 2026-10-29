import { useState } from 'react';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import type { HubLocalPolicy } from '../../lib/hub';
import { HubCeilingsCard, type CeilingField } from './HubCeilingsCard';

const POLICY: HubLocalPolicy = {
  read_enabled: true,
  spend_enabled: true,
  gates_enabled: false,
  run_ceiling_cents: 500,
  daily_ceiling_cents: 2000,
};

function Harness({
  onSave,
  saving = false,
}: {
  onSave: (field: CeilingField, cents: number | null) => void;
  saving?: boolean;
}) {
  const [policy, setPolicy] = useState(POLICY);
  return (
    <HubCeilingsCard
      policy={policy}
      saving={saving}
      onSave={(field, cents) => {
        onSave(field, cents);
        setPolicy((current) => ({ ...current, [field]: cents }));
      }}
    />
  );
}

function setup(saving = false) {
  const onSave = vi.fn();
  render(<Harness onSave={onSave} saving={saving} />);
  return {
    onSave,
    run: screen.getByLabelText('Per-run maximum'),
    daily: screen.getByLabelText('Rolling 24-hour total'),
  };
}

describe('HubCeilingsCard', () => {
  it('shows stored ceilings in dollars', () => {
    const { run, daily } = setup();

    expect(run).toHaveValue('5.00');
    expect(daily).toHaveValue('20.00');
    expect(screen.queryByText('Not set — every Hub start is refused')).not.toBeInTheDocument();
  });

  it('saves a dollar entry as integer cents', async () => {
    const { onSave, run } = setup();

    await userEvent.clear(run);
    await userEvent.type(run, '12.50{Enter}');

    expect(onSave).toHaveBeenCalledWith('run_ceiling_cents', 1250);
    expect(onSave).toHaveBeenCalledTimes(1);
    expect(run).toHaveValue('12.50');
  });

  it('saves a cleared ceiling as null and says every Hub start is refused', async () => {
    const { onSave, daily } = setup();

    await userEvent.clear(daily);
    await userEvent.tab();

    expect(onSave).toHaveBeenCalledWith('daily_ceiling_cents', null);
    expect(screen.getByText('Not set — every Hub start is refused')).toBeInTheDocument();
    expect(screen.queryByText(/unlimited/i)).not.toBeInTheDocument();
  });

  it('saves 0 as a real ceiling', async () => {
    const { onSave, run } = setup();

    await userEvent.clear(run);
    await userEvent.type(run, '0{Enter}');

    expect(onSave).toHaveBeenCalledWith('run_ceiling_cents', 0);
    expect(screen.queryByText('Not set — every Hub start is refused')).not.toBeInTheDocument();
  });

  it.each(['-5', 'ten'])('does not save %s and shows an inline error', async (entry) => {
    const { onSave, run } = setup();

    await userEvent.clear(run);
    await userEvent.type(run, `${entry}{Enter}`);
    await userEvent.tab();

    expect(onSave).not.toHaveBeenCalled();
    expect(screen.getByText('Enter an amount in dollars, like 12.50')).toBeInTheDocument();
    expect(run).toHaveAttribute('aria-invalid', 'true');
  });

  it('disables both ceilings while a policy save is pending', () => {
    const { run, daily } = setup(true);

    expect(run).toBeDisabled();
    expect(daily).toBeDisabled();
  });
});
