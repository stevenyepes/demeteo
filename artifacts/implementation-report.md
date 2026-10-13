# AC6 mutation-check report

```
Watched-red record for AC6. Every new test in this feature was run
against a deliberate mutation of the code it covers, seen to fail, and
then the mutation was reverted with `git checkout -- <file>` and the
file re-run green. This commit changes no file: the evidence is this
body. Baseline before any mutation: prefs 8/8, runMetaDefaults 4/4,
harnessVerdict 26/26 passing.

M1 (AC2) - src/components/FeatureDetail/FeatureDetail.tsx
  Mutation: `activityChoice ?? activityOpensByDefault({...})`
            -> `activityChoice ?? true`
  Run: npx vitest run src/components/FeatureDetail/FeatureDetail.prefs.test.tsx
  Result: 1 failed | 7 passed (8)
  Red: FeatureDetail.prefs.test.tsx > the run view restores what was
       stored for it > opens the activity log collapsed, whatever the
       last mount left it on
  Assertion (FeatureDetail.prefs.test.tsx:221):
    expect(element).toHaveAttribute("aria-expanded", "false")
    Expected: aria-expanded="false"   Received: aria-expanded="true"
  Reverted -> 8 passed (8)

M2 (AC3) - src/components/FeatureDetail/runMetaDefaults.ts
  Mutation: `return input.remote && !input.terminal;` -> `return true;`
  Run: npx vitest run src/components/FeatureDetail/runMetaDefaults.test.ts
  Result: 3 failed | 1 passed (4)
  Red: activityOpensByDefault > keeps a live local run collapsed
       activityOpensByDefault > keeps a finished local run collapsed
       activityOpensByDefault > keeps a finished detached run collapsed
  Assertion (runMetaDefaults.test.ts:7 and :11, same at :15):
    AssertionError: expected true to be false // Object.is equality
  Reverted -> 4 passed (4)
  Variant M2b: `return !input.remote || !input.terminal;`
    Result: 2 failed | 2 passed (4) - red: 'keeps a live local run
    collapsed', 'keeps a finished local run collapsed', both
    "expected true to be false". Reverted -> 4 passed (4)

M3 (AC5a) - src/lib/harnessVerdict.ts
  Mutation: NOW_BUCKETS `['not-reported', 'no failure reported']`
            -> `['not-reported', 'all passed']`
  Run: npx vitest run src/lib/harnessVerdict.test.ts
  Result: 3 failed | 23 passed (26)
  Red: summarizeGateRows > does not call a run with no failure
       reported green
  Assertion (harnessVerdict.test.ts:297):
    AssertionError: expected '2 all passed' to be
    '2 no failure reported' // Object.is equality
  Also red: 'names an unmeasured baseline instead of reading it as a
    pass' (expected '2 all passed · not measured' not to match
    /pass|green|ok|healthy|all clear/i) and 'turns ruby on a single
    failed gate' (expected '1 failed · 1 excluded · 1 all passed' to be
    '1 failed · 1 excluded · 1 no failure reported').
  Reverted -> 26 passed (26)

M4 (AC4) - src/components/FeatureDetail/FeatureDetail.tsx
  Mutation: harnessOpen `useState(false)` -> `useState(true)`
  Run: npx vitest run src/components/FeatureDetail/FeatureDetail.prefs.test.tsx
  Result: 1 failed | 7 passed (8)
  Red: the run view restores what was stored for it > opens the
       harness gates collapsed, whatever the last mount left them on
  Assertion (FeatureDetail.prefs.test.tsx:285):
    expect(element).toHaveAttribute("aria-expanded", "false")
    Expected: aria-expanded="false"   Received: aria-expanded="true"
  Reverted -> 8 passed (8)

M5 (AC3, the click wins) - src/components/FeatureDetail/FeatureDetail.tsx
  Mutation: drop `activityChoice ??`, so activityOpensByDefault alone
            decides activityOpen
  Run: npx vitest run src/components/FeatureDetail/
  Result: 4 failed | 254 passed (258)
  Red: prefs > keeps the user's activity choice when the run under it
       finishes (FeatureDetail.prefs.test.tsx:270)
       prefs > opens the activity log collapsed, whatever the last
       mount left it on - its post-click assertion (:226)
    Both: expect(element).toHaveAttribute("aria-expanded", "true")
    Expected: aria-expanded="true"   Received: aria-expanded="false"
  Also red, in FeatureDetail.test.tsx: 'gives the canvas the newest
    execution and the timeline every attempt' and 'selects detached
    evidence once for both run views'. These tests click Activity open
    and then look for its content ("Unable to find an element with the
    text: /Agent codex/").
  Reverted -> green

Final tree: `git status` shows no modified tracked files. The touched
suites (src/components/FeatureDetail/, harnessVerdict.test.ts,
HarnessGateTable.test.tsx) pass: 36 files, 298 tests.
`npm run checks:code` exits 0.
No mutation failed to turn its expected test red.
```
