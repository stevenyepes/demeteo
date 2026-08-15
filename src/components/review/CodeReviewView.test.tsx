// The degradation requirement, asserted where it can regress: a rejected token
// and an empty queue must not render as the same page. The failure mode this
// guards is silent — an error swallowed into `[]` renders "Nothing is waiting
// for review right now", which is the one message that makes a user stop
// looking.

import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { useEffect, useState, type ReactElement, type ReactNode } from 'react';
import { describe, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';

import { NavigationProvider, ProjectProvider, useProject } from '../../context';
import type { Project } from '../../types';
import { CodeReviewView } from './CodeReviewView';

const PROJECT: Project = {
  id: 'proj-1',
  name: 'Demo Project',
  status: 'idle',
  repos: 1,
  nodes: 0,
  spend: 0,
  tokens: 0,
};

const PULL_REQUEST = {
  number: 412,
  title: 'Tighten the refspec guard',
  author: 'octocat',
  source_branch: 'patch-1',
  target_branch: 'main',
  draft: true,
  web_url: 'https://github.com/acme/app/pull/412',
  created_at: '2026-08-12T09:00:00Z',
  updated_at: '2026-08-15T07:00:00Z',
  head_repo_path: 'octocat/app',
  head_fetch_spec: 'refs/pull/412/head',
  from_fork: true,
  maintainer_can_modify: false,
  head_repo_push: false,
  additions: 120,
  deletions: 8,
  changed_files: 3,
};

/** Answers `list_open_pull_requests` and rejects everything else: a stub that
 *  resolves every command would let this suite pass against a view that called
 *  the wrong one. */
function backend(answer: () => Promise<unknown>) {
  vi.mocked(invoke).mockImplementation((cmd: string) => {
    if (cmd === 'list_open_pull_requests') return answer();
    return Promise.reject(new Error(`unexpected command: ${cmd}`));
  });
}

function ProjectSeed({ children }: { children: ReactNode }): ReactElement | null {
  const { dispatch } = useProject();
  const [seeded, setSeeded] = useState(false);
  useEffect(() => {
    dispatch({ type: 'LOAD_PROJECTS', projects: [PROJECT], reposByProject: {} });
    dispatch({ type: 'SET_CURRENT', id: PROJECT.id });
    setSeeded(true);
  }, [dispatch]);
  return seeded ? <>{children}</> : null;
}

function mount() {
  return render(
    <NavigationProvider>
      <ProjectProvider>
        <ProjectSeed>
          <CodeReviewView />
        </ProjectSeed>
      </ProjectProvider>
    </NavigationProvider>,
  );
}

describe('CodeReviewView', () => {
  it('names an empty queue as empty', async () => {
    backend(() => Promise.resolve([]));
    mount();

    expect(await screen.findByText('No open pull requests')).toBeInTheDocument();
    expect(screen.queryByTestId('code-review-failure')).not.toBeInTheDocument();
  });

  it('reads a rejected token as a token to reconnect, not as an empty queue', async () => {
    backend(() =>
      Promise.reject(
        JSON.stringify({
          kind: 'token-rejected',
          provider: 'github',
          host: 'api.github.com',
          status: 401,
        }),
      ),
    );
    mount();

    const card = await screen.findByTestId('code-review-failure');
    expect(card).toHaveAttribute('data-failure', 'token-rejected');
    expect(screen.getByText('Your GitHub token was rejected')).toBeInTheDocument();
    expect(screen.getByText(/api\.github\.com answered 401/)).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Reconnect GitHub' })).toBeInTheDocument();
    expect(screen.queryByText('No open pull requests')).not.toBeInTheDocument();
  });

  it('quotes a provider error instead of paraphrasing it', async () => {
    backend(() =>
      Promise.reject(
        JSON.stringify({
          kind: 'api-error',
          host: 'gitlab.example.com',
          status: 503,
          body: '{"message":"upstream unavailable"}',
        }),
      ),
    );
    mount();

    expect(await screen.findByTestId('code-review-failure-detail')).toHaveTextContent(
      'upstream unavailable',
    );
  });

  it('refetches when the failure card offers a retry', async () => {
    let attempts = 0;
    backend(() => {
      attempts += 1;
      return attempts === 1
        ? Promise.reject(JSON.stringify({ kind: 'rate-limited', host: 'api.github.com' }))
        : Promise.resolve([]);
    });
    mount();

    await userEvent.click(await screen.findByRole('button', { name: 'Retry' }));

    expect(await screen.findByText('No open pull requests')).toBeInTheDocument();
  });

  it('renders a pull request across its three tiers', async () => {
    backend(() => Promise.resolve([PULL_REQUEST]));
    mount();

    const row = await screen.findByTestId('pull-request-row');
    expect(row).toHaveAttribute('href', 'https://github.com/acme/app/pull/412');
    expect(row).toHaveTextContent('Tighten the refspec guard');
    expect(row).toHaveTextContent('#412');
    expect(row).toHaveTextContent('patch-1 → main');
    expect(row).toHaveTextContent('+120');
    expect(row).toHaveTextContent('3 files');
  });

  it('launches a review on the head ref, measured against the target branch', async () => {
    const launches: Record<string, unknown>[] = [];
    vi.mocked(invoke).mockImplementation((cmd: string, args?: unknown) => {
      if (cmd === 'list_open_pull_requests') return Promise.resolve([PULL_REQUEST]);
      if (cmd === 'start_feature') {
        launches.push(typeof args === 'object' && args !== null ? { ...args } : {});
        return Promise.resolve({ id: 'feat-1', title: 'Review PR #412', status: 'running' });
      }
      return Promise.reject(new Error(`unexpected command: ${cmd}`));
    });
    mount();

    await userEvent.click(await screen.findByRole('button', { name: 'Add instructions' }));
    await userEvent.type(
      screen.getByLabelText('Extra instructions (optional)'),
      'Concentrate on the fence.',
    );
    await userEvent.click(screen.getByTestId('review-this-pr'));

    await waitFor(() => expect(launches).toHaveLength(1));
    expect(launches[0].workflowId).toBe('wf-starter-code-review');
    expect(launches[0].origin).toEqual({
      kind: 'ref',
      fetch_spec: 'refs/pull/412/head',
      label: 'patch-1',
    });
    expect(launches[0].diffBaseBranch).toBe('main');
    expect(launches[0].description).toContain('Concentrate on the fence.');
  });

  it('shows flat skeleton rows while the list is in flight', async () => {
    let release: (value: unknown) => void = () => {};
    backend(() => new Promise((resolve) => { release = resolve; }));
    mount();

    const skeleton = await screen.findByLabelText('Loading open pull requests');
    expect(skeleton).toHaveAttribute('role', 'status');

    release([]);
    await waitFor(() => expect(screen.getByText('No open pull requests')).toBeInTheDocument());
  });
});
