/**
 * MachineDot — a small status dot marking the host machine of a terminal
 * (spec §5). Local terminals get a cyan dot (matching the TerminalWindow
 * status palette); remote terminals get an emerald dot so the strip can be
 * scanned at a glance. The colour rule is copied from `machineDotColor` in
 * `src/components/TerminalTab.tsx` so the two can't drift.
 */

const MACHINE_LABEL_LOCAL = 'local';

export interface MachineDotProps {
  machineId: string;
  machineLabel: string;
  /** When true, apply the project's `animate-pulse-glow`; else dim to `opacity-60`. */
  pulse?: boolean;
  className?: string;
}

/** True when the dot represents the local host rather than a remote machine. */
function isLocalMachine(machineId: string, machineLabel: string): boolean {
  return machineId === 'local' || machineLabel.toLowerCase() === MACHINE_LABEL_LOCAL;
}

export function MachineDot({
  machineId,
  machineLabel,
  pulse = false,
  className = '',
}: MachineDotProps): React.ReactElement {
  const local = isLocalMachine(machineId, machineLabel);

  return (
    <span
      aria-hidden="true"
      data-testid="machine-dot"
      data-machine-kind={local ? 'local' : 'remote'}
      className={`w-1.5 h-1.5 rounded-full shrink-0 ${local ? 'bg-cyan-400' : 'bg-emerald-400'} ${
        pulse ? 'animate-pulse-glow' : 'opacity-60'
      } ${className}`}
    />
  );
}
