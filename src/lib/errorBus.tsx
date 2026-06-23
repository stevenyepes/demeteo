import React, {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
} from "react";
import type { AppErrorKind } from "../types";
import { asAppError } from "./errors";

/**
 * One row in the error toast stack.
 */
export interface ErrorToast {
  /** Stable id for React keys and dismiss-by-id. */
  id: string;
  /** Stable error kind (drives icon + accent color). */
  kind: AppErrorKind;
  /** User-facing message (already redacted on the backend). */
  message: string;
  /** Unix ms when reported. */
  timestamp: number;
  /** If false, the toast is sticky (no auto-dismiss). */
  dismissable: boolean;
  /** Original unknown value, kept for debugging in the console. */
  raw?: unknown;
}

export interface ReportOptions {
  kind?: AppErrorKind;
  /** Override the auto-dismiss timeout (ms). `0` = sticky. */
  ttlMs?: number;
  /** Mark as sticky regardless of kind. */
  sticky?: boolean;
}

export interface ErrorBus {
  /** Currently visible toasts (newest first). */
  toasts: ErrorToast[];
  /** Push a new error. Accepts anything caught from a try/catch. */
  reportError: (err: unknown, options?: ReportOptions) => string;
  /** Remove a specific toast by id. */
  dismiss: (id: string) => void;
  /** Remove all toasts. */
  clear: () => void;
}

const DEFAULT_TTL_MS = 6000;
const MAX_VISIBLE = 4;
let idCounter = 0;
const nextId = () => `err-${Date.now()}-${++idCounter}`;

const subscribers = new Set<(toasts: ErrorToast[]) => void>();
let toasts: ErrorToast[] = [];
const timers = new Map<string, ReturnType<typeof setTimeout>>();

const emit = () => {
  for (const cb of subscribers) cb(toasts);
};

const scheduleDismiss = (id: string, ttlMs: number) => {
  const existing = timers.get(id);
  if (existing) clearTimeout(existing);
  if (ttlMs <= 0) return;
  const handle = setTimeout(() => dismissInternal(id), ttlMs);
  timers.set(id, handle);
};

const dismissInternal = (id: string) => {
  const existing = timers.get(id);
  if (existing) {
    clearTimeout(existing);
    timers.delete(id);
  }
  const next = toasts.filter((t) => t.id !== id);
  if (next.length !== toasts.length) {
    toasts = next;
    emit();
  }
};

/**
 * Imperative entry point for non-React code (e.g. background event handlers).
 * For React catch sites, prefer the {@link useErrorBus} hook.
 */
export function reportError(err: unknown, options: ReportOptions = {}): string {
  const appErr = asAppError(err);
  const kind: AppErrorKind = options.kind ?? appErr?.kind ?? "internal";
  const message =
    appErr?.message ??
    (typeof err === "string"
      ? err
      : err instanceof Error
      ? err.message
      : "Unknown error");

  const toast: ErrorToast = {
    id: nextId(),
    kind,
    message,
    timestamp: Date.now(),
    dismissable: !(options.sticky ?? false),
    raw: err,
  };

  // Cap visible count — drop the oldest non-sticky if we'd overflow.
  const next = [toast, ...toasts];
  if (next.length > MAX_VISIBLE) {
    const overflow = next.length - MAX_VISIBLE;
    const drop = next.slice(MAX_VISIBLE).filter((t) => t.dismissable);
    const dropIds = new Set(drop.slice(0, overflow).map((t) => t.id));
    const trimmed = next.filter((t) => !dropIds.has(t.id));
    toasts = trimmed;
  } else {
    toasts = next;
  }
  emit();
  if (toast.dismissable) {
    scheduleDismiss(toast.id, options.ttlMs ?? DEFAULT_TTL_MS);
  }
  // Always log to console for debugging.
  console.warn(`[errorBus] ${kind}: ${message}`, err);
  return toast.id;
}

const ErrorBusContext = createContext<ErrorBus | null>(null);

export const ErrorBusProvider: React.FC<{ children: React.ReactNode }> = ({
  children,
}) => {
  const [state, setState] = useState<ErrorToast[]>(toasts);

  useEffect(() => {
    subscribers.add(setState);
    return () => {
      subscribers.delete(setState);
    };
  }, []);

  const reportErrorMemo = useCallback(
    (err: unknown, options?: ReportOptions) => reportError(err, options),
    [],
  );

  const dismiss = useCallback((id: string) => dismissInternal(id), []);
  const clear = useCallback(() => {
    for (const handle of timers.values()) clearTimeout(handle);
    timers.clear();
    toasts = [];
    emit();
  }, []);

  const value = useMemo<ErrorBus>(
    () => ({ toasts: state, reportError: reportErrorMemo, dismiss, clear }),
    [state, reportErrorMemo, dismiss, clear],
  );

  return (
    <ErrorBusContext.Provider value={value}>{children}</ErrorBusContext.Provider>
  );
};

export function useErrorBus(): ErrorBus {
  const ctx = useContext(ErrorBusContext);
  if (!ctx) {
    // Allow use outside the provider by falling back to the imperative API.
    return {
      toasts: [],
      reportError,
      dismiss: dismissInternal,
      clear: () => {
        for (const handle of timers.values()) clearTimeout(handle);
        timers.clear();
        toasts = [];
        emit();
      },
    };
  }
  return ctx;
}
