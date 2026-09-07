// Test harness for the selection fields that live on `AppView` rather than in
// a component's own state — `discovery.selectedTicketId`, `ask.threadId`.
//
// Those components take the value and its writer as props so they stay
// mountable without a `NavigationProvider`, the way `useStepSelection` takes
// `navigate` as an input. A test that passed a constant would assert against a
// selection that can never move; this stands in for the router by holding the
// value the same tri-state way the route does, `undefined` included.

import { useState, type ReactElement } from 'react';

/**
 * Render `children` with a selection that behaves like the routed one.
 *
 * Starts `undefined` — nothing chosen yet — so the component under test runs
 * its own seeding policy, which is the case a fixed initial value would skip.
 */
export function RoutedSelection<T extends string>({
  children,
  initial,
}: {
  children: (
    value: T | null | undefined,
    onSelect: (next: T | null | undefined) => void,
  ) => ReactElement;
  initial?: T | null;
}): ReactElement {
  const [value, setValue] = useState<T | null | undefined>(initial);
  return children(value, setValue);
}
