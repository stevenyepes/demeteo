import { HarnessModelPicker } from 'demeteo';

const AGENTS = ['claude-code', 'opencode', 'hermes', 'codex'];
const MODELS = [
  { value: 'claude-opus-5', name: 'Claude Opus 5' },
  { value: 'claude-sonnet-5', name: 'Claude Sonnet 5' },
  { value: 'claude-haiku-4-5', name: 'Claude Haiku 4.5' },
];

const noop = () => {};

/** Harness + model, the two-column form the settings panels use. */
export const HarnessAndModel = () => (
  <div className="w-full max-w-2xl">
    <HarnessModelPicker
      agentKinds={AGENTS}
      models={MODELS}
      agentKind="claude-code"
      model="claude-opus-5"
      onAgentKindChange={noop}
      onModelChange={noop}
    />
  </div>
);

/** Opting into the effort control adds a third column. */
export const WithEffort = () => (
  <div className="w-full max-w-2xl">
    <HarnessModelPicker
      agentKinds={AGENTS}
      models={MODELS}
      agentKind="claude-code"
      model="claude-sonnet-5"
      effort="high"
      onAgentKindChange={noop}
      onModelChange={noop}
      onEffortChange={noop}
      onClear={noop}
    />
  </div>
);

/** No harness picked — the model select is disabled and says why. */
export const NoHarnessYet = () => (
  <div className="w-full max-w-2xl">
    <HarnessModelPicker
      agentKinds={AGENTS}
      models={[]}
      agentKind=""
      model=""
      onAgentKindChange={noop}
      onModelChange={noop}
    />
  </div>
);

/** Probing the machine for available models. */
export const ProbingModels = () => (
  <div className="w-full max-w-2xl">
    <HarnessModelPicker
      agentKinds={AGENTS}
      models={[]}
      modelsLoading
      agentKind="opencode"
      model=""
      onAgentKindChange={noop}
      onModelChange={noop}
    />
  </div>
);

/** An agent with no per-invocation effort control (hermes): the effort
 *  select is disabled rather than silently offering a level that would
 *  be dropped. The saved flag shows the confirmation affordance. */
export const EffortUnsupported = () => (
  <div className="w-full max-w-2xl">
    <HarnessModelPicker
      agentKinds={AGENTS}
      models={MODELS}
      agentKind="hermes"
      model=""
      effort=""
      effortLevels={[]}
      saved
      onAgentKindChange={noop}
      onModelChange={noop}
      onEffortChange={noop}
      onClear={noop}
    />
  </div>
);
