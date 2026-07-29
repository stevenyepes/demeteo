// Integration tests for the Strategy tab's harness section (HB6) — the panel
// where a project's commands are authored, probed, and selected as validation
// gates.
//
// The claims these rest on:
//
//   1. A binary the machine found renders resolved and one it did not renders
//      missing — per command, so a healthy lint gate is not painted red beside
//      a broken unit gate.
//
//   2. The panel says *which machine* it asked. On a remote-compute project
//      the commands run on the runner, not on the laptop showing the panel, so
//      an indicator that omits this is a lie on exactly those projects.
//
//   3. Ticking "gates validation" persists `validation_gates` — tier 2 of the
//      engine's resolution chain, and the only thing that makes the harnesses
//      map reachable at all, since every shipped starter names no harness.
//
//   4. Reordering persists in the new order: cheap gates first is the user's
//      call, and `harnesses` is a map with no order to inherit.
//
//   5. A probe that cannot answer does not block a save. Configuring a command
//      for a machine you are not sitting at is legitimate; the gate stays at
//      launch, where which machine will run it is known.
//
// `invoke` is mocked globally in `src/test/setup.ts`; each test scripts it with
// a per-command router (the `scriptIpc` / `callsTo` idiom from
// `OverridesTab.test.tsx`).

import { invoke } from '@tauri-apps/api/core';
import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { useEffect, type ReactNode } from 'react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import ProjectSettingsView from './ProjectSettingsShell';
import { NavigationProvider, ProjectProvider, useProject } from '../../context';
import { ErrorBusProvider } from '../../lib/errorBus';
import type { CommandProbeReport } from '../../lib/project';

const mockedInvoke = vi.mocked(invoke);

const PROJECT_ID = 'p-1';

const HARNESSES = { lint: 'npm run lint', unit: 'cargo test' };

/** What the engine reports for `HARNESSES` on a machine with npm but no cargo. */
const PROBE: CommandProbeReport = {
  machine: 'runner-01',
  commands: [
    { source: 'test', harness: null, command: 'npm test', binaries: [{ name: 'npm', resolved: true }] },
    { source: 'harness', harness: 'lint', command: 'npm run lint', binaries: [{ name: 'npm', resolved: true }] },
    { source: 'harness', harness: 'unit', command: 'cargo test', binaries: [{ name: 'cargo', resolved: false }] },
  ],
  detail: "The project's configured commands name a binary the login shell cannot find: cargo.\nCheck with:\n  bash -l -i -c 'command -v cargo'",
  guidance: 'Run the command below in a *fresh* checkout — that is what this step gets, with no `node_modules` and no `target/`.',
  blocks_launch: true,
};

interface Scenario {
  harnesses?: Record<string, string>;
  validationGates?: string[] | null;
  probe?: CommandProbeReport;
  /** When set, `probe_project_commands` rejects with this message. */
  probeError?: string;
}

// Routes `invoke(cmd, args)` to a scripted handler. Anything unscripted throws,
// so a stray command surfaces as a failure rather than a silent `undefined`.
function scriptIpc(scenario: Scenario) {
  const handlers: Record<string, (args: Record<string, unknown>) => unknown> = {
    get_proposed_strategy: () => ({
      project_id: PROJECT_ID,
      worktree_strategy: {
        default_branch: 'main',
        branch_prefix: 'demeteo/features/',
        test_command: 'npm test',
        harnesses: scenario.harnesses ?? HARNESSES,
        validation_gates: scenario.validationGates ?? null,
      },
      conflict_policy: 'always_gate',
      feature_lifecycle: 'archive',
    }),
    get_repositories_for_project: () => [],
    get_machines: () => [{ id: 'runner-01', name: 'build-box', host: '10.0.0.5', port: 22, username: 'ci', auth_type: 'key' }],
    get_agent_configs: () => [],
    list_agents: () => [],
    set_agent_configs: () => undefined,
    update_project: () => undefined,
    save_project_settings: () => undefined,
    probe_project_commands: () => {
      if (scenario.probeError) throw new Error(scenario.probeError);
      return scenario.probe ?? PROBE;
    },
  };
  mockedInvoke.mockImplementation((async (cmd: string, args?: unknown) => {
    const handler = handlers[cmd];
    if (!handler) throw new Error(`unscripted invoke('${cmd}')`);
    return handler((args ?? {}) as Record<string, unknown>);
  }) as typeof invoke);
}

function callsTo(cmd: string): Record<string, unknown>[] {
  return mockedInvoke.mock.calls
    .filter(([name]) => name === cmd)
    .map(([, args]) => (args ?? {}) as Record<string, unknown>);
}

/** The `worktree_strategy` of the last settings save. */
function lastSavedStrategy(): Record<string, unknown> | undefined {
  const calls = callsTo('save_project_settings');
  const last = calls[calls.length - 1];
  const settings = last?.settings as { worktree_strategy?: Record<string, unknown> } | undefined;
  return settings?.worktree_strategy;
}

function Harness({ children }: { children: ReactNode }) {
  const { state, dispatch } = useProject();
  useEffect(() => {
    if (state.projects.length === 0) {
      dispatch({
        type: 'ADD_PROJECT',
        project: {
          id: PROJECT_ID,
          name: 'Demeteo',
          // Anything but 'idle' — an idle project triggers a workspace-health
          // probe this test has no interest in.
          status: 'active',
          repos: 1,
          nodes: 1,
          spend: 0,
          tokens: 0,
          compute_type: 'remote',
          remote_host: 'runner-01',
        },
      });
      dispatch({ type: 'SET_CURRENT', id: PROJECT_ID });
    }
  }, [state.projects.length, dispatch]);

  if (state.currentProjectId !== PROJECT_ID) return null;
  return <>{children}</>;
}

/** Mount the real settings view and land on the Strategy tab. */
async function mount(scenario: Scenario) {
  scriptIpc(scenario);
  render(
    <ErrorBusProvider>
      <NavigationProvider>
        <ProjectProvider>
          <Harness>
            <ProjectSettingsView />
          </Harness>
        </ProjectProvider>
      </NavigationProvider>
    </ErrorBusProvider>,
  );
  await userEvent.click(await screen.findByRole('button', { name: /Agent Strategy/ }));
  await screen.findByText('Validation Harnesses');
}

/** The row of the harness table for `name`. */
function harnessRow(name: string): HTMLElement {
  return screen.getByLabelText(`${name} gates validation`).closest('div[class*="rounded-lg"]') as HTMLElement;
}

async function save() {
  await userEvent.click(screen.getByRole('button', { name: /Save Changes/ }));
  await waitFor(() => expect(callsTo('save_project_settings').length).toBeGreaterThan(0));
}

beforeEach(() => {
  mockedInvoke.mockReset();
});

describe('the probe indicator', () => {
  it('marks each command resolved or missing without leaving the panel', async () => {
    await mount({});
    await waitFor(() => expect(callsTo('probe_project_commands')).toHaveLength(1));

    // npm is there, cargo is not — and the two gates say so independently.
    await waitFor(() => expect(within(harnessRow('lint')).getByText('resolved')).toBeInTheDocument());
    expect(within(harnessRow('lint')).getByText('npm')).toBeInTheDocument();
    expect(within(harnessRow('unit')).getByText('missing')).toBeInTheDocument();
    expect(within(harnessRow('unit')).getByText('cargo')).toBeInTheDocument();
  });

  it('names the machine it asked, and renders the engine\'s own message', async () => {
    await mount({});
    // The project is remote: the commands run on the runner, not here.
    await waitFor(() => expect(screen.getByText(/checked on build-box/)).toBeInTheDocument());
    // Verbatim, so the panel and a blocked launch cannot disagree — including
    // the reproduce line, in the shell that actually matters.
    expect(screen.getByText(/bash -l -i -c 'command -v cargo'/)).toBeInTheDocument();
    // …and the engine's own sentence about what the command has to survive:
    // a fresh worktree with no `node_modules` and no `target/`.
    expect(screen.getByText(/fresh\* checkout/)).toBeInTheDocument();
  });

  it('does not block a save when a binary is missing', async () => {
    // The whole "indicator, not a gate" claim: a command may legitimately be
    // configured for a machine the user is not sitting at. The gate stays at
    // launch, where which machine will run it is known.
    await mount({});
    await waitFor(() => expect(screen.getByText(/checked on build-box/)).toBeInTheDocument());
    expect(screen.getByText('missing')).toBeInTheDocument();

    await save();
    expect(lastSavedStrategy()?.test_command).toBe('npm test');
  });

  it('probes the command that was typed, not the one that was saved', async () => {
    await mount({});
    await waitFor(() => expect(callsTo('probe_project_commands')).toHaveLength(1));

    await userEvent.clear(screen.getByLabelText('Default Test Command'));
    await userEvent.type(screen.getByLabelText('Default Test Command'), 'nosuchtool');

    await waitFor(
      () => {
        const probes = callsTo('probe_project_commands');
        const last = probes[probes.length - 1];
        expect((last?.draft as { test_command?: string })?.test_command).toBe('nosuchtool');
      },
      { timeout: 3000 },
    );
    // …and it never went near the DB to find out.
    expect(callsTo('save_project_settings')).toHaveLength(0);
  });
});

describe('gating validation', () => {
  it('persists a ticked harness as an ordered selection', async () => {
    await mount({});
    await userEvent.click(screen.getByLabelText('lint gates validation'));
    await save();
    expect(lastSavedStrategy()?.validation_gates).toEqual(['lint']);
  });

  it('persists the order the user put the gates in', async () => {
    // lint runs before unit today; the user wants the cheap one first is the
    // usual case, so prove the *other* order survives a save.
    await mount({ validationGates: ['lint', 'unit'] });
    await userEvent.click(screen.getByRole('button', { name: 'Run unit earlier' }));
    await save();
    expect(lastSavedStrategy()?.validation_gates).toEqual(['unit', 'lint']);
  });

  it('clears the selection back to unset when the last gate is unticked', async () => {
    // Not the same as saving an empty list: the engine reads an *absent*
    // selection as "fall through to test_command", and the DB keeps writing
    // its pre-HB5 column shape for such a project.
    await mount({ validationGates: ['lint'] });
    await userEvent.click(screen.getByLabelText('lint gates validation'));
    await save();
    expect(lastSavedStrategy()?.validation_gates).toBeNull();
  });

  it('does not resurrect a gate when a deleted harness is added back', async () => {
    // Deleting the harness retires the tick as well. Otherwise re-adding a
    // command under a name used before would silently gate every workflow with
    // it, which nobody asked for and nothing in the panel would show.
    await mount({ validationGates: ['lint', 'unit'] });
    await userEvent.click(screen.getByRole('button', { name: 'Delete unit harness' }));
    await userEvent.type(screen.getByLabelText('New harness name'), 'unit');
    await userEvent.type(screen.getByLabelText('New harness command'), 'cargo test --lib');
    await userEvent.click(screen.getByRole('button', { name: 'Add harness' }));

    expect(screen.getByLabelText('unit gates validation')).not.toBeChecked();
    await save();
    expect(lastSavedStrategy()?.validation_gates).toEqual(['lint']);
  });

  it('does not re-persist a stored tick whose harness no longer exists', async () => {
    // A stale name is not an authored declaration — the engine drops names its
    // harnesses map no longer defines, so writing one back keeps alive a
    // selection nothing can honour.
    await mount({ validationGates: ['lint', 'ghost'] });
    await save();
    expect(lastSavedStrategy()?.validation_gates).toEqual(['lint']);
  });
});

describe('a probe that cannot answer', () => {
  it('says so and still lets the settings be saved', async () => {
    await mount({ probeError: 'machine unreachable' });
    await waitFor(() => expect(screen.getByText(/could not be checked right now/)).toBeInTheDocument());

    await userEvent.click(screen.getByLabelText('lint gates validation'));
    await save();
    // The gate is at launch, where the machine is known — not here.
    expect(lastSavedStrategy()?.validation_gates).toEqual(['lint']);
  });
});
