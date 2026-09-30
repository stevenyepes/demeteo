import { describe, expect, it } from "vitest";

import { parseCacheIdleTtlDays } from "./cacheIdleTtl";
import { viewSweep } from "./cacheSweepView";
import type { CacheSweepEntry, CacheSweepReport } from "./featureDetail";

function entry(overrides: Partial<CacheSweepEntry>): CacheSweepEntry {
  return {
    path: "/w/repo-cache-x",
    kind: "cache",
    verdict: "keep",
    reason: { code: "feature_may_run", text: "feature_may_run text" },
    outcome: null,
    ...overrides,
  };
}

const REPORT: CacheSweepReport = {
  dry_run: false,
  projects: [
    {
      project_id: "p-1",
      clone_dir: "/w/repo",
      error: null,
      worktree_list_error: null,
      entries: [
        entry({ path: "/w/a", verdict: "keep" }),
        entry({ path: "/w/b", verdict: "delete", reason: { code: "feature_released", text: "feature_released text" }, outcome: { status: "deleted" } }),
        entry({ path: "/w/c", verdict: "unknown", reason: { code: "age_unknown", text: "age_unknown text" } }),
        entry({
          path: "/w/d",
          kind: "worktree",
          verdict: "delete",
          reason: { code: "unregistered", text: "unregistered text" },
          outcome: { status: "failed", detail: "EACCES" },
        }),
        entry({
          path: "/w/e",
          verdict: "delete",
          reason: { code: "feature_idle", text: "feature_idle text" },
          outcome: { status: "spared", detail: { code: "feature_may_run", text: "feature_may_run text" } },
        }),
      ],
    },
    {
      project_id: "p-gone",
      clone_dir: null,
      error: "ssh: host unreachable",
      worktree_list_error: null,
      entries: [],
    },
    {
      project_id: "p-2",
      clone_dir: "/w/two",
      error: null,
      worktree_list_error: "git worktree list: not a repository",
      entries: [entry({ path: "/w/f", verdict: "keep", reason: { code: "registration_unknown", text: "registration_unknown text" } })],
    },
  ],
};

describe("viewSweep", () => {
  const view = viewSweep(REPORT, { "p-1": "Demeteo", "p-2": "Other" });

  it("groups each project's entries by verdict, deletions first, dropping empty groups", () => {
    expect(view.projects[0].groups.map((g) => [g.verdict, g.entries.map((e) => e.path)])).toEqual([
      ["delete", ["/w/b", "/w/d", "/w/e"]],
      ["unknown", ["/w/c"]],
      ["keep", ["/w/a"]],
    ]);
    expect(view.projects[1].groups).toEqual([]);
    expect(view.projects[2].groups.map((g) => g.verdict)).toEqual(["keep"]);
  });

  it("names projects it knows and falls back to the id for one it does not", () => {
    expect(view.projects.map((p) => p.name)).toEqual(["Demeteo", "p-gone", "Other"]);
  });

  it("counts verdicts, outcomes and projects that reported an error", () => {
    expect(view.totals).toEqual({
      delete: 3,
      unknown: 1,
      keep: 2,
      deleted: 1,
      failed: 1,
      spared: 1,
      projectErrors: 2,
    });
  });
});

describe("parseCacheIdleTtlDays", () => {
  it("reads blank as the engine default, not as zero", () => {
    expect(parseCacheIdleTtlDays("  ")).toEqual({ ok: true, days: null });
  });

  it("keeps 0, which switches idle release off", () => {
    expect(parseCacheIdleTtlDays("0")).toEqual({ ok: true, days: 0 });
  });

  it("accepts a whole number of days", () => {
    expect(parseCacheIdleTtlDays(" 30 ")).toEqual({ ok: true, days: 30 });
  });

  it.each(["-1", "2.5", "1e3", "abc", "4294967296"])("refuses %s", (text) => {
    expect(parseCacheIdleTtlDays(text).ok).toBe(false);
  });
});
