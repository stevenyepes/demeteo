import React, { useEffect, useState, useCallback } from "react";
import { Plus, X, Terminal as TerminalIcon } from "lucide-react";
import { listen, UnlistenFn } from "@tauri-apps/api/event";
import SSHTerminal from "./SSHTerminal";
import {
  sessionRegistry,
  destroySession,
} from "../sessionRegistry";

interface TerminalTabsProps {
  machineId: string;
  host: string;
}

interface Tab {
  id: string;
  status: "starting" | "ready" | "ended" | "error";
}

const STORAGE_KEY = (machineId: string) => `demeteo.tabs.${machineId}`;

const TerminalTabs: React.FC<TerminalTabsProps> = ({ machineId, host }) => {
  const [tabs, setTabs] = useState<Tab[]>([]);
  const [activeIdx, setActiveIdx] = useState<number>(0);

  // ---- Persistence of tab layout (ids only; sessions themselves are server-side) ----
  useEffect(() => {
    // On machine change, the registry entries for the previous machine are
    // intentionally left alone — the backend sessions are still alive, and
    // if the user returns they'll be reattached automatically.
  }, [machineId]);

  // Restore tab layout for this machine
  useEffect(() => {
    try {
      const raw = localStorage.getItem(STORAGE_KEY(machineId));
      if (raw) {
        const ids: string[] = JSON.parse(raw);
        if (Array.isArray(ids) && ids.length > 0) {
          const restored: Tab[] = ids.map((id) => ({ id, status: "starting" }));
          setTabs(restored);
          return;
        }
      }
    } catch {
      // ignore
    }
    // First-time: one default tab
    setTabs([{ id: `tab_${Date.now()}`, status: "starting" }]);
  }, [machineId]);

  // Persist layout
  useEffect(() => {
    if (tabs.length === 0) return;
    try {
      localStorage.setItem(
        STORAGE_KEY(machineId),
        JSON.stringify(tabs.map((t) => t.id)),
      );
    } catch {
      // ignore
    }
  }, [tabs, machineId]);

  useEffect(() => {
    if (activeIdx >= tabs.length) {
      setActiveIdx(Math.max(0, tabs.length - 1));
    }
  }, [tabs.length, activeIdx]);

  // ---- Listen for backend EOF events and update the registry ----
  useEffect(() => {
    let unlisten: UnlistenFn | undefined;
    (async () => {
      unlisten = await listen<{ session_id: string; machine_id: string }>(
        "terminal-session-ended",
        (e) => {
          if (e.payload.machine_id !== machineId) return;
          const tabId = sessionRegistry.findTabBySessionId(
            machineId,
            e.payload.session_id,
          );
          if (!tabId) return;
          sessionRegistry.update(machineId, tabId, { status: "ended" });
          setTabs((prev) =>
            prev.map((t) => (t.id === tabId ? { ...t, status: "ended" } : t)),
          );
        },
      );
    })();
    return () => unlisten?.();
  }, [machineId]);

  // ---- Tab actions ----
  const openTab = useCallback(() => {
    setTabs((prev) => {
      const next: Tab = {
        id: `tab_${Date.now()}_${Math.random().toString(36).slice(2, 6)}`,
        status: "starting",
      };
      setActiveIdx(prev.length);
      return [...prev, next];
    });
  }, []);

  const closeTab = useCallback(
    async (idx: number) => {
      const tab = tabs[idx];
      if (!tab) return;
      await destroySession(machineId, tab.id);
      setTabs((prev) => {
        if (prev.length === 1) {
          // Replace with a fresh tab so we always have one
          return [{ id: `tab_${Date.now()}`, status: "starting" }];
        }
        return prev.filter((_, i) => i !== idx);
      });
    },
    [tabs, machineId],
  );

  return (
    <div className="flex flex-col h-full bg-[#050508]">
      <div className="flex items-end gap-1 px-2 pt-2 border-b border-white/5 bg-[#050508] overflow-x-auto flex-shrink-0">
        {tabs.map((tab, idx) => {
          const isActive = idx === activeIdx;
          const ended = tab.status === "ended";
          const errored = tab.status === "error";
          const dotColor =
            tab.status === "ready"
              ? "bg-emerald-400 shadow-[0_0_6px_1px_rgba(16,185,129,0.7)]"
              : errored
              ? "bg-red-500"
              : ended
              ? "bg-amber-400"
              : "bg-cyan-400 animate-pulse";

          return (
            <div
              key={tab.id}
              onClick={() => setActiveIdx(idx)}
              className={`group flex items-center gap-2 px-3 py-1.5 rounded-t-lg cursor-pointer border border-b-0 transition-colors min-w-[120px] max-w-[200px] flex-shrink-0 ${
                isActive
                  ? "bg-[#0a0a0e] border-white/10 text-slate-100"
                  : "bg-transparent border-transparent text-slate-500 hover:text-slate-300 hover:bg-white/[0.02]"
              }`}
            >
              <span className={`w-1.5 h-1.5 rounded-full flex-shrink-0 ${dotColor}`} />
              <TerminalIcon size={12} className="flex-shrink-0" />
              <span className="text-[11px] font-mono truncate flex-1">
                shell {idx + 1}
              </span>
              {(ended || errored) && (
                <button
                  type="button"
                  onClick={(e) => {
                    e.stopPropagation();
                    sessionRegistry.update(machineId, tab.id, {
                      sessionId: null,
                      status: "starting",
                      errorDetail: undefined,
                    });
                    setTabs((prev) =>
                      prev.map((t) =>
                        t.id === tab.id
                          ? { ...t, id: `tab_${Date.now()}_${Math.random().toString(36).slice(2, 6)}`, status: "starting" }
                          : t,
                      ),
                    );
                  }}
                  className="text-[10px] font-mono px-1.5 py-0.5 rounded text-amber-400 hover:bg-amber-500/10 border border-amber-500/30"
                  title="Reconnect"
                >
                  retry
                </button>
              )}
              <button
                type="button"
                onClick={(e) => {
                  e.stopPropagation();
                  void closeTab(idx);
                }}
                className="text-slate-600 hover:text-red-400 transition-colors flex-shrink-0"
                title="Close session"
              >
                <X size={12} />
              </button>
            </div>
          );
        })}
        <button
          type="button"
          onClick={openTab}
          className="ml-1 mb-1.5 p-1.5 rounded-md text-slate-500 hover:text-cyan-400 hover:bg-white/[0.03] transition-colors flex items-center gap-1 text-[11px] font-mono"
          title="New session"
        >
          <Plus size={12} />
        </button>
      </div>

      <div className="flex-1 relative overflow-hidden">
        {tabs.map((tab, idx) => {
          if (idx !== activeIdx) {
            // Hidden but mounted when active, so the SSH session keeps streaming
            return (
              <div
                key={tab.id}
                className="absolute inset-0 invisible pointer-events-none"
                aria-hidden="true"
              >
                <SSHTerminal machineId={machineId} host={host} tabId={tab.id} />
              </div>
            );
          }
          return (
            <div key={tab.id} className="absolute inset-0">
              <SSHTerminal machineId={machineId} host={host} tabId={tab.id} />
            </div>
          );
        })}
      </div>
    </div>
  );
};

export default TerminalTabs;
