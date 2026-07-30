// The claim this file defends: the artifact preview and the Gate overlay never
// occupy the screen at the same time.
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

import { render, screen, waitFor } from '@testing-library/react';
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
} from '../context';
import { FeatureDetail } from './FeatureDetail';
import type { StepExecution } from '../types';

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
});
