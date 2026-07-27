/**
 * The schedule drawer — the write side of workflow scheduling, which the v2
 * builder dropped when it replaced `WorkflowEditor`.
 *
 * The regression these guard against is a quiet one: `WorkflowList` still
 * *renders* cron and next-run, and the backend scheduler still fires, so a
 * product with no way to edit a schedule looks completely normal until someone
 * tries to turn one off.
 */
import { describe, expect, it, vi, beforeEach } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';

import { ScheduleDrawer, isCleared, validateSchedule } from './ScheduleDrawer';

const invoke = vi.hoisted(() => vi.fn());
vi.mock('@tauri-apps/api/core', () => ({ invoke }));

const reportError = vi.hoisted(() => vi.fn());
vi.mock('../../lib/errorBus', () => ({ useErrorBus: () => ({ reportError }) }));

const PROJECTS = [
  { id: 'p1', name: 'Demeteo' },
  { id: 'p2', name: 'Side quest' },
];

beforeEach(() => {
  invoke.mockReset();
  reportError.mockReset();
  invoke.mockImplementation((cmd: string) => {
    if (cmd === 'get_projects') return Promise.resolve(PROJECTS);
    return Promise.resolve(undefined);
  });
});

function renderDrawer(schedule: Parameters<typeof ScheduleDrawer>[0]['schedule'] = null) {
  const onSaved = vi.fn();
  const onClose = vi.fn();
  render(
    <ScheduleDrawer
      workflowId="wf-1"
      schedule={schedule}
      onSaved={onSaved}
      onClose={onClose}
    />,
  );
  return { onSaved, onClose };
}

describe('validateSchedule', () => {
  it('requires a target project', () => {
    expect(validateSchedule('0 0 * * *', '')).toMatch(/project/i);
  });

  it('requires a cron expression', () => {
    expect(validateSchedule('  ', 'p1')).toMatch(/cron/i);
  });

  it('requires exactly five cron fields', () => {
    // A 6-field expression is the common paste-from-elsewhere mistake, and the
    // backend parser would take it and then never fire.
    expect(validateSchedule('0 0 * * * *', 'p1')).toMatch(/5 fields/);
    expect(validateSchedule('0 0 * *', 'p1')).toMatch(/5 fields/);
  });

  it('accepts a well-formed one', () => {
    expect(validateSchedule('0 0 * * *', 'p1')).toBeNull();
  });
});

describe('isCleared', () => {
  it('is true only when every field is empty', () => {
    expect(isCleared('', '', '')).toBe(true);
    expect(isCleared('   ', '  ', '')).toBe(true);
    expect(isCleared('0 0 * * *', '', '')).toBe(false);
    expect(isCleared('', '', 'p1')).toBe(false);
  });
});

describe('ScheduleDrawer', () => {
  it('creates a schedule', async () => {
    const user = userEvent.setup();
    const { onSaved, onClose } = renderDrawer(null);
    await screen.findByRole('option', { name: 'Demeteo' });

    await user.selectOptions(screen.getByLabelText('Target project'), 'p1');
    await user.type(screen.getByLabelText('Cron expression'), '0 3 * * 1');
    await user.type(screen.getByLabelText('Feature title template'), 'Weekly sweep');
    await user.click(screen.getByRole('button', { name: 'Save schedule' }));

    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith('workflow_save_schedule', {
        workflowId: 'wf-1',
        schedule: {
          cron: '0 3 * * 1',
          title_template: 'Weekly sweep',
          project_id: 'p1',
        },
      }),
    );
    expect(onSaved).toHaveBeenCalledWith(
      expect.objectContaining({ cron: '0 3 * * 1', project_id: 'p1' }),
    );
    expect(onClose).toHaveBeenCalled();
  });

  it('loads an existing schedule into the form', async () => {
    renderDrawer({ cron: '0 0 * * *', title_template: 'Nightly', project_id: 'p2' });
    await screen.findByRole('option', { name: 'Side quest' });

    expect(screen.getByLabelText('Cron expression')).toHaveValue('0 0 * * *');
    expect(screen.getByLabelText('Feature title template')).toHaveValue('Nightly');
    expect(screen.getByLabelText('Target project')).toHaveValue('p2');
  });

  it('clears a schedule by emptying every field', async () => {
    const user = userEvent.setup();
    const { onSaved } = renderDrawer({
      cron: '0 0 * * *',
      title_template: 'Nightly',
      project_id: 'p2',
    });
    await screen.findByRole('option', { name: 'Side quest' });

    await user.clear(screen.getByLabelText('Cron expression'));
    await user.clear(screen.getByLabelText('Feature title template'));
    await user.selectOptions(screen.getByLabelText('Target project'), '');
    // The button says what it will do, so "remove" is never a surprise.
    await user.click(screen.getByRole('button', { name: 'Remove schedule' }));

    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith('workflow_save_schedule', {
        workflowId: 'wf-1',
        schedule: null,
      }),
    );
    expect(onSaved).toHaveBeenCalledWith(null);
  });

  it('refuses a half-filled schedule instead of writing one that never fires', async () => {
    const user = userEvent.setup();
    renderDrawer(null);
    await screen.findByRole('option', { name: 'Demeteo' });

    await user.type(screen.getByLabelText('Cron expression'), '0 0 * * *');
    await user.click(screen.getByRole('button', { name: 'Save schedule' }));

    expect(reportError).toHaveBeenCalledWith(expect.stringMatching(/project/i), {
      kind: 'validation',
    });
    expect(invoke).not.toHaveBeenCalledWith('workflow_save_schedule', expect.anything());
  });
});
