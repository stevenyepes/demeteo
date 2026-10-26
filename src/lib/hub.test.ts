import { invoke } from "@tauri-apps/api/core";
import { beforeEach, describe, expect, it, vi } from "vitest";

import {
  type HubLocalPolicy,
  type HubRequestLogEntry,
  type HubStatus,
  hubDisconnect,
  hubPair,
  hubPolicyGet,
  hubPolicySet,
  hubRequestLog,
  hubStatus,
} from "./hub";

const PAIRED: HubStatus = {
  state: "paired",
  hub_url: "https://hub.example.test",
  instance_id: "inst-1",
  fingerprint: "SHA256:abcd efgh ijkl mnop",
  rp_id: "hub.example.test",
  scopes: ["gate:decide"],
  paired_at: 1_790_000_000,
  keyring_backend: "secret-service",
  keyring_available: true,
};

const UNPAIRED: HubStatus = {
  state: "unpaired",
  hub_url: null,
  instance_id: null,
  fingerprint: null,
  rp_id: null,
  scopes: [],
  paired_at: null,
  keyring_backend: "secret-service",
  keyring_available: true,
};

const POLICY: HubLocalPolicy = {
  spend_enabled: true,
  gates_enabled: false,
  run_ceiling_cents: 500,
  daily_ceiling_cents: null,
};

const LOG: HubRequestLogEntry[] = [
  {
    request_id: "r-1",
    kind: "start_feature",
    scope: "spend",
    outcome: "refused",
    reason: "over_run_ceiling",
    feature_id: null,
    received_at_ms: 1_790_000_000_000,
  },
];

beforeEach(() => {
  vi.mocked(invoke).mockReset();
});

describe("hub wrappers", () => {
  it("hubPair sends the URL as camelCase hubUrl and returns the status unchanged", async () => {
    vi.mocked(invoke).mockResolvedValue(PAIRED);

    const result = await hubPair("https://hub.example.test");

    expect(invoke).toHaveBeenCalledWith("hub_pair", { hubUrl: "https://hub.example.test" });
    expect(result).toBe(PAIRED);
  });

  it("hubStatus calls hub_status with no arguments and returns the status unchanged", async () => {
    vi.mocked(invoke).mockResolvedValue(PAIRED);

    const result = await hubStatus();

    expect(invoke).toHaveBeenCalledWith("hub_status");
    expect(result).toBe(PAIRED);
  });

  it("hubDisconnect calls hub_disconnect with no arguments and returns the status unchanged", async () => {
    vi.mocked(invoke).mockResolvedValue(UNPAIRED);

    const result = await hubDisconnect();

    expect(invoke).toHaveBeenCalledWith("hub_disconnect");
    expect(result).toBe(UNPAIRED);
  });

  it("hubPolicyGet calls hub_policy_get with no arguments and returns the policy unchanged", async () => {
    vi.mocked(invoke).mockResolvedValue(POLICY);

    const result = await hubPolicyGet();

    expect(invoke).toHaveBeenCalledWith("hub_policy_get");
    expect(result).toBe(POLICY);
  });

  it("hubPolicySet sends the snake_case policy under policy", async () => {
    vi.mocked(invoke).mockResolvedValue(undefined);

    await hubPolicySet(POLICY);

    expect(invoke).toHaveBeenCalledWith("hub_policy_set", { policy: POLICY });
  });

  it("hubRequestLog sends the limit and returns the entries unchanged", async () => {
    vi.mocked(invoke).mockResolvedValue(LOG);

    const result = await hubRequestLog(50);

    expect(invoke).toHaveBeenCalledWith("hub_request_log", { limit: 50 });
    expect(result).toBe(LOG);
  });
});
