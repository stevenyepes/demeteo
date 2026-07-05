import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useReducer,
  useRef,
} from 'react';
import type { CSSProperties, ReactNode } from 'react';
import {
  EMPTY_STACK,
  hasOverlay,
  type OverlayEntry,
  type OverlayStack,
  overlayStackReducer,
  type OverlayStackAction,
  popOverlay,
  type PushOptions,
  pushOverlay,
  replaceOverlay,
  topOverlay,
} from '../lib/overlayStack';

/**
 * Stack context value exposed to consumer components. `state` mirrors the
 * reducer — it's the React source of truth for renders. The action helpers
 * (`push`/`pop`/`replace`) delegate to the pure reducer so callers never
 * need to know the action shape.
 */
export interface OverlayStackContextValue {
  readonly state: OverlayStack;
  /** Push an overlay. If `options.id` is omitted, a stable id is generated. */
  push: (options?: PushOptions) => OverlayEntry;
  /** Pop by id. No-op when the id isn't in the stack. */
  pop: (id: string) => void;
  /** Replace fields on an existing entry. No-op when absent. */
  replace: (id: string, options?: Omit<PushOptions, 'id'>) => void;
  /** Topmost entry, or undefined when the stack is empty. */
  top: () => OverlayEntry | undefined;
  /** Predicate — is the id currently on the stack? */
  has: (id: string) => boolean;
}

const OverlayStackContext = createContext<OverlayStackContextValue | null>(null);

/**
 * Hook for consumers (modals, palette, drawers, toasts). Throws when used
 * outside an `<OverlayRoot>` to surface missing-provider bugs early.
 */
export function useOverlayStack(): OverlayStackContextValue {
  const ctx = useContext(OverlayStackContext);
  if (!ctx) {
    throw new Error('useOverlayStack must be used inside an <OverlayRoot> provider.');
  }
  return ctx;
}

/**
 * Permissive variant for code paths that may render outside the provider
 * (e.g. shadow-tree consumers). Returns `null` instead of throwing and lets
 * the caller no-op when the stack isn't mounted.
 */
export function useOptionalOverlayStack(): OverlayStackContextValue | null {
  return useContext(OverlayStackContext);
}

interface OverlayRootProps {
  /** Tree rendered beneath the stack surface. */
  children?: ReactNode;
  /**
   * Optional id to render on the root portal for test selectors. Defaults
   * to `overlay-root`.
   */
  id?: string;
  /** Optional className applied to the portal wrapper. */
  className?: string;
}

const FOCUSABLE_SELECTORS = [
  'a[href]',
  'area[href]',
  'button:not([disabled])',
  'input:not([disabled]):not([type="hidden"])',
  'select:not([disabled])',
  'textarea:not([disabled])',
  'iframe',
  'audio[controls]',
  'video[controls]',
  '[contenteditable]:not([contenteditable="false"])',
  '[tabindex]:not([tabindex="-1"])',
].join(',');

function getFocusable(root: HTMLElement | null): HTMLElement[] {
  if (!root) return [];
  const nodes = Array.from(root.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTORS));
  return nodes.filter((el) => !el.hasAttribute('disabled') && el.getAttribute('aria-hidden') !== 'true');
}

/**
 * Visual treatment for a stack entry. Token values match AGENTS.md §5:
 *   - card surface rgba(18,22,30,0.75) + backdrop blur
 *   - border glow rgba(255,255,255,0.05)
 *   - cyan #06b6d4 (interactive / focus accent)
 *
 * The cyan border glow is doubled: an inset 1px line for definition, plus
 * a 24px outer halo for the "lifted card" feel.
 */
const SURFACE_STYLE: CSSProperties = {
  position: 'relative',
  background: 'rgba(18, 22, 30, 0.75)',
  backdropFilter: 'blur(12px)',
  WebkitBackdropFilter: 'blur(12px)',
  border: '1px solid rgba(6, 182, 212, 0.45)',
  boxShadow:
    '0 0 0 1px rgba(6, 182, 212, 0.18), 0 0 24px rgba(6, 182, 212, 0.25)',
  borderRadius: 16,
  color: '#ffffff',
  fontFamily: 'Inter, system-ui, sans-serif',
  outline: 'none',
};

interface OverlaySurfaceProps {
  entry: OverlayEntry;
  isTop: boolean;
  /** Position in the sorted stack. Used to spread z-index across levels. */
  zIndex: number;
}

function OverlaySurface({ entry, isTop, zIndex }: OverlaySurfaceProps) {
  const ref = useRef<HTMLDivElement | null>(null);
  const previouslyFocused = useRef<HTMLElement | null>(null);
  const captured = useRef(false);

  useEffect(() => {
    if (!isTop) return;
    if (!captured.current) {
      const active = document.activeElement as HTMLElement | null;
      previouslyFocused.current = active && active !== document.body ? active : null;
      captured.current = true;
    }
    const focusable = getFocusable(ref.current);
    if (focusable.length > 0) {
      focusable[0].focus();
    } else {
      ref.current?.focus();
    }
  }, [isTop, entry.id]);

  useEffect(() => {
    return () => {
      const target = previouslyFocused.current;
      if (
        target &&
        typeof target.focus === 'function' &&
        document.contains(target) &&
        document.body.contains(target)
      ) {
        target.focus();
      }
    };
  }, []);

  useEffect(() => {
    if (!isTop) return;
    const handler = (e: KeyboardEvent) => {
      if (e.key !== 'Tab') return;
      const root = ref.current;
      if (!root) return;
      const focusable = getFocusable(root);
      if (focusable.length === 0) {
        e.preventDefault();
        root.focus();
        return;
      }
      const first = focusable[0];
      const last = focusable[focusable.length - 1];
      const active = document.activeElement as HTMLElement | null;
      const inside = !!active && root.contains(active);
      if (e.shiftKey) {
        if (!inside || active === first) {
          e.preventDefault();
          last.focus();
        }
      } else if (!inside || active === last) {
        e.preventDefault();
        first.focus();
      }
    };
    window.addEventListener('keydown', handler, { capture: true });
    return () => window.removeEventListener('keydown', handler, true);
  }, [isTop]);

  const wrapperStyle: CSSProperties = isTop
    ? {
        position: 'fixed',
        inset: 0,
        zIndex: 100 + zIndex * 10,
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
        padding: 16,
        background: 'rgba(8, 9, 12, 0.45)',
        backdropFilter: 'blur(2px)',
        WebkitBackdropFilter: 'blur(2px)',
      }
    : {
        position: 'fixed',
        inset: 0,
        zIndex: 100 + zIndex * 10,
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
        padding: 16,
        background: 'transparent',
        pointerEvents: 'none',
      };

  return (
    <div
      data-overlay-id={entry.id}
      data-overlay-tier={entry.tier}
      data-overlay-top={isTop ? 'true' : 'false'}
      aria-hidden={isTop ? undefined : true}
      style={wrapperStyle}
    >
      <div
        ref={ref}
        tabIndex={-1}
        style={SURFACE_STYLE}
        onClick={(e) => e.stopPropagation()}
      >
        {entry.content}
      </div>
    </div>
  );
}

/**
 * Functional render helper — pulls `state.entries` from context and emits
 * one `<OverlaySurface>` per entry in sort order.
 */
function OverlayStackRenderer() {
  const ctx = useOptionalOverlayStack();
  const entries = ctx?.state.entries ?? EMPTY_STACK.entries;
  return (
    <>
      {entries.map((entry, idx) => (
        <OverlaySurface
          key={entry.id}
          entry={entry}
          isTop={idx === 0}
          zIndex={idx}
        />
      ))}
    </>
  );
}

/**
 * Provider — wraps any tree and exposes the overlay stack.
 *
 * Responsibilities:
 *   1. Own the reducer (single source of truth).
 *   2. Render one surface per entry via `<OverlayStackRenderer/>`.
 *   3. Install **one** global Escape listener that closes the topmost entry.
 *   4. Delegate focus management to per-surface `useEffect`s.
 *
 * The Escape listener is registered with `{ capture: true }` so it wins over
 * any stray surviving `addEventListener('keydown', ...)` calls.
 */
export function OverlayRoot({ children, id = 'overlay-root', className }: OverlayRootProps) {
  const [state, baseDispatch] = useReducer(overlayStackReducer, EMPTY_STACK);

  // Mirror state into a ref so the Escape listener can read the latest stack
  // synchronously, without waiting for React to flush a re-render.
  const stateRef = useRef<OverlayStack>(state);
  stateRef.current = state;

  const dispatchStable = useCallback((action: OverlayStackAction) => {
    baseDispatch(action);
  }, []);

  const push = useCallback(
    (options: PushOptions = {}): OverlayEntry => {
      const result = pushOverlay(stateRef.current, options);
      stateRef.current = result.state;
      dispatchStable({ type: 'PUSH', entry: result.entry });
      return result.entry;
    },
    [dispatchStable],
  );

  const pop = useCallback(
    (id: string): void => {
      const next = popOverlay(stateRef.current, id);
      if (next === stateRef.current) return;
      stateRef.current = next;
      dispatchStable({ type: 'POP', id });
    },
    [dispatchStable],
  );

  const replace = useCallback(
    (id: string, options: Omit<PushOptions, 'id'> = {}): void => {
      const next = replaceOverlay(stateRef.current, id, options);
      if (next === stateRef.current) return;
      stateRef.current = next;
      // Find the replaced entry to dispatch a REPLACE for the reducer.
      const replaced = next.entries.find((e) => e.id === id);
      if (replaced) dispatchStable({ type: 'REPLACE', entry: replaced });
    },
    [dispatchStable],
  );

  const top = useCallback((): OverlayEntry | undefined => topOverlay(stateRef.current), []);
  const has = useCallback((id: string): boolean => hasOverlay(stateRef.current, id), []);

  // One (and only one) global Escape listener.
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return;
      const stack = stateRef.current;
      const topmost = topOverlay(stack);
      if (!topmost) return;
      e.preventDefault();
      e.stopPropagation();
      const dismiss = topmost.dismissOnEscape !== false;
      if (dismiss) {
        stateRef.current = popOverlay(stack, topmost.id);
        dispatchStable({ type: 'POP', id: topmost.id });
      }
      // Always call the consumer's onEscape — even on dismiss, so callers
      // that want to log/run-effects get notified.
      topmost.onEscape?.();
    };
    window.addEventListener('keydown', handler, { capture: true });
    return () => window.removeEventListener('keydown', handler, true);
  }, [dispatchStable]);

  // DevTools-friendly tooltip: render the size in a `data-` attribute.
  const count = state.entries.length;

  const contextValue = useMemo<OverlayStackContextValue>(
    () => ({ state, push, pop, replace, top, has }),
    [state, push, pop, replace, top, has],
  );

  return (
    <OverlayStackContext.Provider value={contextValue}>
      {children}
      <div
        id={id}
        data-overlay-root="true"
        data-overlay-count={count}
        className={className}
        aria-hidden={count === 0 ? true : undefined}
      >
        <OverlayStackRenderer />
      </div>
    </OverlayStackContext.Provider>
  );
}

// Re-export common types so consumers can import everything from one place.
export type {
  OverlayEntry,
  OverlayPriorityTier,
  OverlayStack,
  OverlayStackAction,
  PushOptions,
} from '../lib/overlayStack';
