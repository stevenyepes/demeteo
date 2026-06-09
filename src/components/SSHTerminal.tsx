import React, { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Terminal as XTermTerminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { SerializeAddon } from "@xterm/addon-serialize";
import "@xterm/xterm/css/xterm.css";
import TerminalStatusOverlay from "./TerminalStatusOverlay";
import {
  ensureSession,
  detachSession,
  sessionRegistry,
} from "../sessionRegistry";

interface SSHTerminalProps {
  machineId: string;
  host?: string;
  tabId: string;
}

type ConnectionPhase = "resolving" | "connecting" | "authenticating" | "ready" | "error";

const PHASE_WALK: ConnectionPhase[] = ["resolving", "connecting", "authenticating"];

const BUFFER_KEY = (machineId: string, tabId: string) =>
  `demeteo.termbuf.${machineId}.${tabId}`;

const SSHTerminal: React.FC<SSHTerminalProps> = ({ machineId, host, tabId }) => {
  const terminalRef = useRef<HTMLDivElement>(null);
  const xtermRef = useRef<XTermTerminal | null>(null);
  const fitAddonRef = useRef<FitAddon | null>(null);
  const isMountedRef = useRef<boolean>(true);
  const [phase, setPhase] = useState<ConnectionPhase>("resolving");
  const [errorDetail, setErrorDetail] = useState<string>("");

  useEffect(() => {
    if (!terminalRef.current) return;
    isMountedRef.current = true;

    setPhase("resolving");
    setErrorDetail("");

    const term = new XTermTerminal({
      cursorBlink: true,
      fontFamily: "Fira Code, JetBrains Mono, monospace",
      fontSize: 13,
      theme: {
        background: "#050508",
        foreground: "#06b6d4",
        cursor: "#8b5cf6",
        selectionBackground: "rgba(139, 92, 246, 0.3)",
      },
    });
    const fitAddon = new FitAddon();
    const serializeAddon = new SerializeAddon();
    term.loadAddon(fitAddon);
    term.loadAddon(serializeAddon);
    term.open(terminalRef.current);
    fitAddon.fit();
    xtermRef.current = term;
    fitAddonRef.current = fitAddon;

    // Restore last-known scrollback (e.g. user switched to supervisor view and back)
    let restoredFromBuffer = false;
    try {
      const saved = localStorage.getItem(BUFFER_KEY(machineId, tabId));
      if (saved) {
        term.write(saved);
        restoredFromBuffer = true;
      }
    } catch {
      // ignore quota / parse errors
    }

    if (!restoredFromBuffer) {
      term.write(
        "\x1b[35m[Demeteo SSH Core]\x1b[0m Connecting to remote machine...\r\n",
      );
    } else {
      term.write(
        "\r\n\x1b[35m[Demeteo SSH Core]\x1b[0m Reconnecting...\r\n",
      );
    }

    let phaseTimer: ReturnType<typeof setTimeout> | undefined;
    const advancePhase = (idx: number) => {
      if (idx >= PHASE_WALK.length) return;
      setPhase(PHASE_WALK[idx]);
      phaseTimer = setTimeout(() => advancePhase(idx + 1), 700);
    };
    advancePhase(0);

    ensureSession(machineId, tabId, (bytes: number[]) => {
      if (!isMountedRef.current) return;
      term.write(new Uint8Array(bytes));
      scheduleSave();
    })
      .then((sessId) => {
        if (phaseTimer) clearTimeout(phaseTimer);
        if (!isMountedRef.current) return;
        setPhase("ready");
        const rec = sessionRegistry.get(machineId, tabId);
        if (rec.status !== "ended") {
          term.write("\x1b[32m[Demeteo SSH Core]\x1b[0m Connection established.\r\n\r\n");
        } else {
          term.write("\x1b[33m[Demeteo SSH Core]\x1b[0m Reattached to live session.\r\n\r\n");
        }
        invoke("resize_terminal_session", {
          sessionId: sessId,
          cols: term.cols,
          rows: term.rows,
        }).catch(console.error);
      })
      .catch((err) => {
        if (phaseTimer) clearTimeout(phaseTimer);
        if (!isMountedRef.current) return;
        const msg = String(err);
        setErrorDetail(msg);
        setPhase("error");
        term.write(`\r\n\x1b[31m[Demeteo SSH Core] Connection failed:\x1b[0m ${msg}\r\n`);
        term.write(
          "\x1b[33m[Demeteo SSH Core]\x1b[0m Verify the target host is reachable, credentials are stored, and try re-selecting this machine.\r\n",
        );
      });

    const onDataDispose = term.onData((data) => {
      const rec = sessionRegistry.get(machineId, tabId);
      if (rec.sessionId && rec.status === "ready") {
        invoke("write_terminal_session", {
          sessionId: rec.sessionId,
          data,
        }).catch(console.error);
      }
      // Local input is echoed back by the remote, so we don't need to write
      // it to the xterm buffer manually here. But the resulting terminal
      // state is what we want to capture for the next mount.
      scheduleSave();
    });

    // Debounced buffer persistence so the latest scrollback is always on disk
    let saveTimer: ReturnType<typeof setTimeout> | undefined;
    const scheduleSave = () => {
      if (saveTimer) clearTimeout(saveTimer);
      saveTimer = setTimeout(() => {
        try {
          const buf = serializeAddon.serialize();
          if (buf && buf.length > 0) {
            localStorage.setItem(BUFFER_KEY(machineId, tabId), buf);
          }
        } catch {
          // localStorage quota exceeded — skip silently
        }
      }, 500);
    };

    const handleResize = () => {
      try {
        fitAddon.fit();
        const rec = sessionRegistry.get(machineId, tabId);
        if (rec.sessionId) {
          invoke("resize_terminal_session", {
            sessionId: rec.sessionId,
            cols: term.cols,
            rows: term.rows,
          }).catch(console.error);
        }
      } catch (e) {
        console.error("Resize fit error:", e);
      }
    };
    window.addEventListener("resize", handleResize);

    return () => {
      isMountedRef.current = false;
      onDataDispose.dispose();
      window.removeEventListener("resize", handleResize);
      if (saveTimer) {
        clearTimeout(saveTimer);
        // Final synchronous save so the latest bytes are captured
        try {
          const buf = serializeAddon.serialize();
          if (buf && buf.length > 0) {
            localStorage.setItem(BUFFER_KEY(machineId, tabId), buf);
          }
        } catch {
          // ignore
        }
      }
      term.dispose();
      xtermRef.current = null;
      fitAddonRef.current = null;
      if (phaseTimer) clearTimeout(phaseTimer);
      // Detach the frontend channel but keep the backend session alive.
      // When the user comes back to this tab, ensureSession will rebind and
      // the saved scrollback will be restored from localStorage.
      void detachSession(machineId, tabId);
    };
  }, [machineId, tabId]);

  // Surface backend session-ended events for this tab into the overlay
  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    (async () => {
      const { listen } = await import("@tauri-apps/api/event");
      unlisten = await listen<{ session_id: string; machine_id: string }>(
        "terminal-session-ended",
        (e) => {
          if (e.payload.machine_id !== machineId) return;
          const rec = sessionRegistry.get(machineId, tabId);
          if (rec.sessionId !== e.payload.session_id) return;
          if (cancelled) return;
          sessionRegistry.update(machineId, tabId, { status: "ended" });
          const term = xtermRef.current;
          if (term) {
            term.write(
              "\r\n\x1b[33m[Demeteo SSH Core] Session ended (remote closed or idle timeout).\x1b[0m\r\n",
            );
          }
        },
      );
    })();
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [machineId, tabId]);

  const rec = sessionRegistry.get(machineId, tabId);
  const phaseForOverlay: ConnectionPhase =
    rec.status === "ended" || rec.status === "error" ? "error" : phase;

  return (
    <div className="relative w-full h-full">
      <div ref={terminalRef} className="w-full h-full p-2.5 box-border" />
      <TerminalStatusOverlay
        phase={phaseForOverlay}
        host={host}
        detail={errorDetail}
      />
    </div>
  );
};

export default SSHTerminal;
