import { CreateZeroLaunchStep } from 'demeteo';

const noop = () => {};

const Frame = ({ children }: { children: React.ReactNode }) => (
  <div className="w-full max-w-lg">{children}</div>
);

/** In flight — the violet rocket pulses while start_feature runs. */
export const Launching = () => (
  <Frame>
    <CreateZeroLaunchStep launching errorMessage={null} onRetry={noop} />
  </Frame>
);

/** Rejected — the failure is surfaced inline with a retry CTA. */
export const Failed = () => (
  <Frame>
    <CreateZeroLaunchStep
      launching={false}
      errorMessage="no workflow version matches 'wf-standard@7'"
      onRetry={noop}
    />
  </Frame>
);

/** Settled — the terminal state once the feature is away. */
export const Launched = () => (
  <Frame>
    <CreateZeroLaunchStep launching={false} errorMessage={null} onRetry={noop} />
  </Frame>
);
