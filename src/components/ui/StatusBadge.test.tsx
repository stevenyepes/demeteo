// Runtime smoke tests for StatusBadge's `LivenessDot`.
//
// The workspace liveness dot (`p.liveness`, populated by
// `check_workspace_liveness`) is a separate vocabulary from the
// existing workflow/pipeline status dot (`StatusBadge` on `p.status`)
// rendered right next to it in ProjectRail. This file verifies the two
// never share a tone map and that each liveness state renders the
// right treatment:
//
//   - 'unknown' / absent  -> nothing rendered (no placeholder dot)
//   - 'checking'          -> renders with a pulse class
//   - 'online'            -> solid emerald tone, no pulse class
//   - 'offline'           -> solid ruby/muted tone, no pulse class
//
// Mirrors the runtime-throws-on-failure pattern used across this repo's
// other `*.test.tsx` smoke tests (see `ProjectRail.test.tsx`).

import { act, create, type ReactTestRenderer } from 'react-test-renderer';
import type { ReactElement } from 'react';

import { LivenessDot } from './StatusBadge';

function mount(element: ReactElement): ReactTestRenderer {
  let renderer: ReactTestRenderer | null = null;
  act(() => { renderer = create(element); });
  if (!renderer) throw new Error('LivenessDot renderer did not initialise');
  return renderer;
}

function findDot(renderer: ReactTestRenderer) {
  return renderer.root.findAll(
    (node) => typeof node.type === 'string' && (node.type as string) === 'div',
  );
}

// ── 'unknown' / absent renders nothing ─────────────────────────────────

const unknownTree = mount(<LivenessDot liveness="unknown" />);
if (findDot(unknownTree).length !== 0) {
  throw new Error("LivenessDot: liveness='unknown' must render no dot element");
}
unknownTree.unmount();

const absentTree = mount(<LivenessDot />);
if (findDot(absentTree).length !== 0) {
  throw new Error('LivenessDot: omitted liveness prop must render no dot element (defaults to unknown)');
}
absentTree.unmount();

// ── 'checking' renders a pulsing dot ───────────────────────────────────

const checkingTree = mount(<LivenessDot liveness="checking" />);
const checkingDots = findDot(checkingTree);
if (checkingDots.length !== 1) {
  throw new Error(`LivenessDot: liveness='checking' must render exactly one dot, got ${checkingDots.length}`);
}
const checkingClassName = (checkingDots[0].props as { className?: string }).className ?? '';
if (!checkingClassName.includes('animate-pulse')) {
  throw new Error(`LivenessDot: liveness='checking' must include the pulse utility, got className='${checkingClassName}'`);
}
checkingTree.unmount();

// ── 'online' renders a solid emerald dot, no pulse ─────────────────────

const onlineTree = mount(<LivenessDot liveness="online" />);
const onlineDots = findDot(onlineTree);
if (onlineDots.length !== 1) {
  throw new Error(`LivenessDot: liveness='online' must render exactly one dot, got ${onlineDots.length}`);
}
const onlineClassName = (onlineDots[0].props as { className?: string }).className ?? '';
if (!onlineClassName.includes('emerald')) {
  throw new Error(`LivenessDot: liveness='online' must use an emerald tone, got className='${onlineClassName}'`);
}
if (onlineClassName.includes('animate-pulse')) {
  throw new Error("LivenessDot: liveness='online' must not pulse");
}
onlineTree.unmount();

// ── 'offline' renders a solid ruby/muted dot, no pulse ──────────────────

const offlineTree = mount(<LivenessDot liveness="offline" />);
const offlineDots = findDot(offlineTree);
if (offlineDots.length !== 1) {
  throw new Error(`LivenessDot: liveness='offline' must render exactly one dot, got ${offlineDots.length}`);
}
const offlineClassName = (offlineDots[0].props as { className?: string }).className ?? '';
if (!offlineClassName.includes('ruby')) {
  throw new Error(`LivenessDot: liveness='offline' must use a ruby/muted tone, got className='${offlineClassName}'`);
}
if (offlineClassName.includes('animate-pulse')) {
  throw new Error("LivenessDot: liveness='offline' must not pulse");
}
offlineTree.unmount();

// ── tones are distinct from each other ──────────────────────────────────

if (onlineClassName === offlineClassName || onlineClassName === checkingClassName || offlineClassName === checkingClassName) {
  throw new Error('LivenessDot: online/offline/checking must each render visually distinct classNames');
}

export const statusBadgeTestResults = {
  livenessDot: {
    unknownRendersNothing: true,
    absentRendersNothing: true,
    checkingPulses: true,
    onlineIsEmeraldNoPulse: true,
    offlineIsRubyNoPulse: true,
    tonesAreDistinct: true,
  },
} as const;
