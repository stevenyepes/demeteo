import { cleanup, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { CanvasNode, Feature, NodeResolution, StepExecution } from '../../types';
import type { CanvasNeighbor } from '../../lib/askCanvasEdges';

const resolveNodeMock = vi.fn();
vi.mock('../../lib/ask', () => ({
  resolveNode: (...args: unknown[]) => resolveNodeMock(...args),
}));

const fetchActiveFeaturesMock = vi.fn();
const listStepsForRunMock = vi.fn();
vi.mock('../../lib/features', () => ({
  fetchActiveFeatures: (...args: unknown[]) => fetchActiveFeaturesMock(...args),
  listStepsForRun: (...args: unknown[]) => listStepsForRunMock(...args),
}));

const navigateMock = vi.fn();
vi.mock('../../context', () => ({
  useNavigation: () => ({ navigate: navigateMock }),
}));

import { AskCanvasNodeInspector } from './AskCanvasNodeInspector';

afterEach(cleanup);

const NODE: CanvasNode = {
  id: 'n1',
  title: 'ExecutionDriver',
  role: 'orchestration',
  path: 'step_executor/driver.rs',
  stage: 0,
  lane: 0,
};

const INCOMING: CanvasNeighbor[] = [{ nodeId: 'n0', title: 'Feature intake', kind: 'hands_off' }];
const OUTGOING: CanvasNeighbor[] = [{ nodeId: 'n2', title: 'Gate approval', kind: 'goes_back' }];

const EDITOR_RESOLUTION: NodeResolution = {
  kind: 'editor',
  machine_id: 'local',
  worktree_path: '/repos/demeteo_wt_f-1',
  branch: 'feature/f-1',
  default_branch: 'master',
  path: 'step_executor/driver.rs',
};

function feature(overrides: Partial<Feature> = {}): Feature {
  return {
    id: 'f-1',
    project_id: 'p-1',
    title: 'Add a metric strip',
    status: 'running',
    total_cost: 0,
    duration: '0s',
    created_at: 0,
    ...overrides,
  };
}

function stepExecution(overrides: Partial<StepExecution> = {}): StepExecution {
  return {
    id: 'se-1',
    feature_id: 'f-1',
    step_id: 'step-1',
    step_index: 0,
    step_kind: 'agent',
    status: 'completed',
    artifact_path: null,
    artifact_paths: [],
    created_at: 0,
    updated_at: 0,
    ...overrides,
  };
}

function renderInspector(overrides: Partial<Parameters<typeof AskCanvasNodeInspector>[0]> = {}) {
  const onDismiss = vi.fn();
  render(
    <AskCanvasNodeInspector
      node={NODE}
      description="What the ExecutionDriver does."
      incoming={INCOMING}
      outgoing={OUTGOING}
      threadId="thread-1"
      messageId="message-1"
      projectId="p-1"
      onDismiss={onDismiss}
      {...overrides}
    />,
  );
  return { onDismiss };
}

beforeEach(() => {
  resolveNodeMock.mockReset();
  fetchActiveFeaturesMock.mockReset();
  listStepsForRunMock.mockReset();
  navigateMock.mockReset();
  fetchActiveFeaturesMock.mockResolvedValue([]);
  listStepsForRunMock.mockResolvedValue([]);
});

describe('AskCanvasNodeInspector', () => {
  it('renders title, role, description, path, edges, and an Open in editor action (AC-5)', async () => {
    resolveNodeMock.mockResolvedValue(EDITOR_RESOLUTION);

    renderInspector();

    expect(screen.getByText('ExecutionDriver')).toBeInTheDocument();
    expect(screen.getByText('Orchestration')).toBeInTheDocument();
    expect(screen.getByText('What the ExecutionDriver does.')).toBeInTheDocument();
    expect(screen.getByText('Feature intake')).toBeInTheDocument();
    expect(screen.getByText('Gate approval')).toBeInTheDocument();

    await waitFor(() =>
      expect(screen.getByRole('button', { name: /open in editor/i })).toBeInTheDocument(),
    );
    expect(screen.getByText('step_executor/driver.rs')).toBeInTheDocument();
  });

  it('shows the stored path with no action while resolution is pending', () => {
    resolveNodeMock.mockReturnValue(new Promise(() => {}));
    fetchActiveFeaturesMock.mockReturnValue(new Promise(() => {}));

    renderInspector();

    expect(screen.getByText('step_executor/driver.rs')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /open in editor/i })).not.toBeInTheDocument();
  });

  it('shows "Show in the pipeline" when the node path matches a Step artifact (AC-6, positive)', async () => {
    resolveNodeMock.mockResolvedValue(EDITOR_RESOLUTION);
    fetchActiveFeaturesMock.mockResolvedValue([feature({ id: 'f-1', title: 'Add a metric strip' })]);
    listStepsForRunMock.mockResolvedValue([
      stepExecution({ step_id: 'step-9', artifact_path: 'step_executor/driver.rs' }),
    ]);

    renderInspector();

    const button = await screen.findByRole('button', { name: /show in the pipeline/i });
    button.click();

    expect(navigateMock).toHaveBeenCalledWith({
      kind: 'detail',
      featureId: 'f-1',
      featureTitle: 'Add a metric strip',
      selectedStepId: 'step-9',
    });
  });

  it('matches via artifact_paths as well as artifact_path (AC-6, positive)', async () => {
    resolveNodeMock.mockResolvedValue(EDITOR_RESOLUTION);
    fetchActiveFeaturesMock.mockResolvedValue([feature({ id: 'f-2', title: 'Other feature' })]);
    listStepsForRunMock.mockResolvedValue([
      stepExecution({ step_id: 'step-3', artifact_paths: ['a.rs', 'step_executor/driver.rs'] }),
    ]);

    renderInspector();

    expect(await screen.findByRole('button', { name: /show in the pipeline/i })).toBeInTheDocument();
  });

  it('omits "Show in the pipeline" entirely when nothing matches (AC-6, negative)', async () => {
    resolveNodeMock.mockResolvedValue(EDITOR_RESOLUTION);
    fetchActiveFeaturesMock.mockResolvedValue([feature()]);
    listStepsForRunMock.mockResolvedValue([stepExecution({ artifact_path: 'unrelated/path.rs' })]);

    renderInspector();

    await waitFor(() => expect(fetchActiveFeaturesMock).toHaveBeenCalled());
    await screen.findByRole('button', { name: /open in editor/i });

    expect(screen.queryByRole('button', { name: /show in the pipeline/i })).not.toBeInTheDocument();
    expect(screen.queryByText(/show in the pipeline/i)).not.toBeInTheDocument();
  });

  it('routes Open in editor through the shared EditorContext / navigate({kind:"editor"}) path (AC-7)', async () => {
    resolveNodeMock.mockResolvedValue(EDITOR_RESOLUTION);

    renderInspector();

    const button = await screen.findByRole('button', { name: /open in editor/i });
    button.click();

    expect(navigateMock).toHaveBeenCalledWith({
      kind: 'editor',
      editorContext: {
        machineId: 'local',
        worktreePath: '/repos/demeteo_wt_f-1',
        branch: 'feature/f-1',
        defaultBranch: 'master',
        initialFile: 'step_executor/driver.rs',
      },
    });
  });

  it('renders moved copy naming the stored checked_commit_sha and no Open in editor action', async () => {
    resolveNodeMock.mockResolvedValue({
      kind: 'moved',
      checked_commit_sha: 'abc1234567890def',
    } satisfies NodeResolution);

    renderInspector();

    await waitFor(() => expect(screen.getByText(/moved since/i)).toBeInTheDocument());
    expect(screen.getByText(/abc12345/)).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /open in editor/i })).not.toBeInTheDocument();
  });

  it('calls resolveNode once per open with the given thread/message/node ids', async () => {
    resolveNodeMock.mockResolvedValue(EDITOR_RESOLUTION);

    renderInspector();

    await waitFor(() => expect(resolveNodeMock).toHaveBeenCalledTimes(1));
    expect(resolveNodeMock).toHaveBeenCalledWith({
      threadId: 'thread-1',
      messageId: 'message-1',
      nodeId: 'n1',
    });
  });
});
