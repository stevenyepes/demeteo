import { CreateZeroNameStep } from 'demeteo';

const noop = () => {};

/** Filled in — the name seeds both the display name and the repo slug. */
export const Filled = () => (
  <div className="w-full max-w-lg">
    <CreateZeroNameStep projectName="billing-service-rust" onChange={noop} />
  </div>
);

/** Empty, showing the placeholder that suggests the expected shape. */
export const Empty = () => (
  <div className="w-full max-w-lg">
    <CreateZeroNameStep projectName="" onChange={noop} />
  </div>
);
