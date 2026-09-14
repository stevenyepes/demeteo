import type { Discovery, TicketView } from '../types';

export interface IntegrationControl {
  enabled: boolean;
  /** Shown as visible body text when `enabled` is false; `null` when enabled. */
  reason: string | null;
}

export interface DiscoveryIntegrationActions {
  /** `discovery.base_branch !== null` — both controls render iff this is true. */
  showControls: boolean;
  sync: IntegrationControl;
  publish: IntegrationControl;
}

export function discoveryIntegrationActions(
  discovery: Discovery,
  tickets: readonly TicketView[],
): DiscoveryIntegrationActions {
  const showControls = discovery.base_branch !== null;
  const hasMergedTicket = tickets.some((t) => t.feature?.mr_state === 'merged');

  return {
    showControls,
    sync: { enabled: showControls, reason: null },
    publish: {
      enabled: hasMergedTicket,
      reason: hasMergedTicket
        ? null
        : 'Open a pull request once at least one ticket has landed.',
    },
  };
}
