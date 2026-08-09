// Integration tests for the Strategy tab's default-workflow picker — the
// place a project actually chooses the workflow its features start on.
//
// The contract the assertions here defend is the difference between "not
// chosen" and "chosen as nothing": `default_workflow_id` is `null` when the
// project has not picked one, never `''` or an absent key. Downstream, the
// launch modal falls back explicitly on `null` and the project header omits
// its workflow clause, so a `''` reaching the DB would read as a real choice.
//
// `invoke` is mocked globally in `src/test/setup.ts`; each test scripts it
// with a per-command router (the `scriptIpc` / `callsTo` idiom from
// `OverridesTab.test.tsx`).

import { invoke } from '@tauri-apps/api/core';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { useEffect, type ReactNode } from 'react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import ProjectSettingsView from './ProjectSettingsShell';
import { NavigationProvider, ProjectProvider, useProject } from '../../context';
import { ErrorBusProvider } from '../../lib/errorBus';

const mockedInvoke = vi.mocked(invoke);

const PROJECT_ID = 'p-1';

const WORKFLOWS = [
  { id: 'wf-standard', name: 'Standard Delivery', description: '', steps: [], version: 1, version_id: 'v1', is_starter: true, created_at: 0, updated_at: 0 },
  { id: 'wf-fast', name: 'Fast Path', description: '', steps: [], version: 1, version_id: 'v1', is_starter: false, created_at: 0, updated_at: 0 },
];

interface Scenario {
  /** What `project_settings.default_workflow_id` holds on disk. */
  storedWorkflowId?: string | null;
}

function scriptIpc(scenario: Scenario) {
  const handlers: Record<string, (args: Record<string, unknown>) => unknown> = {
    get_proposed_strategy: () => ({
      project_id: PROJECT_ID,
      worktree_strategy: { default_branch: 'main', branch_prefix: 'demeteo/features/' },
      conflict_policy: 'always_gate',
      feature_lifecycle: 'archive',
      default_agent_kind: null,
      default_model: null,
      default_effort: null,
      default_workflow_id: scenario.storedWorkflowId ?? null,
    }),
    get_repositories_for_project: () => [],
    get_machines: () => [],
    get_agent_configs: () => [],
    get_agent_models: () => [],
    list_agents: () => [],
    set_agent_configs: () => undefined,
    workflow_list: () => WORKFLOWS,
    probe_project_commands: () => ({ machine: 'local', commands: [], detail: null, guidance: '', blocks_launch: false }),
    update_project: () => undefined,
    save_project_settings: () => undefined,
  };
  mockedInvoke.mockImplementation((async (cmd: string, args?: unknown) => {
    const handler = handlers[cmd];
    if (!handler) throw new Error(`unscripted invoke('${cmd}')`);
    return handler((args ?? {}) as Record<string, unknown>);
  }) as typeof invoke);
}

function savedSettings(): Record<string, unknown> {
  const calls = mockedInvoke.mock.calls
    .filter(([name]) => name === 'save_project_settings')
    .map(([, args]) => (args ?? {}) as Record<string, unknown>);
  const last = calls[calls.length - 1];
  if (!last) throw new Error('save_project_settings was never called');
  return last.settings as Record<string, unknown>;
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
          // 'active' with an explicit empty remote_host keeps `handleSave` on
          // the plain save path: an 'idle'/'error' status or a `remote_host`
          // mismatch routes it into re-bootstrap instead, which never reaches
          // `save_project_settings` without a second confirmation.
          status: 'active',
          repos: 1,
          nodes: 1,
          spend: 0,
          tokens: 0,
          compute_type: 'local',
          remote_host: '',
        },
      });
      dispatch({ type: 'SET_CURRENT', id: PROJECT_ID });
    }
  }, [state.projects.length, dispatch]);

  if (state.currentProjectId !== PROJECT_ID) return null;
  return <>{children}</>;
}

async function mount(scenario: Scenario = {}) {
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
  await userEvent.click(await screen.findByRole('tab', { name: /Agent Strategy/ }));
  return (await screen.findByLabelText('Default Workflow')) as HTMLSelectElement;
}

async function save() {
  await userEvent.click(screen.getByRole('button', { name: /Save Changes/ }));
  await waitFor(() => savedSettings());
}

beforeEach(() => {
  mockedInvoke.mockReset();
});

describe('default workflow picker', () => {
  it('shows the stored workflow as the current selection', async () => {
    const select = await mount({ storedWorkflowId: 'wf-fast' });
    await waitFor(() => expect(select).toHaveValue('wf-fast'));
    expect(screen.getByRole('option', { name: 'Fast Path' })).toBeInTheDocument();
  });

  it('persists a chosen workflow', async () => {
    const select = await mount({ storedWorkflowId: null });
    await userEvent.selectOptions(select, 'wf-standard');

    await save();
    expect(savedSettings().default_workflow_id).toBe('wf-standard');
  });

  it('persists "not set" as null, not an empty string', async () => {
    const select = await mount({ storedWorkflowId: 'wf-fast' });
    await waitFor(() => expect(select).toHaveValue('wf-fast'));

    const notSet = screen.getByRole('option', { name: /Not set/ }) as HTMLOptionElement;
    await userEvent.selectOptions(select, notSet.value);
    expect(select).toHaveValue('');

    await save();
    expect(savedSettings()).toHaveProperty('default_workflow_id', null);
  });

  it('degrades a stored id no workflow answers to, and says which one went missing', async () => {
    const select = await mount({ storedWorkflowId: 'wf-deleted' });

    await waitFor(() => expect(screen.getByText(/wf-deleted/)).toBeInTheDocument());
    expect(select).toHaveValue('');

    await save();
    expect(savedSettings()).toHaveProperty('default_workflow_id', null);
  });
});
