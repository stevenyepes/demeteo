import { CreateZeroStepFooter } from 'demeteo';

const noop = () => {};

const Frame = ({ children }: { children: React.ReactNode }) => (
  <div className="w-full max-w-xl">{children}</div>
);

/** Mid-wizard: Back enabled, cyan Next. */
export const Ready = () => (
  <Frame>
    <CreateZeroStepFooter step="provider" canProceed reason="" onBack={noop} onNext={noop} />
  </Frame>
);

/** Blocked — the CTA greys out and the reason says why. */
export const Blocked = () => (
  <Frame>
    <CreateZeroStepFooter
      step="machine"
      canProceed={false}
      reason="SSH probe failed"
      onBack={noop}
      onNext={noop}
    />
  </Frame>
);

/** First step — Back is disabled, since there is nowhere to go. */
export const FirstStep = () => (
  <Frame>
    <CreateZeroStepFooter step="name" canProceed reason="" onBack={noop} onNext={noop} />
  </Frame>
);

/** The CTA changes label and icon per step: approve, bootstrap, launch. */
export const CtaVariants = () => (
  <div className="w-full max-w-xl flex flex-col gap-6">
    <CreateZeroStepFooter step="strategy" canProceed reason="" onBack={noop} onNext={noop} />
    <CreateZeroStepFooter step="agent" canProceed reason="" onBack={noop} onNext={noop} />
    <CreateZeroStepFooter step="workflow" canProceed reason="" onBack={noop} onNext={noop} />
  </div>
);
