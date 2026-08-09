import { describe, expect, it } from 'vitest';

import {
  describeProjectProvenance,
  PROVENANCE_SEPARATOR,
  type ProjectProvenanceInput,
  type ProvenanceProvider,
} from './projectProvenance';

const GITHUB: ProvenanceProvider = { id: 'github_github_com', type: 'github', host: 'github.com' };
const GHES: ProvenanceProvider = { id: 'github_git_acme_dev', type: 'github', host: 'git.acme.dev' };
const GITLAB: ProvenanceProvider = { id: 'gitlab_gitlab_com', type: 'gitlab', host: 'gitlab.com' };

function repo(providerId: string, id = `r-${providerId}`) {
  return { id, repo_path: 'acme/service', provider_id: providerId };
}

function input(over: Partial<ProjectProvenanceInput> = {}): ProjectProvenanceInput {
  return { repositories: [repo(GITHUB.id)], providers: [GITHUB], ...over };
}

describe('describeProjectProvenance', () => {
  it('names the provider a repository actually resolves to', () => {
    expect(describeProjectProvenance(input()).text).toBe(
      `Connected via GitHub (github.com)${PROVENANCE_SEPARATOR}Runs locally`,
    );
  });

  it('reports a self-hosted host verbatim instead of inferring an edition', () => {
    const out = describeProjectProvenance(
      input({ repositories: [repo(GHES.id)], providers: [GHES] }),
    );
    expect(out.segments[0]).toBe('Connected via GitHub (git.acme.dev)');
    expect(out.text).not.toContain('Enterprise');
  });

  it('says nothing about a default workflow, which no project has', () => {
    const out = describeProjectProvenance(
      input({ repositories: [repo(GITLAB.id)], providers: [GITLAB], computeType: 'remote', remoteHost: 'build-01' }),
    );
    expect(out.text.toLowerCase()).not.toContain('workflow');
  });

  it('states that no repository is connected rather than naming a provider', () => {
    const out = describeProjectProvenance(input({ repositories: [] }));
    expect(out.segments).toEqual(['No repository connected', 'Runs locally']);
    expect(out.unresolvedRepositories).toBe(0);
  });

  it('omits the provider clause when the id resolves to nothing', () => {
    const out = describeProjectProvenance(input({ providers: [GITLAB] }));
    expect(out.segments).toEqual(['Runs locally']);
    expect(out.unresolvedRepositories).toBe(1);
  });

  it('treats an absent provider id as unresolved', () => {
    const out = describeProjectProvenance(input({ repositories: [repo('')] }));
    expect(out.segments).toEqual(['Runs locally']);
    expect(out.unresolvedRepositories).toBe(1);
  });

  it('lists every distinct provider when repositories disagree', () => {
    const out = describeProjectProvenance(
      input({
        repositories: [repo(GITHUB.id), repo(GITLAB.id)],
        providers: [GITHUB, GITLAB],
      }),
    );
    expect(out.segments[0]).toBe('Connected via GitHub (github.com), GitLab (gitlab.com)');
  });

  it('names a shared provider once', () => {
    const out = describeProjectProvenance(
      input({ repositories: [repo(GITHUB.id, 'r-1'), repo(GITHUB.id, 'r-2')], providers: [GITHUB] }),
    );
    expect(out.segments[0]).toBe('Connected via GitHub (github.com)');
  });

  it('names the providers it can while counting the ones it cannot', () => {
    const out = describeProjectProvenance(
      input({ repositories: [repo(GITHUB.id), repo('deleted-provider')], providers: [GITHUB] }),
    );
    expect(out.segments[0]).toBe('Connected via GitHub (github.com)');
    expect(out.unresolvedRepositories).toBe(1);
  });

  it('passes an unrecognised provider kind through verbatim', () => {
    const gitea = { id: 'gitea_1', type: 'gitea', host: 'code.acme.dev' };
    const out = describeProjectProvenance(input({ repositories: [repo(gitea.id)], providers: [gitea] }));
    expect(out.segments[0]).toBe('Connected via gitea (code.acme.dev)');
  });

  it('drops the parenthetical when the provider has no host', () => {
    const hostless = { id: 'p-1', type: 'github', host: '' };
    const out = describeProjectProvenance(
      input({ repositories: [repo(hostless.id)], providers: [hostless] }),
    );
    expect(out.segments[0]).toBe('Connected via GitHub');
  });

  it('names the machine a remote project runs on', () => {
    const out = describeProjectProvenance(input({ computeType: 'Remote', remoteHost: 'build-01' }));
    expect(out.segments[1]).toBe('Runs on build-01');
  });

  it('does not invent a machine name for a remote project without one', () => {
    const out = describeProjectProvenance(input({ computeType: 'remote', remoteHost: null }));
    expect(out.segments[1]).toBe('Runs remotely');
  });

  it('reads a missing compute type as local, matching the column default', () => {
    const out = describeProjectProvenance(input({ computeType: undefined }));
    expect(out.segments[1]).toBe('Runs locally');
  });
});

/**
 * The default-workflow clause. Its whole content is the difference between a
 * stored choice and the three states that are not one — unset, an id whose
 * workflow was deleted, and a workflow list that has not answered yet. F10 was
 * the clause rendering regardless of all four.
 */
describe('describeProjectProvenance — default workflow', () => {
  const WORKFLOWS = new Map([
    ['wf-1', { name: 'Standard Feature Pipeline' }],
    ['wf-2', { name: 'Docs Update' }],
  ]);

  const base = {
    repositories: [{ provider_id: 'p-1' }],
    providers: [{ id: 'p-1', type: 'github', host: 'github.com' }],
  };

  it('names the workflow the project chose', () => {
    const { text } = describeProjectProvenance({
      ...base,
      defaultWorkflowId: 'wf-2',
      workflowsById: WORKFLOWS,
    });
    expect(text).toContain('Default workflow: Docs Update');
  });

  it('says nothing when the project has not chosen one', () => {
    const { text } = describeProjectProvenance({
      ...base,
      defaultWorkflowId: null,
      workflowsById: WORKFLOWS,
    });
    expect(text).not.toMatch(/workflow/i);
  });

  /** The column carries no foreign key by design, so this is reachable in
   *  normal use rather than a corrupt-data case. */
  it('says nothing when the chosen workflow has been deleted', () => {
    const { text } = describeProjectProvenance({
      ...base,
      defaultWorkflowId: 'wf-deleted',
      workflowsById: WORKFLOWS,
    });
    expect(text).not.toMatch(/workflow/i);
  });

  it('waits rather than degrading while the workflow list is unanswered', () => {
    const { text } = describeProjectProvenance({ ...base, defaultWorkflowId: 'wf-1' });
    expect(text).not.toMatch(/workflow/i);
  });
});
