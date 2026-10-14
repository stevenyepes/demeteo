import { invoke } from "@tauri-apps/api/core";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { type HubStatus, hubDisconnect, hubPair, hubStatus } from "./hub";

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
});
