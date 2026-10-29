import type { ReactNode } from 'react';

interface PairingStepProps {
  number: number;
  title: string;
  children: ReactNode;
}

export function PairingStep({ number, title, children }: PairingStepProps) {
  return (
    <li className="flex gap-4">
      <span
        aria-hidden="true"
        className="flex h-7 w-7 shrink-0 items-center justify-center rounded-full border border-hairline-raised font-heading text-xs font-semibold text-slate-300"
      >
        {number}
      </span>
      <div className="flex min-w-0 flex-1 flex-col gap-1.5">
        <span className="font-medium text-slate-100">{title}</span>
        <div className="text-sm text-slate-400">{children}</div>
      </div>
    </li>
  );
}
