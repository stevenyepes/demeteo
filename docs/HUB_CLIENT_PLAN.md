# Hub as a client of runners — implementation plan

Status: planned, 2026-11-09. Branch `feat/hub-projects-and-runner-catalog`.
Delete this file, or reduce it to a pointer, once S7 lands: its decisions then live in
[DECISIONS.md](DECISIONS.md) and [HUB.md](HUB.md).

## 1. Goal and the two decisions

**D1. The Hub is another client of runners.**
- The Hub owns its own Projects and Workflows in Postgres. They are never synced with a desktop.
- A dispatch to a **runner** is built only from a Hub Project and a Hub Workflow.
- A dispatch to a **desktop** keeps using that desktop's reported Projects and Workflows, unchanged.
- This supersedes decision 65 and removes the source-desktop runner path built in #270 and #275:
  - `DispatchBody.source_instance_id`
  - `api/runner_start.rs` reading a desktop snapshot
  - hub-web's "Run from" picker
- A Hub Project holds the repository identity and nothing else of the repository: `name`, `provider_kind` (github|gitlab), `host` and `full_name`.
- It also holds run-shape defaults: agent kind, model, effort, workflow id and max budget.
- It has no path, no URL with userinfo and no secret (AGENTS §2).
- Hub Workflows in v1 are the bundled starters (`src-tauri/workflows/*.json`), embedded at build time. They can be selected, not edited.
- The Hub does not link core. The runner validates what it ingests.

**D2. Instances report a probed harness catalog.**
- Every instance probes its registered harnesses' models on its own machine. Desktop and runner use the same code, which always targets `LOCAL_MACHINE`:
  - on a desktop, `Router` routes "local" to `LocalSubprocessAdapter`
  - on a runner, `LocalOnly` uses that adapter directly
- It caches the result and reports it as a new optional `Snapshot.harness_catalog`.
- It probes on socket connect and on a slow timer. A snapshot build never waits on a probe.
- This amends decision 70.
- hub-web's DispatchForm offers:
  - a Model `<select>` built from the **target** instance's catalog, with free text only when the target reports no catalog
  - a Default agent list limited to the target's harnesses

### Claims verified in code

- **The runner never reads `source_instance_id` or `start.project_id`.**
  - `domain/hub_request/runner_spec.rs` reads only `runner.repository.*`, `runner.workflow.{name, steps_json}` and `runner.workflow.workflow_id == start.workflow_id`.
  - The only other occurrence is a test fixture, `demeteo-runner/src/hub/runs_tests.rs`.
- **`RunnerStart` and its nested types cannot change on the wire.** They are all `deny_unknown_fields`, and `source_instance_id: String` is required (`hub-protocol/src/request.rs`).
- **Every runtime returns `SessionInfo::default()`** (`cli_runtime.rs`, `noop.rs`, `stub_runtime.rs`). The ACP step of `discover_models` therefore never answers.
- **codex's custom-provider entry carries a home path.** `discover_codex_models` sets `description: "~/.codex/config.toml (<provider>)"`, which must never cross to the Hub.
- **`noop` and `stub` leak into `snapshot.harnesses` today.** Composition passes every `agent_registry.runtimes()` kind, and the builder's `harnesses()` does not filter them.
- **Both instance kinds already run the Hub socket from composition.**
  - The desktop goes through the keyring `HubStore`.
  - The runner goes through `build_core_context_with_hub_store`.
  - `RepoSnapshotSource` is built at the `hub_socket(...)` call, before `AppContext` exists. `agent_registry`, `exec_inner` and `snapshot_wake` already exist at that point.
- **The starter list lives only in the desktop crate.**
  - `STARTER_FILES` is `pub(crate)` in `src-tauri/src/commands/workflows.rs`.
  - `StarterDefinition` is in core `domain/workflow_starters.rs`.
  - `migrate_v1_to_v2` is in core `domain/models/workflow_migrate.rs`.
  - Starter files carry `id`, `name`, `description`, `is_starter` and `steps`, with no version.
- **What a Hub build already has:**
  - `sha2` is a dependency.
  - `build.rs` has only a migrations rerun line.
  - `deploy/hub/Dockerfile` copies `src-tauri/`.
- **src-tauri tests can reach the Hub only on Linux.** `src-tauri` links `demeteo-hub` with `test-support` under `[target.'cfg(target_os = "linux")'.dev-dependencies]`.
- **Some files are close to the size limits.**
  - `hub-web/src/components/DispatchForm.tsx` is 392 LOC, so it must shed a component before it grows.
  - `api/dispatch.rs` is 618 lines, including its tests.

## 2. Out of scope (follow-ups)

- **A Hub workflow editor.** Workflows can be selected, not authored, in v1.
- **GitLab tokens for runners** (decision 71's GitLab leg). Until it exists, a GitLab Hub Project can be created but is refused at runner dispatch.
- **A Hub→instance "re-probe now" request kind.** That would be a new `RequestKind` and a protocol change.
- **Per-SSH-machine catalogs for a desktop.** The catalog describes the instance's own machine only.
- **The desktop `HarnessModelPicker` reading the cached catalog** instead of probing each time it opens.
- **`EFFORT_SUPPORT` gaining `pi`** (`src/lib/effortLevels.ts`). This matters only for the fallback path.
- **Labelling Hub-started runner runs with the Hub Project's name** in `run_labels`.
- **Checking GitHub App installation on the repository when a Hub Project is created.**
- **Removing `source_instance_id` from the wire.** This waits until every deployed runner has been replaced.

## 3. Wire and protocol

| Change | Compatibility |
|---|---|
| `Snapshot.harness_catalog: Option<Vec<HarnessCatalogEntry>>` with `#[serde(default)]` | The Hub ships first. An old instance omits the field, which reads `None`. An old Hub with `deny_unknown_fields` on `Snapshot` would refuse it, so **deploy the Hub before any instance build that sends it**, which is the existing rule. |
| `HarnessCatalogEntry { kind, models: Vec<ModelChoice>, #[serde(default)] effort_levels: Option<Vec<Effort>>, #[serde(default)] source: Option<String>, probed_at: i64 }` | Uses `deny_unknown_fields`. `probed_at` is Unix seconds, like `created_at`. |
| `ModelChoice { value, name }` | Uses `deny_unknown_fields`. It deliberately has no `description` and no `supports_images`. |
| `RunnerStart` | **Byte-for-byte unchanged, including the signing layout.** The Hub fills `source_instance_id` with the constant `HUB_ORIGIN = "hub"`. Only its rustdoc changes: the field is an unread origin label, kept because deployed runners require it. |
| `StartFeature.project_id` on a runner dispatch | Carries the Hub Project UUID. `workflow_id` carries the starter id (`wf-starter-…`). |
| `POST /v1/instances/{id}/runs` body | Loses `source_instance_id`. The body is `deny_unknown_fields`, so a stale tab gets `400`. That is acceptable because the Hub serves hub-web. |
| New Hub routes | `GET /v1/projects`, `POST /v1/projects`, `POST /v1/projects/{id}`, `POST /v1/projects/{id}/remove`, `GET /v1/workflows`. |
| `GET /v1/instances/{id}` | Gains `harness_catalog`. It is `null` without `read`, before the first snapshot, and from instances that predate the field. |

`None` and `Some([])` mean different things. `None` means not reported or not yet probed, and the UI falls back to free text. `Some([])` means the instance probed and found nothing installed.

### Decisions this plan takes

Confirm these in review.

- **On runner dispatch, the Hub fills each omitted `agent_kind`, `model`, `effort` and `max_cost_cents` from the Hub Project's defaults *before signing*.** A runner runs with `project_settings: None`, so this is the only place defaults exist. What is signed is the effective value.
- **A GitLab Hub Project is refused at runner dispatch with `409 project_not_dispatchable`**, before any spend or attachment binding. `ProjectView.runner_ready` is false for it.
- **A starter's version is content-derived.** It is the first 4 bytes of SHA-256 over the serialized `steps`, masked to 31 bits, with a non-zero floor.
- **The probe cadence uses these constants:**
  - interval 6 h
  - connect floor 15 min
  - per-kind timeout 60 s
  - 500 models per harness
  - 200 chars per value or name
- **Default agent choices fall back in order:** the catalog kinds, else `snapshot.harnesses`, else the `EFFORT_SUPPORT` keys (old instances only).

## 4. Slices

Rules for every slice:
- `npm run checks` exits 0.
- Every new test is watched red once by breaking the code it covers, then reverted (AGENTS §7).
- Comments on any code a slice changes are kept true by that slice. HUB.md and DECISIONS.md prose is consolidated in S7.
- Slices run in order. File ownership is disjoint except where marked *(extends Sx)*, which means additive edits on top of an earlier slice, never a rewrite of it.

### S1 — Protocol field and pure catalog policy

**Owned files**
- `crates/demeteo-hub-protocol/src/snapshot.rs`
- `crates/demeteo-hub-protocol/tests/snapshot.rs`
- `crates/demeteo-hub-protocol/tests/fixtures/snapshot.json` and a new `snapshot_v1_no_catalog.json` if the current fixture is the old shape
- `crates/demeteo-core/src/domain/hub_socket/harness_catalog.rs` (new)
- `crates/demeteo-core/src/domain/hub_socket/mod.rs`
- `crates/demeteo-core/src/domain/hub_socket/snapshot.rs`
- `crates/demeteo-core/src/domain/models/` (`ModelSource`)
- `crates/demeteo-core/src/adapters/hub/snapshot_source.rs`, which sets `harness_catalog: None` until S2
- Every `Snapshot { … }` literal, gaining `harness_catalog: None`:
  - hub `api/{run_labels,runner_start,port,fleet}.rs`
  - hub `socket/session.rs`
  - hub `tests/{api,socket,tenancy}.rs`
  - `src-tauri/tests/{hub_desktop_socket,hub_api,hub_snapshot}.rs`
- `src-tauri/tests/hub_snapshot.rs`

**Steps**
1. Protocol:
   - Add `ModelChoice`, `HarnessCatalogEntry` and the `Snapshot.harness_catalog` field as in §3.
   - Rewrite the `Snapshot` and `harnesses` doc comments, which today say "harness kinds and nothing about any of them".
2. `domain/hub_socket/harness_catalog.rs`, pure, with no `async fn`:
   - The constants from §3.
   - `ProbeTrigger { Connected, Timer }` and `probe_due(last_finished_ms, now_ms, trigger)`.
   - `HarnessProbeOutcome { Listed { models, source }, NotInstalled, Unreachable }`, `HarnessProbe` and `CatalogEntry`.
   - `merged(previous, fresh, effort_levels)`:
     - `Listed` replaces the entry.
     - `NotInstalled` drops it.
     - `Unreachable` keeps the previous entry.
   - `outbound_catalog(&[CatalogEntry]) -> Vec<HarnessCatalogEntry>`:
     - Keep only `AgentKind::is_supported` kinds, sorted by kind.
     - Keep probe order within a kind and dedup by value, first wins.
     - Drop empty values and anything containing NUL.
     - Run `scrub_secrets` over value and name, cut both to 200 chars, and cap each list at 500.
     - Never read `description` or `supports_images`.
   - Move `effort()` from `snapshot.rs` here, or make it `pub(super)`, so both modules share one mapper.
3. `snapshot.rs`:
   - `SnapshotInput.harness_catalog: Option<Vec<CatalogEntry>>`, mapped through `outbound_catalog`.
   - `harnesses()` filters `AgentKind::is_supported`, which fixes `noop` and `stub` leaking.

**Tests**
- Protocol:
  - round trip with a catalog
  - an old fixture reads `None`
  - an unknown key in an entry is refused
  - an unknown key in a `ModelChoice` is refused
- `hub_snapshot.rs`:
  - `probe_due` at the floor and interval boundaries, for both triggers
  - each `merged` arm
  - `outbound_catalog`: sort, dedup, caps, a scrubbed secret-shaped name, NUL dropped, `noop` excluded
  - a key walk of serialised JSON for a codex entry whose description is `~/.codex/config.toml (minimax)`: no `description` key and no `.codex` substring
  - `None` input gives `None`, `[]` gives `Some([])`
  - `harnesses` excludes `noop` and `stub`
- Fix any `hub_api` assertion that expected the raw harness list.

**Docs**
- The rustdoc above.
- No HUB.md edits yet.

### S2 — Probe on the instance: adapter, cache, session, composition

**Owned files**
- `crates/demeteo-core/src/application/agent_probe.rs`
- `crates/demeteo-core/src/ports/hub.rs`
- `crates/demeteo-core/src/adapters/agent/model_probe.rs` (new) and `adapters/agent/mod.rs`
- `crates/demeteo-core/src/adapters/hub/harness_catalog.rs` (new) and `adapters/hub/mod.rs`
- `crates/demeteo-core/src/adapters/hub/snapshot_source.rs` *(extends S1)*
- `crates/demeteo-core/src/adapters/hub/socket/{session,mod}.rs`
- `crates/demeteo-core/src/adapters/hub/testing.rs`
- `crates/demeteo-core/src/composition/mod.rs`
- `crates/demeteo-core/tests/application/agent_probe.rs`
- an adapter test file under `crates/demeteo-core/tests/`
- `src-tauri/tests/hub_desktop_socket.rs` *(extends S1)*

**Steps**
1. `agent_probe.rs`:
   - Extract `pub async fn listed_models(exec: &dyn ExecutionPort, registry: &AgentRegistry, machine_id, kind) -> (Vec<ConfigOptionValue>, ModelSource)`. It tries the codex config, then the CLI `ModelListing`, then `fallback_models`, and reports which one answered.
   - `discover_models(ctx, …)` keeps its signature and its ACP step, then delegates to `listed_models`. Leaving the ACP step in `discover_models` keeps this clear of the spawn-logic Gate.
2. `ports/hub.rs`:
   - `#[async_trait] HarnessModelProbe { availability(kinds), models(kind) }`.
   - Sync `HarnessCatalogSource { current() -> Option<Vec<CatalogEntry>>, request_probe(ProbeTrigger) }`. `request_probe` never awaits.
   - `NoHarnessCatalog` null impl.
3. `adapters/agent/model_probe.rs`:
   - `LocalModelProbe { registry, exec }`, always against `LOCAL_MACHINE`.
   - Availability through `registry.availability_of(kinds, exec, LOCAL_MACHINE, false)`.
   - Models through `listed_models`.
4. `adapters/hub/harness_catalog.rs`, `CachedHarnessCatalog`:
   - A `std::sync::Mutex` cache, an `AtomicBool` single-flight guard (CAS), and an injected `now_ms`.
   - `request_probe`:
     1. Check `probe_due`.
     2. Take the CAS guard.
     3. `runtime.spawn` the probe:
        - Check availability.
        - Wrap each installed kind's `models` in `tokio::time::timeout(60 s)`. A timeout is `Unreachable`.
     4. Store `merged`.
     5. Release the guard.
     6. Call `wake.notify_one()`.
   - Kinds and effort levels come from the registry's supported kinds, fixed at construction.
5. `snapshot_source.rs`:
   - `with_harness_catalog(Arc<dyn HarnessCatalogSource>)`, defaulting to `NoHarnessCatalog`.
   - `input()` reads `current()`.
   - Rewrite the module doc paragraph that says nothing beyond the kind is read.
6. Session:
   - `SessionContext.with_harness_catalog`. `SocketConfig.harness_catalog` is threaded through `adapters/hub/mod.rs`.
   - After the first `CursorAck`, and only when `relaying`, call `request_probe(Connected)` immediately before the first snapshot frame. Do not await it.
   - Add a `select!` arm on `interval(HARNESS_PROBE_INTERVAL_SECS)`, first tick skipped, that calls `request_probe(Timer)`.
   - Update the module doc.
7. Composition:
   - Build `LocalModelProbe(agent_registry, exec_inner)` and `CachedHarnessCatalog(snapshot_wake.clone(), runtime.clone(), …)`.
   - Pass it to `RepoSnapshotSource::with_harness_catalog` and `hub_socket(...)`, keeping both `cfg(feature = "hub-socket")` arms in step.
   - Filter `harnesses` with `AgentKind::is_supported`.
   - There is no desktop or runner branch.
8. `testing.rs`:
   - `ScriptedModelProbe` answers only scripted kinds, returns `Unreachable` and counts `unscripted()` otherwise, and takes an optional `Notify` gate.
   - `ScriptedHarnessCatalog` records triggers. An unset `current()` counts as unscripted.

**Tests**
- `listed_models`:
  - `Bundled` for claude-code
  - `Probed` for opencode, with an exec double that errors on any unscripted command
  - codex with a custom provider
- `CachedHarnessCatalog`:
  - `request_probe` returns while the probe is gated, and `current()` answers the previous value meanwhile
  - a second request in flight leaves the per-kind call count at 1
  - completion fires the wake
  - a timeout under `tokio::time::pause` keeps the old entry
  - a request inside the floor is a no-op
  - `RepoSnapshotSource` with a gated catalog returns at once
- Session (`hub_desktop_socket.rs`):
  - no trigger before the first ack
  - `Connected` exactly once after it when relaying, and none when `relaying = false`
  - advancing one interval under paused time adds `Timer`
  - the first snapshot is sent while the probe is still gated

**Notes**
- The conformance suites are optional, because no `ExecutionPort` impl changes.
- Windows is not verified. The probe reuses the `get_agent_models` path: `cat ~/…` and a login-interactive shell.
- No new `cfg` is added.

### S3 — Hub projection of the catalog, and the model select in hub-web

**Owned files**
- `crates/demeteo-hub/src/api/{snapshot_view,fleet}.rs`
- `src-tauri/tests/hub_api.rs` (catalog cases only)
- `hub-web/src/lib/{fleet,fleet.test,dispatchForm,dispatchForm.test}.ts`
- `hub-web/src/components/ModelSelect.tsx` (new)
- `hub-web/src/components/DispatchRunShapeFields.tsx` (new, extracted from DispatchForm to stay under 400 LOC)
- `hub-web/src/components/{DispatchForm,DispatchPipeline,StepOverridePanel}.tsx`
- `hub-web/src/components/dispatchParts.test.tsx`
- `src/lib/effortLevels.ts` (comment only)

**Steps**
1. Hub:
   - `HarnessCatalogView { kind, models: [{value, name}], effort_levels, source, probed_at }`, projected field by field in `pub(super) fn harness_catalog`.
   - `InstanceDetailView.harness_catalog: Option<Vec<_>>`. It is `None` without `read`, before a snapshot, or from an old instance. `Reported` and `reported_of` carry it.
   - The fleet list is unchanged.
2. `fleet.ts`:
   - The `HarnessCatalog` type, its wire type and an `isHarnessCatalog` guard.
   - `harnessCatalog: HarnessCatalog[] | null`.
   - Unknown effort strings are filtered with `isEffortLevel`, not rejected.
3. `dispatchForm.ts`:
   - `agentChoicesFor(instance)`, `modelChoicesFor(instance, kind)` and `effortsFor(instance, kind)`. `modelChoicesFor` returns `null` when the catalog has no entry for the kind, which selects free text.
   - `prefillFor` validates the default agent against `agentChoicesFor`.
   - Read only the target `instance`, never `source`.
4. `ModelSelect.tsx`:
   - an empty "Project default" option
   - one option per catalog model
   - a held value missing from the list shown as "`<value>` (not reported by this instance)"
   - disabled, with "Pick an agent first", when no agent is in effect
   - a text input when `choices === null`
   - a "Bundled list" hint in words when `source === 'bundled'`
   - `input-field` classes, no hex
5. DispatchForm:
   - Move the agent, model and effort fields into `DispatchRunShapeFields`.
   - The model field follows the effective agent: the chosen agent, else the project default.
   - Thread `instance` to `DispatchPipeline` and `StepOverridePanel`.
   - Rewrite `StepOverridePanel`'s header comment, which says the Hub knows neither harnesses nor models.

**Tests**
- Hub unit tests in `fleet.rs`, and `hub_api.rs`:
  - `GET /v1/instances/{id}` exposes `harness_catalog`
  - it is `null` without `read`
  - the projection is field by field
- vitest:
  - `fleet.test` parses a null, a valid and a malformed catalog
  - `dispatchForm.test` covers the three helpers and their fallbacks
  - `dispatchParts.test` checks that:
    - a Model `<select>` is built from the target's catalog
    - a text input appears without a catalog
    - `noop` is never offered
    - `StepOverridePanel` follows the catalog

**Docs**
- Component and module comments only.

### S4 — Hub Projects and Workflows: storage and API

**Owned files**
- `crates/demeteo-hub/migrations/0012_hub_projects.sql`
- `crates/demeteo-hub/src/store/{hub_projects,mod}.rs`
- `crates/demeteo-hub/src/projects/{mod,port,postgres,testing,verdict,handlers}.rs`
- `crates/demeteo-hub/build.rs`
- `crates/demeteo-hub/src/workflows/{mod,graph,handlers}.rs`
- `crates/demeteo-hub/src/lib.rs`
- `crates/demeteo-hub/src/routes.rs`
- `crates/demeteo-hub/src/api/snapshot_view.rs` *(extends S3)*: widen `WorkflowGraphView`, `GraphNodeView` and `GraphEdgeView` to `pub(crate)` for reuse
- `crates/demeteo-hub/tests/projects.rs` (new, Postgres, `#[ignore]` like `tests/api.rs`)
- `src-tauri/tests/hub_starters.rs` (new)
- `src-tauri/src/commands/workflows.rs` (test module only)

**Steps**
1. The migration `0012_hub_projects.sql`:
   - Columns:
     - `id` UUID primary key
     - `user_id` with `ON DELETE CASCADE`
     - `name` non-empty
     - `provider_kind` CHECK github|gitlab
     - `host` non-empty and lowercase
     - `full_name` containing `/`
     - `default_agent_kind` and `default_model`
     - `default_effort` CHECK low|medium|high|xhigh|max
     - `default_workflow_id` with **no FK**, because starters live in the binary and a dangling id reads as unset
     - `default_max_budget_cents` > 0
     - `created_at` and `updated_at`
   - `UNIQUE(user_id, id)` and `UNIQUE(user_id, host, full_name)`.
   - No path or secret column.
2. Store:
   - `HubProjectId`, `HubProjectRow` and `HubProjectFields`.
   - `insert`, `list` (ordered by name, then id), `find`, `update -> bool` and `delete -> bool`.
   - `UserId` first, `user_id = $1` in every query, runtime `query`/`query_as` only.
3. `projects/`, shaped like `git_hosts/`:
   - A `HubProjectStore` port, a Postgres impl and `MemoryProjects`.
   - `verdict.rs`, pure and synchronous:
     - `project_verdict(body)` applies the repository rules moved from `runner_start.rs::repository`: provider check, lowercase host with no `@`, whitespace, control characters or `/`, and `owner/repo` with no empty segment and no trailing slash.
     - Name 1–200 chars with no NUL.
     - `default_workflow_id` must be an embedded starter.
     - Budget > 0.
     - `parse_remote(url)` accepts `https://host/owner/repo` and returns `(host, full_name)`.
   - Handlers:
     - `GET /v1/projects` returns `200 {projects}`.
     - `POST /v1/projects` returns `201`. A duplicate is `409 project_exists`.
     - `POST /v1/projects/{id}` is a full replace and returns `200`.
     - `POST /v1/projects/{id}/remove` returns `204`.
     - Another tenant's id is `404`.
     - `ProjectView` adds a derived `remote_url` and `runner_ready` (false for GitLab).
4. Starters:
   - `build.rs`, std only, no new dependency:
     - Read `../../src-tauri/workflows/*.json` in sorted order.
     - Emit `rerun-if-changed` for that directory.
     - Write `OUT_DIR/starters.rs` with `STARTER_FILES: &[&str]` of absolute-path `include_str!`.
   - `workflows/mod.rs`:
     - `Starter { id, name, description, version, steps_json, graph }`.
     - `starters()`, parsed once in a `OnceLock`, and `starter(id)`.
     - A file that does not parse is skipped at runtime and fails a test, never a panic.
   - `graph.rs`:
     - `graph_of(steps)` mirrors `migrate_v1_to_v2`: nodes are id, type (`parallel` becomes `sequence`), title and the synthesized position.
     - Edges are the chain plus `task_list_from`.
   - `GET /v1/workflows` returns `{workflows: [{workflow_id, version, name, description, graph}]}`, ordered by name. It requires a session. It never sends `steps_json` or prompts.
5. `routes.rs`: register the five routes inside `v1()`, before `route_layer(guard_mutations)`.

**Tests**
- `verdict.rs` tables:
  - the twelve bad-remote cases ported from the old `runner_start` tests
  - host lowercasing
  - an unknown default workflow
  - `parse_remote`
- `workflows`:
  - every file parses with non-empty `steps`
  - ids are unique
  - the version is stable and changes when `steps` change
  - `graph_of` handles `parallel` and `task_list_from`
- `tests/projects.rs`:
  - CRUD
  - tenant isolation (another tenant's id is `404` on update and on remove)
  - `project_exists`
  - cascade on user delete
- `src-tauri/tests/hub_starters.rs`, Linux only like the other Hub tests. For each Hub starter:
  - (a) `runner_start_refusal` and `runner_run_spec` accept a `StartFeature` built from it
  - (b) `graph_of` equals the `migrate_v1_to_v2` nodes and edges over the same steps
- A `#[cfg(all(test, target_os = "linux"))]` case beside `STARTER_FILES` in `src-tauri/src/commands/workflows.rs`: the Hub's starter id set equals the desktop's seeded set. It sits there because `STARTER_FILES` is `pub(crate)`.

**Docs**
- Module rustdoc for `projects/` and `workflows/`, citing decision 65 as superseded.

### S5 — Runner dispatch from Hub Projects (server)

**Owned files**
- `crates/demeteo-hub/src/api/{runner_start,dispatch,port,postgres,testing}.rs`
- `crates/demeteo-hub/src/api/snapshot_view.rs` *(extends S4)*: delete `version_name` if it is dead
- `crates/demeteo-hub-protocol/src/request.rs` (rustdoc only)
- `crates/demeteo-hub/tests/api.rs`
- `src-tauri/tests/hub_api.rs` *(extends S3)*: the runner-dispatch section

**Steps**
1. `ApiStore::hub_project(user, HubProjectId) -> Option<HubProjectRow>`:
   - Postgres calls `find_hub_project`.
   - `MemoryFleet.with_hub_project` keeps a per-tenant map and still panics on unscripted calls.
2. Rewrite `runner_start.rs` as `runner_start(project: Option<&HubProjectRow>, starter: Option<&Starter>) -> Result<RunnerStart, DispatchRefusal>`:
   - It is pure.
   - It sets `source_instance_id: HUB_ORIGIN`.
   - It judges the workflow before the project.
   - Refusals:
     - `UnknownWorkflow` is `409 unknown_workflow`.
     - `UnknownProject` is `409 unknown_project`.
     - `ProjectNotDispatchable` (GitLab) is `409 project_not_dispatchable`.
   - Delete `SCRUB_MASK`, `latest()` and `repository()`.
   - Pure `run_shape_defaults(start, row)` fills each omitted `agent_kind`, `model`, `effort` and `max_cost_cents`.
3. `dispatch.rs`:
   - Remove `DispatchBody.source_instance_id`, `source_verdict`, `source_row_verdict` and `resolved_start`, and their now-unused imports.
   - Remove the source id from `body_verdict`'s id chain.
   - For a runner target:
     1. Parse `project_id` as a UUID. A non-UUID is `409 unknown_project`.
     2. Read `hub_project`.
     3. Look up `workflows::starter`.
     4. Apply the defaults.
     5. Set `start.runner`.
   - Order is unchanged: `enqueue_verdict`, then `body_verdict`, then runner resolution, then attachments. Every refusal happens before anything is signed, queued or bound.
   - The desktop path is untouched.
4. Protocol rustdoc on `RunnerStart`, `source_instance_id` and `signing_bytes`: an unread origin label, `"hub"` since D1, and older Hubs sent a desktop id.

**Tests**
- `runner_start.rs`:
  - a row plus a starter gives the expected `RunnerStart` with `source_instance_id == "hub"`
  - each refusal
  - defaults are filled only where omitted
- `dispatch.rs`, with `MemoryFleet`:
  - `source_instance_id` removed from the `every_string` and id-bound tables
  - another tenant's Hub project is `409 unknown_project` with nothing queued
  - a non-UUID is `409`
  - GitLab is `409 project_not_dispatchable` before spend
  - a valid dispatch signs the row's repository and the starter
- `tests/api.rs`: rewrite the source-desktop tenancy test (around line 382) as Hub-project tenancy plus the signed starter.
- `hub_api.rs`, around lines 1525–1700:
  - rewrite the `runner_dispatch(source)` helper and its three tests
  - a body carrying `source_instance_id` is `400`
- Leave the `RunnerStart` fixtures elsewhere unchanged; they are still valid wire:
  - core `runner_spec`
  - runner `runs_tests`
  - `hub_requests`
  - `git_token`

**Docs**
- Module and field rustdoc only.

### S6 — hub-web: Projects page and runner dispatch flow

**Owned files**
- `hub-web/src/lib/{projects,workflows,projectForm}.ts` and their `.test.ts`
- `hub-web/src/views/{Projects,Projects.test}.tsx`
- `hub-web/src/components/{ProjectRow,ProjectForm}.tsx`
- `hub-web/src/lib/{router,router.test}.ts`
- `hub-web/src/components/HubNav.tsx`
- `hub-web/src/App.tsx` and `App.test.tsx`
- `hub-web/src/views/{Dispatch,Dispatch.test,views.test}.tsx`
- `hub-web/src/lib/{dispatchForm,dispatchForm.test,dispatch}.ts` *(extends S3)*
- `hub-web/src/components/{DispatchForm,dispatchParts.test}.tsx` *(extends S3)*

**Steps**
1. Typed wrappers following the `gitHosts.ts` pattern, using `postInSession`, with wire↔camel guards:
   - `listProjects`, `createProject`, `updateProject`, `removeProject`
   - `listWorkflows`
2. `projectForm.ts`:
   - remote paste into `{host, fullName}`, mirroring `parse_remote`
   - budget through `parseMaxCostCents` / `usdOfCents`
   - effort through `isEffortLevel`
3. The Projects page:
   - list, create, edit and remove
   - the default workflow is a `<select>` from `/v1/workflows`
   - the default agent is a `<select>` over the `EFFORT_SUPPORT` kinds
   - a GitLab row reads "Runners cannot clone GitLab projects yet" in words on a neutral tone, not amber
   - each file stays at or under 400 LOC
4. Routing: `#/projects`, a `HubNav` section using the lucide `FolderGit2` icon (lucide is already a dependency), and the route in `App.tsx`.
5. `dispatchForm.ts`:
   - Remove `sourceInstanceId`, `dispatchSources`, `runnerProjectProblem` and `isRunnerRemote`.
   - Add `Catalog`, `catalogOf(detail)` and `hubCatalog(projects, workflows)`.
   - `prefillFor(catalog, instance)`, where `instance` drives S3's helpers.
   - `buildDispatchRequest` drops `source_instance_id`.
6. `DispatchForm`:
   - The `source?` prop becomes `catalog: Catalog`.
   - Delete `problemOf`. A GitLab project is greyed through `runner_ready`.
7. `Dispatch.tsx`:
   - `RunnerTarget` loads projects and workflows through `settle`.
   - With none, an empty state links to `#/projects`.
   - Otherwise it renders `DispatchForm catalog={hubCatalog(...)}`.
   - Delete "Run from" and `RunnerSource`.
   - The desktop path is unchanged.

**Tests**
- wrapper and guard tests
- `projectForm` parsing
- `router` round trip
- the Projects view: create, edit, remove, and the GitLab wording
- `Dispatch.test`, around line 857:
  - a runner dispatch from a Hub project sends no `source_instance_id`
  - the empty state appears when there are no projects
- `dispatchForm.test`: replace the source and runner-problem cases with `hubCatalog` and prefill cases
- update the `DispatchForm` render sites in `dispatchParts.test` and `views.test`

**Docs**
- Component comments only.

### S7 — Docs

**Owned files**
- `docs/DECISIONS.md`
- `docs/HUB.md`
- `docs/ARCHITECTURE.md` (only if it describes the Hub snapshot or dispatch flow)
- `AGENTS.md` (§8 Hub row only)
- this file

**Steps**
1. `DECISIONS.md`:
   - **Decision 65.**
     - Rewrite the row to D1.
     - Rejected alternatives:
       - syncing with a desktop's copy
       - dispatch through a source desktop
       - Hub-authored Workflows in v1
     - Add a superseded entry with `| **Was** |` holding the old row verbatim, `| **Now** |` and `| **Changed** | 2026-11-09 |`, then *Why it changed*:
       - a runner run should not depend on a desktop being paired with `read` and holding the project
       - the source-desktop path signed whatever a desktop's scrubber left
   - **Decision 64.** Amend the *Hub-started runner runs* paragraph of its detail.
   - **New decision, the next number: "Instance-probed harness catalog".**
     - It amends 70. Add an "amended by" note on row 70 rather than rewriting it.
     - Rejected alternatives:
       - Hub-side or desktop-side probing of a runner (it reads the wrong machine and needs that machine's credentials)
       - free text only
     - Accepted: the configured codex model slug, and through it the provider name, now crosses. The config file and the `description` never do.
2. `HUB.md`:
   - §2: the status rows for the protocol, the Hub, hub-web (Projects page; dispatch to a runner names a Hub project), and Runner dispatch from the Hub.
   - §3, what the Hub holds:
     - Hub Projects (repository identity and run shape only)
     - model lists added to the "harness kinds" row
   - §3, never leaves an instance: codex's `config.toml` is read locally and only the model slug crosses.
   - §5 *Snapshots*: a "Harness catalog" paragraph:
     - the probe runs on the instance, on connect and on a timer
     - the cache never blocks a build, and a completed probe wakes a rebuild
     - caps and scrub
     - `description` never crosses
     - `None` vs `[]`
     - point at `domain/hub_socket/harness_catalog.rs` and `adapters/hub/harness_catalog.rs`
   - §6.1: `source_instance_id` is now `"hub"` and unread.
   - §6.4:
     - routes: the projects and workflows routes, and `harness_catalog` on the instance read
     - "A dispatch to a runner names a Hub project": the defaults fill, the content-derived starter version, the graph derivation pinned by the parity test, and that removing a Hub Project does not stop a queued or running run
     - error table: drop the `source_instance_id` cases and add `project_exists` and `project_not_dispatchable`
   - §9: the `hub_projects` table, and `projects/` and `workflows/` in the code layout.
   - §10 and §11:
     - the source-desktop paragraph
     - the probe constants as carried defaults
3. `AGENTS.md` §8: the Hub row mentions the Hub-owned Projects page and the embedded starter Workflows.
4. Run `scripts/check-doc-refs.sh`, which `npm run checks` includes. Then delete this plan or reduce it to a pointer.

## 5. Human approval

- **Dependencies: none.**
  - `sha2` is already a Hub dependency.
  - `build.rs` is std-only.
  - `lucide-react` is already in hub-web.
  - If any slice finds it needs a new crate or npm package, it **stops and asks** (AGENTS §6).
- **Tauri capabilities: none touched.**
- **Core migrations: none.** The one new Hub migration is a new file and drops or renames nothing.
- **Agent spawn logic, for awareness.** S2 extracts model listing from `discover_models`. Its ACP step and every agent spawn path are left unchanged, so this is not a Gate item. If review disagrees, S2 pauses for approval.
- **New runtime behaviour, for awareness.** A desktop that relays with `read` now runs `opencode models`, `pi …` and `cat ~/.codex/config.toml` in the background, at most every 6 h, without the user opening a picker.

## 6. Risks

- **`RunnerStart` must stay byte-identical.** Any field change breaks every deployed runner. A unit test in S5 pins the serialized key set.
- **The Hub has its own copy of the v1→v2 graph** (`graph_of`). It is only trustworthy while `hub_starters.rs` (Linux) pins it to core.
- **The Hub build reads `src-tauri/workflows/`.** A build of `demeteo-hub` without that directory fails. The Dockerfile already copies it.
- **A Hub newer than a runner** can sign a starter whose `StepConfig` the runner cannot parse. The runner refuses it as `MalformedBody`.
- **The desktop catalog describes the desktop's machine,** not an SSH compute host. A model it lists can fail at run time there.
- **The snapshot is resent at least once per probe interval,** because `probed_at` changes on every completed probe.
- **Many `Snapshot` literals change in S1.** S5 deletes `runner_start.rs`'s literal, so S1 and S5 must land in order.
