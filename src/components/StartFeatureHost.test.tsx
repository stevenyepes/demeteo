// The Start Feature modal under the real navigation, UI-state, project and
// error-bus providers. Every other suite mocks `navigate`, which proves a call
// was made but not what the user then sees: here the assertion is the view and
// whether the modal is still in the document.

import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { useEffect } from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';

import {
  NavigationProvider,
  ProjectProvider,
  UIStateProvider,
  useNavigation,
  useProject,
  useUIState,
} from '../context';
import { type LaunchRunParams, useLaunchRun } from '../hooks/useLaunchRun';
import { ErrorBusProvider, useErrorBus } from '../lib/errorBus';
import { invalidateRunnerCompatibility } from '../lib/runnerCompatibility';
import type { RunnerCompatibilityReport } from '../types';
import { ErrorToast } from './ErrorToast';
import { StartFeatureHost } from './StartFeatureHost';

vi.mock('@tauri-apps/api/webview', () => ({
  getCurrentWebview: () => ({
    onDragDropEvent: vi.fn().mockResolvedValue(() => {}),
  }),
}));

vi.mock('../lib/agentCatalog', () => ({
  useAgentCatalog: () => ({ agents: [] }),
  effortLevelsFor: () => [],
}));

const box = { id: 'machine-box', name: 'box', host: 'box.lan', port: 22, username: 'dev', auth_type: 'key' };

const compatible: RunnerCompatibilityReport = {
  verdict: 'compatible',
  version: '1.2.0',
  channel: 'stable',
  message: 'demeteo-runner 1.2.0 (stable) on box matches Demeteo.',
};

const behind: RunnerCompatibilityReport = {
  verdict: 'runner_behind',
  runner: '1.1.0',
  runner_channel: 'stable',
  app: '1.2.0',
  app_channel: 'stable',
  message: 'demeteo-runner 1.1.0 (stable) on box is older than Demeteo 1.2.0 (stable) — upgrade the runner from Machines settings.',
};

/** Answers only the commands this flow is expected to send; anything else is a
 *  rejection, so a call nobody scripted fails the test instead of reading `undefined`. */
function mockBackend(opts: {
  probes: RunnerCompatibilityReport[];
  submit: () => Promise<unknown>;
  /** Read on every `get_machines`, so a test can change it between opens. */
  machines?: { current: (typeof box)[] };
}) {
  const machines = opts.machines ?? { current: [box] };
  const probe = vi.fn((_machineId: string) => {
    const next = opts.probes.length > 1 ? opts.probes.shift() : opts.probes[0];
    return Promise.resolve(next);
  });
  const submit = vi.fn(opts.submit);
  vi.mocked(invoke).mockImplementation((command: string, args?: unknown) => {
    switch (command) {
      case 'workflow_list':
        return Promise.resolve([{ id: 'workflow-1', name: 'Default', version: 1 }]);
      case 'workflow_get':
        return Promise.resolve({ steps: [], version_id: 'version-1' });
      case 'workflow_version_graph':
        return Promise.resolve(null);
      case 'get_proposed_strategy':
        return Promise.resolve({ default_workflow_id: 'workflow-1' });
      case 'get_machines':
        return Promise.resolve(machines.current);
      case 'get_agent_configs':
      case 'fetch_active_features':
      case 'get_feature_status_rollup':
        return Promise.resolve([]);
      case 'remote_runner_compatibility':
        return probe((args as { machineId: string }).machineId);
      case 'launch_run':
        return submit();
      default:
        return Promise.resolve(undefined);
    }
  });
  return { probe, submit };
}

const launchedElsewhere: LaunchRunParams = {
  workflowId: 'workflow-1',
  title: 'Launched elsewhere',
  description: 'Not the draft',
  machineId: 'machine-elsewhere',
};

/** What `App` mounts around the host, plus a readout of the current view. */
function Harness() {
  const { view, navigate } = useNavigation();
  const { state: proj, dispatch } = useProject();
  const { ui, uiDispatch } = useUIState();
  const launchRun = useLaunchRun({ projectId: proj.currentProjectId });

  useEffect(() => {
    dispatch({
      type: 'LOAD_PROJECTS',
      projects: [
        { id: 'project-1', name: 'Demo', status: 'idle', repos: 0, nodes: 0, spend: 0, tokens: 0 },
        { id: 'project-2', name: 'Other', status: 'idle', repos: 0, nodes: 0, spend: 0, tokens: 0 },
      ],
      reposByProject: { 'project-1': [], 'project-2': [] },
    });
    dispatch({ type: 'SET_CURRENT', id: 'project-1' });
    navigate({ kind: 'home' });
    uiDispatch({ type: 'OPEN_START_FEATURE' });
  }, [dispatch, navigate, uiDispatch]);

  return (
    <>
      <output data-testid="view">{view.kind}</output>
      <output data-testid="selected-step">{view.kind === 'detail' ? (view.selectedStepId ?? '') : ''}</output>
      <output data-testid="start-feature-open">{String(ui.startFeatureOpen)}</output>
      <button
        type="button"
        onClick={() =>
          navigate(
            { kind: 'detail', featureId: 'other', featureTitle: 'Other', gateStepExecutionId: 'gate-1' },
            'replace',
          )
        }
      >
        raise gate
      </button>
      <button type="button" onClick={() => navigate({ kind: 'detail', featureId: 'feature-a', featureTitle: 'A' })}>
        open run A
      </button>
      <button type="button" onClick={() => uiDispatch({ type: 'OPEN_START_FEATURE' })}>
        new feature
      </button>
      {/* What ProjectHome's `openStartFeature` sends from an empty composer. */}
      <button
        type="button"
        onClick={() =>
          uiDispatch({ type: 'OPEN_START_FEATURE', seed: { title: undefined, attachments: undefined } })
        }
      >
        new feature from composer
      </button>
      {/* What `useStepSelection` does when a detail view's first step row arrives. */}
      <button
        type="button"
        onClick={() => view.kind === 'detail' && navigate({ ...view, selectedStepId: 'step-1' }, 'replace')}
      >
        seed inspector
      </button>
      <button type="button" onClick={() => navigate({ kind: 'workflows' }, 'replace')}>
        replace with workflows
      </button>
      {/* A detached launch from `FeatureDetail` or `CodeReviewView`, which share the hook. */}
      <button type="button" onClick={() => void launchRun(launchedElsewhere)}>
        launch from elsewhere
      </button>
      {['project-1', 'project-2'].map((id) => (
        <button key={id} type="button" onClick={() => dispatch({ type: 'SET_CURRENT', id })}>
          switch to {id}
        </button>
      ))}
      <StartFeatureHost launchRun={launchRun} />
    </>
  );
}

function ClearToasts() {
  const { clear } = useErrorBus();
  useEffect(() => clear, [clear]);
  return null;
}

function renderApp() {
  render(
    <ErrorBusProvider>
      <ClearToasts />
      <NavigationProvider>
        <ProjectProvider>
          <UIStateProvider>
            <Harness />
            <ErrorToast />
          </UIStateProvider>
        </ProjectProvider>
      </NavigationProvider>
    </ErrorBusProvider>,
  );
}

const modalHeading = () => screen.queryByRole('heading', { name: /start a feature/i });
const titleInput = () => screen.findByPlaceholderText(/add oauth2 login flow/i);
const descriptionInput = () => screen.getByPlaceholderText(/what does this feature do/i);
const launchButton = () => screen.getByRole('button', { name: /launch feature/i });

async function fillInDetachedOn(machineId: string) {
  fireEvent.change(await screen.findByPlaceholderText(/add oauth2 login flow/i), {
    target: { value: 'A feature' },
  });
  fireEvent.change(screen.getByPlaceholderText(/what does this feature do/i), {
    target: { value: 'What the feature does' },
  });
  const select = await screen.findByDisplayValue('This machine');
  await screen.findByRole('option', { name: /box — detached/i });
  fireEvent.change(select, { target: { value: machineId } });
}

/** The draft `fillInDetachedOn(box.id)` typed, as the reopened modal shows it. */
async function expectDraftRestored() {
  expect(await titleInput()).toHaveValue('A feature');
  expect(descriptionInput()).toHaveValue('What the feature does');
  expect(await screen.findByDisplayValue(/box — detached/i)).toHaveValue(box.id);
}

function zIndexOf(element: Element): number {
  const match = /(?:^|\s)z-(?:\[(\d+)\]|(\d+))(?:\s|$)/.exec(element.className);
  if (!match) throw new Error(`no z-index class on ${element.className}`);
  return Number(match[1] ?? match[2]);
}

beforeEach(() => {
  invalidateRunnerCompatibility();
});

afterEach(() => {
  invalidateRunnerCompatibility();
});

describe('StartFeatureHost under the real providers', () => {
  it('opens the modal over the view it was opened with', async () => {
    mockBackend({ probes: [compatible], submit: () => new Promise(() => {}) });
    renderApp();

    expect(await screen.findByRole('heading', { name: /start a feature/i })).toBeInTheDocument();
    expect(screen.getByTestId('view')).toHaveTextContent('home');
  });

  it('keeps the draft open when a background run raises a gate', async () => {
    mockBackend({ probes: [compatible], submit: () => new Promise(() => {}) });
    renderApp();
    fireEvent.change(await screen.findByPlaceholderText(/add oauth2 login flow/i), {
      target: { value: 'Half-typed title' },
    });

    fireEvent.click(screen.getByRole('button', { name: 'raise gate' }));

    await waitFor(() => expect(screen.getByTestId('view')).toHaveTextContent('detail'));
    expect(screen.getByPlaceholderText(/add oauth2 login flow/i)).toHaveValue('Half-typed title');
  });

  it('keeps the draft open when the inspector seeds a step on the detail view underneath', async () => {
    mockBackend({ probes: [compatible], submit: () => new Promise(() => {}) });
    renderApp();
    fireEvent.click(screen.getByRole('button', { name: 'open run A' }));
    await waitFor(() => expect(screen.getByTestId('view')).toHaveTextContent('detail'));
    fireEvent.click(screen.getByRole('button', { name: 'new feature' }));
    fireEvent.change(await screen.findByPlaceholderText(/add oauth2 login flow/i), {
      target: { value: 'Feature B' },
    });

    fireEvent.click(screen.getByRole('button', { name: 'seed inspector' }));

    await waitFor(() => expect(screen.getByTestId('selected-step')).toHaveTextContent('step-1'));
    expect(screen.getByTestId('start-feature-open')).toHaveTextContent('true');
    expect(screen.getByPlaceholderText(/add oauth2 login flow/i)).toHaveValue('Feature B');
  });

  it('keeps the draft open across a navigation the user did not start from the modal', async () => {
    mockBackend({ probes: [compatible], submit: () => new Promise(() => {}) });
    renderApp();
    fireEvent.change(await screen.findByPlaceholderText(/add oauth2 login flow/i), {
      target: { value: 'Half-typed title' },
    });

    fireEvent.click(screen.getByRole('button', { name: 'replace with workflows' }));

    await waitFor(() => expect(screen.getByTestId('view')).toHaveTextContent('workflows'));
    expect(screen.getByTestId('start-feature-open')).toHaveTextContent('true');
    expect(screen.getByPlaceholderText(/add oauth2 login flow/i)).toHaveValue('Half-typed title');
  });

  it("closes the modal when the notice's machine-settings link navigates", async () => {
    mockBackend({ probes: [behind], submit: () => new Promise(() => {}) });
    renderApp();

    await fillInDetachedOn(box.id);
    const notice = await screen.findByRole('alert');
    fireEvent.click(within(notice).getByRole('button', { name: /open machine settings/i }));

    await waitFor(() => expect(modalHeading()).not.toBeInTheDocument());
    expect(screen.getByTestId('start-feature-open')).toHaveTextContent('false');
    expect(screen.getByTestId('view')).toHaveTextContent('settings');
  });

  it.each(['new feature', 'new feature from composer'])(
    'restores the draft and target machine the settings link left with when reopened by %s',
    async (reopen) => {
      mockBackend({ probes: [behind], submit: () => new Promise(() => {}) });
      renderApp();
      await fillInDetachedOn(box.id);
      const notice = await screen.findByRole('alert');
      fireEvent.click(within(notice).getByRole('button', { name: /open machine settings/i }));
      await waitFor(() => expect(modalHeading()).not.toBeInTheDocument());

      fireEvent.click(screen.getByRole('button', { name: reopen }));

      await expectDraftRestored();
    },
  );

  it.each([
    ['another remote machine remains', [{ ...box, id: 'machine-other', name: 'other' }]],
    ['no remote machine remains', []],
  ])('runs here when the kept machine was deleted while away and %s', async (_case, remaining) => {
    const machines = { current: [box] };
    mockBackend({ probes: [behind], submit: () => new Promise(() => {}), machines });
    renderApp();
    await fillInDetachedOn(box.id);
    const notice = await screen.findByRole('alert');
    fireEvent.click(within(notice).getByRole('button', { name: /open machine settings/i }));
    await waitFor(() => expect(modalHeading()).not.toBeInTheDocument());

    machines.current = remaining;
    fireEvent.click(screen.getByRole('button', { name: 'new feature' }));

    expect(await titleInput()).toHaveValue('A feature');
    // Still detached, the stale id's `behind` probe would hold Launch disabled.
    await waitFor(() => expect(launchButton()).toBeEnabled());
    expect(screen.queryByText(/detached — runs on/i)).not.toBeInTheDocument();
    if (remaining.length > 0) {
      expect(await screen.findByDisplayValue('This machine')).toBeInTheDocument();
    } else {
      expect(screen.getByText(/add a remote machine under machines/i)).toBeInTheDocument();
    }
  });

  it('restores a kept draft only in the project it was kept in', async () => {
    mockBackend({ probes: [behind], submit: () => new Promise(() => {}) });
    renderApp();
    await fillInDetachedOn(box.id);
    const notice = await screen.findByRole('alert');
    fireEvent.click(within(notice).getByRole('button', { name: /open machine settings/i }));
    await waitFor(() => expect(modalHeading()).not.toBeInTheDocument());

    fireEvent.click(screen.getByRole('button', { name: 'switch to project-2' }));
    fireEvent.click(screen.getByRole('button', { name: 'new feature' }));

    expect(await titleInput()).toHaveValue('');
    expect(descriptionInput()).toHaveValue('');
    expect(await screen.findByDisplayValue('This machine')).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    await waitFor(() => expect(modalHeading()).not.toBeInTheDocument());
    fireEvent.click(screen.getByRole('button', { name: 'switch to project-1' }));
    fireEvent.click(screen.getByRole('button', { name: 'new feature' }));

    await expectDraftRestored();
  });

  it.each([
    ['Escape', () => fireEvent.keyDown(screen.getByPlaceholderText(/add oauth2 login flow/i), { key: 'Escape' })],
    ['Cancel', () => fireEvent.click(screen.getByRole('button', { name: 'Cancel' }))],
  ])('reopens empty after %s', async (_how, close) => {
    mockBackend({ probes: [compatible], submit: () => new Promise(() => {}) });
    renderApp();
    fireEvent.change(await titleInput(), { target: { value: 'Abandoned title' } });

    close();
    await waitFor(() => expect(modalHeading()).not.toBeInTheDocument());
    fireEvent.click(screen.getByRole('button', { name: 'new feature' }));

    expect(await titleInput()).toHaveValue('');
  });

  it('after a runner refusal re-probes, blocks Start, and its toast reaches settings over the modal', async () => {
    const { probe, submit } = mockBackend({
      probes: [compatible, behind],
      submit: () => Promise.reject({ kind: 'runner_incompatible', message: behind.message, compatibility: behind }),
    });
    renderApp();

    await fillInDetachedOn(box.id);
    await waitFor(() => expect(probe).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(launchButton()).toBeEnabled());
    fireEvent.click(launchButton());

    await waitFor(() => expect(submit).toHaveBeenCalledOnce());
    const toast = await screen.findByTestId('error-toast-runner_incompatible');
    await waitFor(() => expect(probe).toHaveBeenCalledTimes(2));
    const modalLayer = modalHeading()?.closest('.fixed.inset-0');
    const toastLayer = toast.closest('.fixed');
    if (!(modalLayer instanceof HTMLElement) || !toastLayer) throw new Error('modal or toast layer not found');
    expect(await within(modalLayer).findByRole('alert')).toHaveTextContent(behind.message);
    expect(launchButton()).toBeDisabled();
    expect(zIndexOf(toastLayer)).toBeGreaterThan(zIndexOf(modalLayer));

    fireEvent.click(within(toast).getByRole('button', { name: /open machine settings/i }));

    await waitFor(() => expect(modalHeading()).not.toBeInTheDocument());
    expect(screen.getByTestId('start-feature-open')).toHaveTextContent('false');
    expect(screen.getByTestId('view')).toHaveTextContent('settings');

    fireEvent.click(screen.getByRole('button', { name: 'new feature' }));
    await expectDraftRestored();
  });

  it("keeps the open modal's draft when a refusal from another launch surface sends the user to settings", async () => {
    const { submit } = mockBackend({
      probes: [compatible],
      submit: () => Promise.reject({ kind: 'runner_incompatible', message: behind.message, compatibility: behind }),
    });
    renderApp();
    await fillInDetachedOn(box.id);

    fireEvent.click(screen.getByRole('button', { name: 'launch from elsewhere' }));

    await waitFor(() => expect(submit).toHaveBeenCalledOnce());
    const toast = await screen.findByTestId('error-toast-runner_incompatible');
    fireEvent.click(within(toast).getByRole('button', { name: /open machine settings/i }));

    await waitFor(() => expect(modalHeading()).not.toBeInTheDocument());
    expect(screen.getByTestId('view')).toHaveTextContent('settings');

    fireEvent.click(screen.getByRole('button', { name: 'new feature' }));
    await expectDraftRestored();
  });

  it('a successful launch lands on the feature and closes the modal without an error', async () => {
    const { submit } = mockBackend({
      probes: [compatible],
      submit: () => Promise.resolve({ feature_id: 'feature-1', run_id: 'run-1', status: 'pending' }),
    });
    renderApp();

    await fillInDetachedOn(box.id);
    await waitFor(() => expect(launchButton()).toBeEnabled());
    await act(async () => {
      fireEvent.click(launchButton());
    });

    await waitFor(() => expect(screen.getByTestId('view')).toHaveTextContent('detail'));
    expect(submit).toHaveBeenCalledOnce();
    expect(modalHeading()).not.toBeInTheDocument();
    expect(screen.getByTestId('start-feature-open')).toHaveTextContent('false');
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
    expect(document.querySelector('[data-testid^="error-toast-"]')).toBeNull();
  });
});
