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
import { pipelineDensityClasses } from '../lib/density';
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

function tier(card: HTMLElement, name: 'scan' | 'context' | 'detail'): HTMLElement {
  const el = card.querySelector<HTMLElement>(`[data-tier="${name}"]`);
  if (!el) throw new Error(`the card rendered no ${name} tier`);
  return el;
}

// The tiers are asserted by *which* tier a field landed in, not by its classes.
// `pipelineCardMeta` has grouped these three ways since Phase 0 and the row
// still rendered all eight at one weight, so a test that only checks a field is
// present passes on exactly the layout this phase exists to replace.
describe('three-tier read', () => {
  it('keeps the scan tier to the title, the status and the elapsed time', () => {
    const { card } = renderCard();
    const scan = tier(card, 'scan');

    expect(scan).toHaveTextContent('Add a retry budget to the verifier');
    expect(scan).toHaveTextContent('Running');
    expect(scan).toHaveTextContent('2m 10s');

    expect(scan).not.toHaveTextContent('Standard Feature Pipeline');
    expect(scan).not.toHaveTextContent('Local');
    expect(scan).not.toHaveTextContent('$1.25');
    expect(scan).not.toHaveTextContent('f-1');
    expect(scan).not.toHaveTextContent('Cap agent retries per step.');
  });

  it('groups the workflow, transport, cost and tokens as context', () => {
    const { card } = renderCard();
    const context = tier(card, 'context');

    expect(context).toHaveTextContent('Standard Feature Pipeline');
    expect(context).toHaveTextContent('Local');
    expect(context).toHaveTextContent('$1.25');
    expect(context).toHaveTextContent('12.5k');
  });

  // Audit Opportunity 5: `pipelineCardMeta` has formatted this all along and
  // the row rendered duration and tokens beside it and never the cost.
  it('renders the cost', () => {
    renderCard({ feature: feature({ total_cost: 12.5 }) });

    expect(screen.getByText('$12.50')).toBeInTheDocument();
  });

  it('drops the feature id and the description to the detail tier', () => {
    const { card } = renderCard();
    const detail = tier(card, 'detail');

    expect(detail).toHaveTextContent('f-1');
    expect(detail).toHaveTextContent('Cap agent retries per step.');
  });

  // "On demand" is the hover title, not a disclosure: one line of prose costs
  // less to read than to reveal, and a per-row open/closed state in a memoized
  // list buys the click back in re-renders.
  it('carries the full description in a title rather than behind a click', () => {
    const { card } = renderCard({
      feature: feature({ description: 'Cap agent retries per step, then fail the run.' }),
    });

    expect(card.querySelector('p')).toHaveAttribute(
      'title',
      'Cap agent retries per step, then fail the run.',
    );
    expect(card.querySelector('button')).toBeNull();
  });

  it('renders no description block when there is nothing to show', () => {
    const { card } = renderCard({ feature: feature({ description: '   ' }) });

    expect(card.querySelector('p')).toBeNull();
  });
});

describe('card contents', () => {
  it('accents the row with the run tone and skips the row off screen', () => {
    const { card } = renderCard();

    expect(card.className).toContain('glass-panel glass-panel-hover');
    expect(card.className).toContain('pipeline-card');
    expect(card.firstElementChild?.className).toContain('bg-cyan-500 shadow-[0_0_10px_rgba(6,182,212,0.8)]');
  });

  it('spells the status through Chip so the tone stays in one registry', () => {
    renderCard();

    const status = screen.getByText('Running').closest('[data-testid="chip"]');
    expect(status?.className).toContain('bg-cyan-500/10 text-cyan-400 border-cyan-500/20');
    // The pulse dot is the `active` affordance, and only active runs get one.
    expect(status?.querySelector('[data-testid="chip-dot"]')).not.toBeNull();
  });

  it('leaves a settled run without a pulse', () => {
    renderCard({ feature: feature({ status: 'completed' }) });

    const status = screen.getByText('Completed').closest('[data-testid="chip"]');
    expect(status?.querySelector('[data-testid="chip-dot"]')).toBeNull();
  });

  // §3.2: the list sorts these to the top, so the row has to look like the
  // reason it is there. `segmentFor` decides it — the card never re-derives it.
  it('rings a row that is waiting on a human', () => {
    const { card } = renderCard({ feature: feature({ status: 'awaiting_gate' }) });

    expect(card.className).toContain('ring-amber-500/40');
    expect(renderCard().card.className).not.toContain('ring-amber-500/40');
  });

  it('mutes the workflow badge when the reference is missing', () => {
    renderCard({ feature: feature({ workflow_id: 'wf-gone' }) });

    expect(screen.getByTitle('Workflow reference missing')).toHaveTextContent('Workflow: unknown');
    expect(screen.queryByText('Standard Feature Pipeline')).not.toBeInTheDocument();
  });

  // Starter-vs-custom is a fact about the workflow, not about this run, and it
  // was a second pill nested inside the first one.
  it('keeps the workflow origin in the tooltip rather than a nested pill', () => {
    renderCard();

    expect(screen.getByText('Standard Feature Pipeline').closest('[data-testid="chip"]'))
      .toHaveAttribute('title', 'Workflow: Standard Feature Pipeline (custom)');
    expect(screen.queryByText('Custom')).not.toBeInTheDocument();
  });

  it('labels a local run', () => {
    renderCard();

    const transport = screen.getByTitle('Executes on this machine');
    expect(transport).toHaveTextContent('Local');
    expect(transport.className).toContain('bg-slate-500/10 text-slate-400 border-slate-500/20');
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

  it('reports the row it was clicked for without the parent closing over it', async () => {
    const { onOpen } = renderCard();

    await userEvent.click(screen.getByText('Add a retry budget to the verifier'));

    expect(onOpen).toHaveBeenCalledWith('f-1', 'Add a retry budget to the verifier');
  });
});

describe('density', () => {
  it('opens comfortable when the caller offers no control', () => {
    const { card } = renderCard();

    expect(card.className).toContain('p-5');
    expect(card.querySelector('h3')?.className).toContain('text-lg');
  });

  it('sizes the card and the title from the classes it is handed', () => {
    const { card } = renderCard({ density: pipelineDensityClasses('compact') });

    expect(card.className).toContain('p-3');
    expect(card.className).not.toContain('p-5');
    expect(card.querySelector('h3')?.className).toContain('text-sm');
  });

  it('sizes the context and detail tiers too, so nothing stays comfortable', () => {
    const { card } = renderCard({ density: pipelineDensityClasses('compact') });

    expect(tier(card, 'context').className).toContain('text-[10px]');
    expect(tier(card, 'detail').className).toContain('text-[10px]');
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
