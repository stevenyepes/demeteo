import React, { useEffect, useRef } from "react";
import { invoke, Channel } from "@tauri-apps/api/core";
import { Terminal as XTermTerminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";

interface SSHTerminalProps {
  machineId: string;
}

const SSHTerminal: React.FC<SSHTerminalProps> = ({ machineId }) => {
  const terminalRef = useRef<HTMLDivElement>(null);
  const sessionIdRef = useRef<string | null>(null);

  useEffect(() => {
    if (!terminalRef.current) return;

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
    term.loadAddon(fitAddon);
    term.open(terminalRef.current);
    fitAddon.fit();

    term.write("\x1b[35m[Demeteo SSH Core]\x1b[0m Connecting to remote machine...\r\n");

    const onDataChannel = new Channel<number[]>();
    onDataChannel.onmessage = (bytes: number[]) => {
      const arr = new Uint8Array(bytes);
      term.write(arr);
    };

    let isActive = true;
    invoke<string>("start_terminal_session", { machineId, tauriChannel: onDataChannel })
      .then((sessId) => {
        if (!isActive) {
          invoke("close_terminal_session", { sessionId: sessId }).catch(console.error);
          return;
        }
        sessionIdRef.current = sessId;
        term.write("\x1b[32m[Demeteo SSH Core]\x1b[0m Connection established.\r\n\r\n");

        invoke("resize_terminal_session", {
          sessionId: sessId,
          cols: term.cols,
          rows: term.rows,
        }).catch(console.error);
      })
      .catch((err) => {
        term.write(`\r\n\x1b[31m[Demeteo SSH Core] Connection failed:\x1b[0m ${err}\r\n`);
      });

    const onDataDispose = term.onData((data) => {
      if (sessionIdRef.current) {
        invoke("write_terminal_session", { sessionId: sessionIdRef.current, data }).catch(console.error);
      }
    });

    const handleResize = () => {
      try {
        fitAddon.fit();
        if (sessionIdRef.current) {
          invoke("resize_terminal_session", {
            sessionId: sessionIdRef.current,
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
      isActive = false;
      onDataDispose.dispose();
      window.removeEventListener("resize", handleResize);
      term.dispose();
      if (sessionIdRef.current) {
        invoke("close_terminal_session", { sessionId: sessionIdRef.current }).catch(console.error);
      }
    };
  }, [machineId]);

  return <div ref={terminalRef} className="w-full h-full p-2.5 box-border" />;
};

export default SSHTerminal;
