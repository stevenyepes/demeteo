import { Monitor, Server } from 'lucide-react';
import { useId } from 'react';

const MATCH_HINT: Record<PairingClaimsProps['kind'], string> = {
  desktop: 'Check that this matches the fingerprint Demeteo shows on the machine. If it does not, choose Deny.',
  runner: 'Check that this matches the fingerprint the runner printed beside its code. If it does not, choose Deny.',
};

interface PairingClaimsProps {
  kind: 'desktop' | 'runner';
  instanceName: string;
  os: string;
  arch: string;
  appVersion: string;
  /** The Hub's rendering, shown as it came: `PairingPrompt.fingerprint` says why. */
  fingerprint: string;
}

/** The labels a pairing machine claims and its device key's fingerprint, as both approval pages show them. */
export function PairingClaims({ kind, instanceName, os, arch, appVersion, fingerprint }: PairingClaimsProps) {
  const id = useId();
  const Icon = kind === 'runner' ? Server : Monitor;
  return (
    <>
      <section aria-labelledby={`${id}-claimed`} className="flex items-start gap-4">
        <span
          aria-hidden="true"
          className="flex h-11 w-11 shrink-0 items-center justify-center rounded-lg border border-hairline bg-white/5 text-slate-300"
        >
          <Icon className="h-5 w-5" />
        </span>
        <div className="flex min-w-0 flex-1 flex-col gap-2">
          <h2 id={`${id}-claimed`} className="text-xs font-medium text-slate-400">
            Claimed by the machine
          </h2>
          <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-sm">
            <dt className="text-slate-400">Name</dt>
            <dd className="font-heading font-semibold text-white break-all">{instanceName}</dd>
            <dt className="text-slate-400">OS</dt>
            <dd className="font-mono text-slate-200 break-all">{os}</dd>
            <dt className="text-slate-400">Architecture</dt>
            <dd className="font-mono text-slate-200 break-all">{arch}</dd>
            <dt className="text-slate-400">Demeteo version</dt>
            <dd className="font-mono text-slate-200 break-all">{appVersion}</dd>
          </dl>
          <p className="text-xs text-slate-400">
            The machine reported these about itself. The Hub has not verified them.
          </p>
        </div>
      </section>
      <section
        aria-labelledby={`${id}-key`}
        className="flex flex-col gap-2 rounded-md border border-hairline bg-surface-well px-4 py-3"
      >
        <h2 id={`${id}-key`} className="text-xs font-medium text-slate-400">
          Device key fingerprint
        </h2>
        <p className="font-mono text-lg text-white break-all">{fingerprint}</p>
        <p className="text-xs text-slate-400">
          {MATCH_HINT[kind]}
        </p>
      </section>
    </>
  );
}
