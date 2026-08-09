import { CreateZeroStrategyStep } from 'demeteo';

const noop = () => {};

const base = {
  defaultBranch: 'master',
  branchPrefix: 'feat/',
  testCommand: 'npm run checks',
  conflictPolicy: 'always_gate',
  featureLifecycle: 'archive',
  onDefaultBranchChange: noop,
  onBranchPrefixChange: noop,
  onTestCommandChange: noop,
  onConflictPolicyChange: noop,
  onFeatureLifecycleChange: noop,
};

/** Detected defaults, including the PR template the bootstrap found. */
export const WithPrTemplate = () => (
  <div className="w-full max-w-2xl">
    <CreateZeroStrategyStep
      {...base}
      prTemplate={
        '## What changed\n\n<!-- one paragraph -->\n\n' +
        '## Why\n\n<!-- link the issue -->\n\n' +
        '## Verification\n\n- [ ] `npm run checks` passes\n- [ ] smoke-tested locally'
      }
    />
  </div>
);

/** No template in the repo — that block is simply absent. */
export const WithoutPrTemplate = () => (
  <div className="w-full max-w-2xl">
    <CreateZeroStrategyStep {...base} prTemplate="" />
  </div>
);

/** Nothing detected — empty fields fall back to their placeholders. */
export const NothingDetected = () => (
  <div className="w-full max-w-2xl">
    <CreateZeroStrategyStep
      {...base}
      defaultBranch="main"
      branchPrefix=""
      testCommand=""
      conflictPolicy="auto_agent"
      featureLifecycle="keep"
      prTemplate=""
    />
  </div>
);
