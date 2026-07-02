import { useCallback, useMemo, useRef, useState } from 'react';
import { useNavigation, useProject } from '../context';
import { ArrowLeft, Sparkles, Terminal } from 'lucide-react';
import { CreateZeroStepHeader, type CreateZeroStepDescriptor } from './ui/CreateZeroStepHeader';
import { CreateZeroBootstrapPanel } from './ui/CreateZeroBootstrapPanel';
import { CreateZeroStepFooter } from './ui/CreateZeroStepFooter';
import { CreateZeroNameStep } from './ui/CreateZeroNameStep';
import { CreateZeroProviderStep } from './ui/CreateZeroProviderStep';
import { CreateZeroMachineStep } from './ui/CreateZeroMachineStep';
import { CreateZeroAgentStep } from './ui/CreateZeroAgentStep';
import { CreateZeroStrategyStep } from './ui/CreateZeroStrategyStep';
import { CreateZeroDescriptionStep } from './ui/CreateZeroDescriptionStep';
import { CreateZeroWorkflowStep } from './ui/CreateZeroWorkflowStep';
import { CreateZeroLaunchStep } from './ui/CreateZeroLaunchStep';
import { useCreateZeroWizardForm } from './ui/useCreateZeroWizardForm';
import { useCreateZeroWizardActions } from './ui/useCreateZeroWizardActions';
import { useCreateZeroBootstrap } from './ui/useCreateZeroBootstrap';
import type { WorktreeStrategy } from '../types';

const AGENT_KINDS = ['opencode', 'hermes', 'claude-code', 'antigravity'];
const SLUG_PATTERN = /^[a-z0-9][a-z0-9._-]{0,99}$/;

type StepId =
  | 'name' | 'provider' | 'machine' | 'agent'
  | 'bootstrap' | 'strategy' | 'description' | 'workflow' | 'launching';

const STEP_ORDER: StepId[] = [
  'name', 'provider', 'machine', 'agent', 'bootstrap', 'strategy', 'description', 'workflow', 'launching',
];
const STEP_LABELS: Record<StepId, string> = {
  name: 'Name', provider: 'Provider', machine: 'Machine', agent: 'Agent',
  bootstrap: 'Bootstrap', strategy: 'Strategy', description: 'Describe', workflow: 'Workflow', launching: 'Launch',
};
const STEP_DESCRIPTORS: CreateZeroStepDescriptor[] =
  STEP_ORDER.map((id) => ({ id, label: STEP_LABELS[id] }));
const FORM_STEPS = ['name', 'provider', 'machine', 'agent', 'strategy', 'description', 'workflow'] as const;
type FormStepId = typeof FORM_STEPS[number];

/** Validate a repository slug per the GitHub / GitLab rules. Exported
 *  so the extracted provider step re-uses the same helper. */
export function validateSlug(value: string): string {
  const trimmed = value.trim();
  if (!trimmed) return 'Repository name is required';
  if (trimmed.length < 2) return 'Use at least 2 characters';
  if (!SLUG_PATTERN.test(trimmed)) {
    return 'Use lowercase letters, digits, dots, dashes or underscores';
  }
  return '';
}

/**
 * The Create-From-Zero wizard — a full-screen, progressive-disclosure
 * flow that takes a user from "I want to start fresh" to a running
 * feature in nine focused steps. Each step surfaces one decision;
 * the Next control stays disabled until that decision is made.
 *
 * Steps: Name → Provider → Machine → Agent → Bootstrap → Strategy →
 * Describe → Workflow → Launch. State + side effects live in three
 * extracted hooks so this file stays focused on step transitions
 * and rendering.
 */
const CreateFromZeroWizard: React.FC = () => {
  const { navigate } = useNavigation();
  const { dispatch: projDispatch } = useProject();
  const form = useCreateZeroWizardForm();
  const bootstrap = useCreateZeroBootstrap();
  const { approveStrategy, launchFeature, launchState } = useCreateZeroWizardActions(form);
  const [step, setStep] = useState<StepId>('name');
  // Guard against re-entering the bootstrap success path while a
  // subsequent run is in flight (the hook resolves onSuccess once).
  const bootstrapAdvancedRef = useRef(false);

  // Bootstrap callback: invoked by the hook when the pipeline
  // finishes. Seeds the strategy-review form, registers the project
  // in the global store, and advances the wizard to 'strategy'.
  const onBootstrapSuccess = useCallback((result: {
    projectId: string; repo: { full_name: string }; strategy: WorktreeStrategy;
  }) => {
    if (bootstrapAdvancedRef.current) return;
    bootstrapAdvancedRef.current = true;
    form.setProjectId(result.projectId);
    form.applyStrategyToForm(result.strategy);
    form.setKeyPassphrase(''); // already written to keyring; clear in-memory copy
    projDispatch({
      type: 'ADD_PROJECT',
      project: {
        id: result.projectId, name: form.projectName, status: 'idle',
        repos: 1, nodes: 0, spend: 0, tokens: 0,
        compute_type: form.machineKind,
        remote_host: form.machineKind === 'remote' ? (form.machineId || null) : null,
      },
    });
    setStep('strategy');
  }, [form, projDispatch]);

  const runBootstrap = useCallback(() => {
    bootstrapAdvancedRef.current = false;
    void bootstrap.run({
      projectName: form.projectName,
      providerId: form.providerId,
      namespaceId: form.namespaceId,
      repoSlug: form.repoSlug,
      repoPrivate: form.repoPrivate,
      machineKind: form.machineKind,
      machineId: form.machineId,
      keyPassphrase: form.keyPassphrase,
      agentKind: form.agentKind,
      model: form.model,
    }, onBootstrapSuccess);
  }, [bootstrap, form, onBootstrapSuccess]);

  // Per-step gating — drives the **Next** enabled state. Each entry
  // returns the human reason for the disabled state, or empty string
  // when the gate is open.
  const gateReason: Record<StepId, string> = {
    name: form.projectName.trim().length < 2 ? 'Type a project name' : '',
    provider:
      !form.providerId ? 'Pick a provider' :
      !form.namespaceId ? 'Pick a namespace' :
      validateSlug(form.repoSlug) ? `Repo name: ${validateSlug(form.repoSlug).toLowerCase()}` : '',
    machine:
      form.machineKind === 'remote' && !form.machineId ? 'Select a remote machine' : '',
    agent: !form.agentKind ? 'Pick a coding agent' : '',
    bootstrap: '',
    strategy: !form.defaultBranch.trim() ? 'Default branch required' : '',
    description: form.description.trim().length < 8 ? 'Describe the feature in a sentence or two' : '',
    workflow: !form.workflowId ? 'Pick a workflow' : '',
    launching: '',
  };
  const canProceed = (s: StepId): boolean => gateReason[s] === '';
  const completedIds = useMemo(
    () => STEP_ORDER.slice(0, STEP_ORDER.indexOf(step)),
    [step],
  );

  // Bottom-nav helpers. `goNext` is the single dispatch point for the
  // wizard's forward transitions; it owns the agent→bootstrap,
  // strategy→description, and workflow→launching special cases.
  const goBack = () => {
    const idx = STEP_ORDER.indexOf(step);
    if (idx > 0) setStep(STEP_ORDER[idx - 1]);
  };
  const goNext = async () => {
    if (!canProceed(step)) return;
    if (step === 'agent') { setStep('bootstrap'); runBootstrap(); return; }
    if (step === 'strategy') { await approveStrategy(); setStep('description'); return; }
    if (step === 'workflow') { setStep('launching'); void launchFeature(); return; }
    const idx = STEP_ORDER.indexOf(step);
    if (idx < STEP_ORDER.length - 1) setStep(STEP_ORDER[idx + 1]);
  };

  const showFooter = step !== 'bootstrap' && step !== 'launching';

  return (
    <div className="flex-1 overflow-y-auto p-6 relative flex items-center justify-center bg-[#08090c]">
      <div className="absolute top-0 left-1/2 -translate-x-1/2 w-[800px] h-[400px] bg-violet-600/10 rounded-full blur-[120px] pointer-events-none" />

      <div className="w-full max-w-3xl z-10 space-y-6">
        <header className="flex items-center gap-3">
          <Sparkles className="w-5 h-5 text-cyan-400" />
          <div>
            <h1 className="text-2xl font-outfit font-bold text-white">Create from scratch</h1>
            <p className="text-xs text-slate-400">A guided, one-decision-at-a-time workspace setup.</p>
          </div>
        </header>

        <div className="glass-panel p-6 rounded-2xl border-white/10 shadow-2xl space-y-6">
          <CreateZeroStepHeader steps={STEP_DESCRIPTORS} activeId={step} completedIds={completedIds} />

          <div key={step} className="animate-fadeIn">
            {step === 'name' && <CreateZeroNameStep projectName={form.projectName} onChange={form.setProjectName} />}

            {step === 'provider' && (
              <CreateZeroProviderStep
                projectName={form.projectName}
                providers={form.providers}
                providerId={form.providerId}
                namespaceId={form.namespaceId}
                repoSlug={form.repoSlug}
                repoPrivate={form.repoPrivate}
                namespaces={form.namespaces}
                namespacesLoading={form.namespacesLoading}
                onProviderChange={form.setProviderId}
                onNamespaceChange={form.setNamespaceId}
                onSlugChange={form.setRepoSlug}
                onPrivateChange={form.setRepoPrivate}
                validateSlug={validateSlug}
              />
            )}

            {step === 'machine' && (
              <CreateZeroMachineStep
                machineKind={form.machineKind}
                machineId={form.machineId}
                machines={form.machines}
                keyPassphrase={form.keyPassphrase}
                onMachineKindChange={form.setMachineKind}
                onMachineIdChange={form.setMachineId}
                onKeyPassphraseChange={form.setKeyPassphrase}
              />
            )}

            {step === 'agent' && (
              <CreateZeroAgentStep
                agentKinds={AGENT_KINDS}
                models={form.models}
                modelsLoading={form.modelsLoading}
                agentKind={form.agentKind}
                model={form.model}
                onAgentKindChange={form.setAgentKind}
                onModelChange={form.setModel}
                onClear={() => { form.setAgentKind(''); form.setModel(''); }}
              />
            )}

            {step === 'bootstrap' && (
              <CreateZeroBootstrapPanel
                phases={bootstrap.phases}
                logs={bootstrap.logs}
                errorMessage={bootstrap.error}
                canRetry={Boolean(bootstrap.error)}
                onRetry={runBootstrap}
              />
            )}

            {step === 'strategy' && (
              <CreateZeroStrategyStep
                defaultBranch={form.defaultBranch}
                branchPrefix={form.branchPrefix}
                testCommand={form.testCommand}
                prTemplate={form.prTemplate}
                conflictPolicy={form.conflictPolicy}
                featureLifecycle={form.featureLifecycle}
                onDefaultBranchChange={form.setDefaultBranch}
                onBranchPrefixChange={form.setBranchPrefix}
                onTestCommandChange={form.setTestCommand}
                onConflictPolicyChange={form.setConflictPolicy}
                onFeatureLifecycleChange={form.setFeatureLifecycle}
              />
            )}

            {step === 'description' && (
              <CreateZeroDescriptionStep description={form.description} onChange={form.setDescription} />
            )}

            {step === 'workflow' && (
              <div className="space-y-3">
                <label className="text-[11px] font-mono text-slate-400 uppercase tracking-widest block">
                  Pick a workflow
                </label>
                <CreateZeroWorkflowStep
                  workflows={form.workflows}
                  workflowId={form.workflowId}
                  onWorkflowChange={form.setWorkflowId}
                />
              </div>
            )}

            {step === 'launching' && (
              <CreateZeroLaunchStep
                launching={launchState.launching}
                errorMessage={launchState.errorMessage}
                onRetry={() => void launchFeature()}
              />
            )}
          </div>

          {showFooter && (
            <CreateZeroStepFooter
              step={step as FormStepId}
              canProceed={canProceed(step)}
              reason={gateReason[step]}
              onBack={goBack}
              onNext={goNext}
            />
          )}
        </div>

        <div className="flex items-center justify-between text-[10px] text-slate-500 font-mono">
          <button
            type="button"
            onClick={() => navigate({ kind: 'home' })}
            className="hover:text-slate-300 transition-colors flex items-center gap-1"
          >
            <ArrowLeft className="w-3 h-3" /> Exit wizard
          </button>
          <span className="flex items-center gap-2">
            <Terminal className="w-3 h-3" />
            {step === 'bootstrap' ? 'Bootstrapping…' : `Step ${STEP_ORDER.indexOf(step) + 1} of ${STEP_ORDER.length}`}
          </span>
        </div>
      </div>
    </div>
  );
};

export default CreateFromZeroWizard;
