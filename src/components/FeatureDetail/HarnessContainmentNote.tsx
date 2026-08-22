import type { ReactElement } from 'react';

import { agentLabel, type AgentCatalogEntry } from '../../lib/agentCatalog';
import {
  pathContainmentFor,
  type Enforcement,
  type PathContainment,
} from '../../lib/pathContainment';

/**
 * What the harness about to run is actually held to, one line per class of
 * access, where the harness is chosen.
 *
 * Three lines and not a verdict because the answers disagree per class: the
 * turn under this note spawns with an all-allow profile, so the worktree is
 * the only thing between the agent and the rest of the disk, and every harness
 * measured refuses one class of access there while serving another. Codex is
 * the case that decides the shape — its sandbox refuses writes and reads the
 * whole filesystem — and a single sentence for it either credits a kernel that
 * is not stopping the read the user cares about, or denies a fence that really
 * does stop the write.
 *
 * Weight follows exposure rather than status: a line nothing refuses is the
 * news, a line something refuses is furniture, and it is normal for one note to
 * carry both. Amber and not ruby throughout, for the reason the personalization
 * note is amber — nothing has failed, the action is fully launchable, and a red
 * block over a launchable run teaches the user to read past red.
 */
export interface HarnessContainmentNoteProps {
  /** The session-wide catalog, for the harness's display name. */
  agents: AgentCatalogEntry[];
  /** What the machine this turn will run on answered for each harness — the
   *  only rows the containment claim may be read from, for the reason
   *  `lib/pathContainment.ts` records. */
  machineAgents: Array<{ kind: string; path_containment?: PathContainment }>;
  /** The harness the turn will use, or the empty string when none is chosen. */
  kind: string;
}

/**
 * The user's own terms for the three dimensions. The first two answer for the
 * agent's own file access and the third for what a command it runs can reach,
 * which is a separate answer rather than a detail of the other two: opencode's
 * check covers reading and writing through its tools and loses a command that
 * does either through a shell. A user stops at the line named for the access
 * they care about, so a `harness` claim names the file tools it actually
 * covers — leaving that qualifier a line below, on `shell`, makes the first
 * two lines false to anyone who reads only one of them.
 */
const DIMENSIONS: ReadonlyArray<{
  key: keyof PathContainment;
  title: string;
  copy: Record<Enforcement, (label: string) => string>;
}> = [
  {
    key: 'reads',
    title: 'Reading files',
    copy: {
      os: () => 'the kernel refuses a read outside this worktree.',
      harness: (label) => `${label}'s own file tools refuse to open a file outside this worktree.`,
      'harness-partial': (label) =>
        `${label} refuses some reads outside this worktree and not others.`,
      none: (label) =>
        `nothing stops ${label} reading any file your account can — your other ` +
        `repositories and their secrets included.`,
    },
  },
  {
    key: 'writes',
    title: 'Changing files',
    copy: {
      // Never a closed list of roots. `build_codex_args` pins `sandbox_mode`,
      // `approval_policy` and `network_access` and never
      // `sandbox_workspace_write.writable_roots`, which codex reads from the
      // user's own `~/.codex/config.toml` — a file AGENTS.md §2 forbids
      // Demeteo touching or reading. So a user who added `/home/me/Projects`
      // there is kernel-permitted across every repo under it, and copy that
      // enumerates the roots as worktree+temp overstates the fence in exactly
      // the way this component exists to avoid.
      os: (label) =>
        `the kernel refuses a write outside the sandbox ${label} is configured with — ` +
        `this worktree and the machine's temporary directories, plus whatever ${label}'s ` +
        'own config adds.',
      harness: (label) => `${label}'s own file tools refuse to write outside this worktree.`,
      'harness-partial': (label) =>
        `${label} refuses some writes outside this worktree and not others.`,
      none: (label) => `nothing stops ${label} writing anywhere your account can.`,
    },
  },
  {
    key: 'shell',
    title: 'Commands it runs',
    copy: {
      os: () => 'a command runs inside that same sandbox — the kernel refuses, not the agent.',
      harness: (label) => `${label} holds a command it runs to the same rule as its file tools.`,
      'harness-partial': (label) =>
        `the rule covers ${label}'s file tools and only part of what it runs through a ` +
        `shell — a command outside that part is not checked against anything.`,
      none: () => 'a command is not checked against anything before it runs.',
    },
  },
];

/** Whether a dimension is the news on this note or the furniture. */
function exposed(enforcement: Enforcement): boolean {
  return enforcement === 'none' || enforcement === 'harness-partial';
}

export function HarnessContainmentNote({
  agents,
  machineAgents,
  kind,
}: HarnessContainmentNoteProps): ReactElement | null {
  const containment = pathContainmentFor(machineAgents, kind);
  if (containment === null) return null;

  const label = agentLabel(agents, kind);
  const anyExposed = DIMENSIONS.some((d) => exposed(containment[d.key]));

  return (
    <div
      data-testid="harness-containment"
      className={
        anyExposed
          ? 'rounded-lg border border-amber-500/20 bg-amber-500/5 px-3 py-2 text-[11px] leading-relaxed'
          : 'text-[11px] leading-relaxed'
      }
    >
      <p className="text-slate-500">
        This worktree is where the turn starts. What holds {label} to it:
      </p>
      <ul className="mt-1 space-y-0.5">
        {DIMENSIONS.map((dimension) => {
          const enforcement = containment[dimension.key];
          return (
            <li
              key={dimension.key}
              data-dimension={dimension.key}
              data-enforcement={enforcement}
              className={exposed(enforcement) ? 'text-amber-200/90' : 'text-slate-500'}
            >
              <span className="font-medium">{dimension.title}</span> —{' '}
              {dimension.copy[enforcement](label)}
            </li>
          );
        })}
      </ul>
      {anyExposed && (
        // The action it names is the control directly above it on this screen,
        // which is why this note may ask for one where the review surface's
        // cannot.
        <p className="mt-1.5 text-amber-200/90">Choose another harness above if that matters here.</p>
      )}
    </div>
  );
}

export default HarnessContainmentNote;
