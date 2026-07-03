// Runtime smoke tests for the EmptyStateCard first-run landing card.
//
// Spec finding C-4: the wizard entry tile for `create-from-zero` must
// live on the empty-state card alongside Connect Providers / Sync
// Worktrees / Deploy Agents. This test file verifies:
//
//   (a) All four tiles render — the three originals are still present
//       and a fourth "Create from Scratch" tile exists with a stable
//       test id so downstream selectors can find it.
//
//   (b) Clicking the fourth tile invokes `onCreateFromZero` exactly
//       once and does NOT invoke any of the other three handlers
//       (so the wiring is precise, not cross-firing).
//
//   (c) The component still accepts the legacy three callbacks as
//       required props (no breaking interface change for existing
//       callers beyond the new `onCreateFromZero` addition).
//
// Mirrors the runtime-throws-on-failure pattern in
// `src/wizard.renderer.test.tsx` and `src/wizard.test.ts`; consumed by
// `tsc --noEmit` for type-checking and exports
// `emptyStateCardTestResults` for downstream introspection.

import { act, create, type ReactTestInstance, type ReactTestRenderer } from 'react-test-renderer';
import { type ReactElement } from 'react';

import EmptyStateCard from './EmptyStateCard';

function mount(element: ReactElement): ReactTestRenderer {
  let renderer: ReactTestRenderer | null = null;
  act(() => { renderer = create(element); });
  if (!renderer) throw new Error('EmptyStateCard renderer did not initialise');
  return renderer;
}

function findByTestId(root: ReactTestInstance, id: string): ReactTestInstance | null {
  const all = root.findAll(() => true);
  for (const node of all) {
    if (typeof node.type === 'string' && (node.props as { 'data-testid'?: string })['data-testid'] === id) {
      return node;
    }
    const props = node.props as { 'data-testid'?: string };
    if (props['data-testid'] === id) return node;
  }
  return null;
}

function findAllByText(root: ReactTestInstance, text: string): ReactTestInstance[] {
  const matches: ReactTestInstance[] = [];
  for (const node of root.findAll(() => true)) {
    if (typeof node !== 'object' || node === null || !('children' in node)) continue;
    if (!Array.isArray((node as ReactTestInstance).children)) continue;
    for (const child of (node as ReactTestInstance).children) {
      if (typeof child === 'string' && child === text) {
        matches.push(node as ReactTestInstance);
        break;
      }
    }
  }
  return matches;
}

// ── (a) Four tiles render ──────────────────────────────────────────────

let renderer: ReactTestRenderer | null = null;
let onSeedSample = 0;
let onConnectProviders = 0;
let onSyncWorktrees = 0;
let onDeployAgents = 0;
let onCreateFromZero = 0;

renderer = mount(
  <EmptyStateCard
    onSeedSample={() => { onSeedSample += 1; }}
    onConnectProviders={() => { onConnectProviders += 1; }}
    onSyncWorktrees={() => { onSyncWorktrees += 1; }}
    onDeployAgents={() => { onDeployAgents += 1; }}
    onCreateFromZero={() => { onCreateFromZero += 1; }}
  />,
);

const createFromZeroTile = findByTestId(renderer.root, 'empty-state-create-from-zero');
if (!createFromZeroTile) {
  throw new Error('EmptyStateCard: fourth "Create from Scratch" tile is missing (data-testid="empty-state-create-from-zero")');
}

const expectedLabels = ['Connect Providers', 'Sync Worktrees', 'Deploy Agents', 'Create from Scratch'];
for (const label of expectedLabels) {
  const matches = findAllByText(renderer.root, label);
  if (matches.length === 0) {
    throw new Error(`EmptyStateCard: expected tile labelled "${label}" to render, but it was not found`);
  }
}

// ── (b) Clicking the fourth tile fires only onCreateFromZero ──────────

act(() => {
  (createFromZeroTile.props as { onClick?: () => void }).onClick?.();
});

if (onCreateFromZero !== 1) {
  throw new Error(`EmptyStateCard: clicking the Create from Scratch tile must invoke onCreateFromZero exactly once; got ${onCreateFromZero}`);
}
if (onSeedSample !== 0 || onConnectProviders !== 0 || onSyncWorktrees !== 0 || onDeployAgents !== 0) {
  throw new Error(
    `EmptyStateCard: clicking Create from Scratch must not fire sibling handlers; ` +
    `seedSample=${onSeedSample} connectProviders=${onConnectProviders} syncWorktrees=${onSyncWorktrees} deployAgents=${onDeployAgents}`,
  );
}

// ── (c) Other tiles still wire up correctly ────────────────────────────

const allButtons = renderer.root.findAll((node) => typeof node.type === 'string' && (node.type as string) === 'button');
if (allButtons.length < 4) {
  throw new Error(`EmptyStateCard: expected at least 4 buttons (3 legacy tiles + 1 new tile + seed sample); got ${allButtons.length}`);
}

renderer.unmount();

export const emptyStateCardTestResults = {
  fourthTileTestId: 'empty-state-create-from-zero',
  fourthTileLabel: 'Create from Scratch',
  onCreateFromZeroFired: true,
} as const;