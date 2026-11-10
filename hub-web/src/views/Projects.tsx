import { useCallback, useEffect, useId, useRef, useState } from 'react';

import { Plus } from 'lucide-react';

import { Button, buttonClasses } from '../../../src/components/ui/Button';
import { CeremonyError } from '../components/CeremonyError';
import { PageHeader } from '../components/PageHeader';
import { ProjectRow } from '../components/ProjectRow';
import { type HubProject, listProjects, removeProject } from '../lib/projects';
import { routeHref } from '../lib/router';
import { type HubWorkflow, listWorkflows } from '../lib/workflows';

type Listed = { projects: HubProject[]; workflows: HubWorkflow[] };

const ADD_HREF = routeHref({ name: 'project' });

/**
 * The repositories a dispatch to a runner can name, and the run shape each
 * starts from. They are this Hub's own and never a desktop's: a desktop keeps
 * offering its own projects when it is the target.
 */
export function Projects() {
  const listId = useId();
  const [listed, setListed] = useState<Listed | null>(null);
  const [busy, setBusy] = useState(false);
  const [failure, setFailure] = useState<{ error: unknown } | null>(null);
  const latestRead = useRef(0);

  const load = useCallback(() => {
    latestRead.current += 1;
    const read = latestRead.current;
    Promise.all([listProjects(), listWorkflows()]).then(
      ([projects, workflows]) => {
        if (latestRead.current === read) setListed({ projects, workflows });
      },
      (error: unknown) => {
        if (latestRead.current === read) setFailure({ error });
      },
    );
  }, []);

  useEffect(() => {
    load();
    return () => {
      latestRead.current += 1;
    };
  }, [load]);

  async function act(action: () => Promise<unknown>) {
    setFailure(null);
    setBusy(true);
    try {
      await action();
    } catch (error) {
      setFailure({ error });
    } finally {
      setBusy(false);
      load();
    }
  }

  const remove = (id: string) => act(() => removeProject(id));

  return (
    <div className="flex flex-col gap-7">
      <PageHeader
        title="Projects"
        summary={listed === null ? undefined : projectCount(listed.projects.length)}
        actions={
          <a href={ADD_HREF} className={buttonClasses('primary')}>
            <Plus aria-hidden="true" className="h-4 w-4" />
            Add project
          </a>
        }
      />
      <p className="-mt-3 max-w-2xl text-sm text-slate-400">
        A runner clones one of these repositories and runs one of this Hub's workflows on it. The Hub keeps the
        repository and the defaults a dispatch starts from, never a path or a credential.
      </p>
      {listed === null && failure === null && <p className="text-sm text-slate-400">Loading projects…</p>}
      {listed !== null && (
        <section aria-labelledby={listId} className="panel min-w-0 p-2">
          <h2 id={listId} className="px-4 pt-3 pb-2 font-heading text-base font-semibold text-white">
            Hub projects
          </h2>
          {listed.projects.length === 0 ? (
            <div className="flex flex-col items-start gap-3 px-4 pb-4">
              <p className="text-sm text-slate-400">None yet. Add one to dispatch a runner.</p>
              <a href={ADD_HREF} className={buttonClasses('secondary', 'sm')}>
                Add a project
              </a>
            </div>
          ) : (
            <ul className="flex flex-col divide-y divide-hairline">
              {listed.projects.map((project) => (
                <ProjectRow
                  key={project.id}
                  project={project}
                  workflows={listed.workflows}
                  disabled={busy}
                  onRemove={remove}
                />
              ))}
            </ul>
          )}
        </section>
      )}
      {failure && (
        <div className="max-w-xl">
          <CeremonyError error={failure.error} />
          {listed === null && (
            <Button
              variant="primary"
              onClick={() => {
                setFailure(null);
                load();
              }}
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

function projectCount(count: number): string {
  return count === 1 ? '1 project' : `${count} projects`;
}
