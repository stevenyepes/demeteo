import type { EffortLevel } from '../../../src/lib/effortLevels';
import type { StepOverride } from '../../../src/types';
import { type Guard, getJson, HubApiError, postJson } from './api';
import { UUID } from './fleet';

/**
 * The body of `POST /v1/instances/{id}/runs`, spelled as the wire spells it.
 * `DispatchBody` in `crates/demeteo-hub/src/api/dispatch.rs` is
 * `deny_unknown_fields`, so a key added here that it lacks refuses every
 * dispatch with `400 invalid_request`.
 */
export type DispatchRequest = {
  project_id: string;
  workflow_id: string;
  title: string;
  description: string;
  agent_kind?: string;
  model?: string;
  effort?: EffortLevel;
  step_overrides: StepOverride[];
  max_budget_cents: number;
  max_wall_clock_secs?: number;
  attachment_ids: string[];
};

export type Enqueued = { requestId: string; expiresAt: number };

type RequestStatusCommon = {
  requestId: string;
  kind: string;
  deliveries: number;
  expiresAt: number;
  /** `null` unless the status is the instance's decision. */
  answeredAt: number | null;
};

export type RequestStatus =
  | (RequestStatusCommon & { status: 'queued' | 'delivered' | 'accepted' | 'expired' })
  | (RequestStatusCommon & { status: 'refused'; reason: string });

type WireEnqueued = { request_id: string; expires_at: number };

type WireRequestStatus = {
  request_id: string;
  kind: string;
  deliveries: number;
  expires_at: number;
  answered_at: number | null;
} & ({ status: Exclude<RequestStatus['status'], 'refused'> } | { status: 'refused'; reason: string });

const UNREFUSED_STATUSES: readonly unknown[] = ['queued', 'delivered', 'accepted', 'expired'] satisfies Exclude<
  RequestStatus['status'],
  'refused'
>[];

export async function dispatchFeature(instanceId: string, body: DispatchRequest): Promise<Enqueued> {
  const path = `/v1/instances/${encodeURIComponent(instanceId)}/runs`;
  refuseNonInstanceId(instanceId, path);
  const wire = await postJson(path, body, isWireEnqueued);
  return { requestId: wire.request_id, expiresAt: wire.expires_at };
}

export async function getRequestStatus(instanceId: string, requestId: string): Promise<RequestStatus> {
  const path = `/v1/instances/${encodeURIComponent(instanceId)}/requests/${encodeURIComponent(requestId)}`;
  refuseNonInstanceId(instanceId, path);
  return requestStatusOf(await getJson(path, isWireRequestStatus));
}

/**
 * The instance's refusal reasons a dispatch can meet, from `RefusalReason` in
 * `crates/demeteo-hub-protocol/src/message.rs`. A `Map` rather than an object
 * so a reason spelled like an `Object.prototype` member finds nothing.
 */
const REFUSAL_SENTENCES = new Map<string, string>([
  ['scope_off', 'Spend is off on this instance, so it does not start Features sent from the Hub.'],
  ['over_run_ceiling', "The max cost is above this instance's per-run ceiling."],
  ['over_rolling_ceiling', "This run would take the instance past its rolling spend ceiling."],
  ['unknown_project', 'The instance has no project with this id.'],
  ['unknown_workflow', 'The instance has no workflow with this id.'],
  ['harness_not_registered', 'An agent this run names is not registered on the instance.'],
  ['attachment_mismatch', 'An attachment the instance fetched does not match the one that was signed.'],
  ['malformed_body', 'The instance could not read the request.'],
  ['expired', 'The request expired before the instance acted on it.'],
]);

/** `null` for a reason with no sentence here; the caller shows the raw code. */
export function refusalSentence(reason: string): string | null {
  return REFUSAL_SENTENCES.get(reason) ?? null;
}

const DISPATCH_ERROR_SENTENCES = new Map<string, string>([
  [
    'attachment_unavailable',
    'An attachment has expired or was already sent with another request. Upload it again.',
  ],
  ['attachment_too_large', "A file is over the Hub's size limit for an attachment, so the Feature was not sent."],
  ['invalid_request', 'The Hub could not accept this request as written.'],
  ['key_service_unavailable', "The Hub's signing key is unavailable right now. Try again shortly."],
]);

/** What the Hub's refusal of a dispatch means, or `null` when there is no sentence for it. */
export function dispatchErrorSentence(error: HubApiError): string | null {
  if (error.code === null) return null;
  if (error.code === 'scope_not_granted') {
    return error.scope === 'spend' ? 'Spend is off on this instance.' : null;
  }
  return DISPATCH_ERROR_SENTENCES.get(error.code) ?? null;
}

function refuseNonInstanceId(instanceId: string, path: string): void {
  if (!UUID.test(instanceId)) {
    throw new HubApiError(`${instanceId} is not an instance id`, path, 404, 'not_found');
  }
}

function requestStatusOf(wire: WireRequestStatus): RequestStatus {
  const common: RequestStatusCommon = {
    requestId: wire.request_id,
    kind: wire.kind,
    deliveries: wire.deliveries,
    expiresAt: wire.expires_at,
    answeredAt: wire.answered_at,
  };
  if (wire.status === 'refused') {
    return { ...common, status: 'refused', reason: wire.reason };
  }
  return { ...common, status: wire.status };
}

const isWireEnqueued: Guard<WireEnqueued> = (value): value is WireEnqueued =>
  typeof value === 'object' &&
  value !== null &&
  typeof Reflect.get(value, 'request_id') === 'string' &&
  typeof Reflect.get(value, 'expires_at') === 'number';

const isWireRequestStatus: Guard<WireRequestStatus> = (value): value is WireRequestStatus => {
  if (typeof value !== 'object' || value === null) return false;
  const status: unknown = Reflect.get(value, 'status');
  const answeredAt: unknown = Reflect.get(value, 'answered_at');
  const statusHolds =
    status === 'refused' ? typeof Reflect.get(value, 'reason') === 'string' : UNREFUSED_STATUSES.includes(status);
  return (
    statusHolds &&
    typeof Reflect.get(value, 'request_id') === 'string' &&
    typeof Reflect.get(value, 'kind') === 'string' &&
    typeof Reflect.get(value, 'deliveries') === 'number' &&
    typeof Reflect.get(value, 'expires_at') === 'number' &&
    (answeredAt === null || typeof answeredAt === 'number')
  );
};
