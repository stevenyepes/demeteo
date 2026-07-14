// Integration tests for the project-settings Overrides tab, which is where a
// user pins a harness / model / effort for a whole workflow or a single step.
//
// The three assertions here are the ones the effort feature actually rests on:
//
//   1. Picking an effort persists it through `set_workflow_override` with the
//      canonical lowercase level — the same spelling Rust's serde emits.
//
//   2. The effort control is disabled when the effective harness declares no
//      effort levels (hermes). The list is data-driven off `list_agents`, so a
//      hardcoded per-agent list in the UI would fail here.
//
//   3. A row that pins nothing shows what it *inherits* as its placeholder,
//      rather than a blank that hides which effort will really run.
//
// `invoke` is mocked globally in `src/test/setup.ts`; each test scripts it with
// a per-command router (the `scriptIpc` / `callsTo` idiom from
// `useCreateZeroWizardForm.test.tsx`).

import { invoke } from '@tauri-apps/api/core';
import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { useEffect, type ReactNode } from 'react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { OverridesTab } from './OverridesTab';
import { ProjectSettingsProvider, useSettings } from './ProjectSettingsContext';
import { NavigationProvider, ProjectProvider, useProject } from '../../context';
import { ErrorBusProvider } from '../../lib/errorBus';
import type { WorkflowOverride } from '../../types';

const mockedInvoke = vi.mocked(invoke);

const PROJECT_ID = 'p-1';
const WORKFLOW_ID = 'wf-1';

const WORKFLOW = {
  id: WORKFLOW_ID,
  name: 'Standard',
  description: 'Research → implement',
  steps: [{ id: 's-implement', kind: 'agent', title: 'Implement' }],
};

// The backend catalog: claude-code takes the whole ladder, hermes takes none.
const CATALOG = [
  {
    kind: 'claude-code',
    display_label: 'Claude Code',
    lists_models: true,
    default_model: null,
    install_command: '',
    effort_levels: ['low', 'medium', 'high', 'xhigh', 'max'],
  },
  {
    kind: 'hermes',
    display_label: 'Hermes',
    lists_models: false,
    default_model: null,
    install_command: '',
    effort_levels: [],
  },
];

const AGENT_CONFIGS = CATALOG.map((a) => ({
  kind: a.kind,
  enabled: true,
  available: true,
  install_command: '',
  display_label: a.display_label,
}));

interface Scenario {
  overrides?: WorkflowOverride[];
  defaultAgentKind?: string | null;
  defaultEffort?: string | null;
}

// Routes `invoke(cmd, args)` to a scripted handler. Anything unscripted throws,
// so a stray command surfaces as a failure rather than a silent `undefined`.
function scriptIpc(scenario: Scenario) {
  const handlers: Record<string, (args: Record<string, unknown>) => unknown> = {
    get_proposed_strategy: () => ({
      project_id: PROJECT_ID,
      worktree_strategy: { default_branch: 'main', branch_prefix: 'demeteo/features/' },
      conflict_policy: 'always_gate',
      feature_lifecycle: 'archive',
      default_agent_kind: scenario.defaultAgentKind ?? 'claude-code',
      default_model: null,
      default_effort: scenario.defaultEffort ?? null,
    }),
    get_repositories_for_project: () => [],
    get_machines: () => [],
    get_agent_configs: () => AGENT_CONFIGS,
    get_agent_models: () => [],
    list_agents: () => CATALOG,
    workflow_list: () => [WORKFLOW],
    get_workflow_overrides: () => scenario.overrides ?? [],
    set_workflow_override: () => undefined,
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

function lastCallTo(cmd: string): Record<string, unknown> | undefined {
  const calls = callsTo(cmd);
  return calls[calls.length - 1];
}

// Seeds the ProjectContext with the active project, then parks the settings
// provider on the Overrides tab (the tab is what triggers the workflow +
// override fetch).
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
          compute_type: 'local',
        },
      });
      dispatch({ type: 'SET_CURRENT', id: PROJECT_ID });
    }
  }, [state.projects.length, dispatch]);

  if (state.currentProjectId !== PROJECT_ID) return null;
  return (
    <ProjectSettingsProvider>
      <OnOverridesTab>{children}</OnOverridesTab>
    </ProjectSettingsProvider>
  );
}

function OnOverridesTab({ children }: { children: ReactNode }) {
  const { activeTab, setActiveTab } = useSettings();
  useEffect(() => {
    if (activeTab !== 'overrides') setActiveTab('overrides');
  }, [activeTab, setActiveTab]);
  return activeTab === 'overrides' ? <>{children}</> : null;
}

function mount(scenario: Scenario) {
  scriptIpc(scenario);
  return render(
    <ErrorBusProvider>
      <NavigationProvider>
        <ProjectProvider>
          <Harness>
            <OverridesTab />
          </Harness>
        </ProjectProvider>
      </NavigationProvider>
    </ErrorBusProvider>,
  );
}

/** Expand the workflow card so its override rows render. */
async function expandWorkflow() {
  const header = await screen.findByRole('button', { name: /Standard/ });
  await userEvent.click(header);
  await waitFor(() => expect(screen.getAllByLabelText('Effort').length).toBeGreaterThan(0));
}

beforeEach(() => {
  mockedInvoke.mockReset();
});

describe('picking an effort', () => {
  it('persists it as a workflow-level override on the canonical ladder', async () => {
    mount({});
    await expandWorkflow();

    // The first row is the workflow-level one ("applies to all steps"), which
    // the backend keys with a null step_id.
    const [workflowEffort] = screen.getAllByLabelText('Effort');
    await userEvent.selectOptions(workflowEffort, 'xhigh');

    await waitFor(() => expect(callsTo('set_workflow_override')).toHaveLength(1));
    expect(lastCallTo('set_workflow_override')).toEqual({
      projectId: PROJECT_ID,
      workflowId: WORKFLOW_ID,
      stepId: null,
      agentKind: null,
      model: null,
      // The lowercase spelling Rust's `#[serde(rename_all = "lowercase")]`
      // emits — `XHigh` is "xhigh", never "x-high".
      effort: 'xhigh',
    });
  });

  it('keeps the effort when the row switches harness', async () => {
    mount({
      overrides: [
        { project_id: PROJECT_ID, workflow_id: WORKFLOW_ID, step_id: null, effort: 'max' },
      ],
    });
    // A seeded override auto-expands its workflow — no click needed.
    await waitFor(() => expect(screen.getAllByLabelText('Effort').length).toBeGreaterThan(0));

    const [harness] = screen.getAllByLabelText('Harness');
    await userEvent.selectOptions(harness, 'claude-code');

    // The model is namespaced to the old harness and is dropped; the effort
    // ladder is canonical across agents, so it survives the switch.
    await waitFor(() => expect(callsTo('set_workflow_override')).toHaveLength(1));
    expect(lastCallTo('set_workflow_override')).toMatchObject({
      agentKind: 'claude-code',
      model: null,
      effort: 'max',
    });
  });
});

describe('an agent with no effort levels', () => {
  it('disables the effort select and says why', async () => {
    mount({
      overrides: [
        {
          project_id: PROJECT_ID,
          workflow_id: WORKFLOW_ID,
          step_id: null,
          agent_kind: 'hermes',
        },
      ],
    });
    // A seeded override auto-expands its workflow.
    await waitFor(() => expect(screen.getAllByLabelText('Effort').length).toBeGreaterThan(0));

    const [workflowEffort] = screen.getAllByLabelText('Effort');
    expect(workflowEffort).toBeDisabled();
    expect(workflowEffort).toHaveAttribute(
      'title',
      'hermes does not support effort selection',
    );
    // …and it offers nothing, rather than a level hermes would silently drop.
    expect(within(workflowEffort).queryByRole('option', { name: 'High' })).toBeNull();
  });
});

describe('a row that pins nothing', () => {
  it('shows the effort it inherits as its placeholder', async () => {
    mount({
      defaultEffort: 'low',
      overrides: [
        // The workflow-level row pins `max`, so the (unset) step row inherits
        // it — not the project default of `low` underneath.
        { project_id: PROJECT_ID, workflow_id: WORKFLOW_ID, step_id: null, effort: 'max' },
      ],
    });
    await waitFor(() => expect(screen.getAllByLabelText('Effort')).toHaveLength(2));

    const [workflowEffort, stepEffort] = screen.getAllByLabelText('Effort');
    // The workflow-level row inherits straight from the project default.
    expect(within(workflowEffort).getByRole('option', { name: 'Inherit · Low' })).toBeInTheDocument();
    expect(within(stepEffort).getByRole('option', { name: 'Inherit · Max' })).toBeInTheDocument();
    expect(stepEffort).toHaveValue('');
  });
});
