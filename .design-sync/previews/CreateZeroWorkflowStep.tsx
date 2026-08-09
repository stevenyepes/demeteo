import { CreateZeroWorkflowStep } from 'demeteo';

const noop = () => {};

const WORKFLOWS = [
  {
    id: 'wf-standard',
    name: 'Standard feature pipeline',
    description: 'Research, spec, decompose into tickets, implement in parallel worktrees, review, validate, then open a PR behind a merge Gate.',
    version: 7,
  },
  {
    id: 'wf-fast',
    name: 'Fast path',
    description: 'Skip the spec and critic steps. For small, well-understood changes where the description is already the spec.',
    version: 3,
  },
  {
    id: 'wf-research',
    name: 'Research only',
    description: 'Produce a written investigation and a proposed plan. Never edits the repository.',
    version: 2,
  },
];

const Frame = ({ children }: { children: React.ReactNode }) => (
  <div className="w-full max-w-xl">{children}</div>
);

/** One workflow selected — violet border and tint mark the choice. */
export const Selected = () => (
  <Frame>
    <CreateZeroWorkflowStep workflows={WORKFLOWS} workflowId="wf-standard" onWorkflowChange={noop} />
  </Frame>
);

/** Nothing chosen yet — every card sits in its resting state. */
export const NoSelection = () => (
  <Frame>
    <CreateZeroWorkflowStep workflows={WORKFLOWS} workflowId="" onWorkflowChange={noop} />
  </Frame>
);

/** The empty case, which points the user at the Workflows view. */
export const NoWorkflows = () => (
  <Frame>
    <CreateZeroWorkflowStep workflows={[]} workflowId="" onWorkflowChange={noop} />
  </Frame>
);
