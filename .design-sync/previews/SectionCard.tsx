import { SectionCard, StatusBadge } from 'demeteo';
import { Server, GitBranch, ShieldCheck } from 'lucide-react';

/** The canonical use: a titled glass panel wrapping a block of settings. */
export const Default = () => (
  <SectionCard title="Execution" className="max-w-md">
    <p className="text-sm text-slate-400 leading-relaxed">
      Steps run in a Git worktree per subtask. Agents are fenced to that
      worktree and cannot reach the rest of the project.
    </p>
  </SectionCard>
);

/** With a leading icon — the common form in Project Settings. */
export const WithIcon = () => (
  <SectionCard title="Default machine" icon={<Server className="w-4 h-4 text-cyan-400" />} className="max-w-md">
    <div className="flex items-center justify-between gap-4">
      <div className="min-w-0">
        <div className="text-sm text-slate-200 font-medium">build-01</div>
        <div className="text-xs font-mono text-slate-500 truncate">ssh://build-01.internal:22</div>
      </div>
      <StatusBadge status="active" variant="pill" label="Reachable" />
    </div>
  </SectionCard>
);

/** Cards stack as the column layout of a settings page. */
export const Stacked = () => (
  <div className="flex flex-col gap-4 max-w-md">
    <SectionCard title="Branch" icon={<GitBranch className="w-4 h-4 text-violet-400" />}>
      <div className="text-sm font-mono text-slate-300">feat/execution-parity</div>
      <p className="mt-2 text-xs text-slate-500">
        Worktrees branch from here and merge back after the gate.
      </p>
    </SectionCard>
    <SectionCard title="Permissions" icon={<ShieldCheck className="w-4 h-4 text-emerald-400" />}>
      <ul className="text-xs font-mono text-slate-400 space-y-1.5">
        <li className="flex justify-between gap-4">
          <span>external_directory</span>
          <span className="text-ruby-400">deny</span>
        </li>
        <li className="flex justify-between gap-4">
          <span>worktree_write</span>
          <span className="text-emerald-400">allow</span>
        </li>
        <li className="flex justify-between gap-4">
          <span>network</span>
          <span className="text-emerald-400">allow</span>
        </li>
      </ul>
    </SectionCard>
  </div>
);

/** A dense card — long body text, to show the panel's reading rhythm. */
export const LongBody = () => (
  <SectionCard title="Gate policy" className="max-w-lg">
    <p className="text-sm text-slate-400 leading-relaxed">
      A Gate pauses the Workflow and waits for a human. Demeteo raises one before
      merging a worktree back when conflicts are detected, and before any release
      is promoted.
    </p>
    <p className="mt-3 text-sm text-slate-400 leading-relaxed">
      Unattended runs may auto-approve gates whose policy marks them advisory;
      everything else parks the run and notifies you.
    </p>
    <div className="mt-4 flex gap-2">
      <StatusBadge status="gated" variant="pill" />
      <StatusBadge status="parked" variant="pill" label="2 waiting" />
    </div>
  </SectionCard>
);
