import { type FormEvent, type ReactNode, useId, useState } from 'react';

import { Button } from '../../../src/components/ui/Button';
import {
  EFFORT_LABELS,
  EFFORT_LEVELS,
  type EffortLevel,
  isEffortLevel,
  reconcileEffort,
  supportedEffortsFor,
} from '../../../src/lib/effortLevels';
import { GITLAB_RUNNER_PROBLEM } from '../lib/dispatchForm';
import {
  agentKindsWith,
  draftOf,
  type ProjectDraft,
  type ProjectErrors,
  type ProjectRepository,
  projectVerdict,
  type SettingsDraft,
} from '../lib/projectForm';
import type { HubProject, ProjectFields } from '../lib/projects';
import type { HubWorkflow } from '../lib/workflows';
import { describedBy, FormField } from './FormField';
import { ProjectSettingsFields } from './ProjectSettingsFields';
import { ProjectStepOverrides } from './ProjectStepOverrides';

interface ProjectFormProps {
  /** `null` adds a project; otherwise the one being edited, whose fields the form starts from. */
  project: HubProject | null;
  /** The picked repository, or, while editing, the one the project holds. */
  repository: ProjectRepository | null;
  /** The name a picked repository suggests, used until the name is typed. */
  nameHint: string;
  workflows: HubWorkflow[];
  busy: boolean;
  onSave: (fields: ProjectFields) => void;
  /** A save refused for want of a repository; the picker shows why. */
  onMissingRepository: (message: string | undefined) => void;
  onCancel: () => void;
  /** The repository panel, which leads the wide column above the settings. */
  repositoryPanel: ReactNode;
}

/**
 * One form across both columns, so one save sends every field: an update
 * replaces the project whole. The wide column holds the repository and the
 * settings a run lays over the runner's own; the narrow one the run defaults
 * a dispatch fills in, and the save.
 */
export function ProjectForm({
  project,
  repository,
  nameHint,
  workflows,
  busy,
  onSave,
  onMissingRepository,
  onCancel,
  repositoryPanel,
}: ProjectFormProps) {
  const ids = {
    heading: useId(),
    name: useId(),
    workflow: useId(),
    agent: useId(),
    model: useId(),
    effort: useId(),
    cost: useId(),
  };
  const [draft, setDraft] = useState<ProjectDraft>(() => draftOf(project, workflows));
  const [errors, setErrors] = useState<ProjectErrors>({});
  /** Until the name is typed in, it follows the picked repository. */
  const [nameTyped, setNameTyped] = useState(project !== null);
  const name = nameTyped ? draft.name : nameHint;

  const set = <K extends keyof ProjectDraft>(field: K, value: ProjectDraft[K]) =>
    setDraft((held) => ({ ...held, [field]: value }));

  const setSetting = <K extends keyof SettingsDraft>(field: K, value: SettingsDraft[K]) =>
    setDraft((held) => ({ ...held, settings: { ...held.settings, [field]: value } }));

  const agents = agentKindsWith(draft.agentKind);
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
    const verdict = projectVerdict({ ...draft, name }, repository);
    setErrors(verdict.ok ? {} : verdict.errors);
    onMissingRepository(verdict.ok ? undefined : verdict.errors.repository);
    if (verdict.ok) onSave(verdict.fields);
  };

  return (
    <form noValidate onSubmit={submit} aria-labelledby={ids.heading} className="flex flex-wrap items-start gap-6">
      <div className="flex min-w-0 flex-[3_1_420px] flex-col gap-6">
        {repositoryPanel}
        <ProjectSettingsFields
          draft={draft.settings}
          errors={errors}
          defaultAgentKind={draft.agentKind}
          onChange={setSetting}
        />
        <ProjectStepOverrides
          workflows={workflows}
          overrides={draft.stepOverrides}
          defaultWorkflowId={draft.workflowId}
          defaultAgentKind={draft.agentKind}
          onChange={(stepOverrides) => set('stepOverrides', stepOverrides)}
        />
      </div>
      <div className="panel flex min-w-0 flex-[2_1_340px] flex-col gap-4 p-5 lg:sticky lg:top-6">
      <h2 id={ids.heading} className="font-heading text-base font-semibold text-white">
        Project
      </h2>
      <FormField id={ids.name} label="Name" error={errors.name}>
        <input
          id={ids.name}
          type="text"
          value={name}
          onChange={(event) => {
            setNameTyped(true);
            set('name', event.target.value);
          }}
          placeholder={repository === null ? 'Named after the repository you pick' : undefined}
          autoComplete="off"
          aria-invalid={Boolean(errors.name)}
          aria-describedby={describedBy(ids.name, false, errors.name)}
          className="input-field"
        />
      </FormField>
      {repository?.providerKind === 'gitlab' && (
        <p className="text-xs text-slate-400">
          {GITLAB_RUNNER_PROBLEM}. The project is kept, but a dispatch to a runner does not offer it.
        </p>
      )}
      <h3 className="pt-1 font-heading text-sm font-semibold text-slate-200">Run defaults</h3>
      <FormField id={ids.workflow} label="Default workflow">
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
      </FormField>
      <div className="grid gap-4 sm:grid-cols-3">
        <FormField id={ids.agent} label="Default agent">
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
        </FormField>
        <FormField id={ids.model} label="Default model">
          <input
            id={ids.model}
            type="text"
            value={draft.model}
            onChange={(event) => set('model', event.target.value)}
            autoComplete="off"
            spellCheck={false}
            className="input-field font-mono"
          />
        </FormField>
        <FormField id={ids.effort} label="Default effort">
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
        </FormField>
      </div>
      <FormField id={ids.cost} label="Default max cost (USD)" error={errors.cost}>
        <input
          id={ids.cost}
          type="text"
          inputMode="decimal"
          value={draft.maxCostUsd}
          onChange={(event) => set('maxCostUsd', event.target.value)}
          autoComplete="off"
          aria-invalid={Boolean(errors.cost)}
          aria-describedby={describedBy(ids.cost, false, errors.cost)}
          className="input-field font-mono"
        />
      </FormField>
      <p className="text-xs text-slate-400">
        A dispatch to a runner fills in each of these it leaves blank, and every run started from the project gets the
        settings beside them. The Hub signs what the run gets, so a later edit here does not change a run already sent.
      </p>
      <div className="flex flex-wrap gap-3">
        <Button variant="primary" type="submit" disabled={busy}>
          {project === null ? 'Add project' : 'Save project'}
        </Button>
        <Button variant="ghost" type="button" disabled={busy} onClick={onCancel}>
          Cancel
        </Button>
      </div>
      {hasSettingErrors(errors) && (
        <p role="alert" className="text-xs text-ruby-300">
          A setting beside this form needs a fix before the project can be saved.
        </p>
      )}
      </div>
    </form>
  );
}

function hasSettingErrors(errors: ProjectErrors): boolean {
  return [errors.branchPrefix, errors.artifactSubdir, errors.conventionsFile, errors.loopIterations].some(Boolean);
}
