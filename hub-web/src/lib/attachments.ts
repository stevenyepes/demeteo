import { type Guard, HubApiError, postBytes, UUID } from './api';

/** Mirrors `crates/demeteo-hub/src/attachments/mod.rs`, kept equal by hand. */
export const MAX_ATTACHMENTS_PER_DISPATCH = 10;

export type StagedAttachment = { attachmentId: string; size: number; sha256: string; expiresAt: number };

type WireStagedAttachment = { attachment_id: string; size: number; sha256: string; expires_at: number };

/**
 * No byte limit is checked here: the Hub's `413 attachment_too_large` is the
 * authority, and a copy of its limit would drift from it unseen.
 */
export async function uploadAttachment(instanceId: string, file: Blob): Promise<StagedAttachment> {
  const path = `/v1/instances/${encodeURIComponent(instanceId)}/attachments`;
  if (!UUID.test(instanceId)) {
    throw new HubApiError(`${instanceId} is not an instance id`, path, 404, 'not_found');
  }
  const wire = await postBytes(path, file, isWireStagedAttachment);
  return {
    attachmentId: wire.attachment_id,
    size: wire.size,
    sha256: wire.sha256,
    expiresAt: wire.expires_at,
  };
}

const isWireStagedAttachment: Guard<WireStagedAttachment> = (value): value is WireStagedAttachment =>
  typeof value === 'object' &&
  value !== null &&
  typeof Reflect.get(value, 'attachment_id') === 'string' &&
  typeof Reflect.get(value, 'size') === 'number' &&
  typeof Reflect.get(value, 'sha256') === 'string' &&
  typeof Reflect.get(value, 'expires_at') === 'number';
