import { type ReactNode, useId } from 'react';

import {
  EFFORT_LABELS,
  EFFORT_LEVELS,
  type EffortLevel,
  isEffortLevel,
  reconcileEffort,
  supportedEffortsFor,
} from '../../../src/lib/effortLevels';
import { agentKindsWith, LOOP_ITERATIONS, type ProjectErrors, type SettingsDraft } from '../lib/projectForm';
import { describedBy, FormField, LABEL_CLASS } from './FormField';

interface ProjectSettingsFieldsProps {
  draft: SettingsDraft;
  errors: ProjectErrors;
  /** The project's default agent, which a blank resolver agent inherits; blank when it has none. */
  defaultAgentKind: string;
  onChange: <K extends keyof SettingsDraft>(field: K, value: SettingsDraft[K]) => void;
}

const SELECT_CLASS = 'input-field disabled:cursor-not-allowed disabled:opacity-40';

/**
 * The desktop's Strategy settings a runner run can honour, minus what is the
 * runner's own to know (its harnesses, its caches) and gate autonomy, which a
 * Hub project never sets: every Hub gate is decided with a passkey.
 */
export function ProjectSettingsFields({ draft, errors, defaultAgentKind, onChange }: ProjectSettingsFieldsProps) {
  return (
    <>
      <BranchesSection draft={draft} errors={errors} onChange={onChange} />
      <CommandsSection draft={draft} errors={errors} onChange={onChange} />
      <RetriesSection draft={draft} errors={errors} defaultAgentKind={defaultAgentKind} onChange={onChange} />
    </>
  );
}

type SectionProps = Omit<ProjectSettingsFieldsProps, 'defaultAgentKind'>;

function Section({ title, intro, children }: { title: string; intro?: string; children: ReactNode }) {
  const heading = useId();
  return (
    <section aria-labelledby={heading} className="panel flex min-w-0 flex-col gap-4 p-5">
      <div className="flex flex-col gap-1">
        <h2 id={heading} className="font-heading text-base font-semibold text-white">
          {title}
        </h2>
        {intro && <p className="text-xs text-slate-400">{intro}</p>}
      </div>
      {children}
    </section>
  );
}

function TextSetting({
  label,
  value,
  onChange,
  placeholder,
  hint,
  error,
  mono = true,
}: {
  label: string;
  value: string;
  onChange: (value: string) => void;
  placeholder?: string;
  hint?: string;
  error?: string;
  mono?: boolean;
}) {
  const id = useId();
  return (
    <FormField id={id} label={label} hint={hint} error={error}>
      <input
        id={id}
        type="text"
        value={value}
        onChange={(event) => onChange(event.target.value)}
        placeholder={placeholder}
        autoComplete="off"
        spellCheck={false}
        aria-invalid={Boolean(error)}
        aria-describedby={describedBy(id, Boolean(hint), error)}
        className={mono ? 'input-field font-mono' : 'input-field'}
      />
    </FormField>
  );
}

function CheckSetting({
  label,
  hint,
  checked,
  onChange,
}: {
  label: string;
  hint: string;
  checked: boolean;
  onChange: (checked: boolean) => void;
}) {
  const id = useId();
  return (
    <div className="flex items-start gap-3">
      <input
        id={id}
        type="checkbox"
        checked={checked}
        onChange={(event) => onChange(event.target.checked)}
        aria-describedby={`${id}-hint`}
        className="mt-0.5 accent-accent"
      />
      <div className="flex min-w-0 flex-col gap-0.5">
        <label htmlFor={id} className={LABEL_CLASS}>
          {label}
        </label>
        <p id={`${id}-hint`} className="text-xs text-slate-400">
          {hint}
        </p>
      </div>
    </div>
  );
}

function BranchesSection({ draft, errors, onChange }: SectionProps) {
  return (
    <Section title="Branches and reports">
      <div className="grid gap-4 sm:grid-cols-2">
        <TextSetting
          label="Branch prefix"
          value={draft.branchPrefix}
          onChange={(value) => onChange('branchPrefix', value)}
          placeholder="demeteo/features/"
          hint="Every run's branch starts with this."
          error={errors.branchPrefix}
        />
        <TextSetting
          label="Report folder"
          value={draft.artifactSubdir}
          onChange={(value) => onChange('artifactSubdir', value)}
          placeholder="artifacts/"
          hint="Where steps write their reports, inside the repository."
          error={errors.artifactSubdir}
        />
      </div>
      <CheckSetting
        label="Commit reports to the pull request"
        hint="Off leaves them in the run's worktree, out of the branch."
        checked={draft.commitArtifacts}
        onChange={(checked) => onChange('commitArtifacts', checked)}
      />
      <TextSetting
        label="Code review entrypoint"
        value={draft.reviewEntrypoint}
        onChange={(value) => onChange('reviewEntrypoint', value)}
        placeholder="/code-review"
        hint="The command a reviewing step starts from, passed to its agent as written."
      />
    </Section>
  );
}

function CommandsSection({ draft, errors, onChange }: SectionProps) {
  return (
    <Section
      title="Repository commands"
      intro="Blank keeps what the runner detects in the clone. Keep secrets out of these: the Hub signs them into every run it sends and keeps that signed copy."
    >
      <div className="grid gap-4 sm:grid-cols-2">
        <TextSetting
          label="Test command"
          value={draft.testCommand}
          onChange={(value) => onChange('testCommand', value)}
          placeholder="npm test"
        />
        <TextSetting
          label="Build command"
          value={draft.buildCommand}
          onChange={(value) => onChange('buildCommand', value)}
          placeholder="npm run build"
        />
        <TextSetting
          label="Coverage command"
          value={draft.coverageCommand}
          onChange={(value) => onChange('coverageCommand', value)}
        />
        <TextSetting
          label="Conventions file"
          value={draft.conventionsFile}
          onChange={(value) => onChange('conventionsFile', value)}
          placeholder="AGENTS.md"
          error={errors.conventionsFile}
        />
      </div>
    </Section>
  );
}

function RetriesSection({ draft, errors, defaultAgentKind, onChange }: ProjectSettingsFieldsProps) {
  const ids = { loops: useId(), agent: useId(), model: useId(), effort: useId() };
  const agents = agentKindsWith(draft.syncResolverAgentKind);
  const effectiveAgent = draft.syncResolverAgentKind || defaultAgentKind;
  const efforts = effectiveAgent === '' ? EFFORT_LEVELS : supportedEffortsFor(effectiveAgent);
  const loopsHint = 'How often a failed validation may send the work back before the run gives up. Blank is 3.';

  const changeAgent = (agentKind: string) => {
    onChange('syncResolverAgentKind', agentKind);
    const effortsThere = (agentKind || defaultAgentKind) === '' ? EFFORT_LEVELS : supportedEffortsFor(agentKind || defaultAgentKind);
    onChange('syncResolverEffort', reconcileEffort(draft.syncResolverEffort, effortsThere));
  };

  return (
    <Section title="Retries and conflicts">
      <FormField id={ids.loops} label="Loop iterations" hint={loopsHint} error={errors.loopIterations}>
        <input
          id={ids.loops}
          type="number"
          min={LOOP_ITERATIONS.min}
          max={LOOP_ITERATIONS.max}
          inputMode="numeric"
          value={draft.loopIterations}
          onChange={(event) => onChange('loopIterations', event.target.value)}
          placeholder="3"
          aria-invalid={Boolean(errors.loopIterations)}
          aria-describedby={describedBy(ids.loops, true, errors.loopIterations)}
          className="input-field w-32 font-mono"
        />
      </FormField>
      <div className="flex flex-col gap-1">
        <h3 className="font-heading text-sm font-semibold text-slate-200">Merge conflict resolver</h3>
        <p className="text-xs text-slate-400">
          Who resolves a conflict when a sync hits one, for that turn only. Blank inherits the run.
        </p>
      </div>
      <div className="grid gap-4 sm:grid-cols-3">
        <FormField id={ids.agent} label="Agent">
          <select
            id={ids.agent}
            value={draft.syncResolverAgentKind}
            onChange={(event) => changeAgent(event.target.value)}
            className={`${SELECT_CLASS} font-mono`}
          >
            <option value="">Inherit</option>
            {agents.map((kind) => (
              <option key={kind} value={kind}>
                {kind}
              </option>
            ))}
          </select>
        </FormField>
        <FormField id={ids.model} label="Model">
          <input
            id={ids.model}
            type="text"
            value={draft.syncResolverModel}
            onChange={(event) => onChange('syncResolverModel', event.target.value)}
            placeholder="Inherit"
            autoComplete="off"
            spellCheck={false}
            className="input-field font-mono"
          />
        </FormField>
        <FormField id={ids.effort} label="Effort">
          <select
            id={ids.effort}
            value={draft.syncResolverEffort}
            onChange={(event) =>
              onChange('syncResolverEffort', isEffortLevel(event.target.value) ? event.target.value : '')
            }
            className={SELECT_CLASS}
          >
            <option value="">Inherit</option>
            {efforts.map((level: EffortLevel) => (
              <option key={level} value={level}>
                {EFFORT_LABELS[level]}
              </option>
            ))}
          </select>
        </FormField>
      </div>
      <CheckSetting
        label="Push a resolved merge without review"
        hint="Off holds a resolution for a look once the run is over. One made mid-run is always pushed, since nothing can review it while the run owns the branch."
        checked={draft.pushResolvedSync}
        onChange={(checked) => onChange('pushResolvedSync', checked)}
      />
    </Section>
  );
}
