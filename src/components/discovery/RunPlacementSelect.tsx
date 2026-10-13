import React from 'react';

import type { RunnerCompatibilityState } from '../../hooks/useRunnerCompatibility';
import { LOCAL_MACHINE } from '../../lib/newDiscovery';
import { blocksLaunch } from '../../lib/runnerCompatibility';
import type { Machine } from '../../types';
import { RunnerVersionNotice } from '../RunnerVersionNotice';

/** `<select>` values are strings, so `null` travels as the empty one. */
const NULL_VALUE = '';

interface RunPlacementSelectProps {
  id?: string;
  /** Every configured machine; only those reachable over SSH become options,
   *  since a run detaches only onto a machine with its own runner. */
  machines: readonly Machine[];
  value: string | null;
  onChange: (value: string | null) => void;
  disabled?: boolean;
  /** What a ticket with no placement of its own resolves to. When given, a
   *  "Default (…)" option stands for `null` and the local option stores an
   *  explicit `"local"`; without it there is nothing for `null` to mean but
   *  "here", so the local option *is* `null` — the Launch dialog's semantics,
   *  where a machine-less launch runs locally. */
  defaultLabel?: string;
  localLabel: string;
  /** Owned by the host, which keys it to whichever machine actually runs —
   *  for a ticket left on Default that is the resolved one, not `value` — and
   *  which also needs the verdict to gate its own launch. */
  compatibility: RunnerCompatibilityState;
  onOpenMachineSettings?: () => void;
}

/**
 * The one "Where to run" picker: the Launch dialog and the ticket drawer both
 * render it, so a detached destination is offered and refused the same way
 * wherever a run starts. Placement is a destination, not an `ExecutionPort`
 * transport — choosing a machine here never selects how Demeteo talks to it.
 */
export function RunPlacementSelect({
  id,
  machines,
  value,
  onChange,
  disabled,
  defaultLabel,
  localLabel,
  compatibility,
  onOpenMachineSettings,
}: RunPlacementSelectProps): React.ReactElement {
  const remoteMachines = machines.filter((m) => m.auth_type !== 'local');
  const localValue = defaultLabel === undefined ? NULL_VALUE : LOCAL_MACHINE;
  const selected = value ?? NULL_VALUE;
  // A stored id whose machine was since deleted still gets an option: without
  // one the select would show its first entry and misreport the placement.
  const orphaned =
    selected !== NULL_VALUE &&
    selected !== localValue &&
    !remoteMachines.some((m) => m.id === selected);
  const blocked = blocksLaunch(compatibility.report);

  return (
    <div className="space-y-2">
      <select
        id={id}
        value={selected}
        disabled={disabled}
        onChange={(e) => onChange(e.target.value === NULL_VALUE ? null : e.target.value)}
        aria-invalid={blocked || orphaned}
        className={`w-full bg-[var(--bg-input)] border rounded-lg px-3 py-2 text-xs text-slate-200 font-mono focus:outline-none ${
          blocked || orphaned
            ? 'border-ruby-500/50 focus:border-ruby-500/70'
            : 'border-white/10 focus:border-violet-500/50'
        }`}
      >
        {defaultLabel !== undefined && <option value={NULL_VALUE}>Default ({defaultLabel})</option>}
        <option value={localValue}>{localLabel}</option>
        {remoteMachines.map((m) => (
          <option key={m.id} value={m.id}>
            {m.name} — detached
          </option>
        ))}
        {orphaned && <option value={selected}>{selected} — no longer configured</option>}
      </select>
      <RunnerVersionNotice
        report={compatibility.report}
        variant="blocking"
        onRecheck={compatibility.refresh}
        checking={compatibility.loading}
        onOpenSettings={onOpenMachineSettings}
      />
    </div>
  );
}

export default RunPlacementSelect;
