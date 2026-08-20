/**
 * What refuses a turn's access to paths outside the worktree it was given, one
 * answer per class of access. Mirrors the Rust `PathContainment`
 * (`domain/models/sandbox.rs`), kebab-serialized.
 *
 * It arrives on the *per-machine* agent rows (`get_agent_configs`) and not on
 * the session-wide catalog, because part of it is a kernel claim and a kernel
 * claim is not the same claim on every host. Anything reading a containment
 * answer therefore has a machine in hand by construction.
 */
export interface PathContainment {
  reads: Enforcement;
  writes: Enforcement;
  shell: Enforcement;
}

/** Mirrors the Rust `Enforcement` wire form. */
export type Enforcement = 'os' | 'harness' | 'harness-partial' | 'none';

const KNOWN: readonly Enforcement[] = ['os', 'harness', 'harness-partial', 'none'];

/**
 * What confines a turn on `kind` to the worktree it is handed, on the machine
 * `rows` were read for.
 *
 * `null` is "nobody has said" — no kind chosen, the rows still loading, a kind
 * the machine has no row for, or a spelling this frontend does not know — and
 * is deliberately not a fence-free answer either. Every value here is a claim
 * about what is protecting the user's other repositories, and the honest answer
 * to an unread machine is silence.
 *
 * An unknown spelling is rejected for the reason `lib/agentCatalog.ts` records
 * over `KNOWN_SUPPORT`, which this wire contract shares.
 */
export function pathContainmentFor(
  rows: readonly { kind: string; path_containment?: PathContainment }[],
  kind: string | null | undefined,
): PathContainment | null {
  if (!kind) return null;
  const declared = rows.find((a) => a.kind === kind)?.path_containment;
  if (!declared) return null;
  const dimensions = [declared.reads, declared.writes, declared.shell];
  if (dimensions.some((d) => !KNOWN.includes(d))) return null;
  return declared;
}
