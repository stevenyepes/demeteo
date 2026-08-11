import { FieldLabel } from 'demeteo';
import { Cpu, Zap, GitBranch, Server } from 'lucide-react';

/** The label alone — uppercase mono, muted, with generous tracking. */
export const Plain = () => (
  <div className="w-full max-w-xs">
    <FieldLabel>Default branch</FieldLabel>
  </div>
);

/** With a leading icon, which is how the settings forms use it. */
export const WithIcon = () => (
  <div className="w-full max-w-xs flex flex-col gap-4">
    <FieldLabel icon={<Cpu className="w-3 h-3" />}>Harness</FieldLabel>
    <FieldLabel icon={<Zap className="w-3 h-3" />}>Model</FieldLabel>
    <FieldLabel icon={<Server className="w-3 h-3" />}>Machine</FieldLabel>
  </div>
);

/** Labelling a real control — the `htmlFor` pairing it exists for. */
export const AboveAnInput = () => (
  <div className="w-full max-w-sm flex flex-col gap-4">
    <div>
      <FieldLabel htmlFor="branch" icon={<GitBranch className="w-3 h-3" />}>
        Default branch
      </FieldLabel>
      <input
        id="branch"
        defaultValue="master"
        className="w-full bg-black/40 border border-white/10 rounded-lg p-2.5 text-xs text-white font-mono"
      />
    </div>
    <div>
      <FieldLabel htmlFor="prefix">Branch prefix</FieldLabel>
      <input
        id="prefix"
        defaultValue="feat/"
        className="w-full bg-black/40 border border-white/10 rounded-lg p-2.5 text-xs text-white font-mono"
      />
    </div>
  </div>
);
