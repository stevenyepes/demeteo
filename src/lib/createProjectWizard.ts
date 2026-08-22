import { invoke } from "@tauri-apps/api/core";
import { asAppError } from "./errors";
import type { AppError, EffortLevel, Feature, FeatureOrigin, WorktreeStrategy } from "../types";

// ── Wire types ──────────────────────────────────────────────────────────
//
// These mirror the Rust structs serialised by the `provider_create_repo`
// and `fetch_provider_groups` commands in
// `src-tauri/src/ports/provider_http.rs` (the Rust names are
// `NamespaceSummary` and `CreatedRepo`). We expose them from this module
// so the wizard component can import them alongside the wrappers without
// reaching into a private file.

/** Namespace a freshly-created repo can be parented under. Mirrors the
 *  Rust `NamespaceSummary` struct. */
export interface ProviderNamespace {
  id: string;
  name: string;
  kind: "personal" | "org" | "group";
}

/** Result of creating a repository on a provider. Mirrors the Rust
 *  `CreatedRepo` struct. `full_name` is the GitHub `full_name` (e.g.
 *  `"org/slug"`) or the GitLab `path_with_namespace`. */
export interface CreatedRepo {
  full_name: string;
  default_branch: string;
  clone_url: string;
}

/** Request shape for {@link providerCreateRepo}. Only the `providerId`
 *  is forwarded; the PAT is resolved backend-side via the credential
 *  cache, so the wizard never sees (or sends) a raw token.
 *
 *  `providerHost` is the host captured from the selected provider on
 *  the Provider step. It MUST be forwarded to the backend (as
 *  `provider_host`) so sub-1's HTTP adapter can route the create-repo
 *  request to a self-hosted enterprise host instead of the default
 *  `api.github.com` / `/api/v4` endpoint. When the host is omitted the
 *  backend falls back to the provider's configured default — see
 *  `src-tauri/src/adapters/provider_http.rs` (`sanitize_host`). */
export interface CreateRepoRequest {
  providerId: string;
  namespaceId: string;
  name: string;
  private: boolean;
  /** Self-hosted provider host (e.g. `https://gh.corp.example.com`).
   *  Plumbed from the Provider step; passed to the backend verbatim. */
  providerHost?: string;
}

// ── Input shape that powers the wizard's submit ─────────────────────────

/** What the wizard submits once the user has walked the
 *  Name → Provider → Group → Machine → Agent → Model → Create repo → Description
 *  flow. The wizard hands this exact object to {@link startFeature} after
 *  it has created + bootstrapped the underlying project, so all the
 *  dependent IDs (`projectId`, `createdRepo`) live here alongside the
 *  free-form fields the user typed in. */
export interface CreateZeroWizardInput {
  /** Provider config the user picked on the create-repo step. */
  createRepo: CreateRepoRequest;
  /** Display name for the new project (slug-derived, but kept human-readable). */
  projectName: string;
  /** Identifier returned from `createProject` after the bootstrap
   *  candidate project row has been inserted. */
  projectId: string;
  /** Repository that was created on the provider. Carried through so the
   *  wizard can navigate to its clone URL on success. */
  createdRepo: CreatedRepo;
  /** `local` for a workstation machine, `remote` for an SSH-attached host. */
  machineKind: "local" | "remote";
  /** Selected machine id (`localMachineId` or remote `machineId`). May be
   *  null for the bootstrapped local backend default — see wizard code. */
  machineId: string | null;
  /** A registered agent kind (`opencode | hermes | claude-code`). */
  agentKind: string;
  /** Either a value returned by `getAgentModels` or a free-form override. */
  model: string;
  /** Free-text user description — becomes the Feature's `description`. */
  description: string;
}

// ── Provider-namespace listing ──────────────────────────────────────────

/** List every namespace (`personal`, `org`, `group`) the authenticated
 *  user can create a repo under, for a connected provider. The backend
 *  resolves the PAT via `credential_cache::get_or_fetch`, so we never
 *  send a raw token from the wizard.
 *
 *  See `commands::providers::fetch_provider_groups` in
 *  `src-tauri/src/commands/providers.rs`. */
export async function listProviderNamespaces(
  providerId: string,
): Promise<ProviderNamespace[]> {
  return invoke<ProviderNamespace[]>("fetch_provider_groups", { providerId });
}

// ── Repo creation on a provider ─────────────────────────────────────────

/** Create a new repository on a connected provider. Surfaces provider
 *  401/403/422 verbatim as `AppError` with `kind` in `{ provider,
 *  transport, validation }` so the wizard can show an inline error and
 *  let the user edit the slug / namespace and retry without losing
 *  prior selections.
 *
 *  When `request.providerHost` is non-empty, it is forwarded to the
 *  backend as `provider_host` (snake_case) so the HTTP adapter can
 *  route to a self-hosted enterprise host. When omitted, the backend
 *  falls back to the provider's configured default.
 *
 *  See `commands::providers::provider_create_repo` in
 *  `src-tauri/src/commands/providers.rs`. */
export async function providerCreateRepo(
  request: CreateRepoRequest,
): Promise<CreatedRepo> {
  return invoke<CreatedRepo>("provider_create_repo", {
    providerId: request.providerId,
    namespaceId: request.namespaceId,
    name: request.name,
    private: request.private,
    providerHost: request.providerHost ?? null,
  });
}

// ── Project record creation ─────────────────────────────────────────────

/** Request body for {@link createProject}. Mirrors the Rust
 *  `application::projects::ProjectConfig`. */
export interface CreateProjectConfig {
  name: string;
  /** `local` or `remote`. */
  compute_type: "local" | "remote";
  remote_host: string | null;
  repos: Array<{
    repo_path: string;
    provider_id: string;
  }>;
}

/** Response body returned from {@link createProject}. Mirrors the Rust
 *  `ProjectCreateResponse`. */
export interface ProjectCreateResponse {
  id: string;
  success: boolean;
}

/** Insert the (status=`bootstrapping`) project row + repository rows
 *  for a freshly-created remote repo. Note: the Rust command generates
 *  its own id (`format!("p{}", now_ms)`); the wizard doesn't seed it. */
export async function createProject(
  config: CreateProjectConfig,
): Promise<ProjectCreateResponse> {
  return invoke<ProjectCreateResponse>("create_project", { config });
}

// ── Bootstrap (clone + worktree strategy detection) ─────────────────────

/** Clone the project's repositories and infer a worktree strategy.
 *  Tolerates a freshly-created repo whose only content is the auto-init
 *  commit (default branch + README pre-exist). */
export async function bootstrapProject(
  projectId: string,
): Promise<WorktreeStrategy> {
  return invoke<WorktreeStrategy>("bootstrap_project", { projectId });
}

// ── Feature launch ──────────────────────────────────────────────────────

/** Input for {@link startFeature}. Mirrors the Tauri camel-cased
 *  command arguments and the seeded `wf-starter-standard` workflow. The
 *  wizard supplies all of these from prior steps. */
export interface StartFeatureInput {
  projectId: string;
  /** Stable id of a seeded or user-defined workflow. The wizard uses
   *  `"wf-starter-standard"`. */
  workflowId: string;
  title: string;
  description: string;
  agentKind: string | null;
  model: string | null;
  /** Feature-wide effort. `null` = inherit the project default. */
  effort?: EffortLevel | null;
  commitArtifacts?: boolean | null;
  loopIterations?: number | null;
  /** Per-run dollar budget. `null` inherits the project default, then the
   *  engine default — the wizard never sets one, the launch composer can. */
  maxBudgetUsd?: number | null;
  /** Per-step overrides. The wizard does not currently produce any. */
  stepOverrides?: unknown[] | null;
  /** Pre-launch attachments. The wizard starts a new feature with no
   *  attachments, so this is always `[]` (sent as `null` to match the
   *  backend's `Option<Vec<_>>`). */
  stagedAttachments?: unknown[] | null;
  /** Where the run's branch is cut from, and what its diff is measured
   *  against (migration V41). Both are **omitted** from the invoke payload
   *  when unset rather than sent as `null`, so a launch that names neither is
   *  the payload that shipped before the origin picker existed. */
  origin?: FeatureOrigin;
  diffBaseBranch?: string;
}

/** Launch a feature against the chosen project + workflow. Wraps
 *  `commands::features::start_feature`. */
export async function startFeature(input: StartFeatureInput): Promise<Feature> {
  return invoke<Feature>("start_feature", {
    projectId: input.projectId,
    workflowId: input.workflowId,
    title: input.title,
    description: input.description,
    agentKind: input.agentKind,
    model: input.model,
    effort: input.effort ?? null,
    commitArtifacts: input.commitArtifacts ?? null,
    loopIterations: input.loopIterations ?? null,
    maxBudgetUsd: input.maxBudgetUsd ?? null,
    stepOverrides: input.stepOverrides ?? null,
    stagedAttachments: input.stagedAttachments ?? null,
    ...(input.origin ? { origin: input.origin } : {}),
    ...(input.diffBaseBranch ? { diffBaseBranch: input.diffBaseBranch } : {}),
  });
}

// ── Coercion helper ────────────────────────────────────────────────────

/** Type-safe accessor: returns the rejection coerced to `AppError | null`
 *  or `null` if the rejection wasn't one. Mirrors the pattern used by
 *  {@link isBlockingError} in `./features.ts`. */
export function wizardError(err: unknown): AppError | null {
  return asAppError(err);
}
