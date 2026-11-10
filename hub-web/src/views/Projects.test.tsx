import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';

import type { HubProject } from '../lib/projects';
import type { HubWorkflow } from '../lib/workflows';
import { Projects } from './Projects';

const api = vi.hoisted(() => ({
  listProjects: vi.fn<() => Promise<HubProject[]>>(),
  removeProject: vi.fn<(id: string) => Promise<void>>(),
  listWorkflows: vi.fn<() => Promise<HubWorkflow[]>>(),
}));

vi.mock('../lib/projects', () => ({
  listProjects: api.listProjects,
  removeProject: api.removeProject,
}));
vi.mock('../lib/workflows', () => ({ listWorkflows: api.listWorkflows }));

const WORKFLOWS: HubWorkflow[] = [
  { workflowId: 'wf-starter-feature', version: 11, name: 'Feature', description: 'Build it', graph: null },
  { workflowId: 'wf-starter-hotfix', version: 12, name: 'Hotfix', description: 'Fix it', graph: null },
];

const WIDGETS: HubProject = {
  id: 'p-1',
  name: 'widgets',
  providerKind: 'github',
  host: 'github.com',
  fullName: 'acme/widgets',
  remoteUrl: 'https://github.com/acme/widgets',
  runnerReady: true,
  defaultAgentKind: 'codex',
  defaultModel: null,
  defaultEffort: 'high',
  defaultWorkflowId: 'wf-starter-hotfix',
  defaultMaxBudgetCents: 250,
  createdAt: 1,
  updatedAt: 1,
};

const NOTES: HubProject = {
  ...WIDGETS,
  id: 'p-2',
  name: 'notes',
  providerKind: 'gitlab',
  host: 'gitlab.com',
  fullName: 'acme/notes',
  remoteUrl: 'https://gitlab.com/acme/notes',
  runnerReady: false,
  defaultAgentKind: null,
  defaultEffort: null,
  defaultWorkflowId: null,
  defaultMaxBudgetCents: null,
};

function listing(projects: HubProject[]) {
  api.listProjects.mockResolvedValue(projects);
  api.listWorkflows.mockResolvedValue(WORKFLOWS);
}

function rowOf(name: string): HTMLElement {
  const row = screen.getByText(name).closest('li');
  if (row === null) throw new Error(`no row for ${name}`);
  return row;
}

afterEach(() => {
  vi.clearAllMocks();
});

describe('Projects', () => {
  it('lists each project with its repository and defaults', async () => {
    listing([WIDGETS]);
    render(<Projects />);

    await screen.findByText('widgets');
    const row = rowOf('widgets');
    expect(row).toHaveTextContent('github.com/acme/widgets');
    expect(row).toHaveTextContent('Hotfix · codex · High effort · $2.50 max');
    expect(screen.getByText('1 project')).toBeVisible();
  });

  it('says in words, on a neutral tone, that a runner cannot clone a GitLab project', async () => {
    listing([NOTES]);
    render(<Projects />);

    await screen.findByText('notes');
    const note = within(rowOf('notes')).getByText('Runners cannot clone GitLab projects yet');
    expect(note.className).not.toMatch(/amber|ruby/);
    expect(within(rowOf('notes')).getByText('No defaults')).toBeVisible();
  });

  it('adds from its own page, and edits each project on its page', async () => {
    listing([WIDGETS]);
    render(<Projects />);

    await screen.findByText('widgets');
    expect(screen.getByRole('link', { name: 'Add project' })).toHaveAttribute('href', '#/projects/new');
    expect(within(rowOf('widgets')).getByRole('link', { name: 'Edit' })).toHaveAttribute('href', '#/projects/p-1');
  });

  it('removes a project only once the removal is confirmed', async () => {
    listing([WIDGETS]);
    api.removeProject.mockResolvedValue();
    render(<Projects />);

    await screen.findByText('widgets');
    await userEvent.click(within(rowOf('widgets')).getByRole('button', { name: 'Remove' }));
    expect(api.removeProject).not.toHaveBeenCalled();
    listing([]);
    await userEvent.click(screen.getByRole('button', { name: 'Remove from this Hub' }));

    expect(api.removeProject).toHaveBeenCalledWith('p-1');
    expect(await screen.findByText(/None yet/)).toBeVisible();
  });
});
