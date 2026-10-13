import type { Guard } from './api';
import { postInSession } from './auth';

export type PairingScope = 'read' | 'spend' | 'gates';

export type PairingPrompt = {
  pairingId: string;
  /**
   * The string the Hub rendered, shown as it came. The user compares it with
   * the one their desktop shows, so it is never derived or reformatted here:
   * a second renderer is a second chance for the two to differ.
   */
  fingerprint: string;
  instanceName: string;
  os: string;
  arch: string;
  appVersion: string;
  /** What the desktop asked for. An approval can only narrow it. */
  scopes: PairingScope[];
  /** Unix seconds. */
  expiresAt: number;
};

type WirePairingPrompt = {
  pairing_id: string;
  fingerprint: string;
  instance_name: string;
  os: string;
  arch: string;
  app_version: string;
  scopes: PairingScope[];
  expires_at: number;
};

type ConsentAnswer = { redirect: string };

const SCOPES: readonly unknown[] = ['read', 'spend', 'gates'] satisfies PairingScope[];

/** `request` is the opaque segment `/authorize` put in the fragment, passed on untouched. */
export async function startPairing(request: string): Promise<PairingPrompt> {
  return promptOf(await postInSession('/v1/pairing/start', { request }, isWirePairingPrompt));
}

/** Resolves to the loopback redirect that carries the authorization code: hand it to `leaveFor`. */
export async function approvePairing(pairingId: string, scopes: PairingScope[]): Promise<string> {
  const { redirect } = await postInSession(
    '/v1/pairing/approve',
    { pairing_id: pairingId, scopes },
    isConsentAnswer,
  );
  return redirect;
}

/** Resolves to the loopback redirect that tells the desktop it was denied: hand it to `leaveFor`. */
export async function denyPairing(pairingId: string): Promise<string> {
  const { redirect } = await postInSession('/v1/pairing/deny', { pairing_id: pairingId }, isConsentAnswer);
  return redirect;
}

/**
 * The one navigation off the Hub's origin. The check is made again here, where
 * the browser is sent, so that a caller holding some other string cannot turn
 * this into an open redirect.
 */
export function leaveFor(redirect: string): void {
  if (!isLoopbackRedirect(redirect)) throw new Error('Not a redirect to the loopback listener');
  window.location.assign(redirect);
}

function promptOf(wire: WirePairingPrompt): PairingPrompt {
  return {
    pairingId: wire.pairing_id,
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
 * `http://127.0.0.1:`, a port, a path, and the query the Hub appends, matched
 * as text and never parsed: `is_loopback_redirect` in
 * `crates/demeteo-hub/src/pairing/authorize.rs` says why, and this is the same
 * rule held a second time at the place the browser follows it. The redirect
 * carries the authorization code, so anything else — the loopback hostname,
 * `https`, no port, userinfo — is somewhere the code must not go.
 */
const LOOPBACK_REDIRECT = /^http:\/\/127\.0\.0\.1:([1-9][0-9]{0,4})\/[A-Za-z0-9._~/-]*(\?[A-Za-z0-9._~=&-]*)?$/;

const HIGHEST_PORT = 65_535;

function isLoopbackRedirect(value: unknown): value is string {
  if (typeof value !== 'string') return false;
  const port = LOOPBACK_REDIRECT.exec(value)?.[1];
  return port !== undefined && Number(port) <= HIGHEST_PORT;
}

/**
 * A scope this build does not know is refused rather than shown: the page
 * explains what each one allows, and a name it cannot explain is one the user
 * would be approving unread.
 */
const isWirePairingPrompt: Guard<WirePairingPrompt> = (value): value is WirePairingPrompt => {
  const scopes = memberOf(value, 'scopes');
  return (
    typeof memberOf(value, 'pairing_id') === 'string' &&
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

const isConsentAnswer: Guard<ConsentAnswer> = (value): value is ConsentAnswer =>
  isLoopbackRedirect(memberOf(value, 'redirect'));

function memberOf(value: unknown, member: string): unknown {
  return typeof value === 'object' && value !== null ? Reflect.get(value, member) : undefined;
}
