import { ArrowLeft, Box } from 'lucide-react';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';

import { Button } from '../../../src/components/ui/Button';
import { CeremonyError } from '../components/CeremonyError';
import { PageHeader } from '../components/PageHeader';
import { ProjectForm } from '../components/ProjectForm';
import { RepositoryPicker, repositoryKey } from '../components/RepositoryPicker';
import { HubApiError } from '../lib/api';
import { listGitHosts, listRepositories, type ReachableRepository, type RepositoryListing } from '../lib/gitHosts';
import { type ProjectRepository, suggestedName } from '../lib/projectForm';
import { createProject, type HubProject, listProjects, type ProjectFields, updateProject } from '../lib/projects';
import { routeHref } from '../lib/router';
import { type HubWorkflow, listWorkflows } from '../lib/workflows';

interface ProjectEditorProps {
  /** Absent adds a project; otherwise the one being edited. */
  projectId?: string;
}

type Loaded = { projects: HubProject[]; workflows: HubWorkflow[]; hostsConnected: boolean };

/** The Hub's refusals a save can meet, in words; anything else is shown as `CeremonyError` shows it. */
const SAVE_REFUSALS = new Map<string, string>([
  ['project_exists', 'This Hub already has a project for that repository.'],
  ['invalid_request', 'The Hub could not accept this project as written. Check the repository and the defaults.'],
]);

const PROJECTS_HREF = routeHref({ name: 'projects' });

/**
 * Adding picks the repository from what a connected App reaches, as the
 * desktop picks from its connected providers. Editing keeps the repository:
 * a project on another repository is another project, and runs already
 * sent from this one name it.
 */
export function ProjectEditor({ projectId }: ProjectEditorProps) {
  const adding = projectId === undefined;
  const [loaded, setLoaded] = useState<Loaded | null>(null);
  const [listing, setListing] = useState<RepositoryListing | null>(null);
  const [picked, setPicked] = useState<ReachableRepository | null>(null);
  const [repositoryError, setRepositoryError] = useState<string | undefined>(undefined);
  const [busy, setBusy] = useState(false);
  const [failure, setFailure] = useState<{ error: unknown } | null>(null);
  const latestRead = useRef(0);

  const load = useCallback(() => {
    latestRead.current += 1;
    const read = latestRead.current;
    const current = () => latestRead.current === read;
    setFailure(null);
    setListing(null);
    Promise.all([listProjects(), listWorkflows(), adding ? listGitHosts() : null]).then(
      ([projects, workflows, hosts]) => {
        if (!current()) return;
        const hostsConnected = hosts !== null && hosts.hosts.length > 0;
        setLoaded({ projects, workflows, hostsConnected });
        if (!adding || !hostsConnected) return;
        listRepositories().then(
          (answer) => {
            if (current()) setListing(answer);
          },
          (error: unknown) => {
            if (current()) setFailure({ error });
          },
        );
      },
      (error: unknown) => {
        if (current()) setFailure({ error });
      },
    );
  }, [adding]);

  useEffect(() => {
    load();
    return () => {
      latestRead.current += 1;
    };
  }, [load]);

  const project = loaded?.projects.find((held) => held.id === projectId) ?? null;
  const taken = useMemo(
    () => new Set((loaded?.projects ?? []).map((held) => repositoryKey(held.host, held.fullName))),
    [loaded],
  );

  const repository: ProjectRepository | null = adding
    ? picked === null
      ? null
      : { providerKind: 'github', host: picked.host, fullName: picked.fullName }
    : project;

  async function save(fields: ProjectFields) {
    setFailure(null);
    setBusy(true);
    try {
      await (project === null ? createProject(fields) : updateProject(project.id, fields));
      window.location.hash = PROJECTS_HREF;
    } catch (error) {
      setFailure({ error });
    } finally {
      setBusy(false);
    }
  }

  const title = adding ? 'Add a project' : project === null ? 'Project' : `Edit ${project.name}`;

  return (
    <div className="flex flex-col gap-7">
      <PageHeader
        title={title}
        eyebrow={
          <a href={PROJECTS_HREF} className="inline-flex items-center gap-1.5 text-slate-400 hover:text-white">
            <ArrowLeft aria-hidden="true" className="h-3.5 w-3.5" />
            Projects
          </a>
        }
      />
      <p className="-mt-3 max-w-2xl text-sm text-slate-400">
        {adding
          ? 'Pick a repository a connected GitHub App can reach, then the defaults a dispatch to a runner starts from.'
          : 'The defaults a dispatch to a runner starts from. The repository stays as it is.'}
      </p>
      {loaded === null && failure === null && <p className="text-sm text-slate-400">Loading…</p>}
      {loaded !== null && !adding && project === null && (
        <p className="text-sm text-slate-400">
          This Hub has no such project. It may have been removed. <a href={PROJECTS_HREF}>Back to projects</a>
        </p>
      )}
      {loaded !== null && (adding || project !== null) && (
        <div className="flex flex-wrap items-start gap-6">
          <div className="min-w-0 flex-[3_1_420px]">
            {adding ? (
              <RepositoryPicker
                listing={listing}
                hostsConnected={loaded.hostsConnected}
                taken={taken}
                selectedKey={picked === null ? null : repositoryKey(picked.host, picked.fullName)}
                onSelect={(repo) => {
                  setPicked(repo);
                  setRepositoryError(undefined);
                }}
                error={repositoryError}
              />
            ) : (
              project !== null && <FixedRepository project={project} />
            )}
          </div>
          <div className="min-w-0 flex-[2_1_340px]">
            <ProjectForm
              project={project}
              repository={repository}
              nameHint={picked === null ? '' : suggestedName(picked.fullName)}
              workflows={loaded.workflows}
              busy={busy}
              onSave={save}
              onMissingRepository={setRepositoryError}
              onCancel={() => {
                window.location.hash = PROJECTS_HREF;
              }}
            />
          </div>
        </div>
      )}
      {failure && (
        <div className="max-w-xl">
          <SaveFailure error={failure.error} />
          {(loaded === null || (adding && listing === null)) && (
            <Button
              variant="primary"
              onClick={load}
              className="mt-4"
            >
              Try again
            </Button>
          )}
        </div>
      )}
    </div>
  );
}

function FixedRepository({ project }: { project: HubProject }) {
  return (
    <section aria-label="Repository" className="panel flex flex-col gap-3 p-5">
      <h2 className="font-heading text-base font-semibold text-white">Repository</h2>
      <div className="nested-card flex items-start gap-3 px-3 py-3">
        <Box aria-hidden="true" className="mt-0.5 h-4 w-4 shrink-0 text-slate-500" />
        <span className="font-mono text-sm text-slate-100 break-all">
          {project.host}/{project.fullName}
        </span>
      </div>
      <p className="text-xs text-slate-400">To work on another repository, add it as its own project.</p>
    </section>
  );
}

function SaveFailure({ error }: { error: unknown }) {
  const sentence = error instanceof HubApiError && error.code !== null ? SAVE_REFUSALS.get(error.code) : undefined;
  if (sentence === undefined) return <CeremonyError error={error} />;
  return (
    <p role="alert" className="rounded-lg border border-ruby-500/30 bg-ruby-500/10 px-3 py-2 text-sm text-ruby-200">
      {sentence}
    </p>
  );
}
