// `docs/PRD_DISCOVERY.md` §5.4 locks a Ticket the moment it has a Feature.
// The drawer has to *show* that rather than take the edit and let the backend
// refuse it a round trip later, with the user's typing thrown away — so these
// pin the read-only rendering and the absence of a save at all.
//
// The unlocked half pins the other rule the wire depends on: `ticket_update`
// takes the whole ticket, because Rust reads an absent key and an explicit
// `null` identically.

import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { TicketEditorModal } from './TicketEditorModal';
import { indexTickets } from '../../lib/ticketPresentation';
import type {
  DiscoveryBoard,
  Machine,
  RunnerCompatibilityReport,
  RunPlacement,
  Ticket,
  TicketView,
} from '../../types';

const { navigate, reports } = vi.hoisted(() => ({
  navigate: vi.fn(),
  reports: new Map<string, RunnerCompatibilityReport>(),
}));

vi.mock('../../lib/discovery', () => ({
  getTicketBriefing: vi.fn(async () => 'DSC-2 has not landed.'),
  updateTicket: vi.fn(
    async () =>
      ({ tickets: [], progress: EMPTY_PROGRESS, discovery_default: { kind: 'local' }, local_host: 'local' }) as DiscoveryBoard,
  ),
  addTicketAttachment: vi.fn(),
  removeTicketAttachment: vi.fn(),
}));

vi.mock('../../hooks/useRunnerCompatibility', () => ({
  useRunnerCompatibility: (machineId: string) => ({
    report: reports.get(machineId) ?? null,
    loading: false,
    refresh: () => {},
  }),
}));

vi.mock('../../context', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../../context')>()),
  useNavigation: () => ({ navigate }),
}));

vi.mock('../../lib/agentModels', () => ({
  getAgentModels: vi.fn(async () => []),
  modelSupportsImages: vi.fn(() => true),
}));

vi.mock('../../lib/agentCatalog', () => ({
  useAgentCatalog: () => ({ agents: [{ kind: 'claude-code' }] }),
  effortLevelsFor: () => ['low', 'medium', 'high'],
}));

const box: Machine = { id: 'machine-box', name: 'box', host: 'box.lan', port: 22, username: 'dev', auth_type: 'key' };
const laptop: Machine = { id: 'local', name: 'laptop', host: '', port: 0, username: '', auth_type: 'local' };

const EMPTY_PROGRESS = { blocked: 0, ready: 0, in_flight: 0, landed: 0, dropped: 0, live: 0 };

afterEach(() => {
  cleanup();
  navigate.mockClear();
  reports.clear();
});

function ticket(extra: Partial<Ticket> = {}): Ticket {
  return {
    id: 't3',
    discovery_id: 'dsc-1',
    seq: 3,
    title: 'Multiplex run streams over one connection',
    description: 'Every client watches its own runs down a single connection.',
    acceptance: ['Two clients stream concurrently without interleaving'],
    files: ['crates/demeteo-runner/src/stream/mux.rs'],
    blocked_by: [],
    test_command: 'npm run checks:code',
    workflow_id: null,
    agent_kind: 'claude-code',
    model: 'opus',
    effort: 'high',
    machine_id: null,
    attachments: [],
    state: 'unstarted',
    drop_reason: null,
    force_start_reason: null,
    force_started_at: null,
    feature_id: null,
    created_at: 0,
    updated_at: 0,
    ...extra,
  };
}

function view(row: Ticket, lane: TicketView['standing']['lane'] = 'ready'): TicketView {
  return {
    ticket: row,
    standing: { id: row.id, lane, startable: lane === 'ready', blockers: [] },
    placement: { placement: { kind: 'local' }, inherited: true },
    feature: null,
  };
}

function renderDrawer(
  subject: TicketView,
  discoveryDefault: RunPlacement = { kind: 'local' },
  localHost = 'local',
) {
  const onSaved = vi.fn();
  const drawer = (shown: TicketView) => (
    <TicketEditorModal
      view={shown}
      index={indexTickets([subject])}
      siblings={[subject]}
      workflows={[]}
      machines={[box, laptop]}
      discoveryDefault={discoveryDefault}
      localHost={localHost}
      busy={false}
      onClose={() => {}}
      onSaved={onSaved}
      onRefresh={() => {}}
      onStart={() => {}}
      onForceStart={() => {}}
      onDrop={() => {}}
    />
  );
  const { rerender } = render(drawer(subject));
  return { onSaved, rerender: (next: TicketView) => rerender(drawer(next)) };
}

describe('a locked ticket', () => {
  it('is shown as locked, with no save to fail', () => {
    renderDrawer(view(ticket({ state: 'started', feature_id: 'f-1' }), 'in_flight'));

    expect(screen.getByTestId('ticket-locked')).toBeTruthy();
    expect(screen.queryByTestId('ticket-save')).toBeNull();
  });

  it('locks on the feature id alone, before the state has caught up', () => {
    renderDrawer(view(ticket({ feature_id: 'f-1' })));

    expect(screen.getByTestId('ticket-locked')).toBeTruthy();
  });

  it('leaves every field read-only', () => {
    renderDrawer(view(ticket({ state: 'started', feature_id: 'f-1' }), 'in_flight'));

    expect(screen.getByLabelText('Title').hasAttribute('disabled')).toBe(true);
    expect(screen.getByLabelText('Description').hasAttribute('disabled')).toBe(true);
    expect(screen.getByLabelText('Acceptance 1').hasAttribute('disabled')).toBe(true);
  });

  it('offers no dropzone — its attachments went to the feature when it started', () => {
    renderDrawer(view(ticket({ state: 'started', feature_id: 'f-1' }), 'in_flight'));

    expect(screen.queryByText(/or drop here/)).toBeNull();
  });
});

describe('an unstarted ticket', () => {
  it('says every field is yours, and offers a save once something changes', () => {
    renderDrawer(view(ticket()));

    expect(screen.queryByTestId('ticket-locked')).toBeNull();
    const save = screen.getByTestId('ticket-save');
    expect(save.hasAttribute('disabled')).toBe(true);

    fireEvent.change(screen.getByLabelText('Title'), { target: { value: 'A new title' } });

    expect(save.hasAttribute('disabled')).toBe(false);
  });

  it('saves the whole ticket, every key present', async () => {
    const { updateTicket } = await import('../../lib/discovery');
    renderDrawer(view(ticket()));

    fireEvent.change(screen.getByLabelText('Title'), { target: { value: 'A new title' } });
    fireEvent.click(screen.getByTestId('ticket-save'));

    await waitFor(() => expect(updateTicket).toHaveBeenCalled());
    expect(vi.mocked(updateTicket).mock.calls[0][1]).toEqual({
      title: 'A new title',
      description: 'Every client watches its own runs down a single connection.',
      acceptance: ['Two clients stream concurrently without interleaving'],
      files: ['crates/demeteo-runner/src/stream/mux.rs'],
      blocked_by: [],
      test_command: 'npm run checks:code',
      workflow_id: null,
      agent_kind: 'claude-code',
      model: 'opus',
      effort: 'high',
      machine_id: null,
    });
  });

  it('takes the board the save returns rather than patching a row', async () => {
    const { onSaved } = renderDrawer(view(ticket()));

    fireEvent.change(screen.getByLabelText('Title'), { target: { value: 'A new title' } });
    fireEvent.click(screen.getByTestId('ticket-save'));

    await waitFor(() =>
      expect(onSaved).toHaveBeenCalledWith({
        tickets: [],
        progress: EMPTY_PROGRESS,
        discovery_default: { kind: 'local' },
        local_host: 'local',
      }),
    );
  });

  it('shows what its agent will be told, composed by the backend', async () => {
    renderDrawer(view(ticket()));

    await waitFor(() => expect(screen.getByText('DSC-2 has not landed.')).toBeTruthy());
  });
});

describe('where a ticket runs', () => {
  async function savedMachineId(subject: TicketView, choice: string): Promise<unknown> {
    const { updateTicket } = await import('../../lib/discovery');
    vi.mocked(updateTicket).mockClear();
    renderDrawer(subject);

    fireEvent.change(screen.getByLabelText('Where to run'), { target: { value: choice } });
    fireEvent.click(screen.getByTestId('ticket-save'));

    await waitFor(() => expect(updateTicket).toHaveBeenCalled());
    return vi.mocked(updateTicket).mock.calls[0][1].machine_id;
  }

  it('offers Default named by the Discovery default, Local, and each remote machine detached', () => {
    renderDrawer(
      {
        ...view(ticket()),
        placement: { placement: { kind: 'detached', machine_id: box.id }, inherited: true },
      },
      { kind: 'detached', machine_id: box.id },
    );

    const options = screen.getAllByRole('option').filter((o) => o.closest('#ticket-placement'));
    expect(options.map((o) => o.textContent)).toEqual([
      'Default (Detached · box)',
      'Local',
      'box — detached',
    ]);
  });

  it('names Default from the board even when no ticket in the plan inherits', () => {
    renderDrawer(
      {
        ...view(ticket({ machine_id: 'local' })),
        placement: { placement: { kind: 'local' }, inherited: false },
      },
      { kind: 'detached', machine_id: box.id },
    );

    const options = screen.getAllByRole('option').filter((o) => o.closest('#ticket-placement'));
    expect(options[0].textContent).toBe('Default (Detached · box)');
  });

  it('saves a chosen machine as its id', async () => {
    expect(await savedMachineId(view(ticket()), box.id)).toBe(box.id);
  });

  it('saves Default as null', async () => {
    const explicit: TicketView = {
      ...view(ticket({ machine_id: box.id })),
      placement: { placement: { kind: 'detached', machine_id: box.id }, inherited: false },
    };
    expect(await savedMachineId(explicit, '')).toBeNull();
  });

  it('saves Local as an explicit "local"', async () => {
    expect(await savedMachineId(view(ticket()), 'local')).toBe('local');
  });

  it('is read-only on a locked ticket', () => {
    renderDrawer(view(ticket({ state: 'started', feature_id: 'f-1' }), 'in_flight'));

    expect(screen.getByLabelText('Where to run').hasAttribute('disabled')).toBe(true);
  });

  it('routes an incompatible runner to machine settings, as the Launch dialog does', () => {
    reports.set(box.id, {
      verdict: 'runner_behind',
      runner: '1.1.0',
      runner_channel: 'stable',
      app: '1.2.0',
      app_channel: 'stable',
      message: 'demeteo-runner 1.1.0 (stable) on box is older than Demeteo 1.2.0 (stable).',
    });
    renderDrawer(view(ticket()));

    fireEvent.change(screen.getByLabelText('Where to run'), { target: { value: box.id } });
    fireEvent.click(screen.getByRole('button', { name: 'Open machine settings' }));

    expect(navigate).toHaveBeenCalledWith({ kind: 'settings' });
  });
});

// The Discovery's own host is `'local'` in every render here, so a probe sent
// there instead of to the draft's placement shows up as that id.
describe('the agent and model probe', () => {
  const BUILD_HOST = 'machine-build';

  async function probedMachines(): Promise<string[]> {
    const { getAgentModels } = await import('../../lib/agentModels');
    return vi.mocked(getAgentModels).mock.calls.map(([machineId]) => machineId);
  }

  afterEach(async () => {
    const { getAgentModels } = await import('../../lib/agentModels');
    vi.mocked(getAgentModels).mockClear();
  });

  it('asks the project compute host when the draft runs Local', async () => {
    renderDrawer(view(ticket()), { kind: 'local' }, BUILD_HOST);

    await waitFor(async () => expect(await probedMachines()).toEqual([BUILD_HOST]));
  });

  it('asks the detached machine a Default draft resolves to', async () => {
    renderDrawer(view(ticket()), { kind: 'detached', machine_id: box.id }, BUILD_HOST);

    await waitFor(async () => expect(await probedMachines()).toEqual([box.id]));
  });

  it('re-probes on the machine the placement is changed to', async () => {
    renderDrawer(view(ticket()), { kind: 'detached', machine_id: 'machine-gpu' }, BUILD_HOST);
    await waitFor(async () => expect(await probedMachines()).toEqual(['machine-gpu']));

    fireEvent.change(screen.getByLabelText('Where to run'), { target: { value: box.id } });
    await waitFor(async () => expect((await probedMachines()).slice(-1)[0]).toBe(box.id));

    fireEvent.change(screen.getByLabelText('Where to run'), { target: { value: 'local' } });
    await waitFor(async () => expect((await probedMachines()).slice(-1)[0]).toBe(BUILD_HOST));
  });
});

// `ticket_start` launches the *stored* row, so a Start pressed beside an
// unsaved draft would run on choices the drawer no longer shows — most
// expensively a placement, where the picker has just shown a compatibility
// verdict for a machine that will not be used.
describe('starting from the drawer', () => {
  const HINT = 'Save before starting.';

  function disabled(testId: string): boolean {
    return screen.getByTestId(testId).hasAttribute('disabled');
  }

  it('holds Start while the placement is unsaved, and releases it once saved', () => {
    const { rerender } = renderDrawer(view(ticket()));
    expect(disabled('ticket-primary-action')).toBe(false);
    expect(screen.queryByText(HINT)).toBeNull();

    fireEvent.change(screen.getByLabelText('Where to run'), { target: { value: box.id } });

    expect(disabled('ticket-primary-action')).toBe(true);
    expect(screen.getByText(HINT)).toBeTruthy();

    rerender(view(ticket({ machine_id: box.id, updated_at: 1 })));

    expect(disabled('ticket-primary-action')).toBe(false);
    expect(screen.queryByText(HINT)).toBeNull();
  });

  it('holds Force start while the placement is unsaved, and leaves Drop alone', () => {
    renderDrawer(view(ticket(), 'blocked'));
    expect(disabled('ticket-force-start')).toBe(false);

    fireEvent.change(screen.getByLabelText('Where to run'), { target: { value: box.id } });

    expect(disabled('ticket-force-start')).toBe(true);
    expect(disabled('ticket-drop')).toBe(false);
    expect(screen.getByText(HINT)).toBeTruthy();
  });

  it('holds a force start already being confirmed when the draft moves under it', () => {
    renderDrawer(view(ticket(), 'blocked'));
    fireEvent.click(screen.getByTestId('ticket-force-start'));
    fireEvent.change(screen.getByLabelText('Reason for bypassing the prerequisites'), {
      target: { value: 'Merged out of band this morning.' },
    });
    expect(disabled('ticket-force-confirm')).toBe(false);

    fireEvent.change(screen.getByLabelText('Where to run'), { target: { value: box.id } });

    expect(disabled('ticket-force-confirm')).toBe(true);
  });

  it('holds Start for any unsaved field, not the placement alone', () => {
    renderDrawer(view(ticket()));

    fireEvent.change(screen.getByLabelText('Title'), { target: { value: 'A new title' } });

    expect(disabled('ticket-primary-action')).toBe(true);
    expect(screen.getByText(HINT)).toBeTruthy();
  });

  it('releases Start once the draft matches the stored ticket again', () => {
    renderDrawer(view(ticket()));

    fireEvent.change(screen.getByLabelText('Where to run'), { target: { value: box.id } });
    fireEvent.change(screen.getByLabelText('Where to run'), { target: { value: '' } });

    expect(disabled('ticket-primary-action')).toBe(false);
  });
});
