import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { HubApiError } from '../lib/api';
import type { HubProject, ProjectFields } from '../lib/projects';
import type { HubWorkflow } from '../lib/workflows';
import { Projects } from './Projects';

const api = vi.hoisted(() => ({
  listProjects: vi.fn<() => Promise<HubProject[]>>(),
  createProject: vi.fn<(fields: ProjectFields) => Promise<HubProject>>(),
  updateProject: vi.fn<(id: string, fields: ProjectFields) => Promise<HubProject>>(),
  removeProject: vi.fn<(id: string) => Promise<void>>(),
  listWorkflows: vi.fn<() => Promise<HubWorkflow[]>>(),
}));

vi.mock('../lib/projects', () => ({
  listProjects: api.listProjects,
  createProject: api.createProject,
  updateProject: api.updateProject,
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

  it('adds a project from a pasted remote, with its defaults, and reads the list again', async () => {
    listing([]);
    api.createProject.mockResolvedValue(WIDGETS);
    render(<Projects />);

    await userEvent.type(await screen.findByLabelText('Name'), 'widgets');
    await userEvent.type(screen.getByLabelText('Repository'), 'https://GitHub.com/acme/widgets');
    await userEvent.selectOptions(screen.getByLabelText('Default workflow'), 'wf-starter-hotfix');
    await userEvent.selectOptions(screen.getByLabelText('Default agent'), 'codex');
    await userEvent.selectOptions(screen.getByLabelText('Default effort'), 'high');
    await userEvent.type(screen.getByLabelText('Default max cost (USD)'), '2.50');
    listing([WIDGETS]);
    await userEvent.click(screen.getByRole('button', { name: 'Add project' }));

    expect(api.createProject).toHaveBeenCalledWith({
      name: 'widgets',
      providerKind: 'github',
      host: 'github.com',
      fullName: 'acme/widgets',
      defaultAgentKind: 'codex',
      defaultModel: null,
      defaultEffort: 'high',
      defaultWorkflowId: 'wf-starter-hotfix',
      defaultMaxBudgetCents: 250,
    });
    expect(await screen.findByText('github.com/acme/widgets')).toBeVisible();
    expect(screen.getByLabelText('Name')).toHaveValue('');
  });

  it('sends nothing for a remote the Hub would refuse, and says why', async () => {
    listing([]);
    render(<Projects />);

    await userEvent.type(await screen.findByLabelText('Name'), 'widgets');
    await userEvent.type(screen.getByLabelText('Repository'), 'git@github.com:acme/widgets.git');
    await userEvent.click(screen.getByRole('button', { name: 'Add project' }));

    expect(api.createProject).not.toHaveBeenCalled();
    expect(screen.getByLabelText('Repository')).toHaveAttribute('aria-invalid', 'true');
    expect(screen.getByText(/https:\/\/host\/owner\/repo/)).toBeVisible();
  });

  it('edits a project in place, starting from what it holds', async () => {
    listing([WIDGETS]);
    api.updateProject.mockResolvedValue({ ...WIDGETS, name: 'gadgets' });
    render(<Projects />);

    await screen.findByText('widgets');
    await userEvent.click(within(rowOf('widgets')).getByRole('button', { name: 'Edit' }));
    expect(screen.getByRole('heading', { name: 'Edit widgets' })).toBeVisible();
    expect(screen.getByLabelText('Repository')).toHaveValue('https://github.com/acme/widgets');
    expect(screen.getByLabelText('Default effort')).toHaveValue('high');

    const name = screen.getByLabelText('Name');
    await userEvent.clear(name);
    await userEvent.type(name, 'gadgets');
    await userEvent.click(screen.getByRole('button', { name: 'Save project' }));

    expect(api.updateProject).toHaveBeenCalledWith('p-1', expect.objectContaining({ name: 'gadgets', host: 'github.com' }));
    expect(api.createProject).not.toHaveBeenCalled();
    expect(await screen.findByRole('heading', { name: 'Add a project' })).toBeVisible();
  });

  it('says a duplicate repository in words', async () => {
    listing([WIDGETS]);
    api.createProject.mockRejectedValue(new HubApiError('refused', '/v1/projects', 409, 'project_exists'));
    render(<Projects />);

    await userEvent.type(await screen.findByLabelText('Name'), 'again');
    await userEvent.type(screen.getByLabelText('Repository'), 'https://github.com/acme/widgets');
    await userEvent.click(screen.getByRole('button', { name: 'Add project' }));

    expect(await screen.findByRole('alert')).toHaveTextContent('already has a project for that repository');
    expect(screen.getByLabelText('Name')).toHaveValue('again');
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
