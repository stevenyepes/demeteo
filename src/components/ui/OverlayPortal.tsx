import { useMemo, type ReactNode } from 'react';
import { createPortal } from 'react-dom';

/**
 * Renders full-screen overlays (modals, palettes, drawers) into `document.body`.
 *
 * `<main>` in `App.tsx` is `position: relative` with `z-index: 0`, and that pair
 * makes it a **stacking context**. Everything inside it — including a
 * `fixed inset-0 z-50` backdrop — is then painted *within* main's z-0 slot, so
 * it lands underneath the project rail and the workspace sidebar, which are
 * siblings at `z-10`. The overlay is not clipped and not mispositioned: it is
 * correctly centred in the viewport and simply covered on its left by the
 * sidebar, which is why a modal appears to have its first few characters
 * shaved off ("PELINE CONTEXT") and why the backdrop dims only the content
 * area.
 *
 * Raising the overlay's own `z-index` cannot fix this — inside a stacking
 * context nothing escapes its parent's slot. Leaving the tree does. React
 * context, event bubbling and hooks all still follow the React tree, so a
 * portalled child behaves exactly as it did in place.
 *
 * `container` exists for tests, which render without a body-mounted root.
 */
export function OverlayPortal({
  children,
  container,
}: {
  children: ReactNode;
  container?: Element | null;
}) {
  const target = useMemo<Element | null>(() => {
    if (container !== undefined) return container;
    if (typeof document === 'undefined') return null;
    return document.body;
  }, [container]);

  if (!target) return null;
  return createPortal(children, target);
}
