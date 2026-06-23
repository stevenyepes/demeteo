import React, { useState } from "react";
import {
  AlertTriangle,
  ShieldAlert,
  WifiOff,
  Database,
  Cpu,
  Wrench,
  HelpCircle,
  X,
  Copy,
  Check,
  Settings,
  FolderOpen,
  RefreshCw,
  ScrollText,
} from "lucide-react";
import type { AppErrorKind } from "../types";
import { useErrorBus } from "../lib/errorBus";

/**
 * Window event the toast dispatches when the user clicks a kind-specific
 * CTA. App-level listeners (mounted near the router) translate these into
 * the appropriate `setView(...)` / `setActiveFeatureId(...)` calls. This
 * keeps the toast decoupled from the app's navigation state.
 */
export type ErrorToastCta =
  | "open-providers"
  | "open-settings"
  | "open-feature"
  | "retry"
  | "view-logs";

export const ERROR_TOAST_CTA_EVENT = "error-toast-cta";

export const dispatchErrorToastCta = (kind: AppErrorKind, cta: ErrorToastCta) => {
  window.dispatchEvent(
    new CustomEvent(ERROR_TOAST_CTA_EVENT, { detail: { kind, cta } }),
  );
};

const KIND_META: Record<
  AppErrorKind,
  {
    icon: React.ComponentType<{ className?: string }>;
    label: string;
    accent: string;
    leftBar: string;
  }
> = {
  not_found: {
    icon: HelpCircle,
    label: "Not found",
    accent: "text-slate-300",
    leftBar: "border-l-slate-500/60",
  },
  validation: {
    icon: AlertTriangle,
    label: "Invalid input",
    accent: "text-amber-300",
    leftBar: "border-l-amber-400/70",
  },
  conflict: {
    icon: ShieldAlert,
    label: "Conflict",
    accent: "text-ruby-300",
    leftBar: "border-l-ruby-400/70",
  },
  provider: {
    icon: ShieldAlert,
    label: "Provider",
    accent: "text-violet-300",
    leftBar: "border-l-violet-400/70",
  },
  transport: {
    icon: WifiOff,
    label: "Network",
    accent: "text-orange-300",
    leftBar: "border-l-orange-400/70",
  },
  database: {
    icon: Database,
    label: "Database",
    accent: "text-cyan-300",
    leftBar: "border-l-cyan-400/70",
  },
  agent: {
    icon: Cpu,
    label: "Agent",
    accent: "text-ruby-300",
    leftBar: "border-l-ruby-400/70",
  },
  internal: {
    icon: Wrench,
    label: "Internal",
    accent: "text-slate-300",
    leftBar: "border-l-slate-500/60",
  },
};

interface CtaDef {
  cta: ErrorToastCta;
  label: string;
  icon: React.ComponentType<{ className?: string }>;
}

const KIND_CTAS: Partial<Record<AppErrorKind, CtaDef[]>> = {
  provider: [
    { cta: "open-providers", label: "Open providers", icon: Settings },
  ],
  conflict: [
    { cta: "open-feature", label: "Open feature", icon: FolderOpen },
  ],
  transport: [
    { cta: "retry", label: "Retry", icon: RefreshCw },
  ],
  agent: [
    { cta: "view-logs", label: "View logs", icon: ScrollText },
  ],
  internal: [
    { cta: "view-logs", label: "View logs", icon: ScrollText },
  ],
};

const CopyButton: React.FC<{ kind: AppErrorKind; message: string }> = ({
  kind,
  message,
}) => {
  const [copied, setCopied] = useState(false);
  const handleClick = async () => {
    try {
      await navigator.clipboard.writeText(`${kind}: ${message}`);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch {
      // Clipboard access denied (e.g. devtools focus) — fail silently.
    }
  };
  return (
    <button
      type="button"
      onClick={handleClick}
      className="inline-flex items-center gap-1 text-xs text-slate-400 hover:text-slate-100 transition-colors"
      aria-label="Copy error details"
    >
      {copied ? <Check className="w-3 h-3" /> : <Copy className="w-3 h-3" />}
      {copied ? "Copied" : "Copy"}
    </button>
  );
};

const ErrorToastRow: React.FC<{
  id: string;
  kind: AppErrorKind;
  message: string;
  dismissable: boolean;
}> = ({ id, kind, message, dismissable }) => {
  const { dismiss } = useErrorBus();
  const meta = KIND_META[kind] ?? KIND_META.internal;
  const Icon = meta.icon;
  const ctas = KIND_CTAS[kind] ?? [];

  const handleCta = (cta: ErrorToastCta) => {
    dispatchErrorToastCta(kind, cta);
    // The CTA usually navigates away; close the toast so it doesn't linger.
    if (dismissable) dismiss(id);
  };

  return (
    <div
      className={`pointer-events-auto flex items-start gap-3 rounded-xl border border-white/5 border-l-4 ${meta.leftBar} bg-[rgba(18,22,30,0.95)] backdrop-blur-md pl-3 pr-4 py-3 shadow-2xl min-w-[320px] max-w-[480px]`}
      role="alert"
      data-testid={`error-toast-${kind}`}
    >
      <Icon className={`w-5 h-5 mt-0.5 flex-shrink-0 ${meta.accent}`} />
      <div className="flex-1 min-w-0">
        <div className={`text-xs uppercase tracking-wider ${meta.accent} font-medium`}>
          {meta.label}
        </div>
        <div className="text-sm text-slate-100 break-words mt-0.5">{message}</div>
        <div className="flex items-center gap-3 mt-2">
          {ctas.map(({ cta, label, icon: CtaIcon }) => (
            <button
              key={cta}
              type="button"
              onClick={() => handleCta(cta)}
              className="inline-flex items-center gap-1 text-xs text-cyan-300 hover:text-cyan-200 transition-colors"
            >
              <CtaIcon className="w-3 h-3" />
              {label}
            </button>
          ))}
          <CopyButton kind={kind} message={message} />
        </div>
      </div>
      {dismissable && (
        <button
          type="button"
          onClick={() => dismiss(id)}
          className="text-slate-500 hover:text-slate-200 transition-colors flex-shrink-0"
          aria-label="Dismiss"
        >
          <X className="w-4 h-4" />
        </button>
      )}
    </div>
  );
};

export const ErrorToast: React.FC = () => {
  const { toasts } = useErrorBus();
  if (toasts.length === 0) return null;
  return (
    <div
      className="fixed bottom-6 right-6 z-50 flex flex-col-reverse gap-2 pointer-events-none"
      aria-live="polite"
      aria-atomic="false"
    >
      {toasts.map((t) => (
        <ErrorToastRow
          key={t.id}
          id={t.id}
          kind={t.kind}
          message={t.message}
          dismissable={t.dismissable}
        />
      ))}
    </div>
  );
};

// Re-export for completeness; the kind type already lives in types.ts.
export type { AppErrorKind };
