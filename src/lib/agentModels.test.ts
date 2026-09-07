import { invoke } from "@tauri-apps/api/core";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { _resetAgentModelsCache, getAgentModels } from "./agentModels";

beforeEach(() => {
  vi.mocked(invoke).mockReset();
  _resetAgentModelsCache();
});

const MODEL = { value: "openai-codex/gpt-6-astra", name: "openai-codex/gpt-6-astra", description: null, supports_images: true };

describe("getAgentModels", () => {
  it("answers a repeat ask from the cache rather than re-probing", async () => {
    vi.mocked(invoke).mockResolvedValue([MODEL]);

    expect(await getAgentModels("local", "pi")).toEqual([MODEL]);
    expect(await getAgentModels("local", "pi")).toEqual([MODEL]);

    expect(invoke).toHaveBeenCalledTimes(1);
  });

  it("probes again after an empty answer, which is what a failed probe looks like", async () => {
    vi.mocked(invoke).mockResolvedValueOnce([]).mockResolvedValueOnce([MODEL]);

    expect(await getAgentModels("local", "pi")).toEqual([]);
    expect(await getAgentModels("local", "pi")).toEqual([MODEL]);

    expect(invoke).toHaveBeenCalledTimes(2);
  });

  it("shares one probe between concurrent askers", async () => {
    vi.mocked(invoke).mockResolvedValue([MODEL]);

    const [a, b] = await Promise.all([
      getAgentModels("local", "pi"),
      getAgentModels("local", "pi"),
    ]);

    expect(a).toEqual([MODEL]);
    expect(b).toEqual([MODEL]);
    expect(invoke).toHaveBeenCalledTimes(1);
  });
});
