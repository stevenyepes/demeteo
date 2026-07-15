// Tests for the run-in-background preference wrappers.
//
// These are thin `invoke` shims, so the only thing worth pinning is the wire
// contract the PreferencesScreen toggle depends on: the command names and that
// `setRunInBackground` forwards the boolean under the `enabled` key exactly as
// the Rust `set_run_in_background(enabled: bool)` command expects. The stored
// round-trip itself is covered on the Rust side by `tests/tray_notification.rs`.

import { invoke } from "@tauri-apps/api/core";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { getRunInBackground, setRunInBackground } from "./background";

beforeEach(() => {
  vi.mocked(invoke).mockReset();
});

describe("getRunInBackground", () => {
  it("reads the run_in_background command and returns the stored boolean", async () => {
    vi.mocked(invoke).mockResolvedValue(true);

    expect(await getRunInBackground()).toBe(true);
    expect(invoke).toHaveBeenCalledWith("get_run_in_background");
  });
});

describe("setRunInBackground", () => {
  it("forwards the enabled flag under the { enabled } key", async () => {
    vi.mocked(invoke).mockResolvedValue(undefined);

    await setRunInBackground(true);
    expect(invoke).toHaveBeenCalledWith("set_run_in_background", { enabled: true });

    await setRunInBackground(false);
    expect(invoke).toHaveBeenCalledWith("set_run_in_background", { enabled: false });
  });
});
