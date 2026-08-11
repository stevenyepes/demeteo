// Regression tests for `proceedWithReBootstrap` — the function behind
// `handleSave`'s auto-trigger into re-bootstrap, and GeneralTab's
// "Re-run Bootstrap" / "Re-bootstrap" buttons.
//
// The bug: after a user edits the Default Test Command field and clicks
// "Save Changes" on a project that needs re-bootstrapping, the handler
// re-fetches a *stale* `get_proposed_strategy` result and a freshly
// re-detected `bootstrap_project` strategy, then overwrites the in-scope
// state — including whatever the user just typed — with
// `ext?.field ?? strategy.field ?? ''`. The user's edit is silently
// discarded and the "Approve Detected Worktree Strategy" pop-up shows a
// stale or re-detected value instead.
//
// `invoke` is mocked globally in `src/test/setup.ts`; each test scripts it
// with a per-command router (the `scriptIpc` / `callsTo` idiom from
// `OverridesTab.test.tsx` / `HarnessesSection.test.tsx`).

import { invoke } from '@tauri-apps/api/core';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { useEffect, type ReactNode } from 'react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import ProjectSettingsView from './ProjectSettingsShell';
import { NavigationProvider, ProjectProvider, useProject } from '../../context';
import { ErrorBusProvider } from '../../lib/errorBus';

const mockedInvoke = vi.mocked(invoke);

const PROJECT_ID = 'p-1';

/** What `bootstrap_project` re-detects from a fresh scan of the repo. */
const STRATEGY_TEST_COMMAND = 'B: freshly re-detected command';
/** What the stale `get_proposed_strategy` "existing" record still has on file. */
const EXISTING_TEST_COMMAND = 'A: stale DB command';

interface Scenario {
  /** Project status that routes `handleSave` into `proceedWithReBootstrap`. */
  status?: 'idle' | 'active' | 'error' | 'bootstrapping';
}

// Routes `invoke(cmd, args)` to a scripted handler. Anything unscripted
// throws, so a stray command surfaces as a failure rather than a silent
// `undefined`.
function scriptIpc(_scenario: Scenario) {
  // The initial mount fetch of `get_proposed_strategy` must not itself seed
  // `testCommand` with `EXISTING_TEST_COMMAND` — the fallback test relies on
  // the field starting empty. Only the second call (inside
  // `proceedWithReBootstrap`, fetching `existing`) returns it.
  let gpsCalls = 0;
  const strategyPayload = (testCommand: string | null) => ({
    project_id: PROJECT_ID,
    worktree_strategy: {
      default_branch: 'main',
      branch_prefix: 'demeteo/features/',
      test_command: testCommand,
      build_command: null,
      coverage_command: null,
      conventions_file: null,
      pr_template: null,
      harnesses: null,
      validation_gates: null,
    },
    conflict_policy: 'always_gate',
    feature_lifecycle: 'archive',
  });

  const handlers: Record<string, (args: Record<string, unknown>) => unknown> = {
    get_proposed_strategy: () => {
      const payload = strategyPayload(gpsCalls === 0 ? null : EXISTING_TEST_COMMAND);
      gpsCalls++;
      return payload;
    },
    get_repositories_for_project: () => [],
    get_machines: () => [],
    get_agent_configs: () => [],
    list_agents: () => [],
    set_agent_configs: () => undefined,
    update_project: () => undefined,
    probe_project_commands: () => ({ machine: 'local', commands: [], detail: null, guidance: '', blocks_launch: false }),
    bootstrap_project: () => ({
      default_branch: 'main',
      branch_prefix: 'demeteo/features/',
      test_command: STRATEGY_TEST_COMMAND,
      build_command: null,
      coverage_command: null,
      conventions_file: null,
      pr_template: null,
      harnesses: null,
      validation_gates: null,
    }),
  };
  mockedInvoke.mockImplementation((async (cmd: string, args?: unknown) => {
    const handler = handlers[cmd];
    if (!handler) throw new Error(`unscripted invoke('${cmd}')`);
    return handler((args ?? {}) as Record<string, unknown>);
  }) as typeof invoke);
}

function Harness({ children, status }: { children: ReactNode; status: Scenario['status'] }) {
  const { state, dispatch } = useProject();
  useEffect(() => {
    if (state.projects.length === 0) {
      dispatch({
        type: 'ADD_PROJECT',
        project: {
          id: PROJECT_ID,
          name: 'Demeteo',
          status: status ?? 'error',
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
  return <>{children}</>;
}

/** Mount the real settings view and land on the Strategy tab, where the
 *  Default Test Command field lives. `ipcScript` defaults to `scriptIpc`;
 *  pass a different router to exercise scenarios `scriptIpc` doesn't cover
 *  (e.g. `get_proposed_strategy` resolving to `null` on mount). */
async function mount(scenario: Scenario = {}, ipcScript: (s: Scenario) => void = scriptIpc) {
  ipcScript(scenario);
  render(
    <ErrorBusProvider>
      <NavigationProvider>
        <ProjectProvider>
          <Harness status={scenario.status}>
            <ProjectSettingsView />
          </Harness>
        </ProjectProvider>
      </NavigationProvider>
    </ErrorBusProvider>,
  );
  await userEvent.click(await screen.findByRole('tab', { name: /Agent Strategy/ }));
  await screen.findByText('Validation Harnesses');
}

async function saveAndOpenStrategyProposal() {
  await userEvent.click(screen.getByRole('button', { name: /Save Changes/ }));
  await screen.findByText('Approve Detected Worktree Strategy');
}

beforeEach(() => {
  mockedInvoke.mockReset();
});

describe('proceedWithReBootstrap', () => {
  it('shows the value the user just typed, not the stale DB or re-detected strategy', async () => {
    await mount({ status: 'error' });

    const typedCommand = 'C: what the user just typed';
    const input = screen.getByLabelText('Default Test Command');
    await userEvent.clear(input);
    await userEvent.type(input, typedCommand);
    expect(input).toHaveValue(typedCommand);

    await saveAndOpenStrategyProposal();

    expect(screen.getByPlaceholderText('e.g. npm test or cargo test')).toHaveValue(typedCommand);
  });

  it('falls back to the stale DB value, then the re-detected strategy, when nothing was typed', async () => {
    await mount({ status: 'error' });

    // Untouched: the initial `get_proposed_strategy` fetch returned a null
    // test_command, so the field starts (and stays) empty.
    expect(screen.getByLabelText('Default Test Command')).toHaveValue('');

    await saveAndOpenStrategyProposal();

    // ext.test_command (EXISTING_TEST_COMMAND) wins over strategy.test_command
    // (STRATEGY_TEST_COMMAND) — unchanged pre-fix fallback behavior.
    expect(screen.getByPlaceholderText('e.g. npm test or cargo test')).toHaveValue(EXISTING_TEST_COMMAND);
  });
});

// Regression tests for the same `proceedWithReBootstrap` precedence bug, but
// for `defaultBranch`/`branchPrefix`. Unlike `testCommand`/`prTemplate`,
// these two used to be seeded with non-empty placeholders (`'main'` /
// `'demeteo/features/'`) that are always truthy, so `currentDefaultBranch ||
// ext?.default_branch || strategy.default_branch` never fell through to the
// detected/persisted value for a brand-new project whose first bootstrap
// attempt failed before any `ProjectSettings` row was ever persisted — the
// mount-time `get_proposed_strategy` fetch resolves to `null` in that case
// (not an object with null fields), so `res` at line 483 is falsy and
// `defaultBranch`/`branchPrefix` never leave the hardcoded placeholder. The
// popup then showed 'main'/'demeteo/features/' regardless of what
// `bootstrap_project` actually detected.
describe('proceedWithReBootstrap — defaultBranch/branchPrefix precedence', () => {
  const DETECTED_DEFAULT_BRANCH = 'staging';
  const DETECTED_BRANCH_PREFIX = 'df/';
  const EXISTING_DEFAULT_BRANCH = 'develop';
  const EXISTING_BRANCH_PREFIX = 'feature/';

  // Both labels are plain sibling text, not wired via htmlFor/id, so
  // getByLabelText can't resolve them — walk from the label text to the
  // input in the same wrapper div instead (same idiom as
  // ProjectHome.test.tsx's `getInputByLabel`).
  function getInputByLabel(text: string): HTMLInputElement {
    const label = screen.getByText(text);
    return label.parentElement!.querySelector('input') as HTMLInputElement;
  }

  /** @param initialResIsNull - `true` reproduces the brand-new-project case:
   *  `get_proposed_strategy` resolves to `null` on every call. `false`
   *  simulates a project with a real, previously-persisted strategy: every
   *  call returns the same non-null `EXISTING_*` values. */
  function mockBackend(initialResIsNull: boolean) {
    return (_scenario: Scenario) => {
      const handlers: Record<string, (args: Record<string, unknown>) => unknown> = {
        get_proposed_strategy: () => {
          if (initialResIsNull) return null;
          return {
            project_id: PROJECT_ID,
            worktree_strategy: {
              default_branch: EXISTING_DEFAULT_BRANCH,
              branch_prefix: EXISTING_BRANCH_PREFIX,
              test_command: null,
              build_command: null,
              coverage_command: null,
              conventions_file: null,
              pr_template: null,
              harnesses: null,
              validation_gates: null,
            },
            conflict_policy: 'always_gate',
            feature_lifecycle: 'archive',
          };
        },
        get_repositories_for_project: () => [],
        get_machines: () => [],
        get_agent_configs: () => [],
        list_agents: () => [],
        set_agent_configs: () => undefined,
        update_project: () => undefined,
        probe_project_commands: () => ({ machine: 'local', commands: [], detail: null, guidance: '', blocks_launch: false }),
        bootstrap_project: () => ({
          default_branch: DETECTED_DEFAULT_BRANCH,
          branch_prefix: DETECTED_BRANCH_PREFIX,
          test_command: null,
          build_command: null,
          coverage_command: null,
          conventions_file: null,
          pr_template: null,
          harnesses: null,
          validation_gates: null,
        }),
      };
      mockedInvoke.mockImplementation((async (cmd: string, args?: unknown) => {
        const handler = handlers[cmd];
        if (!handler) throw new Error(`unscripted invoke('${cmd}')`);
        return handler((args ?? {}) as Record<string, unknown>);
      }) as typeof invoke);
    };
  }

  it('shows the freshly re-detected values, not the "main"/"demeteo/features/" placeholders, when no ProjectSettings row was ever persisted', async () => {
    await mount({ status: 'error' }, mockBackend(true));

    // Untouched: the mount-time `get_proposed_strategy` fetch resolved to
    // `null`, so the fields start (and stay) empty rather than the old
    // hardcoded 'main' / 'demeteo/features/' seeds.
    expect(getInputByLabel('Default Branch')).toHaveValue('');
    expect(getInputByLabel('Branch Prefix')).toHaveValue('');

    await saveAndOpenStrategyProposal();

    expect(getInputByLabel('Default Branch')).toHaveValue(DETECTED_DEFAULT_BRANCH);
    expect(getInputByLabel('Branch Prefix')).toHaveValue(DETECTED_BRANCH_PREFIX);
  });

  it('keeps a real persisted value over a differently re-detected strategy (no regression)', async () => {
    await mount({ status: 'error' }, mockBackend(false));

    expect(getInputByLabel('Default Branch')).toHaveValue(EXISTING_DEFAULT_BRANCH);
    expect(getInputByLabel('Branch Prefix')).toHaveValue(EXISTING_BRANCH_PREFIX);

    await saveAndOpenStrategyProposal();

    // The mount-time fetch already populated real, non-empty values, so they
    // win over `strategy.*` regardless of what `bootstrap_project` re-detects.
    expect(getInputByLabel('Default Branch')).toHaveValue(EXISTING_DEFAULT_BRANCH);
    expect(getInputByLabel('Branch Prefix')).toHaveValue(EXISTING_BRANCH_PREFIX);
  });
});
