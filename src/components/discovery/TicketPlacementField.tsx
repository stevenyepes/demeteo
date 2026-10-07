import React from 'react';

import { useRunnerCompatibility } from '../../hooks/useRunnerCompatibility';
import { draftPlacement, placementLabel } from '../../lib/ticketPlacement';
import type { Machine, RunPlacement } from '../../types';
import { FieldLabel } from '../ui/FieldLabel';
import { RunPlacementSelect } from './RunPlacementSelect';

interface TicketPlacementFieldProps {
  /** `DiscoveryBoard.discovery_default`: what the Default entry runs on. */
  discoveryDefault: RunPlacement;
  machines: readonly Machine[];
  /** The draft's `machineId`: `''` is Default, `'local'` an explicit local. */
  value: string;
  disabled: boolean;
  onChange: (value: string) => void;
}

/** The drawer's "Where to run": a ticket's placement, edited as part of the
 *  draft and saved with it (the precedence is `domain/run_placement.rs`'s). */
export function TicketPlacementField({
  discoveryDefault,
  machines,
  value,
  disabled,
  onChange,
}: TicketPlacementFieldProps): React.ReactElement {
  const runsOn = draftPlacement(value, discoveryDefault);
  const compatibility = useRunnerCompatibility(
    runsOn.kind === 'detached' ? runsOn.machine_id : '',
  );

  return (
    <div className="col-span-2">
      <FieldLabel htmlFor="ticket-placement">Where to run</FieldLabel>
      <RunPlacementSelect
        id="ticket-placement"
        machines={machines}
        value={value || null}
        onChange={(next) => onChange(next ?? '')}
        disabled={disabled}
        defaultLabel={placementLabel(discoveryDefault, machines)}
        localLabel="Local"
        compatibility={compatibility}
      />
    </div>
  );
}

export default TicketPlacementField;
