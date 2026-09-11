import { invoke } from "@tauri-apps/api/core";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { getFeatureStatusRollup } from "./project";

beforeEach(() => {
  vi.mocked(invoke).mockReset();
});

describe("getFeatureStatusRollup", () => {
  it("asks for the command the Rust side registers, and returns its rows unchanged", async () => {
    const rows = [
      { project_id: "p1", status: "running", count: 2 },
      { project_id: "p1", status: "gated", count: 1 },
    ];
    vi.mocked(invoke).mockResolvedValue(rows);

    expect(await getFeatureStatusRollup()).toEqual(rows);
    expect(invoke).toHaveBeenCalledWith("feature_status_rollup");
  });
});
