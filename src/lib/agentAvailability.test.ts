import { describe, expect, it } from "vitest";

import { offerableAgentKinds } from "./agentAvailability";

const row = (kind: string, enabled: boolean, available: boolean) => ({ kind, enabled, available });

describe("offerableAgentKinds", () => {
  it("keeps a harness that is enabled and available", () => {
    expect(offerableAgentKinds([row("pi", true, true)])).toEqual(["pi"]);
  });

  it("drops a harness the machine does not have", () => {
    expect(offerableAgentKinds([row("pi", true, false)])).toEqual([]);
  });

  it("drops a disabled harness", () => {
    expect(offerableAgentKinds([row("pi", false, true)])).toEqual([]);
  });

  it("drops a harness that is neither enabled nor available", () => {
    expect(offerableAgentKinds([row("pi", false, false)])).toEqual([]);
  });

  it("preserves input order across a mixed list", () => {
    expect(
      offerableAgentKinds([
        row("opencode", true, true),
        row("hermes", true, false),
        row("claude-code", true, true),
        row("codex", false, true),
        row("pi", true, true),
      ]),
    ).toEqual(["opencode", "claude-code", "pi"]);
  });
});
