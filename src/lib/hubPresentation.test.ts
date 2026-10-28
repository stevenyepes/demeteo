import { describe, expect, it } from "vitest";

import type { HubRequestKind, HubStopReason } from "./hub";
import {
  type HubRefusalReason,
  UNSET_CEILING_COPY,
  ceilingInputValue,
  formatCeiling,
  formatEventNumber,
  isRefusalReason,
  logKindLabel,
  parseCeilingDollars,
  refusalReasonCopy,
  stopReasonCopy,
  syncStateLabel,
} from "./hubPresentation";

/** Spelled by hand from `RefusalReason` in `demeteo-hub-protocol`, so a
 *  rename on either side shows up here rather than as a raw string in the log. */
const EVERY_REFUSAL: HubRefusalReason[] = [
  "scope_off",
  "expired",
  "replayed",
  "wrong_instance",
  "unknown_kind",
  "malformed_body",
  "over_run_ceiling",
  "over_rolling_ceiling",
  "unknown_project",
  "unknown_workflow",
  "unknown_feature",
  "unknown_step",
  "gate_not_pending",
  "credential_not_pinned",
  "assertion_invalid",
  "step_not_reassignable",
  "harness_not_registered",
  "attachment_mismatch",
  "last_credential",
  "self_revocation",
  "runner_owned",
];

const EVERY_STOP: HubStopReason[] = [
  "displaced",
  "protocol_violation",
  "unauthorized",
  "refused",
  "needs_repair",
  "refresh_uncertain",
  "clock_skew",
  "stopped",
];

const EVERY_KIND: HubRequestKind[] = [
  "start_feature",
  "cancel_feature",
  "set_step_assignment",
  "gate_decision",
  "passkey_endorsement",
  "passkey_revocation",
  "unknown",
];

function cents(input: string): number | null {
  const parsed = parseCeilingDollars(input);
  if (!parsed.ok) throw new Error(`rejected ${input}: ${parsed.error}`);
  return parsed.cents;
}

describe("parseCeilingDollars", () => {
  it("reads blank as unset, not as zero", () => {
    expect(cents("")).toBeNull();
    expect(cents("   ")).toBeNull();
  });

  it("keeps zero as a real ceiling", () => {
    expect(cents("0")).toBe(0);
    expect(cents("0.00")).toBe(0);
  });

  it("parses whole and fractional dollars exactly", () => {
    expect(cents(".01")).toBe(1);
    expect(cents("0.1")).toBe(10);
    expect(cents("12")).toBe(1200);
    expect(cents("12.")).toBe(1200);
    expect(cents("12.5")).toBe(1250);
    expect(cents(" $4.20 ")).toBe(420);
    expect(cents("1.15")).toBe(115);
  });

  it("rounds a sub-cent fraction half up on its third digit", () => {
    expect(cents(".005")).toBe(1);
    expect(cents("0.0049")).toBe(0);
    expect(cents("1.005")).toBe(101);
    expect(cents("0.995")).toBe(100);
    expect(cents("2.0050")).toBe(201);
  });

  it("accepts large values up to the safe-integer bound and rejects past it", () => {
    expect(cents("1000000")).toBe(100_000_000);
    expect(cents("90071992547409.91")).toBe(Number.MAX_SAFE_INTEGER);
    expect(parseCeilingDollars("90071992547409.92").ok).toBe(false);
    expect(parseCeilingDollars("1e30").ok).toBe(false);
    expect(parseCeilingDollars("99999999999999999999").ok).toBe(false);
  });

  it("rejects negatives and non-numbers", () => {
    for (const bad of ["-1", "-0.01", "abc", "1,000", "1.2.3", ".", "$", "1e3", "Infinity", "NaN", "0x10"]) {
      const parsed = parseCeilingDollars(bad);
      expect(parsed.ok, bad).toBe(false);
      if (!parsed.ok) expect(parsed.error).not.toBe("");
    }
  });
});

describe("formatCeiling", () => {
  it("shows unset as refuse-all, never as unlimited", () => {
    expect(formatCeiling(null)).toBe(UNSET_CEILING_COPY);
    expect(UNSET_CEILING_COPY).toBe("Not set — every Hub start is refused");
    expect(formatCeiling(null)).not.toMatch(/unlimited/i);
  });

  it("shows zero as a ceiling of nothing", () => {
    expect(formatCeiling(0)).toBe("$0.00");
  });

  it("formats cents as grouped dollars", () => {
    expect(formatCeiling(1)).toBe("$0.01");
    expect(formatCeiling(1250)).toBe("$12.50");
    expect(formatCeiling(123_456_789)).toBe("$1,234,567.89");
  });
});

describe("ceilingInputValue", () => {
  it("leaves an unset ceiling blank", () => {
    expect(ceilingInputValue(null)).toBe("");
  });

  it("round-trips through the parser", () => {
    for (const value of [0, 1, 99, 100, 1250, 123_456_789, Number.MAX_SAFE_INTEGER]) {
      expect(cents(ceilingInputValue(value))).toBe(value);
    }
  });
});

describe("refusalReasonCopy", () => {
  it.each(EVERY_REFUSAL)("gives %s its own human copy", (reason) => {
    expect(isRefusalReason(reason)).toBe(true);
    const copy = refusalReasonCopy(reason);
    expect(copy.trim()).not.toBe("");
    expect(copy).not.toBe(reason);
  });

  it("gives every reason distinct copy", () => {
    const copies = EVERY_REFUSAL.map((r) => refusalReasonCopy(r));
    expect(new Set(copies).size).toBe(EVERY_REFUSAL.length);
  });

  it("names the scope a scope_off refusal hit when the row has one", () => {
    expect(refusalReasonCopy("scope_off", "spend")).toBe("spend is off on this desktop");
    expect(refusalReasonCopy("scope_off", "gates")).toBe("gates is off on this desktop");
  });

  it("names the 24-hour ceiling for a rolling refusal", () => {
    expect(refusalReasonCopy("over_rolling_ceiling")).toBe("over the 24-hour ceiling");
  });

  it("falls back to the raw string for a reason it does not know", () => {
    expect(isRefusalReason("over_moon_ceiling")).toBe(false);
    expect(refusalReasonCopy("over_moon_ceiling")).toBe("over_moon_ceiling");
    expect(refusalReasonCopy("constructor")).toBe("constructor");
  });

  it("is never blank", () => {
    expect(refusalReasonCopy("")).toBe("no reason recorded");
  });
});

describe("stopReasonCopy", () => {
  it.each(EVERY_STOP)("gives %s its own human copy", (reason) => {
    const copy = stopReasonCopy(reason);
    expect(copy.trim()).not.toBe("");
    expect(copy).not.toBe(reason);
  });

  it("tells the user to pair again when only a re-pair helps", () => {
    expect(stopReasonCopy("needs_repair")).toMatch(/pair again/);
  });
});

describe("syncStateLabel", () => {
  it("labels every sync state", () => {
    expect(syncStateLabel("in_sync")).toBe("in sync");
    expect(syncStateLabel("catching_up")).toBe("catching up");
    expect(syncStateLabel("not_reporting")).toBe("not reporting (read off)");
    expect(syncStateLabel("offline")).toBe("offline");
  });
});

describe("logKindLabel", () => {
  it.each(EVERY_KIND)("labels %s", (kind) => {
    const label = logKindLabel(kind);
    expect(label.trim()).not.toBe("");
    expect(label).not.toContain("_");
  });
});

describe("formatEventNumber", () => {
  it("groups thousands behind a hash", () => {
    expect(formatEventNumber(9512)).toBe("#9,512");
    expect(formatEventNumber(0)).toBe("#0");
    expect(formatEventNumber(1_234_567)).toBe("#1,234,567");
  });
});
