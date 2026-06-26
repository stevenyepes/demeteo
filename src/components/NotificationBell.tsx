import { useEffect, useRef, useState } from "react";
import { useTauriEvent } from "../hooks/useTauriEvent";
import { Bell, Check, X } from "lucide-react";
import {
  listNotifications,
  markNotificationRead,
  unreadNotificationCount,
} from "../lib/notifications";
import type { Notification, MrMergedEvent } from "../types";

/**
 * Global notification bell — lives in the header and surfaces
 * `MrMerged` (and the future gate-pending / step-failed) events.
 *
 * State management: owns its own list + unread count. On mount it
 * fetches both; a `mr_merged` Tauri listener refetches the list
 * and bumps the badge without waiting for the 2-minute poll. Click
 * outside the panel closes it.
 */
export function NotificationBell() {
  const [open, setOpen] = useState(false);
  const [items, setItems] = useState<Notification[]>([]);
  const [unread, setUnread] = useState(0);
  const [toast, setToast] = useState<string | null>(null);
  const panelRef = useRef<HTMLDivElement>(null);

  const refresh = async () => {
    try {
      const [list, count] = await Promise.all([
        listNotifications(),
        unreadNotificationCount(),
      ]);
      setItems(list);
      setUnread(count);
    } catch (err) {
      console.error("Failed to load notifications", err);
    }
  };

  // Initial fetch + on every `mr_merged` event.
  useEffect(() => { refresh(); }, []);

  useTauriEvent<MrMergedEvent>("mr_merged", ({ feature_title }) => {
    setToast(`MR for "${feature_title}" was merged`);
    refresh();
    setTimeout(() => setToast(null), 4000);
  });

  // Click outside closes the panel.
  useEffect(() => {
    if (!open) return;
    const handler = (e: MouseEvent) => {
      if (panelRef.current && !panelRef.current.contains(e.target as Node)) {
        setOpen(false);
      }
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, [open]);

  const handleClick = (n: Notification) => {
    // Mark as read locally for instant feedback, then persist.
    if (!n.read) {
      setItems((prev) =>
        prev.map((it) => (it.id === n.id ? { ...it, read: true } : it))
      );
      setUnread((c) => Math.max(0, c - 1));
      markNotificationRead(n.id).catch((err) =>
        console.error("markNotificationRead failed", err)
      );
    }
    // Future: navigate to feature detail via feature_url. For now
    // we just close the panel.
    setOpen(false);
  };

  return (
    <>
      <div className="relative" ref={panelRef}>
        <button
          onClick={() => setOpen((o) => !o)}
          className="text-slate-400 hover:text-white transition-colors hover:bg-white/5 p-1.5 rounded relative"
          title="Notifications"
          aria-label={`Notifications (${unread} unread)`}
        >
          <Bell className="w-5 h-5" />
          {unread > 0 && (
            <span
              data-testid="notif-badge"
              className="absolute -top-0.5 -right-0.5 min-w-[16px] h-4 px-1 rounded-full bg-emerald-500 text-[10px] font-mono font-bold text-black flex items-center justify-center shadow-[0_0_8px_rgba(16,185,129,0.5)]"
            >
              {unread > 99 ? "99+" : unread}
            </span>
          )}
        </button>
        {open && (
          <div className="absolute right-0 top-full mt-2 w-80 glass-panel border border-white/10 rounded-lg shadow-2xl overflow-hidden z-50">
            <div className="flex items-center justify-between px-3 py-2 border-b border-white/5">
              <span className="font-outfit text-xs font-semibold uppercase tracking-wider text-slate-400">
                Notifications
              </span>
              <span className="text-[10px] font-mono text-slate-500">
                {items.length} total
              </span>
            </div>
            <div className="max-h-80 overflow-y-auto">
              {items.length === 0 ? (
                <div className="px-4 py-8 text-center text-xs text-slate-500">
                  No notifications yet
                </div>
              ) : (
                items.map((n) => (
                  <NotificationRow
                    key={n.id}
                    item={n}
                    onClick={() => handleClick(n)}
                  />
                ))
              )}
            </div>
          </div>
        )}
      </div>
      {toast && (
        <div
          data-testid="notif-toast"
          className="fixed bottom-6 right-6 z-50 glass-panel border-l-2 border-l-emerald-400 rounded-lg px-4 py-3 shadow-2xl flex items-start gap-3 max-w-sm animate-[fadeIn_120ms_ease-out]"
        >
          <Check className="w-4 h-4 text-emerald-400 mt-0.5 shrink-0" />
          <div className="text-sm text-slate-200 flex-1">{toast}</div>
          <button
            onClick={() => setToast(null)}
            className="text-slate-500 hover:text-white"
            aria-label="Dismiss"
          >
            <X className="w-3.5 h-3.5" />
          </button>
        </div>
      )}
    </>
  );
}

function NotificationRow({
  item,
  onClick,
}: {
  item: Notification;
  onClick: () => void;
}) {
  const accent = kindAccent(item.kind);
  return (
    <button
      onClick={onClick}
      className={`w-full text-left px-3 py-2.5 flex items-start gap-2 hover:bg-white/5 transition-colors border-b border-white/[0.03] last:border-0 ${
        item.read ? "opacity-60" : ""
      }`}
    >
      <span
        className={`mt-1.5 w-1.5 h-1.5 rounded-full shrink-0 ${accent.dot} ${item.read ? "" : "shadow-[0_0_6px_currentColor]"}`}
      />
      <div className="flex-1 min-w-0">
        <div className="text-sm text-slate-200 leading-snug">
          {item.message}
        </div>
        <div className="text-[10px] font-mono text-slate-500 mt-0.5">
          {kindLabel(item.kind)} · {relativeTime(item.created_at)}
        </div>
      </div>
    </button>
  );
}

function kindLabel(kind: string): string {
  switch (kind) {
    case "mr_merged":
      return "MR merged";
    case "gate_pending":
      return "Gate pending";
    case "step_failed":
      return "Step failed";
    case "feature_completed":
      return "Completed";
    case "merge_conflict":
      return "Merge conflict";
    default:
      return kind;
  }
}

function kindAccent(kind: string): { dot: string } {
  switch (kind) {
    case "mr_merged":
    case "feature_completed":
      return { dot: "bg-emerald-400 text-emerald-400" };
    case "gate_pending":
      return { dot: "bg-cyan-400 text-cyan-400" };
    case "step_failed":
    case "merge_conflict":
      return { dot: "bg-ruby-400 text-ruby-400" };
    default:
      return { dot: "bg-slate-400 text-slate-400" };
  }
}

function relativeTime(ms: number): string {
  const diff = Date.now() - ms;
  if (diff < 60_000) return "just now";
  if (diff < 3_600_000) return `${Math.floor(diff / 60_000)}m ago`;
  if (diff < 86_400_000) return `${Math.floor(diff / 3_600_000)}h ago`;
  return `${Math.floor(diff / 86_400_000)}d ago`;
}
