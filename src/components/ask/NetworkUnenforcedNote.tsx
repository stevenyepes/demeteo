import React from 'react';

/**
 * The scope the §6 sign-off (`f01e1b1a`) named, said out loud where the
 * control is. Demeteo compiles `network: Deny` for every harness, but it
 * reaches hermes through `opencode_permission_env` — a namespace nothing in
 * this tree shows hermes reads — so for this one agent the profile is a
 * request, not an established fence. The toggle stays live either way:
 * disabling it would claim a certainty in the other direction.
 *
 * Both dialogs that carry the web-access toggle render this — the settings
 * panel, and the creation modal where the posture is first chosen and its
 * first turn is bound by it. One component rather than two copies, per
 * AGENTS.md §7.
 */
export function NetworkUnenforcedNote(): React.ReactElement {
  return (
    <p
      data-testid="ask-network-unenforced-note"
      className="mt-2 rounded-lg border border-amber-500/20 bg-amber-500/5 px-3 py-2.5 text-[11px] leading-relaxed text-amber-200/90"
    >
      Demeteo hands hermes the same permission environment it hands opencode, and nothing in this
      tree shows hermes reads it. The setting is recorded and passed on, but Demeteo cannot
      establish that this agent enforces it either way.
    </p>
  );
}

export default NetworkUnenforcedNote;
