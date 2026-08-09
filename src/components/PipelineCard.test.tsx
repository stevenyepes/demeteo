// Regression: a keystroke in the project view's composer must not re-render
// the pipeline list.
//
// `ProjectHome` owns the composer's `featureInput`, the staged attachments and
// the feature rows in the same component, so every character typed re-rendered
// every card — each one re-deriving its own workflow and transport badges on
// the way through.
//
// Renders are counted through a pass-through `Cpu` stub rather than by
// asserting `PipelineCard` is memoized: the transport badge is the one icon a
// card always renders and no other element of this view renders (the empty
// state's `Cpu` needs an empty list), so the count is renders of the real card
// subtree — including the case where an unstable prop defeats the memo.

import { invoke, type InvokeArgs } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { act, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { useEffect, useState, type ComponentProps, type ReactElement, type ReactNode } from 'react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import {
  NavigationProvider,
  ProjectProvider,
  TerminalPanelProvider,
  UIStateProvider,
  useProject,
} from '../context';
import { PipelineCard, type PipelineCardProps } from './PipelineCard';
import ProjectHome from './ProjectHome';
import { buildWorkflowById } from '../lib/workflowBadge';
import type { Feature, Project } from '../types';

let cardIconRenders = 0;

vi.mock('lucide-react', async (importOriginal) => {
  const actual = await importOriginal<typeof import('lucide-react')>();
  const Cpu = (props: ComponentProps<typeof actual.Cpu>) => {
    cardIconRenders += 1;
    return <actual.Cpu {...props} />;
  };
  return { ...actual, Cpu };
});

vi.mock('@tauri-apps/api/webview', () => ({
  getCurrentWebview: () => ({
    onDragDropEvent: vi.fn().mockResolvedValue(() => {}),
  }),
}));

function feature(overrides: Partial<Feature> = {}): Feature {
  return {
    id: 'f-1',
    project_id: 'proj-1',
    workflow_id: 'wf-feature',
    title: 'Add a retry budget to the verifier',
    description: 'Cap agent retries per step.',
    status: 'running',
    total_cost: 1.25,
    tokens: 12_500,
    duration: '2m 10s',
    created_at: 1,
    ...overrides,
  };
}

const FEATURES: Feature[] = [
  feature({ id: 'f-1', title: 'Retry budget', status: 'running' }),
  feature({ id: 'f-2', title: 'Gate strip', status: 'awaiting_gate' }),
  feature({ id: 'f-3', title: 'Cost column', status: 'failed', workflow_id: 'wf-gone' }),
];

/** Deliver a `feature_status_changed` payload to the listener `ProjectHome`
 *  registered through `useTauriEvent`. */
function emitStatusChanged(payload: { feature_id: string; status: string }) {
  for (const [event, handler] of vi.mocked(listen).mock.calls) {
    if (event === 'feature_status_changed') {
      (handler as (e: { payload: unknown }) => void)({ payload });
    }
  }
}

function mockBackend(features: Feature[]) {
  vi.mocked(invoke).mockImplementation((cmd: string, args?: InvokeArgs) => {
    switch (cmd) {
      case 'fetch_active_features':
        return Promise.resolve(features);
      case 'get_repositories_for_project': {
        const { projectId } = (args ?? {}) as { projectId?: string };
        return Promise.resolve([
          { id: `${projectId ?? 'proj'}-repo-0`, repo_path: '/repo/one', provider_id: 'provider-1' },
        ]);
      }
      case 'workflow_list':
        return Promise.resolve([
          { id: 'wf-feature', name: 'Standard Feature Pipeline', is_starter: false },
        ]);
      case 'remote_list_mirrored_runs':
        return Promise.resolve([]);
      case 'list_terminal_sessions':
        return Promise.resolve([]);
      case 'list_terminal_locations':
        return Promise.resolve({ main_branch: 'main', worktrees: [] });
      default:
        return Promise.resolve(undefined);
    }
  });
}

// `ProjectHome` resolves `activeProject` with a non-null assertion during its
// first render, so the project has to be in context before it mounts at all.
function ProjectSeed({ project, children }: { project: Project; children: ReactNode }): ReactElement | null {
  const { dispatch } = useProject();
  const [seeded, setSeeded] = useState(false);
  useEffect(() => {
    dispatch({ type: 'LOAD_PROJECTS', projects: [project], reposByProject: {} });
    dispatch({ type: 'SET_CURRENT', id: project.id });
    setSeeded(true);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  if (!seeded) return null;
  return <>{children}</>;
}

function mountHome(project: Partial<Project> = {}) {
  render(
    <NavigationProvider>
      <ProjectProvider>
        <UIStateProvider>
          <TerminalPanelProvider>
            <ProjectSeed
              project={{
                id: 'proj-1',
                name: 'Demo Project',
                status: 'idle',
                repos: 1,
                nodes: 0,
                spend: 0,
                tokens: 0,
                compute_type: 'local',
                ...project,
              }}
            >
              <ProjectHome />
            </ProjectSeed>
          </TerminalPanelProvider>
        </UIStateProvider>
      </ProjectProvider>
    </NavigationProvider>,
  );
}

beforeEach(() => {
  cardIconRenders = 0;
  vi.mocked(invoke).mockReset();
});

const workflowById = buildWorkflowById([
  { id: 'wf-feature', name: 'Standard Feature Pipeline', is_starter: false },
]);

function renderCard(overrides: Partial<PipelineCardProps> = {}) {
  const onOpen = vi.fn();
  const { container } = render(
    <PipelineCard
      feature={feature()}
      workflowById={workflowById}
      detached={false}
      computeType="local"
      remoteHost={null}
      onOpen={onOpen}
      {...overrides}
    />,
  );
  return { container, onOpen, card: container.firstElementChild as HTMLElement };
}

describe('card contents', () => {
  it('renders the status chip, workflow, transport, id, title and metrics', () => {
    const { card } = renderCard();

    expect(card.className).toContain('glass-panel glass-panel-hover');
    expect(card.firstElementChild?.className).toContain('bg-cyan-500 shadow-[0_0_10px_rgba(6,182,212,0.8)]');

    const status = screen.getByText('Running');
    expect(status.className).toContain('bg-cyan-500/10 text-cyan-400 border-cyan-500/20');
    // The pulse dot is the `active` affordance, and only active runs get one.
    expect(status.querySelector('.animate-pulse')).not.toBeNull();

    expect(screen.getByText('Standard Feature Pipeline')).toBeInTheDocument();
    expect(screen.getByText('Custom')).toBeInTheDocument();
    expect(screen.getByText('f-1')).toBeInTheDocument();
    expect(screen.getByText('Add a retry budget to the verifier')).toBeInTheDocument();
    expect(screen.getByText('Cap agent retries per step.')).toBeInTheDocument();
    expect(screen.getByText('2m 10s')).toBeInTheDocument();
    expect(screen.getByText('12.5k')).toBeInTheDocument();
  });

  it('mutes the workflow badge when the reference is missing', () => {
    renderCard({ feature: feature({ workflow_id: 'wf-gone' }) });

    expect(screen.getByText('Workflow: unknown')).toHaveAttribute('title', 'Workflow reference missing');
    expect(screen.queryByText('Custom')).not.toBeInTheDocument();
  });

  it('labels a local run without competing with the status chip', () => {
    renderCard();

    const transport = screen.getByTitle('Executes on this machine');
    expect(transport).toHaveTextContent('Local');
    expect(transport.className).toContain('bg-white/5 text-slate-500 border-white/10');
  });

  it('names the host for an attached remote run', () => {
    renderCard({ computeType: 'remote', remoteHost: 'gpu-box' });

    const transport = screen.getByTitle('Executes on gpu-box over SSH');
    expect(transport).toHaveTextContent('Remote · SSH');
    expect(transport.className).toContain('bg-cyan-500/10 text-cyan-400 border-cyan-500/20');
  });

  it('reports a detached run even on a local project', () => {
    renderCard({ detached: true });

    expect(screen.getByText(/Detached/)).toBeInTheDocument();
  });

  it('renders no description block when there is nothing to show', () => {
    const { card } = renderCard({ feature: feature({ description: '   ' }) });

    expect(card.querySelector('p')).toBeNull();
  });

  it('reports the row it was clicked for without the parent closing over it', async () => {
    const { onOpen } = renderCard();

    await userEvent.click(screen.getByText('Add a retry budget to the verifier'));

    expect(onOpen).toHaveBeenCalledWith('f-1', 'Add a retry budget to the verifier');
  });
});

describe('pipeline list re-renders', () => {
  it('does not re-render the cards while the composer is typed into', async () => {
    mockBackend(FEATURES);
    mountHome();

    await waitFor(() => expect(screen.getByText('Retry budget')).toBeInTheDocument());
    const rendersAfterLoad = cardIconRenders;
    expect(rendersAfterLoad).toBeGreaterThanOrEqual(FEATURES.length);

    await userEvent.type(
      screen.getByPlaceholderText('Draft and delegate a new feature pipeline...'),
      'cap the retries',
    );

    expect(cardIconRenders).toBe(rendersAfterLoad);
  });

  it('re-renders only the card a status event names', async () => {
    mockBackend(FEATURES);
    mountHome();

    await waitFor(() => expect(screen.getByText('Retry budget')).toBeInTheDocument());
    const rendersAfterLoad = cardIconRenders;

    await act(async () => {
      emitStatusChanged({ feature_id: 'f-2', status: 'completed' });
    });

    await waitFor(() => expect(screen.getByText('Completed')).toBeInTheDocument());
    expect(cardIconRenders).toBe(rendersAfterLoad + 1);
  });
});
