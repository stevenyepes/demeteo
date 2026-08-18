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
  /** What `project_settings.review_entrypoint` holds on disk. */
  storedReviewEntrypoint?: string | null;
  /** What `project_settings.sync_resolver_agent_kind` holds on disk. */
  storedSyncResolverAgentKind?: string | null;
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
      review_entrypoint: scenario.storedReviewEntrypoint ?? null,
      sync_resolver_agent_kind: scenario.storedSyncResolverAgentKind ?? null,
      sync_resolver_model: null,
      sync_resolver_effort: null,
    }),
    get_repositories_for_project: () => [],
    get_machines: () => [],
    get_agent_configs: () => [
      { kind: 'opencode', enabled: true, available: true, install_command: '', display_label: 'Opencode' },
      { kind: 'codex', enabled: true, available: true, install_command: '', display_label: 'Codex' },
    ],
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

// The field the user owns, on the save path that carries it. Both save call
// sites spell the whole settings object out, so a field the form holds and the
// literal omits is dropped without a type error anywhere.
describe('code review entrypoint', () => {
  const field = () => screen.getByLabelText('Code review entrypoint') as HTMLInputElement;

  it('shows the stored entrypoint and persists an edit', async () => {
    await mount({ storedReviewEntrypoint: '/code-review' });
    await waitFor(() => expect(field()).toHaveValue('/code-review'));

    await userEvent.clear(field());
    await userEvent.type(field(), '/review --deep');

    await save();
    expect(savedSettings().review_entrypoint).toBe('/review --deep');
  });

  it('persists a cleared field as null, not an empty string', async () => {
    await mount({ storedReviewEntrypoint: '/code-review' });
    await waitFor(() => expect(field()).toHaveValue('/code-review'));

    await userEvent.clear(field());

    await save();
    expect(savedSettings()).toHaveProperty('review_entrypoint', null);
  });
});

// The same two-call-site trap as the entrypoint above, on a control whose
// blank state is a third meaning: not "none" and not "the run's harness", but
// "inherit" — which is why it must reach the DB as null rather than ''.
describe('sync conflict resolver default', () => {
  const harness = () => screen.getByLabelText('Harness') as HTMLSelectElement;

  it('shows the stored resolver and persists a change', async () => {
    await mount({ storedSyncResolverAgentKind: 'opencode' });
    await waitFor(() => expect(harness()).toHaveValue('opencode'));

    await userEvent.selectOptions(harness(), 'codex');

    await save();
    expect(savedSettings().sync_resolver_agent_kind).toBe('codex');
  });

  it('persists an untouched picker as null, not an empty string', async () => {
    await mount({ storedSyncResolverAgentKind: null });
    await waitFor(() => expect(harness()).toHaveValue(''));

    await save();
    expect(savedSettings()).toHaveProperty('sync_resolver_agent_kind', null);
  });
});
