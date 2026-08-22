import type { ReactElement } from 'react';

import {
  agentLabel,
  personalizationFor,
  type AgentCatalogEntry,
  type PersonalizationSupport,
} from '../../lib/agentCatalog';

/**
 * What the chosen harness brings to a review, and what Demeteo's own flags do
 * to it — said before the run starts rather than discovered in a report that
 * ignored the skill the user wrote for exactly this.
 *
 * The copy names what Demeteo does, never what the harness's review capability
 * is called: that vocabulary belongs to another product, changes on their
 * release schedule, and nothing here would fail when it does.
 *
 * `suppressed` is the only state with weight, because it is the only one where
 * the user loses something they already had. Amber and not ruby: nothing has
 * failed and the run is fully launchable, and a red block over a launchable run
 * teaches the user to read past red.
 */
export interface HarnessPersonalizationNoteProps {
  agents: AgentCatalogEntry[];
  /** The harness the run will use, or the empty string when none is chosen. */
  kind: string;
  /** Whether the step about to run keeps the harness's own personalization —
   *  which moves the answer for a harness Demeteo would otherwise strip. */
  stepKeepsPersonalization: boolean;
}

const NOTE: Record<PersonalizationSupport, (label: string) => string> = {
  loaded: (label) =>
    `Demeteo starts ${label} with your own setup loaded — skills, commands, project ` +
    `settings — so anything you have taught it about reviewing applies on top of your ` +
    `conventions.`,
  native: (label) =>
    `${label} starts with whatever it normally loads on this machine — Demeteo passes it ` +
    `no personalization flags either way.`,
  // Names the project's harness setting rather than "pick another harness":
  // this surface has no picker, and copy that asks for an action the screen
  // cannot perform reads as a broken control the user failed to find.
  suppressed: (label) =>
    `Demeteo starts ${label} with its own skills and prompt templates switched off, so ` +
    `this review runs on your conventions alone. Change the project's default harness in ` +
    `Settings if you want the agent's own review method too.`,
};

const TONE: Record<PersonalizationSupport, string> = {
  loaded: 'text-slate-500',
  native: 'text-slate-500',
  suppressed:
    'rounded-lg border border-amber-500/20 bg-amber-500/5 px-3 py-2 text-amber-200/90',
};

export function HarnessPersonalizationNote({
  agents,
  kind,
  stepKeepsPersonalization,
}: HarnessPersonalizationNoteProps): ReactElement | null {
  const support = personalizationFor(agents, kind, stepKeepsPersonalization);
  if (support === null) return null;

  return (
    <p
      data-testid="harness-personalization"
      data-support={support}
      className={`text-[11px] leading-relaxed ${TONE[support]}`}
    >
      {NOTE[support](agentLabel(agents, kind))}
    </p>
  );
}

export default HarnessPersonalizationNote;
