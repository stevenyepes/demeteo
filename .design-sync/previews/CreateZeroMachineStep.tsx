import { CreateZeroMachineStep } from 'demeteo';

const noop = () => {};

const MACHINES = [
  { id: 'm-8f21', name: 'build-01', host: 'build-01.internal', port: 22, username: 'ci', auth_type: 'key' },
  { id: 'm-3c04', name: 'gpu-02', host: 'gpu-02.internal', port: 22, username: 'ci', auth_type: 'password' },
];

const base = {
  machines: MACHINES,
  keyPassphrase: '',
  probeError: null,
  onRetest: noop,
  onMachineKindChange: noop,
  onMachineIdChange: noop,
  onKeyPassphraseChange: noop,
};

const Frame = ({ children }: { children: React.ReactNode }) => (
  <div className="w-full max-w-xl">{children}</div>
);

/** Local compute — the violet choice; no dropdown, no probe. */
export const Local = () => (
  <Frame>
    <CreateZeroMachineStep {...base} machineKind="local" machineId="" probeStatus="idle" />
  </Frame>
);

/** Remote with a key-auth machine: the passphrase field appears and the
 *  SSH probe reports success in emerald. */
export const RemoteProbeSucceeded = () => (
  <Frame>
    <CreateZeroMachineStep {...base} machineKind="remote" machineId="m-8f21" probeStatus="success" />
  </Frame>
);

/** The probe is running — cyan, spinner, Next still blocked. */
export const RemoteProbing = () => (
  <Frame>
    <CreateZeroMachineStep {...base} machineKind="remote" machineId="m-8f21" probeStatus="running" />
  </Frame>
);

/** The probe failed — ruby, the reason inline, and a retry affordance.
 *  This is the state that explains a disabled Next button. */
export const RemoteProbeFailed = () => (
  <Frame>
    <CreateZeroMachineStep
      {...base}
      machineKind="remote"
      machineId="m-8f21"
      probeStatus="error"
      probeError="ssh: handshake failed: unable to authenticate, attempted methods [none publickey]"
    />
  </Frame>
);

/** No machines configured — the dropdown says where to add one. */
export const NoMachines = () => (
  <Frame>
    <CreateZeroMachineStep {...base} machines={[]} machineKind="remote" machineId="" probeStatus="idle" />
  </Frame>
);
