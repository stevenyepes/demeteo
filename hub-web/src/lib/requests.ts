import type { EffortLevel } from '../../../src/lib/effortLevels';
import { type Guard, getJson, HubApiError, rejectSyntheticInstance } from './api';
import { postInSession } from './auth';

/** The Hub's `202` to a queued request. */
export type RequestTicket = { requestId: string; expiresAt: number };

/**
 * `reason` is any string: the Hub's `RefusalReason` grows with the instances
 * it relays, and a refusal this build cannot name is still the instance's word.
 */
export type RequestOutcome =
  | { status: 'queued' }
  | { status: 'delivered' }
  | { status: 'accepted' }
  | { status: 'refused'; reason: string }
  | { status: 'expired' };

export type RequestState = {
  requestId: string;
  kind: string;
  deliveries: number;
  expiresAt: number;
  answeredAt: number | null;
} & RequestOutcome;

export type StepAssignment = { agentKind?: string; model?: string; effort?: EffortLevel };

export type PollOptions = {
  /** Every failed read; polling carries on after it unless the answer was a `401`. */
  onError?: (error: unknown) => void;
  /** Polling reached expiry and the last read was a failure or a non-terminal state. */
  onStopped?: () => void;
};

type WireRequestTicket = { request_id: string; expires_at: number };

type WireRequestState = {
  request_id: string;
  kind: string;
  deliveries: number;
  expires_at: number;
  answered_at: number | null;
} & RequestOutcome;

const STATUSES: readonly unknown[] = [
  'queued',
  'delivered',
  'accepted',
  'refused',
  'expired',
] satisfies RequestOutcome['status'][];

const FIRST_DELAY_MS = 1000;
const MAX_DELAY_MS = 8000;

export async function getRequestStatus(instanceId: string, requestId: string): Promise<RequestState> {
  const path = `/v1/instances/${encodeURIComponent(instanceId)}/requests/${encodeURIComponent(requestId)}`;
  rejectSyntheticInstance(instanceId, path);
  return requestStateOf(await getJson(path, isWireRequestState));
}

export async function setStepAssignment(
  instanceId: string,
  featureId: string,
  stepId: string,
  assignment: StepAssignment,
): Promise<RequestTicket> {
  const path = `/v1/instances/${encodeURIComponent(instanceId)}/runs/${encodeURIComponent(featureId)}/steps/${encodeURIComponent(stepId)}/assignment`;
  rejectSyntheticInstance(instanceId, path);
  const body: Record<string, string> = {};
  if (assignment.agentKind !== undefined) body.agent_kind = assignment.agentKind;
  if (assignment.model !== undefined) body.model = assignment.model;
  if (assignment.effort !== undefined) body.effort = assignment.effort;
  return requestTicketOf(await postInSession(path, body, isWireRequestTicket));
}

export function isTerminal(state: RequestOutcome): boolean {
  return state.status === 'accepted' || state.status === 'refused' || state.status === 'expired';
}

/**
 * Reads the request now and again with backoff until its status is terminal,
 * the session is gone (`401`), or the returned function is called. The wait is
 * clamped to the expiry, seeded from the ticket so it holds when no read ever
 * succeeds, then taken from each read. The last read lands on it and none
 * follows, whatever that read said: past expiry nothing is left to deliver. The
 * Hub answers `expired` from that second on by its own clock, which need not
 * be this one's. A last read that is not terminal (still `queued` or
 * `delivered`, or failed) ends in `onStopped`, not in a verdict.
 *
 * The first read is issued even when the ticket has already expired: a status
 * remounted after expiry still learns whether the instance answered in time.
 */
export function pollRequestStatus(
  instanceId: string,
  ticket: RequestTicket,
  onState: (state: RequestState) => void,
  opts: PollOptions = {},
): () => void {
  let cancelled = false;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let delay = FIRST_DELAY_MS;
  let expiresAt = ticket.expiresAt;

  const schedule = () => {
    const left = expiresAt * 1000 - Date.now();
    if (left <= 0) {
      opts.onStopped?.();
      return;
    }
    const wait = Math.min(delay, left);
    delay = Math.min(delay * 2, MAX_DELAY_MS);
    timer = setTimeout(read, wait);
  };

  const read = () => {
    getRequestStatus(instanceId, ticket.requestId).then(
      (state) => {
        if (cancelled) return;
        expiresAt = state.expiresAt;
        onState(state);
        if (!isTerminal(state)) schedule();
      },
      (error: unknown) => {
        if (cancelled) return;
        opts.onError?.(error);
        if (!(error instanceof HubApiError && error.status === 401)) schedule();
      },
    );
  };

  read();
  return () => {
    cancelled = true;
    clearTimeout(timer);
  };
}

export function requestTicketOf(wire: WireRequestTicket): RequestTicket {
  return { requestId: wire.request_id, expiresAt: wire.expires_at };
}

function requestStateOf(wire: WireRequestState): RequestState {
  const common = {
    requestId: wire.request_id,
    kind: wire.kind,
    deliveries: wire.deliveries,
    expiresAt: wire.expires_at,
    answeredAt: wire.answered_at,
  };
  if (wire.status === 'refused') return { ...common, status: 'refused', reason: wire.reason };
  return { ...common, status: wire.status };
}

export const isWireRequestTicket: Guard<WireRequestTicket> = (value): value is WireRequestTicket =>
  typeof memberOf(value, 'request_id') === 'string' && typeof memberOf(value, 'expires_at') === 'number';

const isWireRequestState: Guard<WireRequestState> = (value): value is WireRequestState => {
  const status = memberOf(value, 'status');
  const answeredAt = memberOf(value, 'answered_at');
  return (
    typeof memberOf(value, 'request_id') === 'string' &&
    typeof memberOf(value, 'kind') === 'string' &&
    STATUSES.includes(status) &&
    (status !== 'refused' || typeof memberOf(value, 'reason') === 'string') &&
    typeof memberOf(value, 'deliveries') === 'number' &&
    typeof memberOf(value, 'expires_at') === 'number' &&
    (answeredAt === null || typeof answeredAt === 'number')
  );
};

function memberOf(value: unknown, member: string): unknown {
  return typeof value === 'object' && value !== null ? Reflect.get(value, member) : undefined;
}
