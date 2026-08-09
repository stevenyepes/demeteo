import { CreateZeroStepHeader } from 'demeteo';

const STEPS = [
  { id: 'name', label: 'Name' },
  { id: 'description', label: 'Describe' },
  { id: 'strategy', label: 'Strategy' },
  { id: 'agent', label: 'Agent' },
  { id: 'machine', label: 'Machine' },
  { id: 'launch', label: 'Launch' },
];

// Full width, no max: the stepper scrolls its own overflow, so a narrow
// frame silently clips the trailing steps instead of shrinking them.
const Frame = ({ children }: { children: React.ReactNode }) => (
  <div className="w-full">{children}</div>
);

/** Mid-wizard: three done, one active, the rest pending. */
export const InProgress = () => (
  <Frame>
    <CreateZeroStepHeader
      steps={STEPS}
      activeId="agent"
      completedIds={['name', 'description', 'strategy']}
    />
  </Frame>
);

/** First step — nothing completed yet. */
export const AtTheStart = () => (
  <Frame>
    <CreateZeroStepHeader steps={STEPS} activeId="name" completedIds={[]} />
  </Frame>
);

/** Last step, everything behind it green. */
export const AtTheEnd = () => (
  <Frame>
    <CreateZeroStepHeader
      steps={STEPS}
      activeId="launch"
      completedIds={['name', 'description', 'strategy', 'agent', 'machine']}
    />
  </Frame>
);

/** A short wizard — the row centres rather than stretching. */
export const ShortWizard = () => (
  <Frame>
    <CreateZeroStepHeader
      steps={[
        { id: 'repo', label: 'Repository' },
        { id: 'workflow', label: 'Workflow' },
        { id: 'go', label: 'Start' },
      ]}
      activeId="workflow"
      completedIds={['repo']}
    />
  </Frame>
);
