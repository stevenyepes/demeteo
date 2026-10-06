import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';

import StartFeatureModal from './StartFeatureModal';
import { invalidateRunnerCompatibility, RUNNER_COMPATIBILITY_TTL_MS } from '../lib/runnerCompatibility';
import { STANDARD_STARTER_WORKFLOW_ID } from '../lib/workflowDefault';
import type { RunnerCompatibilityReport } from '../types';

const { navigate } = vi.hoisted(() => ({ navigate: vi.fn() }));

vi.mock('../context', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../context')>()),
  useNavigation: () => ({ navigate }),
}));

vi.mock('@tauri-apps/api/webview', () => ({
  getCurrentWebview: () => ({
    onDragDropEvent: vi.fn().mockResolvedValue(() => {}),
  }),
}));

vi.mock('../lib/agentCatalog', () => ({
  useAgentCatalog: () => ({ agents: [] }),
  effortLevelsFor: () => [],
}));

interface ClipboardItemFixture {
  kind: string;
  type: string;
  getAsFile: () => File | null;
}

function clipboardData(items: ClipboardItemFixture[]): DataTransfer {
  return { items } as unknown as DataTransfer;
}

function paste(node: Element, items: ClipboardItemFixture[]) {
  const event = new Event('paste', { bubbles: true, cancelable: true });
  Object.defineProperty(event, 'clipboardData', { value: clipboardData(items) });
  const preventDefault = vi.spyOn(event, 'preventDefault');
  fireEvent(node, event);
  return preventDefault;
}

function imageItem(file: File): ClipboardItemFixture {
  return { kind: 'file', type: file.type, getAsFile: () => file };
}

function textItem(): ClipboardItemFixture {
  return { kind: 'string', type: 'text/plain', getAsFile: () => null };
}

function unavailableImageItem(): ClipboardItemFixture {
  return { kind: 'file', type: 'image/png', getAsFile: () => null };
}

function renderModal(onLaunch = vi.fn(), defaultWorkflowId?: string | null) {
  render(
    <StartFeatureModal
      isOpen
      projectId="project-1"
      repositories={[]}
      defaultWorkflowId={defaultWorkflowId}
      onClose={vi.fn()}
      onLaunch={onLaunch}
    />,
  );
  return onLaunch;
}

beforeEach(() => {
  vi.mocked(invoke).mockImplementation((command: string) => {
    switch (command) {
      case 'workflow_list':
        return Promise.resolve([{ id: 'workflow-1', name: 'Default', version: 1 }]);
      case 'workflow_get':
        return Promise.resolve({ steps: [], version_id: 'version-1' });
      case 'workflow_version_graph':
        return Promise.resolve(null);
      case 'get_machines':
      case 'get_agent_configs':
      case 'fetch_active_features':
        return Promise.resolve([]);
      default:
        return Promise.resolve(undefined);
    }
  });
});

describe('StartFeatureModal clipboard paste', () => {
  it('stages an image pasted into the initially focused title once and launches it', async () => {
    const onLaunch = renderModal();
    const title = await screen.findByPlaceholderText(/add oauth2 login flow/i);
    const image = new File(['image bytes'], 'pasted.png', { type: 'image/png' });

    await waitFor(() => expect(title).toHaveFocus());

    const preventDefault = paste(title, [imageItem(image)]);

    expect(preventDefault).toHaveBeenCalledTimes(1);
    await screen.findByText('pasted.png');
    expect(screen.getAllByRole('button', { name: /^remove /i })).toHaveLength(1);

    fireEvent.change(title, { target: { value: 'Pasted image feature' } });
    fireEvent.change(screen.getByPlaceholderText(/what does this feature do/i), {
      target: { value: 'Describe the pasted image feature' },
    });
    await waitFor(() => expect(screen.getByRole('button', { name: /launch feature/i })).toBeEnabled());
    fireEvent.click(screen.getByRole('button', { name: /launch feature/i }));

    expect(onLaunch).toHaveBeenCalledWith(expect.objectContaining({
      attachments: [expect.objectContaining({
        name: 'pasted.png',
        file: image,
        sourcePath: null,
      })],
    }));
  });

  it('recovers an image from the async clipboard after WebKitGTK supplies empty items', async () => {
    const clipboardRead = vi.fn().mockResolvedValue([{
      types: ['image/png'],
      getType: vi.fn().mockResolvedValue(new Blob(['png bytes'], { type: 'image/png' })),
    }]);
    const previousClipboard = Object.getOwnPropertyDescriptor(navigator, 'clipboard');
    Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { read: clipboardRead } });
    try {
      renderModal();
      const title = await screen.findByPlaceholderText(/add oauth2 login flow/i);

      paste(title, []);

      await waitFor(() => expect(clipboardRead).toHaveBeenCalledTimes(1));
      expect(await screen.findByText('pasted-image.png')).toBeInTheDocument();
    } finally {
      if (previousClipboard) Object.defineProperty(navigator, 'clipboard', previousClipboard);
      else Reflect.deleteProperty(navigator, 'clipboard');
    }
  });

  it('shows a soft error when the async clipboard read is denied', async () => {
    const previousClipboard = Object.getOwnPropertyDescriptor(navigator, 'clipboard');
    Object.defineProperty(navigator, 'clipboard', {
      configurable: true,
      value: { read: vi.fn().mockRejectedValue(new DOMException('denied', 'NotAllowedError')) },
    });
    try {
      renderModal();
      const title = await screen.findByPlaceholderText(/add oauth2 login flow/i);

      paste(title, []);

      expect(await screen.findByRole('alert')).toHaveTextContent(/could not read image bytes/i);
    } finally {
      if (previousClipboard) Object.defineProperty(navigator, 'clipboard', previousClipboard);
      else Reflect.deleteProperty(navigator, 'clipboard');
    }
  });

  it.each([
    ['title', /add oauth2 login flow/i],
    ['description', /what does this feature do/i],
  ])('leaves text-only paste native in the %s', async (_field, placeholder) => {
    renderModal();
    const input = await screen.findByPlaceholderText(placeholder);

    const preventDefault = paste(input, [textItem()]);

    expect(preventDefault).not.toHaveBeenCalled();
    expect(screen.queryByRole('button', { name: /^remove /i })).not.toBeInTheDocument();
  });

  it('shows the attachment soft error for an unavailable image without consuming text paste', async () => {
    renderModal();
    const title = await screen.findByPlaceholderText(/add oauth2 login flow/i);

    const preventDefault = paste(title, [textItem(), unavailableImageItem()]);

    expect(preventDefault).not.toHaveBeenCalled();
    expect(await screen.findByRole('alert')).toHaveTextContent(
      /clipboard offered an image, but this webview could not access its file/i,
    );
    expect(screen.queryByRole('button', { name: /^remove /i })).not.toBeInTheDocument();
  });
});

/**
 * The launch path the audit's F10 finding is really about: the header claimed a
 * default the schema did not have, but the modal *behaved* as if one existed by
 * taking whatever `workflow_list` returned first.
 *
 * Every list below is ordered so that the removed `workflows[0]` fall-through
 * would answer differently from the rule — otherwise these pass against the bug
 * they exist to pin.
 */
describe('StartFeatureModal seed', () => {
  it('fills the description with the title when the composer seeds only a title', async () => {
    render(
      <StartFeatureModal
        isOpen
        projectId="project-1"
        repositories={[]}
        seed={{ title: 'From the composer' }}
        onClose={vi.fn()}
        onLaunch={vi.fn()}
      />,
    );

    expect(await screen.findByPlaceholderText(/add oauth2 login flow/i)).toHaveValue('From the composer');
    expect(screen.getByPlaceholderText(/what does this feature do/i)).toHaveValue('From the composer');
  });
});

describe('StartFeatureModal workflow default', () => {
  const listed = [
    { id: 'wf-alpha', name: 'Alpha', version: 1 },
    { id: 'wf-beta', name: 'Beta', version: 2 },
    { id: STANDARD_STARTER_WORKFLOW_ID, name: 'Standard Feature Pipeline', version: 3 },
  ];

  function mockBackend(options: {
    workflows?: Array<{ id: string; name: string; version: number }>;
    storedDefault?: string | null;
    settingsFail?: boolean;
    /** Held-open reads. Both loads resolve in the same tick under jsdom, which
     *  hides the orderings the app actually sees — a SQLite settings read and a
     *  workflow list have no fixed order between them, and the seed is wrong if
     *  it commits before either one answers. */
    settingsPending?: Promise<unknown>;
    workflowsPending?: Promise<unknown>;
  }) {
    vi.mocked(invoke).mockImplementation((command: string) => {
      switch (command) {
        case 'workflow_list':
          return options.workflowsPending ?? Promise.resolve(options.workflows ?? listed);
        case 'workflow_get':
          return Promise.resolve({ steps: [], version_id: 'version-1' });
        case 'workflow_version_graph':
          return Promise.resolve(null);
        case 'get_proposed_strategy':
          if (options.settingsFail) return Promise.reject(new Error('settings unavailable'));
          return options.settingsPending
            ?? Promise.resolve({ default_workflow_id: options.storedDefault ?? null });
        case 'get_machines':
        case 'get_agent_configs':
        case 'fetch_active_features':
          return Promise.resolve([]);
        default:
          return Promise.resolve(undefined);
      }
    });
  }

  function picker(): HTMLSelectElement {
    return screen.getByLabelText('Workflow') as HTMLSelectElement;
  }

  async function fillAndLaunch() {
    fireEvent.change(screen.getByPlaceholderText(/add oauth2 login flow/i), {
      target: { value: 'A feature' },
    });
    fireEvent.change(screen.getByPlaceholderText(/what does this feature do/i), {
      target: { value: 'What the feature does' },
    });
    const button = screen.getByRole('button', { name: /launch feature/i });
    await waitFor(() => expect(button).toBeEnabled());
    fireEvent.click(button);
  }

  it('launches on the workflow the project stored, not the first one listed', async () => {
    mockBackend({ storedDefault: 'wf-beta' });
    const onLaunch = renderModal();

    await waitFor(() => expect(picker().value).toBe('wf-beta'));
    await fillAndLaunch();

    expect(onLaunch).toHaveBeenCalledWith(expect.objectContaining({ workflowId: 'wf-beta' }));
  });

  it('prefers the workflow the caller pointed at over the stored default', async () => {
    mockBackend({ storedDefault: 'wf-beta' });
    renderModal(vi.fn(), 'wf-alpha');

    await waitFor(() => expect(picker().value).toBe('wf-alpha'));
  });

  it('falls to the standard starter when the stored default no longer exists', async () => {
    mockBackend({ storedDefault: 'wf-deleted' });
    renderModal();

    await waitFor(() => expect(picker().value).toBe(STANDARD_STARTER_WORKFLOW_ID));
  });

  it('treats an unreadable project setting as unset instead of blocking the launch', async () => {
    mockBackend({ settingsFail: true });
    const onLaunch = renderModal();

    await waitFor(() => expect(picker().value).toBe(STANDARD_STARTER_WORKFLOW_ID));
    await fillAndLaunch();

    expect(onLaunch).toHaveBeenCalledWith(
      expect.objectContaining({ workflowId: STANDARD_STARTER_WORKFLOW_ID }),
    );
  });

  it('asks rather than guesses when no rule names a workflow', async () => {
    mockBackend({ workflows: listed.slice(0, 2) });
    renderModal();

    await screen.findByText('Choose a workflow…');
    expect(picker().value).toBe('');
    fireEvent.change(screen.getByPlaceholderText(/add oauth2 login flow/i), {
      target: { value: 'A feature' },
    });
    fireEvent.change(screen.getByPlaceholderText(/what does this feature do/i), {
      target: { value: 'What the feature does' },
    });
    expect(screen.getByRole('button', { name: /launch feature/i })).toBeDisabled();
  });

  it('waits for the stored setting instead of seeding a fallback ahead of it', async () => {
    let landed: (settings: unknown) => void = () => {};
    mockBackend({
      settingsPending: new Promise((resolve) => {
        landed = resolve;
      }),
    });
    renderModal();

    await screen.findByText('Beta (v2)');
    expect(picker().value).toBe('');

    landed({ default_workflow_id: 'wf-beta' });
    await waitFor(() => expect(picker().value).toBe('wf-beta'));
  });

  it('waits for the workflow list instead of latching on an empty one', async () => {
    let landed: (workflows: unknown) => void = () => {};
    mockBackend({
      storedDefault: 'wf-beta',
      workflowsPending: new Promise((resolve) => {
        landed = resolve;
      }),
    });
    renderModal();

    await act(async () => {});
    expect(picker().value).toBe('');

    landed(listed);
    await waitFor(() => expect(picker().value).toBe('wf-beta'));
  });

  it('never re-seeds over a pick the user made after the modal opened', async () => {
    mockBackend({ storedDefault: 'wf-beta' });
    const modal = (seedTitle?: string) => (
      <StartFeatureModal
        isOpen
        projectId="project-1"
        repositories={[]}
        defaultWorkflowId={STANDARD_STARTER_WORKFLOW_ID}
        seed={{ title: seedTitle }}
        onClose={vi.fn()}
        onLaunch={vi.fn()}
      />
    );
    const { rerender } = render(modal());

    await waitFor(() => expect(picker().value).toBe(STANDARD_STARTER_WORKFLOW_ID));
    fireEvent.change(picker(), { target: { value: 'wf-alpha' } });
    rerender(modal(''));

    await waitFor(() => expect(picker().value).toBe('wf-alpha'));
  });
});

describe('StartFeatureModal runner compatibility', () => {
  const box = { id: 'machine-box', name: 'box', host: 'box.lan', port: 22, username: 'dev', auth_type: 'key' };

  const reports: Record<string, RunnerCompatibilityReport> = {
    compatible: {
      verdict: 'compatible',
      version: '1.2.0',
      channel: 'stable',
      message: 'demeteo-runner 1.2.0 (stable) on box matches Demeteo.',
    },
    runner_behind: {
      verdict: 'runner_behind',
      runner: '1.1.0',
      runner_channel: 'stable',
      app: '1.2.0',
      app_channel: 'stable',
      message: 'demeteo-runner 1.1.0 (stable) on box is older than Demeteo 1.2.0 (stable) — upgrade the runner from Machines settings.',
    },
    runner_ahead: {
      verdict: 'runner_ahead',
      runner: '1.3.0',
      runner_channel: 'stable',
      app: '1.2.0',
      app_channel: 'stable',
      message: 'demeteo-runner 1.3.0 (stable) on box is newer than Demeteo 1.2.0 (stable) — upgrade Demeteo to match.',
    },
    not_installed: {
      verdict: 'not_installed',
      app: '1.2.0',
      app_channel: 'stable',
      message: 'demeteo-runner is not installed on box — install it from Machines settings.',
    },
    unknown: {
      verdict: 'unknown',
      app: '1.2.0',
      app_channel: 'stable',
      detail: 'ssh: connection refused',
      message: "Couldn't verify the demeteo-runner version on box — check the machine in Machines settings.",
    },
  };

  /** Every compatibility answer comes from `report`; `undefined` holds the
   *  probe open, the state the modal is in until the runner answers. */
  function mockBackend(report: RunnerCompatibilityReport | undefined) {
    const probe = vi.fn((_machineId: string) =>
      report ? Promise.resolve(report) : new Promise<never>(() => {}),
    );
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
          return Promise.resolve([box]);
        case 'get_agent_configs':
        case 'fetch_active_features':
          return Promise.resolve([]);
        case 'remote_runner_compatibility':
          return probe((args as { machineId: string }).machineId);
        default:
          return Promise.resolve(undefined);
      }
    });
    return probe;
  }

  async function fillIn() {
    fireEvent.change(await screen.findByPlaceholderText(/add oauth2 login flow/i), {
      target: { value: 'A feature' },
    });
    fireEvent.change(screen.getByPlaceholderText(/what does this feature do/i), {
      target: { value: 'What the feature does' },
    });
  }

  async function pickMachine(machineId: string) {
    const select = await screen.findByDisplayValue('This machine');
    await screen.findByRole('option', { name: /box — detached/i });
    fireEvent.change(select, { target: { value: machineId } });
  }

  const launchButton = () => screen.getByRole('button', { name: /launch feature/i });

  beforeEach(() => {
    invalidateRunnerCompatibility();
    navigate.mockReset();
  });

  it.each(['runner_behind', 'runner_ahead', 'not_installed'])(
    'disables launch on a detached machine whose runner is %s',
    async (verdict) => {
      const probe = mockBackend(reports[verdict]);
      renderModal();
      await fillIn();
      await waitFor(() => expect(launchButton()).toBeEnabled());

      await pickMachine(box.id);

      expect(await screen.findByRole('alert')).toHaveTextContent(reports[verdict].message);
      expect(launchButton()).toBeDisabled();
      expect(probe).toHaveBeenCalledWith(box.id);
    },
  );

  it.each(['compatible', 'unknown'])(
    'leaves launch enabled when the runner is %s',
    async (verdict) => {
      const probe = mockBackend(reports[verdict]);
      renderModal();
      await fillIn();

      await pickMachine(box.id);

      await waitFor(() => expect(probe).toHaveBeenCalledWith(box.id));
      await act(async () => {});
      expect(screen.queryByRole('alert')).not.toBeInTheDocument();
      expect(launchButton()).toBeEnabled();
    },
  );

  describe('repository selection', () => {
    const repo = { id: 'repo-1', repo_path: '/src/app', provider_id: 'provider-1' };

    function renderWithRepo() {
      const onLaunch = vi.fn();
      render(
        <StartFeatureModal
          isOpen
          projectId="project-1"
          repositories={[repo]}
          onClose={vi.fn()}
          onLaunch={onLaunch}
        />,
      );
      return onLaunch;
    }

    // `launch_run` refuses a target repo on a local launch, as it does every
    // other detached-only option.
    it('sends no target repo for a run on this machine', async () => {
      mockBackend(reports.compatible);
      const onLaunch = renderWithRepo();
      await fillIn();
      await waitFor(() => expect(launchButton()).toBeEnabled());

      fireEvent.click(launchButton());

      expect(onLaunch).toHaveBeenCalledOnce();
      expect(onLaunch.mock.calls[0][0]).toMatchObject({ targetRepos: undefined, unattended: undefined });
    });

    it('sends the selected repo for a detached run', async () => {
      mockBackend(reports.compatible);
      const onLaunch = renderWithRepo();
      await fillIn();
      await pickMachine(box.id);
      await waitFor(() => expect(launchButton()).toBeEnabled());

      fireEvent.click(launchButton());

      expect(onLaunch).toHaveBeenCalledWith(
        expect.objectContaining({ machineId: box.id, targetRepos: [repo.id], unattended: true }),
      );
    });
  });

  // `launch_run` refuses a non-positive cap, and dropping one instead would
  // launch an uncapped paid run the user meant to bound.
  describe('detached caps', () => {
    const costCap = () => screen.getByLabelText(/max cost/i);
    const wallClockCap = () => screen.getByLabelText(/max wall-clock/i);

    async function detachedAndReady() {
      mockBackend(reports.compatible);
      const onLaunch = renderModal();
      await fillIn();
      await pickMachine(box.id);
      await waitFor(() => expect(launchButton()).toBeEnabled());
      return onLaunch;
    }

    it.each([
      ['cost', '0', costCap, /greater than 0/i],
      ['cost', '-5', costCap, /greater than 0/i],
      ['cost', 'abc', costCap, /greater than 0/i],
      ['wall-clock', '0', wallClockCap, /whole number of minutes/i],
      ['wall-clock', '-1', wallClockCap, /whole number of minutes/i],
      ['wall-clock', '1.5', wallClockCap, /whole number of minutes/i],
      ['wall-clock', 'soon', wallClockCap, /whole number of minutes/i],
    ])('flags a %s cap of %s and refuses to launch', async (_name, value, field, message) => {
      const onLaunch = await detachedAndReady();

      fireEvent.change(field(), { target: { value } });

      expect(screen.getByText(message)).toBeInTheDocument();
      expect(field()).toHaveAttribute('aria-invalid', 'true');
      expect(launchButton()).toBeDisabled();
      fireEvent.click(launchButton());
      fireEvent.keyDown(field(), { key: 'Enter', ctrlKey: true });
      expect(onLaunch).not.toHaveBeenCalled();
    });

    it('sends positive caps unchanged', async () => {
      const onLaunch = await detachedAndReady();

      fireEvent.change(costCap(), { target: { value: '2.5' } });
      fireEvent.change(wallClockCap(), { target: { value: '45' } });
      fireEvent.click(launchButton());

      expect(onLaunch).toHaveBeenCalledWith(
        expect.objectContaining({ machineId: box.id, maxCostUsd: 2.5, maxWallClockMins: 45 }),
      );
    });

    it('treats blank caps as no cap', async () => {
      const onLaunch = await detachedAndReady();

      fireEvent.change(costCap(), { target: { value: '0' } });
      fireEvent.change(costCap(), { target: { value: '  ' } });
      expect(launchButton()).toBeEnabled();
      fireEvent.click(launchButton());

      expect(onLaunch).toHaveBeenCalledOnce();
      expect(onLaunch.mock.calls[0][0]).toMatchObject({
        maxCostUsd: undefined,
        maxWallClockMins: undefined,
      });
    });

    it('stops blocking launch once the run is no longer detached', async () => {
      const onLaunch = await detachedAndReady();
      fireEvent.change(costCap(), { target: { value: '0' } });
      expect(launchButton()).toBeDisabled();

      fireEvent.change(screen.getByDisplayValue(/box — detached/i), { target: { value: '' } });

      await waitFor(() => expect(launchButton()).toBeEnabled());
      fireEvent.click(launchButton());
      expect(onLaunch.mock.calls[0][0]).toMatchObject({ maxCostUsd: undefined });
    });
  });

  it('does not block launch while the probe is still in flight', async () => {
    const probe = mockBackend(undefined);
    renderModal();
    await fillIn();

    await pickMachine(box.id);

    await waitFor(() => expect(probe).toHaveBeenCalledWith(box.id));
    expect(launchButton()).toBeEnabled();
  });

  it('never probes a run on this machine', async () => {
    const probe = mockBackend(reports.runner_behind);
    renderModal();
    await fillIn();

    await screen.findByRole('option', { name: /box — detached/i });
    await waitFor(() => expect(launchButton()).toBeEnabled());

    expect(probe).not.toHaveBeenCalled();
    expect(screen.queryByText('Open machine settings')).not.toBeInTheDocument();
  });

  it('does not re-probe the runner after a launch that failed for another reason', async () => {
    const probe = mockBackend(reports.compatible);
    const onLaunch = renderModal(vi.fn(() => Promise.resolve('failed' as const)));
    await fillIn();
    await pickMachine(box.id);
    await waitFor(() => expect(probe).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(launchButton()).toBeEnabled());

    fireEvent.click(launchButton());
    await act(async () => {});

    expect(onLaunch).toHaveBeenCalledOnce();
    expect(probe).toHaveBeenCalledTimes(1);
    expect(launchButton()).toBeEnabled();
  });

  it('does not re-probe after a failed launch even once the cached verdict has expired', async () => {
    const probe = mockBackend(reports.compatible);
    const onLaunch = renderModal(vi.fn(() => Promise.resolve('failed' as const)));
    await fillIn();
    await pickMachine(box.id);
    await waitFor(() => expect(probe).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(launchButton()).toBeEnabled());

    // A description that took longer to write than the verdict lives.
    const expired = Date.now() + RUNNER_COMPATIBILITY_TTL_MS + 1;
    const clock = vi.spyOn(Date, 'now').mockReturnValue(expired);
    try {
      fireEvent.click(launchButton());
      await act(async () => {});
    } finally {
      clock.mockRestore();
    }

    expect(onLaunch).toHaveBeenCalledOnce();
    expect(probe).toHaveBeenCalledTimes(1);
  });

  it('re-probes the runner after the runner refused the launch, and blocks on the fresh verdict', async () => {
    const probe = mockBackend(reports.compatible);
    probe.mockResolvedValueOnce(reports.compatible).mockResolvedValue(reports.runner_behind);
    // What `useLaunchRun` does with a `runner_incompatible` refusal.
    const onLaunch = renderModal(
      vi.fn(() => {
        invalidateRunnerCompatibility(box.id);
        return Promise.resolve('runner_refused' as const);
      }),
    );
    await fillIn();
    await pickMachine(box.id);
    await waitFor(() => expect(probe).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(launchButton()).toBeEnabled());

    fireEvent.click(launchButton());

    expect(onLaunch).toHaveBeenCalledOnce();
    expect(await screen.findByRole('alert')).toHaveTextContent(reports.runner_behind.message);
    expect(probe).toHaveBeenCalledTimes(2);
    expect(launchButton()).toBeDisabled();
  });

  it('links the mismatch notice to machine settings', async () => {
    mockBackend(reports.runner_behind);
    renderModal();

    await pickMachine(box.id);
    fireEvent.click(await screen.findByRole('button', { name: /open machine settings/i }));

    expect(navigate).toHaveBeenCalledWith({ kind: 'settings' });
  });

  it('re-checks a cached blocking verdict on request and enables launch once the runner matches', async () => {
    const probe = mockBackend(reports.runner_behind);
    probe.mockResolvedValueOnce(reports.runner_behind).mockResolvedValue(reports.compatible);
    renderModal();
    await fillIn();
    await pickMachine(box.id);
    expect(await screen.findByRole('alert')).toHaveTextContent(reports.runner_behind.message);
    expect(launchButton()).toBeDisabled();
    expect(probe).toHaveBeenCalledTimes(1);

    fireEvent.click(screen.getByRole('button', { name: /check again/i }));

    await waitFor(() => expect(probe).toHaveBeenCalledTimes(2));
    await waitFor(() => expect(screen.queryByRole('alert')).not.toBeInTheDocument());
    expect(launchButton()).toBeEnabled();
  });
});
