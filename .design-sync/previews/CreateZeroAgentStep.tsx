import { CreateZeroAgentStep } from 'demeteo';

const noop = () => {};

const AGENTS = ['claude-code', 'opencode', 'hermes', 'codex'];
const MODELS = [
  { value: 'claude-opus-5', name: 'Claude Opus 5' },
  { value: 'claude-sonnet-5', name: 'Claude Sonnet 5' },
  { value: 'claude-haiku-4-5', name: 'Claude Haiku 4.5' },
];
const LEVELS = ['low', 'medium', 'high', 'xhigh', 'max'] as const;

const base = {
  agentKinds: AGENTS,
  onAgentKindChange: noop,
  onModelChange: noop,
  onEffortChange: noop,
  onClear: noop,
};

const Frame = ({ children }: { children: React.ReactNode }) => (
  <div className="w-full max-w-2xl">{children}</div>
);

/** Chosen agent, model and effort — the settled state. */
export const Chosen = () => (
  <Frame>
    <CreateZeroAgentStep
      {...base}
      models={MODELS}
      modelsLoading={false}
      agentKind="claude-code"
      model="claude-opus-5"
      effort="high"
      effortLevels={LEVELS}
    />
  </Frame>
);

/** Probing the selected machine — the hint explains why it's machine-scoped. */
export const Probing = () => (
  <Frame>
    <CreateZeroAgentStep
      {...base}
      models={[]}
      modelsLoading
      agentKind="opencode"
      model=""
      effort=""
      effortLevels={LEVELS}
    />
  </Frame>
);

/** An agent that declares no effort levels greys that control out. */
export const EffortUnsupported = () => (
  <Frame>
    <CreateZeroAgentStep
      {...base}
      models={MODELS}
      modelsLoading={false}
      agentKind="hermes"
      model=""
      effort=""
      effortLevels={[]}
    />
  </Frame>
);
