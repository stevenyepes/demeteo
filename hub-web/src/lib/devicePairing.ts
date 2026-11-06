import type { Guard } from './api';
import { postInSession } from './auth';
import type { PairingScope } from './pairing';
import { type AssertionFields, getAssertion } from './webauthn';

/** A runner's pending pairing as `POST /v1/pairing/device/lookup` answers it. */
export type DevicePrompt = {
  /** The Hub's spelling, `XXXX-XXXX`: the one every later call names it by. */
  userCode: string;
  /** Shown as it came, for the reason `PairingPrompt.fingerprint` gives. */
  fingerprint: string;
  instanceName: string;
  os: string;
  arch: string;
  appVersion: string;
  /** What the runner asked for. An approval can only narrow it. */
  scopes: PairingScope[];
  /** Unix seconds. */
  expiresAt: number;
};

type WireDevicePrompt = {
  user_code: string;
  fingerprint: string;
  instance_name: string;
  os: string;
  arch: string;
  app_version: string;
  scopes: PairingScope[];
  expires_at: number;
};

type DeviceChallenge = { ceremony_id: string; options: unknown };

const SCOPES: readonly unknown[] = ['read', 'spend', 'gates'] satisfies PairingScope[];

/** `userCode` as the human typed it: the Hub reads it in any case, with or without its `-`. */
export async function lookUpDevice(userCode: string): Promise<DevicePrompt> {
  const wire = await postInSession('/v1/pairing/device/lookup', { user_code: userCode }, isWireDevicePrompt);
  return {
    userCode: wire.user_code,
    fingerprint: wire.fingerprint,
    instanceName: wire.instance_name,
    os: wire.os,
    arch: wire.arch,
    appVersion: wire.app_version,
    scopes: wire.scopes,
    expiresAt: wire.expires_at,
  };
}

/**
 * The Hub issues a challenge over this pairing's device key and `scopes`, and
 * records the approval only once the assertion over it verifies: holding the
 * session is not enough, because anyone can start a device code and hand its
 * `user_code` to a signed-in human (`docs/HUB.md` §4). `assertion` is passed
 * on untouched, for the reason `assertOver` in `passkeys.ts` gives.
 */
export async function approveDevice(userCode: string, scopes: PairingScope[]): Promise<void> {
  const challenge = await postInSession(
    '/v1/pairing/device/challenge',
    { user_code: userCode, scopes },
    isDeviceChallenge,
  );
  const assertion: AssertionFields = await getAssertion(challenge.options);
  await postInSession(
    '/v1/pairing/device/approve',
    { ceremony_id: challenge.ceremony_id, assertion },
    answeredAs('approved'),
  );
}

export async function denyDevice(userCode: string): Promise<void> {
  await postInSession('/v1/pairing/device/deny', { user_code: userCode }, answeredAs('denied'));
}

/** A scope this build does not know is refused, for the reason `isWirePairingPrompt` gives. */
const isWireDevicePrompt: Guard<WireDevicePrompt> = (value): value is WireDevicePrompt => {
  const scopes = memberOf(value, 'scopes');
  return (
    typeof memberOf(value, 'user_code') === 'string' &&
    typeof memberOf(value, 'fingerprint') === 'string' &&
    typeof memberOf(value, 'instance_name') === 'string' &&
    typeof memberOf(value, 'os') === 'string' &&
    typeof memberOf(value, 'arch') === 'string' &&
    typeof memberOf(value, 'app_version') === 'string' &&
    Array.isArray(scopes) &&
    scopes.every((scope) => SCOPES.includes(scope)) &&
    typeof memberOf(value, 'expires_at') === 'number'
  );
};

const isDeviceChallenge: Guard<DeviceChallenge> = (value): value is DeviceChallenge =>
  typeof memberOf(value, 'ceremony_id') === 'string' && memberOf(value, 'options') !== undefined;

function answeredAs(answer: 'approved' | 'denied'): Guard<{ answer: typeof answer }> {
  return (value): value is { answer: typeof answer } => memberOf(value, 'answer') === answer;
}

function memberOf(value: unknown, member: string): unknown {
  return typeof value === 'object' && value !== null ? Reflect.get(value, member) : undefined;
}
