// Unit tests for the `task-list` viewType wired into ArtifactViewer: a
// well-formed task-list.json renders TaskListArtifact instead of Monaco, a
// malformed or legacy payload at the same path falls back to Monaco without
// throwing, and a non-task-list JSON artifact is unaffected.

import { invoke } from '@tauri-apps/api/core';
import { render, screen, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { ArtifactViewer } from './ArtifactViewer';

describe('ArtifactViewer task-list viewType', () => {
  it('renders TaskListArtifact for a well-formed task-list.json payload', async () => {
    const plan = {
      kind: 'greenfield',
      cycle: 0,
      tasks: [
        { id: 't1', title: 'Wire the artifact viewer', description: 'Do the thing' },
      ],
    };
    vi.mocked(invoke).mockResolvedValue(JSON.stringify(plan));

    render(<ArtifactViewer artifactPath="/tmp/artifacts/task-list.json" />);

    await waitFor(() => {
      expect(screen.getByText('Wire the artifact viewer')).toBeInTheDocument();
    });

    expect(screen.queryByTestId('monaco-editor')).not.toBeInTheDocument();
    expect(screen.queryByText(/"kind":\s*"greenfield"/)).not.toBeInTheDocument();
  });

  it('falls back to Monaco for malformed JSON at a task-list.json path', async () => {
    vi.mocked(invoke).mockResolvedValue('{not valid json');

    render(<ArtifactViewer artifactPath="/tmp/artifacts/task-list.json" />);

    await waitFor(() => {
      expect(screen.getByTestId('monaco-editor')).toBeInTheDocument();
    });

    expect(screen.queryByText(/Failed to/)).not.toBeInTheDocument();
  });

  it('falls back to Monaco for a legacy subtasks payload at a task-list.json path', async () => {
    const legacy = { subtasks: [{ id: 't1', title: 'Legacy', description: 'Old shape' }] };
    vi.mocked(invoke).mockResolvedValue(JSON.stringify(legacy));

    render(<ArtifactViewer artifactPath="/tmp/artifacts/task-list.json" />);

    await waitFor(() => {
      expect(screen.getByTestId('monaco-editor')).toBeInTheDocument();
    });

    expect(screen.queryByText(/Failed to/)).not.toBeInTheDocument();
  });

  it('renders plain Monaco code view for a non-task-list.json JSON artifact', async () => {
    const payload = { kind: 'greenfield', cycle: 0, tasks: [{ id: 't1', title: 'Nope', description: 'x' }] };
    vi.mocked(invoke).mockResolvedValue(JSON.stringify(payload));

    render(<ArtifactViewer artifactPath="/tmp/artifacts/other.json" />);

    await waitFor(() => {
      expect(screen.getByTestId('monaco-editor')).toBeInTheDocument();
    });

    expect(screen.queryByText('Nope')).not.toBeInTheDocument();
  });
});
