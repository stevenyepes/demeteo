/**
 * What the project header may truthfully say about where a project's code
 * comes from and where its agents run.
 *
 * It exists to close audit finding F10 (`docs/ux-audit/findings.md`), where the
 * header rendered "Connected via GitHub Enterprise • Default Workflow: Standard
 * Feature Pipeline" for every project, both halves invented. The rule is a
 * module rather than a template string because the interesting cases are the
 * degraded ones, and each needed a decision:
 *
 *  1. **The default-workflow clause reports a stored choice or says nothing.**
 *     It once named the same workflow for every project while no such setting
 *     existed anywhere; `project_settings.default_workflow_id` (migration V40)
 *     is that setting, and a project that has not chosen one omits the clause
 *     rather than naming whatever the workflow list happens to start with.
 *
 *     An id that resolves to no workflow omits it too. A workflow can be
 *     deleted while a project names it — the column deliberately carries no
 *     foreign key, because the remote runner re-ingests a workflow under a
 *     fresh id and persists the launching client's settings verbatim, so on the
 *     runner's own database the id names nothing and a constraint would abort
 *     the run. Unset and dangling are therefore the same state to a reader,
 *     which is what makes omission the honest rendering of both.
 *
 *  2. **A host is reported, an edition is never inferred.** `github` + a host
 *     that is not `github.com` most likely *is* GitHub Enterprise Server, but
 *     "most likely" is what F10 was made of. The stored host is a fact and says
 *     the same thing to anyone who would recognise the label.
 *
 *  3. **An unresolvable `provider_id` removes the clause, it does not fill it.**
 *     Deleting a provider instance does not touch the repositories that name it
 *     (`delete_provider_instance`), so a dangling id is reachable in normal use.
 *     `unresolvedRepositories` reports it so a caller can surface the breakage
 *     deliberately; silence in the label is not the same as hiding it.
 *
 * Incompleteness is permitted where invention is not: with one repository
 * resolved and one dangling, the resolved provider is still named. Dropping a
 * known-true fact because a neighbour is broken helps nobody, and the count
 * carries the rest.
 */

/** The subset of a connected provider this module reads. `Provider` from
 *  `types.ts` satisfies it as-is; `ProviderInstance` from `lib/providers`
 *  spells the same lowercase discriminator `kind` rather than `type`. */
export interface ProvenanceProvider {
  id: string;
  type: string;
  host: string;
}

/** Satisfied by `Repository` from `types.ts`. */
export interface ProvenanceRepository {
  provider_id: string;
}

/** The subset of a workflow this module reads. `ProjectHome`'s `workflowById`
 *  values satisfy it. */
export interface ProvenanceWorkflow {
  name: string;
}

export interface ProjectProvenanceInput {
  repositories: ReadonlyArray<ProvenanceRepository>;
  /** Every provider instance the app knows about — `ProjectContext`'s
   *  `providers`, which `App` already loads on startup. */
  providers: ReadonlyArray<ProvenanceProvider>;
  /** `ProjectSettings.default_workflow_id` (migration V40). */
  defaultWorkflowId?: string | null;
  /** Workflow id → display meta. Absent means the list has not answered yet,
   *  which is not the same as an id that resolves to nothing: an unanswered
   *  list would degrade every valid id, so the clause waits instead. */
  workflowsById?: ReadonlyMap<string, ProvenanceWorkflow>;
  /** `Project.compute_type`. Absent reads as `'local'`: the column is
   *  `NOT NULL DEFAULT 'local'`, so only the optional TS field can be missing. */
  computeType?: string | null;
  remoteHost?: string | null;
}

export interface ProjectProvenance {
  /** Rendered left to right. Never empty, and never a placeholder for something
   *  that was not knowable — a clause that cannot be filled is simply absent. */
  segments: string[];
  /** `segments` joined with {@link PROVENANCE_SEPARATOR}. */
  text: string;
  /** Repositories whose `provider_id` names no connected provider. Non-zero
   *  means the label under-reports; it never means the label is wrong. */
  unresolvedRepositories: number;
}

export const PROVENANCE_SEPARATOR = ' • ';

const BRAND: Record<string, string> = {
  github: 'GitHub',
  gitlab: 'GitLab',
};

function providerLabel(provider: ProvenanceProvider): string {
  const brand = BRAND[provider.type.trim().toLowerCase()] ?? provider.type.trim();
  const host = provider.host.trim();
  return host ? `${brand} (${host})` : brand;
}

function computeSegment(computeType: string | null | undefined, remoteHost: string | null | undefined): string {
  if ((computeType ?? 'local').trim().toLowerCase() !== 'remote') return 'Runs locally';
  const host = (remoteHost ?? '').trim();
  return host ? `Runs on ${host}` : 'Runs remotely';
}

export function describeProjectProvenance(input: ProjectProvenanceInput): ProjectProvenance {
  const byId = new Map(input.providers.map((p) => [p.id, p] as const));

  const labels: string[] = [];
  let unresolvedRepositories = 0;
  for (const repo of input.repositories) {
    const provider = byId.get(repo.provider_id);
    if (!provider) {
      unresolvedRepositories += 1;
      continue;
    }
    const label = providerLabel(provider);
    if (!labels.includes(label)) labels.push(label);
  }

  const segments: string[] = [];
  if (input.repositories.length === 0) {
    segments.push('No repository connected');
  } else if (labels.length > 0) {
    segments.push(`Connected via ${labels.join(', ')}`);
  }
  segments.push(computeSegment(input.computeType, input.remoteHost));

  const workflow = input.defaultWorkflowId
    ? input.workflowsById?.get(input.defaultWorkflowId)
    : undefined;
  if (workflow) segments.push(`Default workflow: ${workflow.name}`);

  return { segments, text: segments.join(PROVENANCE_SEPARATOR), unresolvedRepositories };
}
