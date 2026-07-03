import React from 'react';

export interface CreateZeroStrategyStepProps {
  defaultBranch: string;
  branchPrefix: string;
  testCommand: string;
  prTemplate: string;
  conflictPolicy: string;
  featureLifecycle: string;
  onDefaultBranchChange: (v: string) => void;
  onBranchPrefixChange: (v: string) => void;
  onTestCommandChange: (v: string) => void;
  onConflictPolicyChange: (v: string) => void;
  onFeatureLifecycleChange: (v: string) => void;
}

/** Tiny text-row helper used by the strategy-review form. */
const FieldInput: React.FC<{
  label: string;
  value: string;
  onChange: (v: string) => void;
  placeholder?: string;
}> = ({ label, value, onChange, placeholder }) => (
  <div>
    <label className="block text-[10px] font-mono text-slate-400 uppercase tracking-widest mb-1.5">
      {label}
    </label>
    <input
      type="text"
      value={value}
      onChange={(e) => onChange(e.target.value)}
      placeholder={placeholder}
      className="w-full bg-black/40 border border-white/10 rounded-lg p-2.5 text-xs text-white font-mono focus:outline-none focus:border-cyan-500/50 placeholder-slate-600"
    />
  </div>
);

/**
 * Step 6 — compact strategy review. Mirrors `NewProjectView`'s
 * `strategy_proposal` panel but trimmed for a single screen: branch
 * name, branch prefix, test command, conflict policy, feature
 * lifecycle, and the detected PR template (read-only).
 */
export function CreateZeroStrategyStep(props: CreateZeroStrategyStepProps) {
  const {
    defaultBranch, branchPrefix, testCommand, prTemplate, conflictPolicy, featureLifecycle,
    onDefaultBranchChange, onBranchPrefixChange, onTestCommandChange,
    onConflictPolicyChange, onFeatureLifecycleChange,
  } = props;
  return (
    <div className="space-y-4">
      <div>
        <h3 className="font-outfit font-semibold text-cyan-400 uppercase tracking-widest text-xs mb-1">
          Strategy detected
        </h3>
        <p className="text-sm text-slate-300">
          Review the auto-detected defaults — change anything you'd like before we lock the project in.
        </p>
      </div>
      <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
        <FieldInput label="Default branch" value={defaultBranch} onChange={onDefaultBranchChange} />
        <FieldInput label="Branch prefix" value={branchPrefix} onChange={onBranchPrefixChange} />
        <FieldInput label="Test command (optional)" value={testCommand} onChange={onTestCommandChange} placeholder="npm test" />
        <div>
          <label className="block text-[10px] font-mono text-slate-400 uppercase tracking-widest mb-1.5">
            Conflict policy
          </label>
          <select value={conflictPolicy} onChange={(e) => onConflictPolicyChange(e.target.value)} className="w-full bg-[#08090c] border border-white/10 rounded-lg p-2.5 text-xs text-white font-mono focus:outline-none focus:border-cyan-500/50">
            <option value="always_gate">Always Gate</option>
            <option value="auto_agent">Auto Agent First</option>
            <option value="auto_human">Immediate Manual Merge</option>
          </select>
        </div>
        <div>
          <label className="block text-[10px] font-mono text-slate-400 uppercase tracking-widest mb-1.5">
            Feature lifecycle
          </label>
          <select value={featureLifecycle} onChange={(e) => onFeatureLifecycleChange(e.target.value)} className="w-full bg-[#08090c] border border-white/10 rounded-lg p-2.5 text-xs text-white font-mono focus:outline-none focus:border-cyan-500/50">
            <option value="archive">Archive by default</option>
            <option value="keep">Keep active</option>
            <option value="auto_delete">Auto delete branch</option>
          </select>
        </div>
      </div>
      {prTemplate && (
        <div>
          <label className="block text-[10px] font-mono text-slate-400 uppercase tracking-widest mb-1.5">
            Detected PR template
          </label>
          <div className="bg-black/40 border border-white/5 rounded-lg p-3 font-mono text-[10px] text-slate-400 max-h-32 overflow-y-auto whitespace-pre-wrap">
            {prTemplate}
          </div>
        </div>
      )}
    </div>
  );
}
