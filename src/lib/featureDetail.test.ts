import { invoke } from "@tauri-apps/api/core";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { RunEvent } from "../types";
import { listRunEventsSince } from "./featureDetail";

beforeEach(() => {
  vi.mocked(invoke).mockReset();
});

describe("listRunEventsSince", () => {
  it("returns durable run events after the exclusive cursor", async () => {
    const events: RunEvent[] = [
      {
        offset: 42,
        run_id: "feature-1",
        kind: "agent_spawned",
        payload_json: '{"step_execution_id":"execution-1"}',
        created_at: 1_723_456_789,
      },
    ];
    vi.mocked(invoke).mockResolvedValue(events);

    await expect(listRunEventsSince("feature-1", 41)).resolves.toEqual(events);
    expect(invoke).toHaveBeenCalledWith("run_events_since", {
      featureId: "feature-1",
      fromOffset: 41,
    });
  });
});
