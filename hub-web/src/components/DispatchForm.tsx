import { type FormEvent, useId, useState } from 'react';

import { type EffortLevel, reconcileEffort } from '../../../src/lib/effortLevels';
import { HubApiError } from '../lib/api';
import { dispatchErrorSentence, dispatchFeature, type Enqueued } from '../lib/dispatch';
import {
  buildDispatchRequest,
  effortsFor,
  parseMaxCostCents,
  type Prefill,
  type PrefillField,
  parseWallClockSecs,
  prefillFor,
  runnerProjectProblem,
  type StepOverrideDraft,
  toStepOverrides,
} from '../lib/dispatchForm';
import type { InstanceDetail, ProjectSummary } from '../lib/fleet';
import { useStagedAttachments } from '../lib/useStagedAttachments';
import { AttachmentPicker } from './AttachmentPicker';
import { CeremonyError } from './CeremonyError';
import { DispatchCapsFields } from './DispatchCapsFields';
import { DispatchPipeline } from './DispatchPipeline';
import { DispatchRunShapeFields } from './DispatchRunShapeFields';
import { RequestStatusCard } from './RequestStatusCard';
import { Button } from '../../../src/components/ui/Button';

interface DispatchFormProps {
  /** An instance `dispatchBlocker` passed, or blocked only on `no-spend`. */
  instance: InstanceDetail;
  spendGranted: boolean;
  /**
   * Set exactly when `instance` is a runner: the desktop whose snapshot the
   * Hub resolves the start from, so its projects, workflows and run-shape
   * defaults are the ones offered (docs/HUB.md §6.4).
   */
  source?: InstanceDetail;
}

type Errors = { title?: string; cost?: string; wallClock?: string };

type Send = { state: 'editing' } | { state: 'sending' } | { state: 'sent'; enqueued: Enqueued };

const FIELD_CLASS =
  'input-field disabled:cursor-not-allowed disabled:opacity-40';

const LABEL_CLASS = 'text-xs font-medium text-slate-300';

const UNFILLED: Prefill = { agentKind: '', model: '', effort: '', workflowId: '', maxCostUsd: '' };

/**
 * Projects and workflows are offered only from a snapshot, never typed: an id
 * the instance does not hold is refused as `unknown_project` /
 * `unknown_workflow` after a round trip through the queue. For a runner that
 * snapshot is the source desktop's, and a project the Hub would refuse to
 * start there is shown but not offered.
 */
export function DispatchForm({ instance, spendGranted, source }: DispatchFormProps) {
  const ids = {
    project: useId(),
    workflow: useId(),
    title: useId(),
    description: useId(),
    what: useId(),
    how: useId(),
  };
  const catalog = source ?? instance;
  const projects = catalog.projects ?? [];
  const workflows = catalog.workflows ?? [];
  const problemOf = (project: ProjectSummary) => (source === undefined ? null : runnerProjectProblem(project));
  const firstProjectId = projects.find((project) => problemOf(project) === null)?.id ?? '';

  const [initial] = useState(() => prefillFor(catalog, firstProjectId, UNFILLED, new Set(), instance));
  const [projectId, setProjectId] = useState(firstProjectId);
  const [workflowId, setWorkflowId] = useState(initial.workflowId);
  const [title, setTitle] = useState('');
  const [description, setDescription] = useState('');
  const [agentKind, setAgentKind] = useState(initial.agentKind);
  const [model, setModel] = useState(initial.model);
  const [effort, setEffort] = useState<EffortLevel | ''>(initial.effort);
  const [overrides, setOverrides] = useState<Record<string, StepOverrideDraft>>({});
  const [maxCostUsd, setMaxCostUsd] = useState(initial.maxCostUsd);
  const [edited, setEdited] = useState<ReadonlySet<PrefillField>>(new Set());
  const [maxWallClockMins, setMaxWallClockMins] = useState('');
  const [errors, setErrors] = useState<Errors>({});
  const [send, setSend] = useState<Send>({ state: 'editing' });
  const [failure, setFailure] = useState<unknown>(null);
  const attachments = useStagedAttachments();

  const projectAgentKind = catalog.runShape?.find((row) => row.projectId === projectId)?.agentKind ?? '';
  const workflow = workflows.find((choice) => choice.workflowId === workflowId);

  const edit = (field: PrefillField) => setEdited((held) => new Set(held).add(field));

  const changeWorkflow = (next: string) => {
    // Overrides are keyed by step id, which means nothing in another workflow's graph.
    setWorkflowId(next);
    setOverrides({});
  };

  const changeAgent = (next: string) => {
    // A model belongs to one harness; an effort the new one cannot run would be re-sent anyway.
    edit('agentKind');
    setAgentKind(next);
    setModel('');
    setEffort((held) => reconcileEffort(held, effortsFor(instance, next)));
  };

  const changeProject = (next: string) => {
    setProjectId(next);
    const filled = prefillFor(catalog, next, { agentKind, model, effort, workflowId, maxCostUsd }, edited, instance);
    setAgentKind(filled.agentKind);
    setModel(filled.model);
    setEffort(filled.effort);
    setMaxCostUsd(filled.maxCostUsd);
    if (filled.workflowId !== workflowId) changeWorkflow(filled.workflowId);
  };

  const submit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (!spendGranted || projectId === '' || send.state !== 'editing') return;
    const cost = parseMaxCostCents(maxCostUsd);
    const wallClock = parseWallClockSecs(maxWallClockMins);
    const next: Errors = {
      ...(title.trim() === '' ? { title: 'A title is required.' } : {}),
      ...(cost.ok ? {} : { cost: cost.error }),
      ...(wallClock.ok ? {} : { wallClock: wallClock.error }),
    };
    setErrors(next);
    setFailure(null);
    if (!cost.ok || !wallClock.ok || next.title !== undefined) return;

    const form = {
      projectId,
      workflowId,
      title: title.trim(),
      description,
      agentKind,
      model,
      effort: effort === '' ? null : effort,
      overrides,
      maxBudgetCents: cost.value,
      maxWallClockSecs: wallClock.value,
      ...(source === undefined ? {} : { sourceInstanceId: source.id }),
    };
    setSend({ state: 'sending' });
    attachments
      .stage(instance.id)
      .then((attachmentIds) => dispatchFeature(instance.id, buildDispatchRequest(form, attachmentIds)))
      .then(
        (enqueued) => setSend({ state: 'sent', enqueued }),
        (error: unknown) => {
          if (error instanceof HubApiError && error.code === 'attachment_unavailable') attachments.unstage();
          setFailure(error);
          setSend({ state: 'editing' });
        },
      );
  };

  if (send.state === 'sent') {
    return <RequestStatusCard instanceId={instance.id} enqueued={send.enqueued} present={instance.present} />;
  }

  const sending = send.state === 'sending';
  const overrideCount = toStepOverrides(overrides).length;

  return (
    <form noValidate onSubmit={submit} className="panel flex flex-col">
      <div className="grid xl:grid-cols-[minmax(0,1fr)_minmax(0,1.3fr)]">
        <section
          aria-labelledby={ids.what}
          className="flex min-w-0 flex-col gap-4 border-hairline p-5 xl:border-r"
        >
          <h2 id={ids.what} className="font-heading text-sm font-semibold text-slate-200">
            What
          </h2>
          <AttachmentPicker files={attachments.files} onChange={attachments.setFiles} disabled={sending} />
          <div className="flex flex-col gap-1.5">
            <label htmlFor={ids.title} className={LABEL_CLASS}>
              Title
            </label>
            <input
              id={ids.title}
              type="text"
              value={title}
              onChange={(event) => setTitle(event.target.value)}
              autoComplete="off"
              aria-invalid={Boolean(errors.title)}
              aria-describedby={errors.title ? `${ids.title}-error` : undefined}
              className={FIELD_CLASS}
            />
            {errors.title && (
              <p id={`${ids.title}-error`} className="text-xs text-ruby-300">
                {errors.title}
              </p>
            )}
          </div>
          <div className="flex flex-col gap-1.5">
            <label htmlFor={ids.description} className={LABEL_CLASS}>
              Description
            </label>
            <textarea
              id={ids.description}
              value={description}
              onChange={(event) => setDescription(event.target.value)}
              rows={5}
              className={FIELD_CLASS}
            />
          </div>
          <div className="grid gap-4 sm:grid-cols-2">
            <div className="flex flex-col gap-1.5">
              <label htmlFor={ids.project} className={LABEL_CLASS}>
                Project
              </label>
              <select
                id={ids.project}
                value={projectId}
                onChange={(event) => changeProject(event.target.value)}
                className={FIELD_CLASS}
              >
                {projectId === '' && <option value="">No project a runner can start</option>}
                {projects.map((project) => {
                  const problem = problemOf(project);
                  return (
                    <option key={project.id} value={project.id} disabled={problem !== null}>
                      {problem === null ? project.name : `${project.name} — ${problem}`}
                    </option>
                  );
                })}
              </select>
            </div>
            <div className="flex flex-col gap-1.5">
              <label htmlFor={ids.workflow} className={LABEL_CLASS}>
                Workflow
              </label>
              <select
                id={ids.workflow}
                value={workflowId}
                onChange={(event) => {
                  edit('workflowId');
                  changeWorkflow(event.target.value);
                }}
                className={FIELD_CLASS}
              >
                {workflows.map((workflow) => (
                  <option key={workflow.workflowId} value={workflow.workflowId}>
                    {workflow.name} (v{workflow.version})
                  </option>
                ))}
              </select>
            </div>
          </div>
        </section>
        <section
          aria-labelledby={ids.how}
          className="flex min-w-0 flex-col gap-4 border-t border-hairline p-5 xl:border-t-0"
        >
          <h2 id={ids.how} className="font-heading text-sm font-semibold text-slate-200">
            How
          </h2>
          <DispatchRunShapeFields
            instance={instance}
            agentKind={agentKind}
            projectAgentKind={projectAgentKind}
            model={model}
            effort={effort}
            onAgentChange={changeAgent}
            onModelChange={(next) => {
              edit('model');
              setModel(next);
            }}
            onEffortChange={(next) => {
              edit('effort');
              setEffort(next);
            }}
          />
          {workflow !== undefined && (
            <DispatchPipeline
              key={workflow.workflowId}
              workflow={workflow}
              overrides={overrides}
              onOverridesChange={setOverrides}
              defaultAgentKind={agentKind || projectAgentKind}
              instance={instance}
            />
          )}
          <DispatchCapsFields
            maxCostUsd={maxCostUsd}
            onMaxCostUsdChange={(raw) => {
              edit('maxCostUsd');
              setMaxCostUsd(raw);
            }}
            costError={errors.cost}
            maxWallClockMins={maxWallClockMins}
            onMaxWallClockMinsChange={setMaxWallClockMins}
            wallClockError={errors.wallClock}
          />
        </section>
      </div>
      <div className="flex flex-col gap-3 border-t border-hairline px-5 py-4">
        {failure !== null && <DispatchFailure error={failure} />}
        {projectId === '' && (
          <p className="text-sm text-slate-300">
            None of this desktop's projects can run on a runner: each is listed with the reason.
          </p>
        )}
        {!spendGranted && (
          <p className="text-sm text-ruby-300">
            Spend has not been granted on this instance, so it would refuse a Feature sent from the Hub. Grant spend for
            this Hub on the machine to dispatch here.
          </p>
        )}
        <div className="flex flex-wrap items-center justify-between gap-3">
          <span className="font-mono text-xs text-slate-400">
            {overrideSummary(overrideCount)}
          </span>
          <Button variant="primary" type="submit" disabled={!spendGranted || projectId === '' || sending}>
            {instance.present ? 'Start feature' : 'Queue feature'}
          </Button>
        </div>
      </div>
    </form>
  );
}

function overrideSummary(count: number): string {
  return count === 1 ? '1 step override' : `${count} step overrides`;
}

/** The Hub's refusal as a sentence where there is one; otherwise its code, as `CeremonyError` shows it. */
function DispatchFailure({ error }: { error: unknown }) {
  const sentence = error instanceof HubApiError ? dispatchErrorSentence(error) : null;
  if (sentence === null) return <CeremonyError error={error} />;
  return (
    <p
      role="alert"
      className="rounded-lg border border-ruby-500/30 bg-ruby-500/10 px-3 py-2 text-sm text-ruby-200 break-words"
    >
      {sentence}
    </p>
  );
}
