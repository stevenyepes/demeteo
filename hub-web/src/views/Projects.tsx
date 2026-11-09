import { useCallback, useEffect, useId, useRef, useState } from 'react';

import { Button } from '../../../src/components/ui/Button';
import { CeremonyError } from '../components/CeremonyError';
import { PageHeader } from '../components/PageHeader';
import { ProjectForm } from '../components/ProjectForm';
import { ProjectRow } from '../components/ProjectRow';
import { HubApiError } from '../lib/api';
import {
  createProject,
  type HubProject,
  listProjects,
  type ProjectFields,
  removeProject,
  updateProject,
} from '../lib/projects';
import { type HubWorkflow, listWorkflows } from '../lib/workflows';

type Listed = { projects: HubProject[]; workflows: HubWorkflow[] };

/** The Hub's refusals a save can meet, in words; anything else is shown as `CeremonyError` shows it. */
const SAVE_REFUSALS = new Map<string, string>([
  ['project_exists', 'This Hub already has a project for that repository.'],
  ['invalid_request', 'The Hub could not accept this project as written. Check the repository and the defaults.'],
]);

/**
 * The repositories a dispatch to a runner can name, and the run shape each
 * starts from. They are this Hub's own and never a desktop's: a desktop keeps
 * offering its own projects when it is the target.
 */
export function Projects() {
  const listId = useId();
  const [listed, setListed] = useState<Listed | null>(null);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [saved, setSaved] = useState(0);
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

  async function act(action: () => Promise<unknown>, done: () => void) {
    setFailure(null);
    setBusy(true);
    try {
      await action();
      done();
    } catch (error) {
      setFailure({ error });
    } finally {
      setBusy(false);
      load();
    }
  }

  const editing = listed?.projects.find((project) => project.id === editingId) ?? null;

  const save = (fields: ProjectFields) =>
    act(
      () => (editing === null ? createProject(fields) : updateProject(editing.id, fields)),
      () => {
        setEditingId(null);
        setSaved((count) => count + 1);
      },
    );

  const remove = (id: string) =>
    act(
      () => removeProject(id),
      () => setEditingId((held) => (held === id ? null : held)),
    );

  return (
    <div className="flex flex-col gap-7">
      <PageHeader
        title="Projects"
        summary={listed === null ? undefined : projectCount(listed.projects.length)}
      />
      <p className="-mt-3 max-w-2xl text-sm text-slate-400">
        A runner clones one of these repositories and runs one of this Hub's workflows on it. The Hub keeps the
        repository and the defaults a dispatch starts from, never a path or a credential.
      </p>
      {listed === null && failure === null && <p className="text-sm text-slate-400">Loading projects…</p>}
      {listed !== null && (
        <div className="flex flex-wrap items-start gap-6">
          <section aria-labelledby={listId} className="panel min-w-0 flex-[2_1_480px] p-2">
            <h2 id={listId} className="px-4 pt-3 pb-2 font-heading text-base font-semibold text-white">
              Hub projects
            </h2>
            {listed.projects.length === 0 ? (
              <p className="px-4 pb-4 text-sm text-slate-400">None yet. Add one to dispatch a runner.</p>
            ) : (
              <ul className="flex flex-col divide-y divide-hairline">
                {listed.projects.map((project) => (
                  <ProjectRow
                    key={project.id}
                    project={project}
                    workflows={listed.workflows}
                    disabled={busy}
                    onEdit={setEditingId}
                    onRemove={remove}
                  />
                ))}
              </ul>
            )}
          </section>
          <div className="min-w-0 flex-[1_1_360px]">
            <ProjectForm
              key={`${editing?.id ?? 'new'}-${saved}`}
              project={editing}
              workflows={listed.workflows}
              busy={busy}
              onSave={save}
              onCancel={() => setEditingId(null)}
            />
          </div>
        </div>
      )}
      {failure && (
        <div className="max-w-xl">
          <SaveFailure error={failure.error} />
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

function SaveFailure({ error }: { error: unknown }) {
  const sentence = error instanceof HubApiError && error.code !== null ? SAVE_REFUSALS.get(error.code) : undefined;
  if (sentence === undefined) return <CeremonyError error={error} />;
  return (
    <p role="alert" className="rounded-lg border border-ruby-500/30 bg-ruby-500/10 px-3 py-2 text-sm text-ruby-200">
      {sentence}
    </p>
  );
}
