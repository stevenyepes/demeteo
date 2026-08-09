// Two claims this file defends. The second one first, because it is the one a
// reader is likely to undo: `FeatureDetail` may stay mounted while the app
// navigates to a view it does not render. It renders nothing then, and the
// hooks that drive the detail view live one component down, so the hook count
// never depends on `view.kind`. `App.tsx` mounts it conditionally today, so no
// screen in the app exercises that state — which is why it takes a test rather
// than a comment to keep it true (audit F17).
//
// The first: the artifact preview and the Gate overlay never occupy the screen
// at the same time.
//
// Why that needs a test rather than a comment. `App.tsx` renders `GateView` in
// an `OverlayPortal` *on top of* a still-mounted `FeatureDetail`, so both
// surfaces can be live at once. Both cards sit at `z-50` and both bind their
// own window-level `keydown`, so with an artifact open when a gate arrives one
// Escape dismisses both — the gate decision the user was routed here to make
// disappears along with the file they were reading. `FeatureDetail` therefore
// suppresses its own modal while `view.gateStepExecutionId` is set; the gate is
// the more urgent surface, and the artifact returns once it is resolved.
//
// The backend double below answers exactly the commands this mount fires and
// rejects everything else (AGENTS §7: the `FakeExec` that returned `Ok("")` for
// every command is what let assertions pass against a default instead of an
// answer). The global setup mock resolves every command to `undefined`, which
// is precisely that failure mode, so it is overridden here.

import { act, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { useEffect, useState, type ReactNode } from 'react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';

import {
  NavigationProvider,
  ProjectProvider,
  TerminalPanelProvider,
  UIStateProvider,
  useNavigation,
} from '../../context';
import { FeatureDetail } from './FeatureDetail';
import type { StepExecution } from '../../types';

vi.mock('react-markdown', () => ({
  default: ({ children }: { children?: ReactNode }) => (
    <div data-testid="markdown-body">{children}</div>
  ),
}));

const FEATURE_ID = 'f-1';
const ARTIFACT = '/tmp/artifacts/research-report.md';

const step = (): StepExecution => ({
  id: 'se-1',
  feature_id: FEATURE_ID,
  step_id: 's-research',
  step_index: 0,
  step_kind: 'agent',
  status: 'completed',
  artifact_paths: [ARTIFACT],
  created_at: 0,
  updated_at: 0,
});

/**
 * Answers only what this mount asks for. An unexpected command rejects so a
 * command that silently starts mattering shows up as a failure here rather
 * than as a bland `undefined` the component quietly renders around.
 */
function mockBackend() {
  vi.mocked(invoke).mockImplementation(((cmd: string) => {
    switch (cmd) {
      case 'step_list_for_run':
        return Promise.resolve([step()]);
      case 'feature_get':
        return Promise.resolve({ id: FEATURE_ID, status: 'awaiting_gate' });
      case 'feature_workflow_graph':
        return Promise.resolve(null);
      case 'feature_list_attachments':
        return Promise.resolve([]);
      case 'remote_run_for_feature':
        return Promise.resolve(null);
      case 'get_machines':
      case 'list_agents':
      case 'list_terminal_sessions':
        return Promise.resolve([]);
      case 'artifact_body':
        return Promise.resolve('# Research report');
      default:
        return Promise.reject(new Error(`unexpected IPC command: ${cmd}`));
    }
  }) as unknown as typeof invoke);
}

/**
 * Drives the real `NavigationProvider` into the detail view, and — once
 * `openGate` flips — into the same view *with* a gate pending, which is exactly
 * the transition `App.tsx` performs when a `gate_required` event lands while
 * the user is reading an artifact.
 */
function NavigationSeed({ openGate, children }: { openGate: boolean; children: ReactNode }) {
  const { navigate } = useNavigation();
  const [seeded, setSeeded] = useState(false);

  useEffect(() => {
    navigate({
      kind: 'detail',
      featureId: FEATURE_ID,
      featureTitle: 'Responsive run view',
      ...(openGate ? { gateStepExecutionId: 'se-1' } : {}),
    });
    setSeeded(true);
  }, [navigate, openGate]);

  if (!seeded) return null;
  return <>{children}</>;
}

function mount() {
  const view = render(
    <NavigationProvider>
      <ProjectProvider>
        <UIStateProvider>
          <TerminalPanelProvider>
            <NavigationSeed openGate={false}>
              <FeatureDetail />
            </NavigationSeed>
          </TerminalPanelProvider>
        </UIStateProvider>
      </ProjectProvider>
    </NavigationProvider>,
  );

  /** Re-renders the same tree with a gate pending on the detail view. */
  const openGate = () =>
    view.rerender(
      <NavigationProvider>
        <ProjectProvider>
          <UIStateProvider>
            <TerminalPanelProvider>
              <NavigationSeed openGate={true}>
                <FeatureDetail />
              </NavigationSeed>
            </TerminalPanelProvider>
          </UIStateProvider>
        </ProjectProvider>
      </NavigationProvider>,
    );

  return { openGate };
}

/**
 * Keeps `FeatureDetail` mounted across every view change, which `App.tsx` does
 * not do — it is the state the component has to be correct in rather than
 * accidentally safe in.
 */
function ViewSwitcher() {
  const { navigate } = useNavigation();
  return (
    <>
      <button
        type="button"
        onClick={() =>
          navigate({ kind: 'detail', featureId: FEATURE_ID, featureTitle: 'Responsive run view' })
        }
      >
        to detail
      </button>
      <button type="button" onClick={() => navigate({ kind: 'home' })}>
        to home
      </button>
      <FeatureDetail />
    </>
  );
}

function mountAlwaysOn() {
  render(
    <NavigationProvider>
      <ProjectProvider>
        <UIStateProvider>
          <TerminalPanelProvider>
            <ViewSwitcher />
          </TerminalPanelProvider>
        </UIStateProvider>
      </ProjectProvider>
    </NavigationProvider>,
  );
}

beforeEach(() => {
  mockBackend();
});

describe('FeatureDetail', () => {
  it('suppresses the artifact modal while a gate overlay is open', async () => {
    const { openGate } = mount();

    const artifact = await screen.findByTitle('research-report.md');
    await userEvent.click(artifact);

    // Baseline: with no gate pending the modal is the artifact surface.
    expect(await screen.findByTestId('artifact-modal-title')).toHaveTextContent(
      'research-report.md',
    );

    openGate();

    // The selection is untouched — the modal is withheld, not cleared — so the
    // artifact returns when the gate is resolved.
    await waitFor(() => {
      expect(screen.queryByTestId('artifact-modal-title')).not.toBeInTheDocument();
    });
    expect(screen.getByTitle('research-report.md')).toBeInTheDocument();
  });

  it('renders nothing while the view is not a detail view', async () => {
    const { container } = render(
      <NavigationProvider>
        <ProjectProvider>
          <UIStateProvider>
            <TerminalPanelProvider>
              <FeatureDetail />
            </TerminalPanelProvider>
          </UIStateProvider>
        </ProjectProvider>
      </NavigationProvider>,
    );

    // Settled, not merely synchronous: a component that fetched and then
    // rendered would still look empty on the first frame.
    await act(async () => {});
    expect(container).toBeEmptyDOMElement();
  });

  it('survives view-kind changes while staying mounted', async () => {
    const consoleError = vi.spyOn(console, 'error').mockImplementation(() => {});
    mountAlwaysOn();

    await userEvent.click(screen.getByRole('button', { name: 'to detail' }));
    expect(await screen.findByTitle('research-report.md')).toBeInTheDocument();

    await userEvent.click(screen.getByRole('button', { name: 'to home' }));
    await waitFor(() => {
      expect(screen.queryByTitle('research-report.md')).not.toBeInTheDocument();
    });

    await userEvent.click(screen.getByRole('button', { name: 'to detail' }));
    expect(await screen.findByTitle('research-report.md')).toBeInTheDocument();

    expect(consoleError).not.toHaveBeenCalled();
    consoleError.mockRestore();
  });
});
