import type { HubRequestKind, HubScope, HubStopReason, HubSyncState } from "./hub";

/** Mirrors the Rust `RefusalReason` (`demeteo-hub-protocol/src/message.rs`).
 *  A log row's `reason` is a plain string, since accepted rows carry notes, so
 *  this list is what tells a refusal apart from free text. */
export type HubRefusalReason =
  | "scope_off"
  | "expired"
  | "replayed"
  | "wrong_instance"
  | "unknown_kind"
  | "malformed_body"
  | "over_run_ceiling"
  | "over_rolling_ceiling"
  | "unknown_project"
  | "unknown_workflow"
  | "unknown_feature"
  | "unknown_step"
  | "gate_not_pending"
  | "credential_not_pinned"
  | "assertion_invalid"
  | "step_not_reassignable"
  | "harness_not_registered"
  | "attachment_mismatch"
  | "last_credential"
  | "self_revocation"
  | "runner_owned";

export const UNSET_CEILING_COPY = "Not set — every Hub start is refused";

export type CeilingParse = { ok: true; cents: number | null } | { ok: false; error: string };

const DOLLARS = /^(\d*)(?:\.(\d*))?$/;

/** Dollars as typed → integer cents, by string arithmetic so no float ever
 *  mis-rounds a ceiling. A fraction past the cent rounds half up on its third
 *  digit (`0.005` → 1, `0.0049` → 0). Blank is `null`, which the desktop reads
 *  as "refuse every start" — never as unlimited, so it must not become `0`. */
export function parseCeilingDollars(input: string): CeilingParse {
  const trimmed = input.trim();
  if (trimmed === "") return { ok: true, cents: null };
  const match = DOLLARS.exec(trimmed.replace(/^\$/, ""));
  const whole = match?.[1] ?? "";
  const fraction = match?.[2] ?? "";
  if (!match || (whole === "" && fraction === "")) {
    return { ok: false, error: "Enter an amount in dollars, like 12.50" };
  }
  const padded = fraction.padEnd(3, "0");
  const roundUp = Number(padded[2]) >= 5 ? 1 : 0;
  const cents = Number(whole || "0") * 100 + Number(padded.slice(0, 2)) + roundUp;
  if (!Number.isSafeInteger(cents)) {
    return { ok: false, error: "That amount is too large" };
  }
  return { ok: true, cents };
}

function groupedDollars(cents: number): string {
  const whole = Math.floor(cents / 100).toLocaleString("en-US");
  return `${whole}.${String(cents % 100).padStart(2, "0")}`;
}

/** `0` is a real ceiling, which refuses every start that costs anything. */
export function formatCeiling(cents: number | null): string {
  return cents === null ? UNSET_CEILING_COPY : `$${groupedDollars(cents)}`;
}

/** The text an input should hold for a stored ceiling; it parses back to the
 *  same cents. */
export function ceilingInputValue(cents: number | null): string {
  if (cents === null) return "";
  return `${Math.floor(cents / 100)}.${String(cents % 100).padStart(2, "0")}`;
}

const REFUSAL_COPY: Readonly<Record<HubRefusalReason, string>> = {
  scope_off: "the scope it needs is off on this desktop",
  expired: "it expired before it arrived",
  replayed: "a repeat of a request already handled",
  wrong_instance: "addressed to another instance",
  unknown_kind: "not a request this desktop accepts",
  malformed_body: "the request was malformed",
  over_run_ceiling: "over the per-run ceiling",
  over_rolling_ceiling: "over the 24-hour ceiling",
  unknown_project: "no such project on this desktop",
  unknown_workflow: "no such workflow on this desktop",
  unknown_feature: "no such feature on this desktop",
  unknown_step: "no such step in that feature",
  gate_not_pending: "the gate was no longer waiting for a decision",
  credential_not_pinned: "signed by a passkey this desktop does not trust",
  assertion_invalid: "the passkey signature did not verify",
  step_not_reassignable: "the step can no longer be reassigned",
  harness_not_registered: "that harness is not registered on this desktop",
  attachment_mismatch: "the attachments did not match what was sent",
  last_credential: "it would remove the last trusted passkey",
  self_revocation: "a passkey cannot revoke itself",
  runner_owned: "the run belongs to a remote runner",
};

const REFUSAL_REASONS: ReadonlySet<string> = new Set(Object.keys(REFUSAL_COPY));

export function isRefusalReason(reason: string): reason is HubRefusalReason {
  return REFUSAL_REASONS.has(reason);
}

/** Any other string is shown as recorded: accepted rows carry notes here, and
 *  a reason newer than this list is still better than a blank. */
export function refusalReasonCopy(reason: string, scope: HubScope | null = null): string {
  if (reason === "scope_off" && scope !== null) return `${scope} is off on this desktop`;
  if (isRefusalReason(reason)) return REFUSAL_COPY[reason];
  return reason === "" ? "no reason recorded" : reason;
}

const STOP_COPY: Readonly<Record<HubStopReason, string>> = {
  displaced: "another connection for this instance took over",
  protocol_violation: "the Hub rejected a message from this desktop",
  unauthorized: "the Hub refused this desktop's credentials",
  refused: "the Hub refused the connection",
  needs_repair: "the pairing is no longer valid — pair again",
  refresh_uncertain: "a token refresh went unanswered — pair again",
  clock_skew: "this machine's clock is too far from the Hub's",
  stopped: "stopped on this desktop",
};

export function stopReasonCopy(reason: HubStopReason): string {
  return STOP_COPY[reason] ?? reason;
}

const SYNC_LABELS: Readonly<Record<HubSyncState, string>> = {
  in_sync: "in sync",
  catching_up: "catching up",
  not_reporting: "not reporting (read off)",
  offline: "offline",
};

export function syncStateLabel(sync: HubSyncState): string {
  return SYNC_LABELS[sync] ?? sync;
}

const KIND_LABELS: Readonly<Record<HubRequestKind, string>> = {
  start_feature: "start feature",
  cancel_feature: "cancel feature",
  set_step_assignment: "reassign step",
  gate_decision: "gate decision",
  passkey_endorsement: "add passkey",
  passkey_revocation: "remove passkey",
  unknown: "unreadable request",
};

export function logKindLabel(kind: HubRequestKind): string {
  return KIND_LABELS[kind] ?? kind;
}

export function formatEventNumber(offset: number): string {
  return `#${offset.toLocaleString("en-US")}`;
}
