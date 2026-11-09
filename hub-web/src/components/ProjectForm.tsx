import { type FormEvent, type ReactNode, useId, useState } from 'react';

import { Button } from '../../../src/components/ui/Button';
import {
  EFFORT_LABELS,
  EFFORT_LEVELS,
  EFFORT_SUPPORT,
  type EffortLevel,
  isEffortLevel,
  reconcileEffort,
  supportedEffortsFor,
} from '../../../src/lib/effortLevels';
import { GITLAB_RUNNER_PROBLEM } from '../lib/dispatchForm';
import {
  draftOf,
  type ProjectDraft,
  type ProjectErrors,
  PROVIDERS,
  type Provider,
  projectVerdict,
} from '../lib/projectForm';
import type { HubProject, ProjectFields } from '../lib/projects';
import type { HubWorkflow } from '../lib/workflows';

interface ProjectFormProps {
  /** `null` adds a project; otherwise the one being edited, whose fields the form starts from. */
  project: HubProject | null;
  workflows: HubWorkflow[];
  busy: boolean;
  onSave: (fields: ProjectFields) => void;
  /** Shown only while editing. */
  onCancel: () => void;
}

const LABEL_CLASS = 'text-xs font-medium text-slate-300';

const PROVIDER_LABELS: Record<Provider, string> = { github: 'GitHub', gitlab: 'GitLab' };

/**
 * The default agent is offered from this build's own table: a Hub project is
 * not tied to one runner, so no instance's catalog can say what it may name.
 * A held kind the table lacks is kept as its own option rather than dropped.
 */
export function ProjectForm({ project, workflows, busy, onSave, onCancel }: ProjectFormProps) {
  const ids = {
    heading: useId(),
    name: useId(),
    provider: useId(),
    remote: useId(),
    workflow: useId(),
    agent: useId(),
    model: useId(),
    effort: useId(),
    cost: useId(),
  };
  const [draft, setDraft] = useState<ProjectDraft>(() => draftOf(project));
  const [errors, setErrors] = useState<ProjectErrors>({});

  const set = <K extends keyof ProjectDraft>(field: K, value: ProjectDraft[K]) =>
    setDraft((held) => ({ ...held, [field]: value }));

  const knownAgents = Object.keys(EFFORT_SUPPORT);
  const agents = draft.agentKind !== '' && !knownAgents.includes(draft.agentKind) ? [...knownAgents, draft.agentKind] : knownAgents;
  const efforts = draft.agentKind === '' ? EFFORT_LEVELS : supportedEffortsFor(draft.agentKind);

  const changeAgent = (agentKind: string) =>
    setDraft((held) => ({
      ...held,
      agentKind,
      effort: reconcileEffort(held.effort, agentKind === '' ? EFFORT_LEVELS : supportedEffortsFor(agentKind)),
    }));

  const submit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (busy) return;
    const verdict = projectVerdict(draft);
    setErrors(verdict.ok ? {} : verdict.errors);
    if (verdict.ok) onSave(verdict.fields);
  };

  return (
    <form noValidate onSubmit={submit} aria-labelledby={ids.heading} className="panel flex min-w-0 flex-col gap-4 p-5">
      <h2 id={ids.heading} className="font-heading text-base font-semibold text-white">
        {project === null ? 'Add a project' : `Edit ${project.name}`}
      </h2>
      <Field id={ids.name} label="Name" error={errors.name}>
        <input
          id={ids.name}
          type="text"
          value={draft.name}
          onChange={(event) => set('name', event.target.value)}
          autoComplete="off"
          aria-invalid={Boolean(errors.name)}
          aria-describedby={errors.name ? `${ids.name}-error` : undefined}
          className="input-field"
        />
      </Field>
      <div className="grid gap-4 sm:grid-cols-[minmax(0,10rem)_minmax(0,1fr)]">
        <Field id={ids.provider} label="Git host">
          <select
            id={ids.provider}
            value={draft.providerKind}
            onChange={(event) => set('providerKind', event.target.value === 'gitlab' ? 'gitlab' : 'github')}
            className="input-field"
          >
            {PROVIDERS.map((provider) => (
              <option key={provider} value={provider}>
                {PROVIDER_LABELS[provider]}
              </option>
            ))}
          </select>
        </Field>
        <Field id={ids.remote} label="Repository" error={errors.remote}>
          <input
            id={ids.remote}
            type="text"
            value={draft.remote}
            onChange={(event) => set('remote', event.target.value)}
            placeholder="https://github.com/owner/repo"
            autoComplete="off"
            spellCheck={false}
            aria-invalid={Boolean(errors.remote)}
            aria-describedby={errors.remote ? `${ids.remote}-error` : undefined}
            className="input-field font-mono"
          />
        </Field>
      </div>
      {draft.providerKind === 'gitlab' && (
        <p className="text-xs text-slate-400">
          {GITLAB_RUNNER_PROBLEM}. The project is kept, but a dispatch to a runner does not offer it.
        </p>
      )}
      <Field id={ids.workflow} label="Default workflow">
        <select
          id={ids.workflow}
          value={draft.workflowId}
          onChange={(event) => set('workflowId', event.target.value)}
          className="input-field"
        >
          <option value="">None</option>
          {workflows.map((workflow) => (
            <option key={workflow.workflowId} value={workflow.workflowId}>
              {workflow.name}
            </option>
          ))}
        </select>
      </Field>
      <div className="grid gap-4 sm:grid-cols-3">
        <Field id={ids.agent} label="Default agent">
          <select
            id={ids.agent}
            value={draft.agentKind}
            onChange={(event) => changeAgent(event.target.value)}
            className="input-field font-mono"
          >
            <option value="">None</option>
            {agents.map((kind) => (
              <option key={kind} value={kind}>
                {kind}
              </option>
            ))}
          </select>
        </Field>
        <Field id={ids.model} label="Default model">
          <input
            id={ids.model}
            type="text"
            value={draft.model}
            onChange={(event) => set('model', event.target.value)}
            autoComplete="off"
            spellCheck={false}
            className="input-field font-mono"
          />
        </Field>
        <Field id={ids.effort} label="Default effort">
          <select
            id={ids.effort}
            value={draft.effort}
            onChange={(event) => set('effort', isEffortLevel(event.target.value) ? event.target.value : '')}
            disabled={efforts.length === 0}
            className="input-field disabled:cursor-not-allowed disabled:opacity-40"
          >
            <option value="">None</option>
            {efforts.map((level: EffortLevel) => (
              <option key={level} value={level}>
                {EFFORT_LABELS[level]}
              </option>
            ))}
          </select>
        </Field>
      </div>
      <Field id={ids.cost} label="Default max cost (USD)" error={errors.cost}>
        <input
          id={ids.cost}
          type="text"
          inputMode="decimal"
          value={draft.maxCostUsd}
          onChange={(event) => set('maxCostUsd', event.target.value)}
          autoComplete="off"
          aria-invalid={Boolean(errors.cost)}
          aria-describedby={errors.cost ? `${ids.cost}-error` : undefined}
          className="input-field font-mono"
        />
      </Field>
      <p className="text-xs text-slate-400">
        A dispatch to a runner fills in each of these it leaves blank. The Hub signs what the run gets, so a later edit
        here does not change a run already sent.
      </p>
      <div className="flex flex-wrap gap-3">
        <Button variant="primary" type="submit" disabled={busy}>
          {project === null ? 'Add project' : 'Save project'}
        </Button>
        {project !== null && (
          <Button variant="ghost" type="button" disabled={busy} onClick={onCancel}>
            Cancel
          </Button>
        )}
      </div>
    </form>
  );
}

function Field({
  id,
  label,
  error,
  children,
}: {
  id: string;
  label: string;
  error?: string;
  children: ReactNode;
}) {
  return (
    <div className="flex min-w-0 flex-col gap-1.5">
      <label htmlFor={id} className={LABEL_CLASS}>
        {label}
      </label>
      {children}
      {error && (
        <p id={`${id}-error`} className="text-xs text-ruby-300">
          {error}
        </p>
      )}
    </div>
  );
}
