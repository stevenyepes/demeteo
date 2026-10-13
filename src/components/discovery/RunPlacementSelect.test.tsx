import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import type { RunnerCompatibilityState } from '../../hooks/useRunnerCompatibility';
import type { Machine, RunnerCompatibilityReport } from '../../types';
import { RunPlacementSelect } from './RunPlacementSelect';

vi.mock('../../context', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../../context')>()),
  useNavigation: () => ({ navigate: vi.fn() }),
}));

const box: Machine = { id: 'machine-box', name: 'box', host: 'box.lan', port: 22, username: 'dev', auth_type: 'key' };
const laptop: Machine = { id: 'machine-laptop', name: 'laptop', host: '', port: 0, username: '', auth_type: 'local' };

const runnerBehind: RunnerCompatibilityReport = {
  verdict: 'runner_behind',
  runner: '1.1.0',
  runner_channel: 'stable',
  app: '1.2.0',
  app_channel: 'stable',
  message: 'demeteo-runner 1.1.0 (stable) on box is older than Demeteo 1.2.0 (stable).',
};

function compatibility(report: RunnerCompatibilityReport | null = null): RunnerCompatibilityState {
  return { report, loading: false, refresh: vi.fn() };
}

function renderSelect(props: Partial<React.ComponentProps<typeof RunPlacementSelect>> = {}) {
  const onChange = vi.fn();
  render(
    <RunPlacementSelect
      machines={[box, laptop]}
      value={null}
      onChange={onChange}
      localLabel="Local"
      compatibility={compatibility()}
      {...props}
    />,
  );
  return onChange;
}

const optionLabels = () => screen.getAllByRole('option').map((o) => o.textContent);

describe('RunPlacementSelect options', () => {
  it('offers Default, Local and each remote machine as detached, never a local-auth machine', () => {
    renderSelect({ defaultLabel: 'Detached · box' });

    expect(optionLabels()).toEqual(['Default (Detached · box)', 'Local', 'box — detached']);
  });

  it('maps Default to null, Local to "local" and a machine to its id', () => {
    const onChange = renderSelect({ defaultLabel: 'Local', value: box.id });
    const select = screen.getByRole('combobox');

    fireEvent.change(select, { target: { value: '' } });
    fireEvent.change(select, { target: { value: 'local' } });
    fireEvent.change(select, { target: { value: box.id } });

    expect(onChange.mock.calls).toEqual([[null], ['local'], [box.id]]);
  });

  it('lets the local option stand for null when there is no Default', () => {
    const onChange = renderSelect({ localLabel: 'This machine', value: box.id });

    expect(optionLabels()).toEqual(['This machine', 'box — detached']);
    const local = screen.getByRole<HTMLOptionElement>('option', { name: 'This machine' });
    fireEvent.change(screen.getByRole('combobox'), { target: { value: local.value } });

    expect(onChange).toHaveBeenCalledWith(null);
  });

  it('keeps a stored machine that is no longer configured visible as the selection', () => {
    renderSelect({ defaultLabel: 'Local', value: 'machine-gone' });

    expect(screen.getByRole('combobox')).toHaveValue('machine-gone');
    expect(screen.getByRole('combobox')).toHaveAttribute('aria-invalid', 'true');
    expect(optionLabels()).toContain('machine-gone — no longer configured');
  });
});

describe('RunPlacementSelect compatibility notice', () => {
  it('refuses an incompatible runner and offers a re-check', () => {
    const state = compatibility(runnerBehind);
    renderSelect({ value: box.id, compatibility: state });

    expect(screen.getByRole('alert')).toHaveTextContent(runnerBehind.message);
    expect(screen.getByRole('combobox')).toHaveAttribute('aria-invalid', 'true');
    fireEvent.click(screen.getByRole('button', { name: /check again/i }));
    expect(state.refresh).toHaveBeenCalledOnce();
  });

  it('routes the notice to the host-provided machine settings', () => {
    const onOpenMachineSettings = vi.fn();
    renderSelect({ value: box.id, compatibility: compatibility(runnerBehind), onOpenMachineSettings });

    fireEvent.click(screen.getByRole('button', { name: /open machine settings/i }));

    expect(onOpenMachineSettings).toHaveBeenCalledOnce();
  });

  it('shows nothing for a compatible runner', () => {
    renderSelect({
      value: box.id,
      compatibility: compatibility({
        verdict: 'compatible',
        version: '1.2.0',
        channel: 'stable',
        message: 'demeteo-runner 1.2.0 (stable) on box matches Demeteo.',
      }),
    });

    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
    expect(screen.getByRole('combobox')).toHaveAttribute('aria-invalid', 'false');
  });
});
