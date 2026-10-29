import { invoke } from "@tauri-apps/api/core";
import { beforeEach, describe, expect, it, vi } from "vitest";

import {
  type HubConnection,
  type HubLocalPolicy,
  type HubMachine,
  type HubRequestLogEntry,
  type HubStatus,
  type HubTrustedPasskey,
  hubConnection,
  hubDisconnect,
  hubMachine,
  hubPair,
  hubPolicyGet,
  hubPolicySet,
  hubRequestLog,
  hubStatus,
  hubTrustedPasskeys,
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

const STOPPED: HubConnection = {
  state: "stopped",
  stop_reason: "displaced",
  latency_ms: null,
  reported_through: 42,
  sync: "offline",
  last_heard_at_ms: 1_790_000_000_000,
};

const MACHINE: HubMachine = {
  instance_name: "Ana's laptop",
  os: "linux",
  arch: "x86_64",
  app_version: "1.4.0",
};

const PASSKEYS: HubTrustedPasskey[] = [
  { credential_id: "cred-a", sign_count: 3, pinned_at: 1_790_000_000, origin: "pinned" },
  { credential_id: "cred-b", sign_count: 0, pinned_at: 1_790_000_060, origin: "endorsed" },
];

const POLICY: HubLocalPolicy = {
  read_enabled: true,
  spend_enabled: true,
  gates_enabled: false,
  run_ceiling_cents: 500,
  daily_ceiling_cents: null,
};

const LOG: HubRequestLogEntry[] = [
  {
    id: 1,
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

  it("hubConnection calls hub_connection with no arguments and returns the connection unchanged", async () => {
    vi.mocked(invoke).mockResolvedValue(STOPPED);

    const result = await hubConnection();

    expect(invoke).toHaveBeenCalledWith("hub_connection");
    expect(result).toBe(STOPPED);
  });

  it("hubMachine calls hub_machine with no arguments and returns the machine unchanged", async () => {
    vi.mocked(invoke).mockResolvedValue(MACHINE);

    const result = await hubMachine();

    expect(invoke).toHaveBeenCalledWith("hub_machine");
    expect(result).toBe(MACHINE);
  });

  it("hubTrustedPasskeys calls hub_trusted_passkeys with no arguments and returns the list unchanged", async () => {
    vi.mocked(invoke).mockResolvedValue(PASSKEYS);

    const result = await hubTrustedPasskeys();

    expect(invoke).toHaveBeenCalledWith("hub_trusted_passkeys");
    expect(result).toBe(PASSKEYS);
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
