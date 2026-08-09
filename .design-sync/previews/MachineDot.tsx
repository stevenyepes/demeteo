import { MachineDot } from 'demeteo';

const Legend = ({ label, children }: { label: string; children: React.ReactNode }) => (
  <div className="flex items-center gap-3">
    <span className="w-4 flex justify-center">{children}</span>
    <span className="text-xs font-mono text-slate-400">{label}</span>
  </div>
);

/** Cyan marks the local host, emerald a remote machine. */
export const LocalAndRemote = () => (
  <div className="flex flex-col gap-3">
    <Legend label="local — cyan">
      <MachineDot machineId="local" machineLabel="local" />
    </Legend>
    <Legend label="build-01 — emerald">
      <MachineDot machineId="m-8f21" machineLabel="build-01" />
    </Legend>
  </div>
);

/** `pulse` marks a live session; without it the dot dims to 60%. */
export const PulseVsIdle = () => (
  <div className="flex flex-col gap-3">
    <Legend label="local, live">
      <MachineDot machineId="local" machineLabel="local" pulse />
    </Legend>
    <Legend label="local, idle">
      <MachineDot machineId="local" machineLabel="local" />
    </Legend>
    <Legend label="remote, live">
      <MachineDot machineId="m-8f21" machineLabel="build-01" pulse />
    </Legend>
    <Legend label="remote, idle">
      <MachineDot machineId="m-8f21" machineLabel="build-01" />
    </Legend>
  </div>
);

/** In place: leading a machine strip, so where a terminal runs is one glance. */
export const InAMachineStrip = () => (
  <div className="flex flex-col gap-1 w-full max-w-sm">
    {[
      { id: 'local', label: 'local', detail: 'this laptop', live: true },
      { id: 'm-8f21', label: 'build-01', detail: 'ssh://build-01.internal:22', live: true },
      { id: 'm-3c04', label: 'gpu-02', detail: 'ssh://gpu-02.internal:22', live: false },
    ].map((m) => (
      <div key={m.id} className="flex items-center gap-2.5 px-3 py-2 rounded-lg bg-white/[0.02] border border-white/5">
        <MachineDot machineId={m.id} machineLabel={m.label} pulse={m.live} />
        <span className="text-xs text-slate-200 font-medium">{m.label}</span>
        <span className="text-[10px] font-mono text-slate-500 truncate">{m.detail}</span>
      </div>
    ))}
  </div>
);
