import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { HubApiError } from '../lib/api';
import type { GitHosts, RepositoryListing } from '../lib/gitHosts';
import type { HubProject, ProjectFields } from '../lib/projects';
import type { HubWorkflow } from '../lib/workflows';
import { ProjectEditor } from './ProjectEditor';

const api = vi.hoisted(() => ({
  listProjects: vi.fn<() => Promise<HubProject[]>>(),
  createProject: vi.fn<(fields: ProjectFields) => Promise<HubProject>>(),
  updateProject: vi.fn<(id: string, fields: ProjectFields) => Promise<HubProject>>(),
  listWorkflows: vi.fn<() => Promise<HubWorkflow[]>>(),
  listGitHosts: vi.fn<() => Promise<GitHosts>>(),
  listRepositories: vi.fn<() => Promise<RepositoryListing>>(),
}));

vi.mock('../lib/projects', () => ({
  listProjects: api.listProjects,
  createProject: api.createProject,
  updateProject: api.updateProject,
}));
vi.mock('../lib/workflows', () => ({ listWorkflows: api.listWorkflows }));
vi.mock('../lib/gitHosts', () => ({
  listGitHosts: api.listGitHosts,
  listRepositories: api.listRepositories,
}));

const WORKFLOWS: HubWorkflow[] = [
  { workflowId: 'wf-starter-feature', version: 11, name: 'Feature', description: 'Build it', graph: null },
  { workflowId: 'wf-starter-hotfix', version: 12, name: 'Hotfix', description: 'Fix it', graph: null },
];

const HOSTS: GitHosts = {
  connectable: ['github.com'],
  hosts: [
    {
      id: 'h-1',
      host: 'github.com',
      appId: 7,
      appSlug: 'demeteo-hub',
      account: 'acme',
      installUrl: 'https://github.com/apps/demeteo-hub/installations/new',
      connectedAt: 1,
    },
  ],
};

const LISTING: RepositoryListing = {
  repositories: [
    { host: 'github.com', fullName: 'acme/gadgets', private: false, defaultBranch: 'main', description: null },
    { host: 'github.com', fullName: 'acme/widgets', private: true, defaultBranch: 'main', description: 'Widgets' },
  ],
  unlisted: [],
  truncated: false,
};

const WIDGETS: HubProject = {
  id: 'p-1',
  name: 'widgets',
  providerKind: 'github',
  host: 'github.com',
  fullName: 'Acme/Widgets',
  remoteUrl: 'https://github.com/Acme/Widgets',
  runnerReady: true,
  defaultAgentKind: 'codex',
  defaultModel: null,
  defaultEffort: 'high',
  defaultWorkflowId: 'wf-starter-hotfix',
  defaultMaxBudgetCents: 250,
  createdAt: 1,
  updatedAt: 1,
};

function setUp({ projects = [] as HubProject[], hosts = HOSTS, listing = LISTING } = {}) {
  api.listProjects.mockResolvedValue(projects);
  api.listWorkflows.mockResolvedValue(WORKFLOWS);
  api.listGitHosts.mockResolvedValue(hosts);
  api.listRepositories.mockResolvedValue(listing);
}

function repositoryButton(fullName: string): HTMLElement {
  const slash = fullName.lastIndexOf('/');
  return screen.getByRole('button', { name: `${fullName.slice(0, slash + 1)}${fullName.slice(slash + 1)}` });
}

beforeEach(() => {
  window.location.hash = '#/projects/new';
});

afterEach(() => {
  vi.clearAllMocks();
});

describe('ProjectEditor, adding', () => {
  it('picks a repository from the connected hosts, names the project after it, and saves', async () => {
    setUp();
    api.createProject.mockResolvedValue(WIDGETS);
    render(<ProjectEditor />);

    await userEvent.click(await screen.findByRole('button', { name: 'acme/gadgets' }));
    await userEvent.click(repositoryButton('acme/widgets'));
    expect(repositoryButton('acme/widgets')).toHaveAttribute('aria-pressed', 'true');
    expect(repositoryButton('acme/gadgets')).toHaveAttribute('aria-pressed', 'false');
    expect(screen.getByLabelText('Name')).toHaveValue('widgets');

    await userEvent.selectOptions(screen.getByLabelText('Default workflow'), 'wf-starter-hotfix');
    await userEvent.selectOptions(screen.getByLabelText('Default agent'), 'codex');
    await userEvent.type(screen.getByLabelText('Default max cost (USD)'), '2.50');
    await userEvent.click(screen.getByRole('button', { name: 'Add project' }));

    expect(api.createProject).toHaveBeenCalledWith({
      name: 'widgets',
      providerKind: 'github',
      host: 'github.com',
      fullName: 'acme/widgets',
      defaultAgentKind: 'codex',
      defaultModel: null,
      defaultEffort: null,
      defaultWorkflowId: 'wf-starter-hotfix',
      defaultMaxBudgetCents: 250,
    });
    expect(window.location.hash).toBe('#/projects');
  });

  it('keeps a typed name when another repository is picked', async () => {
    setUp();
    render(<ProjectEditor />);

    await userEvent.click(await screen.findByRole('button', { name: 'acme/gadgets' }));
    const name = screen.getByLabelText('Name');
    await userEvent.clear(name);
    await userEvent.type(name, 'storefront');
    await userEvent.click(repositoryButton('acme/widgets'));

    expect(name).toHaveValue('storefront');
  });

  it('narrows the list by search', async () => {
    setUp();
    render(<ProjectEditor />);

    await userEvent.type(await screen.findByLabelText('Search repositories'), 'WIDG');

    expect(repositoryButton('acme/widgets')).toBeVisible();
    expect(screen.queryByRole('button', { name: 'acme/gadgets' })).toBeNull();
  });

  it('sends nothing until a repository is picked, and says so beside the list', async () => {
    setUp();
    render(<ProjectEditor />);

    await screen.findByRole('button', { name: 'acme/gadgets' });
    await userEvent.type(screen.getByLabelText('Name'), 'widgets');
    await userEvent.click(screen.getByRole('button', { name: 'Add project' }));

    expect(api.createProject).not.toHaveBeenCalled();
    expect(screen.getByText('Choose a repository.')).toBeVisible();
  });

  it('offers a repository that already has a project, but not to pick', async () => {
    setUp({ projects: [WIDGETS] });
    render(<ProjectEditor />);

    await screen.findByRole('button', { name: 'acme/gadgets' });
    const taken = repositoryButton('acme/widgets');
    expect(taken).toBeDisabled();
    expect(within(taken).getByText('Already a Hub project')).toBeVisible();
  });

  it('points at Git hosts when none is connected, without asking GitHub', async () => {
    setUp({ hosts: { connectable: ['github.com'], hosts: [] } });
    render(<ProjectEditor />);

    expect(await screen.findByRole('link', { name: 'Open Git hosts' })).toHaveAttribute('href', '#/git-hosts');
    expect(api.listRepositories).not.toHaveBeenCalled();
  });

  it('says when an App could not be listed, rather than implying the list is whole', async () => {
    setUp({ listing: { ...LISTING, unlisted: [{ host: 'github.com', appId: 9 }] } });
    render(<ProjectEditor />);

    expect(await screen.findByText(/One GitHub App could not be asked/)).toBeVisible();
  });

  it('says a duplicate repository in words', async () => {
    setUp();
    api.createProject.mockRejectedValue(new HubApiError('refused', '/v1/projects', 409, 'project_exists'));
    render(<ProjectEditor />);

    await userEvent.click(await screen.findByRole('button', { name: 'acme/gadgets' }));
    await userEvent.click(screen.getByRole('button', { name: 'Add project' }));

    expect(await screen.findByRole('alert')).toHaveTextContent('already has a project for that repository');
    expect(window.location.hash).toBe('#/projects/new');
  });
});

describe('ProjectEditor, editing', () => {
  it('keeps the repository and saves the defaults', async () => {
    setUp({ projects: [WIDGETS] });
    api.updateProject.mockResolvedValue({ ...WIDGETS, name: 'gadgets' });
    render(<ProjectEditor projectId="p-1" />);

    expect(await screen.findByRole('heading', { name: 'Edit widgets' })).toBeVisible();
    expect(screen.getByText('github.com/Acme/Widgets')).toBeVisible();
    expect(screen.getByLabelText('Default effort')).toHaveValue('high');
    expect(api.listRepositories).not.toHaveBeenCalled();

    const name = screen.getByLabelText('Name');
    await userEvent.clear(name);
    await userEvent.type(name, 'gadgets');
    await userEvent.click(screen.getByRole('button', { name: 'Save project' }));

    expect(api.updateProject).toHaveBeenCalledWith(
      'p-1',
      expect.objectContaining({ name: 'gadgets', host: 'github.com', fullName: 'Acme/Widgets' }),
    );
    expect(api.createProject).not.toHaveBeenCalled();
  });

  it('says so when the project is gone', async () => {
    setUp();
    render(<ProjectEditor projectId="p-gone" />);

    expect(await screen.findByText(/no such project/)).toBeVisible();
  });
});
