import type { Machine, RunPlacement } from '../types';
import { LOCAL_MACHINE } from './newDiscovery';

/** "Local" or "Detached · {machine name}" — the one wording for a resolved
 *  placement, falling back to the id for a machine no longer configured. */
export function placementLabel(placement: RunPlacement, machines: readonly Machine[]): string {
  if (placement.kind === 'local') return 'Local';
  const name = machines.find((m) => m.id === placement.machine_id)?.name;
  return `Detached · ${name || placement.machine_id}`;
}

/** What a draft's `machineId` runs on: `''` is Default, and `'local'` an
 *  explicit Local that opts out of a detached default. */
export function draftPlacement(value: string, discoveryDefault: RunPlacement): RunPlacement {
  if (!value) return discoveryDefault;
  return value === LOCAL_MACHINE ? { kind: 'local' } : { kind: 'detached', machine_id: value };
}

/** The machine a run on `placement` executes on; `localHost` is
 *  `DiscoveryBoard.local_host`, the project's compute. */
export function placementHost(placement: RunPlacement, localHost: string): string {
  return placement.kind === 'local' ? localHost : placement.machine_id;
}
