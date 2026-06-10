import React, { useState, useEffect, useRef } from "react";
import { Settings, Terminal, Activity, ShieldAlert, Menu, AlertCircle } from "lucide-react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open, ask } from "@tauri-apps/plugin-dialog";
import "./App.css";

import {
  Machine,
  FrontendMachine,
  ThreadSession,
  FileReference,
  InterceptPayload,
  ExecutionResult,
  AgentEvent,
  ThreadStatusChangedEvent,
  Message,
  InterceptCard,
  SessionInfo,
} from "./types";
import TerminalTabs from "./components/TerminalTabs";
import Sidebar from "./components/Sidebar";
import SupervisorPlane from "./components/SupervisorPlane";
import CodeInspector from "./components/CodeInspector";
import NewThreadModal from "./components/NewThreadModal";
import EnvModal from "./components/EnvModal";
import CommandSelector from "./components/CommandSelector";
import {
  agentSessionRegistry,
  agentStart,
  agentInstallAndStart,
  agentPrompt,
  agentCancel,
  loadWorkingMemory,
} from "./agentSessionRegistry";

const DISPLAY_LABEL: Record<string, string> = {
  opencode: "OpenCode",
  hermes: "Hermes",
};

const mapMachineToFrontend = (m: Machine): FrontendMachine => ({
  ...m,
  type: m.auth_type === "local" ? "local" : "server",
  status: "connected",
  user: `${m.username}@${m.host}`,
});

function parseNotFoundError(msg: string): { binary: string; install_command: string } | null {
  if (!msg.startsWith("NOT_FOUND:")) return null;
  const rest = msg.slice("NOT_FOUND:".length);
  const colon = rest.indexOf(":");
  if (colon < 0) return null;
  return {
    binary: rest.slice(0, colon),
    install_command: rest.slice(colon + 1),
  };
}

function App() {
  const [machinesList, setMachinesList] = useState<FrontendMachine[]>([]);
  const [activeMachine, setActiveMachine] = useState<FrontendMachine | null>(null);
  const [showMachineSelector, setShowMachineSelector] = useState(false);
  const [isSidebarCollapsed, setIsSidebarCollapsed] = useState(false);

  const [threads, setThreads] = useState<ThreadSession[]>([]);
  const [activeThreadId, setActiveThreadId] = useState<string | null>(null);
  const [workingMemory, setWorkingMemory] = useState<FileReference[]>([]);

  const [isEnvModalOpen, setIsEnvModalOpen] = useState(false);
  const [envForm, setEnvForm] = useState({
    id: "", name: "", connection: "", authType: "key", keyPath: "", secret: "", agents: [] as string[],
  });

  const [isNewThreadModalOpen, setIsNewThreadModalOpen] = useState(false);

  const [installPrompt, setInstallPrompt] = useState<{
    threadId: string; agentKind: string; binary: string; installCommand: string; machineName: string;
  } | null>(null);

  const [workspaceMode, setWorkspaceMode] = useState<string>("supervisor");
  const [inspectedFile, setInspectedFile] = useState<{ name: string; content: string } | null>(null);

  // Command selector state — null = closed, object = open with config
  const [selectorConfig, setSelectorConfig] = useState<{
    title: string;
    currentLabel?: string;
    options: Array<{ value: string; label: string; description?: string; current?: boolean }>;
    onSelect: (value: string) => void;
  } | null>(null);

  // ==== Message-based conversation system ====
  // Only 'user' and 'assistant' roles are persisted.
  // 'system' messages (info, error, status) are shown but transient.
  const [messages, setMessages] = useState<Record<string, Message[]>>({});
  // Transient intercept cards (not persisted across restarts)
  const [intercepts, setIntercepts] = useState<Record<string, InterceptCard[]>>({});
  // In-memory buffer for streaming assistant text deltas
  const pendingAssistantText = useRef<Record<string, string>>({});
  // Auto-inspector bookkeeping
  const lastStreamEventAt = useRef<Record<string, number>>({});
  const lastInspectedPath = useRef<string | null>(null);

  const [supervisorInput, setSupervisorInput] = useState("");

  // Strict Mode guard: only load messages once per machine selection
  const hasLoadedKey = useRef<string | null>(null);

  // ==== Message primitives ====

  const appendMessage = (threadId: string, msg: Message, skipPersist = false) => {
    setMessages((prev) => ({
      ...prev,
      [threadId]: [...(prev[threadId] || []), msg],
    }));
    if (!skipPersist) {
      invoke("append_message", { message: msg }).catch(console.error);
    }
  };

  const setActiveThreadIdAndPersist = (id: string | null) => {
    setActiveThreadId(id);
    if (id) {
      invoke("set_app_session", { key: "active_thread_id", value: id }).catch(() => {});
    }
  };

  useEffect(() => {
    loadMachines();
  }, []);

  useEffect(() => {
    agentSessionRegistry.ensureInstalled().catch(console.error);
  }, []);

  // Persist active_machine_id and active_thread_id on visibility change or before unload.
  useEffect(() => {
    const persist = () => {
      if (activeMachine) {
        invoke("set_app_session", { key: "active_machine_id", value: activeMachine.id }).catch(() => {});
      }
      if (activeThreadId) {
        invoke("set_app_session", { key: "active_thread_id", value: activeThreadId }).catch(() => {});
      }
    };
    const onVisibilityChange = () => {
      if (document.visibilityState === "hidden") persist();
    };
    window.addEventListener("beforeunload", persist);
    document.addEventListener("visibilitychange", onVisibilityChange);
    return () => {
      window.removeEventListener("beforeunload", persist);
      document.removeEventListener("visibilitychange", onVisibilityChange);
    };
  }, [activeMachine, activeThreadId]);

  // ==== Tauri event listeners ====

  useEffect(() => {
    const unlistens: Array<Promise<() => void>> = [];

    unlistens.push(
      listen<InterceptPayload>("permission_requested", (e) => {
        const p = e.payload;
        if (!p?.intercept_id) return;
        setIntercepts((prev) => {
          const list = prev[p.thread_id] || [];
          if (list.some((c) => c.intercept_id === p.intercept_id)) return prev;
          return {
            ...prev,
            [p.thread_id]: [
              ...list,
              {
                id: crypto.randomUUID(),
                thread_id: p.thread_id,
                intercept_id: p.intercept_id,
                action: p.action,
                target: p.target,
                code: p.preview ?? "",
                created_at: p.created_at,
                tool_call_id: p.tool_call_id,
                status: 'pending',
              },
            ],
          };
        });
        invoke("update_thread_status", { id: p.thread_id, status: "pending_approval" }).catch(console.error);
      }),
    );

    unlistens.push(
      listen<{ thread_id: string; machine_id: string; result: ExecutionResult; intercept_id?: string | null }>(
        "command_executed",
        (e) => {
          const { thread_id, result, intercept_id } = e.payload;
          if (!thread_id || !result) return;

          if (intercept_id) {
            setIntercepts((prev) => {
              const list = prev[thread_id] || [];
              return {
                ...prev,
                [thread_id]: list.filter((c) => c.intercept_id !== intercept_id),
              };
            });
          }

          const now = Date.now();
          if (result.kind === "bash") {
            appendMessage(thread_id, {
              id: crypto.randomUUID(),
              thread_id,
              role: "system",
              content: result.output || "(no output)",
              metadata: { action: "bash_result" },
              created_at: now,
            }, true);
          } else if (result.kind === "file_changed") {
            appendMessage(thread_id, {
              id: crypto.randomUUID(),
              thread_id,
              role: "system",
              content: `Edited \`${result.path}\` (+${result.lines_added} -${result.lines_removed})`,
              metadata: { action: "file_changed", path: result.path },
              created_at: now,
            }, true);
          } else if (result.kind === "file_read") {
            appendMessage(thread_id, {
              id: crypto.randomUUID(),
              thread_id,
              role: "system",
              content: `Read \`${result.path}\``,
              metadata: { action: "file_read", path: result.path },
              created_at: now,
            }, true);
          }
          invoke("update_thread_status", { id: thread_id, status: "running" }).catch(console.error);
        },
      ),
    );

    unlistens.push(
      listen<ThreadStatusChangedEvent>("thread_status_changed", (e) => {
        const { thread_id, status } = e.payload;
        if (!thread_id || !status) return;
        setThreads((prev) =>
          prev.map((t) => (t.id === thread_id ? { ...t, status } : t)),
        );
        agentSessionRegistry.setStatus(thread_id, status as any);
        if (status === "idle" && thread_id === activeThreadId) {
          loadWorkingMemory(thread_id)
            .then((entries) => {
              setWorkingMemory(
                entries.map((e) => ({
                  name: e.file_path,
                  lines: e.line_count ?? 0,
                  type: "file",
                })),
              );
            })
            .catch(console.error);
        }
      }),
    );

    return () => {
      unlistens.forEach((p) => p.then((u) => u()).catch(console.error));
    };
  }, [activeThreadId]);

  // Subscribe to per-thread agent events
  useEffect(() => {
    const unsubs: Array<() => void> = [];
    for (const t of threads) {
      const u = agentSessionRegistry.subscribe(t.id, (ev) =>
        handleAgentEvent(t.id, ev),
      );
      unsubs.push(u);
    }
    return () => {
      unsubs.forEach((u) => u());
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [threads.map((t) => t.id).join("|")]);

  // ==== Machine & thread loading ====

  const loadMachines = async () => {
    try {
      const list: Machine[] = await invoke("get_machines");
      const mapped = list.map(mapMachineToFrontend);
      setMachinesList(mapped);

      const savedMachineId: string | null = await invoke<string | null>("get_app_session", { key: "active_machine_id" })
        .catch(() => null);
      const machineToSelect = savedMachineId
        ? mapped.find((m) => m.id === savedMachineId) ?? mapped[0]
        : mapped[0];
      if (machineToSelect) {
        handleMachineSelect(machineToSelect, false);
      }
    } catch (err) {
      console.error("Failed to load nodes:", err);
    }
  };

  const handleMachineSelect = async (m: FrontendMachine, shouldPersist = true) => {
    setActiveMachine(m);
    setShowMachineSelector(false);

    const loadKey = m.id;
    if (hasLoadedKey.current === loadKey) return;
    hasLoadedKey.current = loadKey;

    if (shouldPersist) {
      invoke("set_app_session", { key: "active_machine_id", value: m.id }).catch(() => {});
    }

    try {
      const threadList: ThreadSession[] = await invoke("get_thread_sessions", { machineId: m.id });
      setThreads(threadList);

      const savedThreadId: string | null = shouldPersist
        ? await invoke<string | null>("get_app_session", { key: "active_thread_id" }).catch(() => null)
        : null;
      const threadToSelect = savedThreadId && threadList.some((t) => t.id === savedThreadId)
        ? savedThreadId
        : threadList.length > 0
          ? threadList[0].id
          : null;
      setActiveThreadId(threadToSelect);
      if (shouldPersist && threadToSelect) {
        invoke("set_app_session", { key: "active_thread_id", value: threadToSelect }).catch(() => {});
      }

      // Load persisted messages for all threads
      const loadedMessages: Record<string, Message[]> = {};
      await Promise.all(
        threadList.map(async (t) => {
          try {
            const msgs: Message[] = await invoke("get_messages", { threadId: t.id });
            if (msgs.length > 0) {
              loadedMessages[t.id] = msgs;
            }
          } catch {}
        }),
      );
      if (Object.keys(loadedMessages).length > 0) {
        setMessages((prev) => ({ ...prev, ...loadedMessages }));
      }
    } catch (err) {
      console.error(err);
    }
  };

  // Refresh working memory on thread switch
  useEffect(() => {
    if (!activeThreadId) {
      setWorkingMemory([]);
      setInspectedFile(null);
      return;
    }
    loadWorkingMemory(activeThreadId)
      .then((entries) => {
        setWorkingMemory(
          entries.map((e) => ({
            name: e.file_path,
            lines: e.line_count ?? 0,
            type: "file",
          })),
        );
      })
      .catch((e) => {
        console.error("Failed to load working memory:", e);
        setWorkingMemory([]);
      });
  }, [activeThreadId]);

  // ==== Thread CRUD ====

  const launchThread = async (
    title: string,
    mode: string,
    branch: string,
    repoPath: string,
    agentKind: string | null,
  ) => {
    if (!activeMachine) return;
    const id = "t_" + Date.now();
    const sandboxPath =
      mode === "worktree"
        ? `${repoPath}/.demeteo/worktrees/${branch.replace(/\//g, "-")}`
        : undefined;

    const threadData: ThreadSession = {
      id,
      machine_id: activeMachine.id,
      title: title || "Feature Sandbox",
      mode,
      branch: mode === "worktree" ? branch : undefined,
      repo_path: mode === "worktree" ? repoPath : undefined,
      sandbox_path: sandboxPath,
      status: "spawning",
      agent_kind: agentKind,
    };

    try {
      await invoke("add_thread_session", { thread: threadData });
      setIsNewThreadModalOpen(false);

      const now = Date.now();
      appendMessage(id, {
        id: crypto.randomUUID(),
        thread_id: id,
        role: "system",
        content: `Workspace sandbox provisioned. Mode: ${mode.toUpperCase()}`,
        metadata: null,
        created_at: now,
      }, true);

      const threadList: ThreadSession[] = await invoke("get_thread_sessions", {
        machineId: activeMachine.id,
      });
      setThreads(threadList);
      setActiveThreadId(id);

      if (agentKind) {
        agentSessionRegistry.setStatus(id, "spawning");
        try {
          await agentStart(id, agentKind);
          agentSessionRegistry.setStatus(id, "idle");
          setThreads((prev) =>
            prev.map((t) => (t.id === id ? { ...t, status: "idle" } : t)),
          );
        } catch (e) {
          const msg = String(e);
          const parsed = parseNotFoundError(msg);
          if (parsed) {
            setInstallPrompt({
              threadId: id, agentKind,
              binary: parsed.binary, installCommand: parsed.install_command,
              machineName: activeMachine.name,
            });
            return;
          }
          agentSessionRegistry.setStatus(id, "error");
          setThreads((prev) =>
            prev.map((t) => (t.id === id ? { ...t, status: "error" } : t)),
          );
          appendMessage(id, {
            id: crypto.randomUUID(),
            thread_id: id,
            role: "system",
            content: `agent_start failed: ${msg}`,
            metadata: { is_error: true },
            created_at: Date.now(),
          }, true);
        }
      } else {
        agentSessionRegistry.setStatus(id, "idle");
        setThreads((prev) =>
          prev.map((t) => (t.id === id ? { ...t, status: "idle" } : t)),
        );
      }
    } catch (err) {
      alert("Failed to provision worktree sandbox: " + err);
    }
  };

  const deleteThread = async (threadId: string) => {
    const ok = await ask("Remove this thread session?", {
      title: "Confirm Delete",
      kind: "warning",
    });
    if (!ok) return;
    try {
      await invoke("delete_thread_session", { id: threadId });
      setMessages((prev) => {
        const next = { ...prev };
        delete next[threadId];
        return next;
      });
      setIntercepts((prev) => {
        const next = { ...prev };
        delete next[threadId];
        return next;
      });
      if (activeMachine) {
        const list: ThreadSession[] = await invoke("get_thread_sessions", {
          machineId: activeMachine.id,
        });
        setThreads(list);
        setActiveThreadId(list.length > 0 ? list[0].id : null);
      }
    } catch (err) {
      console.error(err);
    }
  };

  // ==== Environment CRUD ====

  const openAddEnv = () => {
    setEnvForm({
      id: "", name: "", connection: "ubuntu@localhost:22",
      authType: "key", keyPath: "~/.ssh/id_rsa", secret: "", agents: [],
    });
    setIsEnvModalOpen(true);
    setShowMachineSelector(false);
  };

  const openEditEnv = (m: FrontendMachine, e: React.MouseEvent) => {
    e.stopPropagation();
    const stored: any[] = JSON.parse(m.agents || "[]");
    const enabledLabels: string[] = [];
    for (const entry of stored) {
      const kind =
        typeof entry === "string"
          ? entry.toLowerCase()
          : entry?.kind?.toLowerCase?.() ?? "";
      if (!entry?.enabled && typeof entry === "object") continue;
      const label = DISPLAY_LABEL[kind];
      if (label && !enabledLabels.includes(label)) enabledLabels.push(label);
    }
    setEnvForm({
      id: m.id, name: m.name,
      connection: `${m.username}@${m.host}:${m.port}`,
      authType: m.auth_type, keyPath: m.key_path || "", secret: "",
      agents: enabledLabels,
    });
    setIsEnvModalOpen(true);
    setShowMachineSelector(false);
  };

  const handleBrowseKey = async (): Promise<string | null> => {
    try {
      const selected = await open({ multiple: false, directory: false });
      if (selected && typeof selected === "string") return selected;
    } catch {}
    return null;
  };

  const deleteEnv = async (id: string, e: React.MouseEvent) => {
    e.stopPropagation();
    const ok = await ask("Are you sure you want to remove this connection profile?", {
      title: "Confirm Delete", kind: "warning",
    });
    if (!ok) return;
    try {
      await invoke("close_machine_sessions", { machineId: id }).catch(console.error);
      await invoke("delete_machine", { id });
      await invoke("delete_machine_secret", { machineId: id }).catch(console.error);
      const updated = machinesList.filter((m) => m.id !== id);
      setMachinesList(updated);
      if (activeMachine?.id === id && updated.length > 0) {
        handleMachineSelect(updated[0]);
      } else if (updated.length === 0) {
        setActiveMachine(null);
        setThreads([]);
      }
      setIsEnvModalOpen(false);
    } catch (err) {
      console.error(err);
    }
  };

  const saveEnv = async (form: any) => {
    let username = "ubuntu";
    let host = "localhost";
    let port = 22;
    const parts = form.connection.split("@");
    if (parts.length > 1) {
      username = parts[0];
      const hostParts = parts[1].split(":");
      host = hostParts[0];
      if (hostParts[1]) port = Number(hostParts[1]);
    } else {
      const hostParts = parts[0].split(":");
      host = hostParts[0];
      if (hostParts[1]) port = Number(hostParts[1]);
    }

    const machineData: Machine = {
      id: form.id || crypto.randomUUID(),
      name: form.name || "unnamed-node",
      host, port, username,
      auth_type: form.authType,
      key_path: form.authType === "key" ? form.keyPath : undefined,
      agents: JSON.stringify(
        (form.agents ?? [])
          .map((name: string) => {
            const lower = String(name).toLowerCase();
            return lower in DISPLAY_LABEL ? { kind: lower, enabled: true } : null;
          })
          .filter((c: any) => c !== null),
      ),
      auto_approved_rules: "[]",
    };

    try {
      if (form.id) {
        await invoke("update_machine", { machine: machineData });
      } else {
        await invoke("add_machine", { machine: machineData });
      }
      if (form.secret) {
        await invoke("set_machine_secret", { machineId: machineData.id, secret: form.secret });
      }
      setIsEnvModalOpen(false);
      loadMachines();
    } catch (err) {
      alert("Error saving connection node: " + err);
    }
  };

  const testSshConnection = async (form: any): Promise<string> => {
    try {
      let username = "ubuntu";
      let host = "localhost";
      let port = 22;
      const parts = form.connection.split("@");
      if (parts.length > 1) {
        username = parts[0];
        const hostParts = parts[1].split(":");
        host = hostParts[0];
        if (hostParts[1]) port = Number(hostParts[1]);
      } else {
        const hostParts = parts[0].split(":");
        host = hostParts[0];
        if (hostParts[1]) port = Number(hostParts[1]);
      }
      await invoke("test_ssh_connection", {
        host, port, username,
        authType: form.authType,
        keyPath: form.authType === "key" ? form.keyPath || null : null,
        secret: form.secret || null,
      });
      return "ok";
    } catch (err) {
      return String(err);
    }
  };

  // ==== Intercept actions ====

  const findInterceptId = (threadId: string, cardId: string): string | undefined => {
    return intercepts[threadId]?.find((c) => c.id === cardId)?.intercept_id;
  };

  const approveAction = async (threadId: string, cardId: string) => {
    const interceptId = findInterceptId(threadId, cardId);
    if (!interceptId) return;
    try {
      await invoke("approve_intercept", { interceptId });
      setIntercepts((prev) => {
        const list = prev[threadId] || [];
        return {
          ...prev,
          [threadId]: list.map((c) =>
            c.id === cardId ? { ...c, status: 'approved' as const } : c,
          ),
        };
      });
    } catch (err) {
      console.error("approve_intercept failed:", err);
    }
  };

  const rejectAction = async (threadId: string, cardId: string, feedback: string) => {
    const interceptId = findInterceptId(threadId, cardId);
    if (!interceptId) return;
    try {
      await invoke("reject_intercept", { interceptId, feedback });
      await invoke("update_thread_status", { id: threadId, status: "running" });
      if (activeMachine) {
        const list: ThreadSession[] = await invoke("get_thread_sessions", {
          machineId: activeMachine.id,
        });
        setThreads(list);
      }
      setIntercepts((prev) => {
        const list = prev[threadId] || [];
        return {
          ...prev,
          [threadId]: list.map((c) =>
            c.id === cardId ? { ...c, status: 'rejected' as const, feedback } : c,
          ),
        };
      });
    } catch (err) {
      console.error("reject_intercept failed:", err);
    }
  };

  const handleInspectContext = async (path: string) => {
    if (!activeMachine) return;
    try {
      const content = await invoke<string>("sftp_read_file", { machineId: activeMachine.id, path });
      setInspectedFile({ name: path, content });
      lastInspectedPath.current = path;
    } catch (e) {
      console.warn("Could not read remote file:", path, e);
      setInspectedFile(null);
    }
  };

  // ==== Send directive ====

  const sendDirective = async (threadId: string) => {
    if (!supervisorInput.trim()) return;
    const thread = threads.find((t) => t.id === threadId);
    if (!thread) return;
    const text = supervisorInput;
    setSupervisorInput("");

    const now = Date.now();

    // Slash-command routing
    if (text.startsWith("/")) {
      const parts = text.slice(1).split(/\s+/);
      const cmd = parts[0];
      const args = parts.slice(1);

      const handled = await handleSlashCommand(threadId, cmd, args, now);
      if (handled) return;
    }

    // Persist user message immediately
    const msg: Message = {
      id: crypto.randomUUID(),
      thread_id: threadId,
      role: "user",
      content: text,
      metadata: null,
      created_at: now,
    };
    appendMessage(threadId, msg);

    // Auto-name thread from first user message if title is still default
    if (thread.title === "Feature Sandbox" && (messages[threadId]?.length ?? 0) === 0) {
      const shortTitle = text.length > 60 ? text.slice(0, 57) + "..." : text;
      setThreads((prev) =>
        prev.map((t) => (t.id === threadId ? { ...t, title: shortTitle } : t)),
      );
    }

    if (thread.status === "running") {
      try {
        await agentCancel(threadId);
      } catch (e) {
        console.error("agent_cancel failed:", e);
      }
    }

    if (!thread.agent_kind) return;

    agentSessionRegistry.setStatus(threadId, "running");
    setThreads((prev) =>
      prev.map((t) => (t.id === threadId ? { ...t, status: "running" } : t)),
    );
    lastStreamEventAt.current[threadId] = Date.now();
    try {
      await agentPrompt(threadId, thread.agent_kind, text);
    } catch (e) {
      const errMsg = String(e);
      appendMessage(threadId, {
        id: crypto.randomUUID(),
        thread_id: threadId,
        role: "system",
        content: `agent_prompt failed: ${errMsg}`,
        metadata: { is_error: true },
        created_at: Date.now(),
      }, true);
      agentSessionRegistry.setStatus(threadId, "error");
      setThreads((prev) =>
        prev.map((t) => (t.id === threadId ? { ...t, status: "error" } : t)),
      );
    }
  };

  const handleSlashCommand = async (
    threadId: string,
    cmd: string,
    args: string[],
    now: number,
  ): Promise<boolean> => {
    const append = (content: string, isError = false) => {
      appendMessage(threadId, {
        id: crypto.randomUUID(),
        thread_id: threadId,
        role: "system",
        content,
        metadata: isError ? { is_error: true } : null,
        created_at: now,
      }, true);
    };

    // Fetch session info (modes, models, etc.)
    const getInfo = async (): Promise<SessionInfo | null> => {
      try {
        return await invoke<SessionInfo>("agent_get_session_info", { threadId });
      } catch {
        return null;
      }
    };

    switch (cmd) {
      case "mode": {
        const modeId = args[0];
        if (modeId) {
          try {
            await invoke("agent_set_mode", { threadId, modeId });
            append(`Switched to mode: ${modeId}`);
          } catch (e) {
            append(`Failed to switch mode: ${e}`, true);
          }
          return true;
        }
        // No arg — show picker from session info
        const info = await getInfo();
        const modes = info?.modes?.availableModes ?? [];
        const currentModeId = info?.modes?.currentModeId;
        setSelectorConfig({
          title: "Select Agent Mode",
          currentLabel: modes.find(m => m.id === currentModeId)?.name ?? currentModeId,
          options: modes.map((m) => ({
            value: m.id,
            label: m.name,
            description: m.description,
            current: m.id === currentModeId,
          })),
          onSelect: (value: string) => {
            setSelectorConfig(null);
            invoke("agent_set_mode", { threadId, modeId: value })
              .then(() => append(`Switched to mode: ${value}`))
              .catch((e) => append(`Failed to switch mode: ${e}`, true));
          },
        });
        return true;
      }
      case "model": {
        const modelName = args[0];
        if (modelName) {
          try {
            await invoke("agent_set_config_option", { threadId, configId: "model", value: modelName });
            append(`Switched model to: ${modelName}`);
          } catch (e) {
            append(`Failed to switch model: ${e}`, true);
          }
          return true;
        }
        // No arg — show picker from session info configOptions
        const info = await getInfo();
        const modelOption = info?.config_options?.find((c) => c.id === "model" || c.category === "model");
        const models = modelOption?.options ?? [];
        const currentModel = modelOption?.currentValue;
        setSelectorConfig({
          title: "Select Model",
          currentLabel: models.find(m => m.value === currentModel)?.name ?? currentModel,
          options: models.map((m) => ({
            value: m.value,
            label: m.name,
            description: m.description,
            current: m.value === currentModel,
          })),
          onSelect: (value: string) => {
            setSelectorConfig(null);
            invoke("agent_set_config_option", { threadId, configId: "model", value })
              .then(() => append(`Switched model to: ${value}`))
              .catch((e) => append(`Failed to switch model: ${e}`, true));
          },
        });
        return true;
      }
      case "help": {
        append(
          "Available commands:\n" +
          "  /mode     – Switch agent mode (plan, build, ask, code)\n" +
          "  /model    – Switch LLM model\n" +
          "  /help     – Show this help\n\n" +
          "Tip: Type /mode or /model with no arguments for an interactive picker.\n" +
          "You can also type a custom value directly in the picker.",
        );
        return true;
      }
      default:
        return false;
    }
  };

  const stopTurn = async (threadId: string) => {
    try {
      await agentCancel(threadId);
    } catch (e) {
      console.error("agent_cancel failed:", e);
    }
  };

  // ==== Agent event handler ====

  const handleAgentEvent = async (threadId: string, event: AgentEvent) => {
    const now = Date.now();
    lastStreamEventAt.current[threadId] = now;

    switch (event.kind) {
      case "text": {
        // Accumulate text deltas in-memory — NOT persisted until turn_complete
        const current = pendingAssistantText.current[threadId] ?? "";
        pendingAssistantText.current[threadId] = current + event.delta;
        // Force re-render by updating a dummy state key on messages
        setMessages((prev) => ({ ...prev, [threadId]: prev[threadId] || [] }));
        break;
      }
      case "tool_call": {
        // Auto-inspector: open inspector on first Read
        if (event.action === "read") {
          const lastAt = lastStreamEventAt.current[threadId] ?? 0;
          const isFirstOrAfterPause = !inspectedFile || (now - lastAt > 5000);
          if (isFirstOrAfterPause || inspectedFile) {
            void handleInspectContext(event.target);
          }
        }
        break;
      }
      case "tool_call_update":
      case "usage":
        break;
      case "plan": {
        const planMsg = `Plan: ${event.entries.map((e) => `${e.step} (${e.status}`).join(" → ")})`;
        const existing = messages[threadId] || [];
        if (existing.some((m) => m.role === "system" && m.content === planMsg)) break;
        appendMessage(threadId, {
          id: crypto.randomUUID(),
          thread_id: threadId,
          role: "system",
          content: planMsg,
          metadata: null,
          created_at: now,
        }, true);
        break;
      }
      case "error": {
        const errKey = `${event.code}:${event.message}`;
        const existing = messages[threadId] || [];
        if (existing.some((m) => m.role === "system" && m.content === errKey)) break;
        // Flush any pending assistant text first
        const pending = pendingAssistantText.current[threadId];
        if (pending) {
          appendMessage(threadId, {
            id: crypto.randomUUID(),
            thread_id: threadId,
            role: "assistant",
            content: pending,
            metadata: null,
            created_at: now,
          });
          pendingAssistantText.current[threadId] = "";
        }
        appendMessage(threadId, {
          id: crypto.randomUUID(),
          thread_id: threadId,
          role: "system",
          content: errKey,
          metadata: { is_error: true },
          created_at: now,
        }, true);
        break;
      }
      case "mode_changed": {
        appendMessage(threadId, {
          id: crypto.randomUUID(),
          thread_id: threadId,
          role: "system",
          content: `Switched to mode: ${event.mode_id}`,
          metadata: null,
          created_at: now,
        }, true);
        break;
      }
      case "config_changed": {
        appendMessage(threadId, {
          id: crypto.randomUUID(),
          thread_id: threadId,
          role: "system",
          content: `Config ${event.config_id} → ${event.value}`,
          metadata: null,
          created_at: now,
        }, true);
        break;
      }
      case "turn_complete": {
        const pending = pendingAssistantText.current[threadId];
        if (pending) {
          // Persist the complete assistant message
          appendMessage(threadId, {
            id: crypto.randomUUID(),
            thread_id: threadId,
            role: "assistant",
            content: pending,
            metadata: null,
            created_at: now,
          });
          pendingAssistantText.current[threadId] = "";
        }
        const reason = event.stop_reason;
        const msg =
          reason === "cancelled"
            ? "[cancelled by user]"
            : reason === "max_tokens"
            ? "Turn complete: max tokens reached."
            : reason === "error"
            ? "Turn complete: error."
            : "Turn complete.";
        const existing = messages[threadId] || [];
        if (existing.some((m) => m.role === "system" && m.content === msg)) break;
        appendMessage(threadId, {
          id: crypto.randomUUID(),
          thread_id: threadId,
          role: "system",
          content: msg,
          metadata: null,
          created_at: now,
        }, true);
        // Trigger working memory refresh is done in thread_status_changed listener
        break;
      }
    }
  };

  // ==== Install flow ====

  const approveInstall = async () => {
    if (!installPrompt) return;
    const { threadId, agentKind, machineName } = installPrompt;
    setInstallPrompt(null);
    agentSessionRegistry.setStatus(threadId, "installing");
    setThreads((prev) =>
      prev.map((t) => (t.id === threadId ? { ...t, status: "installing" } : t)),
    );
    appendMessage(threadId, {
      id: crypto.randomUUID(),
      thread_id: threadId,
      role: "system",
      content: `Installing ${agentKind} on ${machineName}…`,
      metadata: null,
      created_at: Date.now(),
    }, true);
    try {
      await agentInstallAndStart(threadId, agentKind);
      agentSessionRegistry.setStatus(threadId, "idle");
      setThreads((prev) =>
        prev.map((t) => (t.id === threadId ? { ...t, status: "idle" } : t)),
      );
    } catch (e) {
      agentSessionRegistry.setStatus(threadId, "error");
      setThreads((prev) =>
        prev.map((t) => (t.id === threadId ? { ...t, status: "error" } : t)),
      );
      appendMessage(threadId, {
        id: crypto.randomUUID(),
        thread_id: threadId,
        role: "system",
        content: `install failed: ${e}`,
        metadata: { is_error: true },
        created_at: Date.now(),
      }, true);
    }
  };

  const cancelInstall = () => {
    if (!installPrompt) return;
    const { threadId, agentKind } = installPrompt;
    setInstallPrompt(null);
    agentSessionRegistry.setStatus(threadId, "error");
    setThreads((prev) =>
      prev.map((t) =>
        t.id === threadId ? { ...t, status: "error", agent_kind: null } : t,
      ),
    );
    appendMessage(threadId, {
      id: crypto.randomUUID(),
      thread_id: threadId,
      role: "system",
      content: `Install of ${agentKind} declined. Thread is in error state; use "Restart thread" to retry.`,
      metadata: { is_error: true },
      created_at: Date.now(),
    }, true);
  };

  return (
    <div className="flex h-screen bg-[#050508] text-slate-300 font-sans selection:bg-cyan-500/30 w-full overflow-hidden">
      <Sidebar
        isCollapsed={isSidebarCollapsed}
        machinesList={machinesList}
        activeMachine={activeMachine}
        showMachineSelector={showMachineSelector}
        setShowMachineSelector={setShowMachineSelector}
        onMachineSelect={handleMachineSelect}
        onAddEnv={openAddEnv}
        onEditEnv={openEditEnv}
        onDeleteEnv={deleteEnv}
        threads={threads}
        activeThreadId={activeThreadId}
        onThreadSelect={setActiveThreadIdAndPersist}
        setWorkspaceMode={setWorkspaceMode}
        onNewThreadClick={() => setIsNewThreadModalOpen(true)}
        onDeleteThread={deleteThread}
        workingMemory={workingMemory}
        inspectedFileName={inspectedFile?.name}
        onInspectFile={handleInspectContext}
      />

      <div className="flex-1 flex flex-col min-w-0 bg-slate-950/40 relative shadow-2xl z-20 h-full">
        <div className="h-14 border-b border-white/5 bg-[#0a0a0e]/50 backdrop-blur-md flex items-center justify-between px-4 z-10 select-none">
          <div className="flex items-center gap-3">
            <button
              type="button"
              onClick={() => setIsSidebarCollapsed(!isSidebarCollapsed)}
              className="p-1.5 rounded-lg text-slate-400 hover:text-slate-200 hover:bg-white/5 transition-all flex items-center justify-center"
              title={isSidebarCollapsed ? "Expand Sidebar" : "Collapse Sidebar"}
            >
              <Menu size={18} />
            </button>
            <div className="flex items-center gap-1.5 bg-slate-900/80 p-1 rounded-lg border border-white/5 shadow-inner">
              <button
                type="button"
                onClick={() => { setWorkspaceMode("supervisor"); }}
                className={`px-4 py-1.5 rounded-md text-xs font-medium transition-all duration-200 flex items-center hover:scale-[1.03] active:scale-[0.97] cursor-pointer ${
                  workspaceMode === "supervisor"
                    ? "bg-cyan-500/20 text-cyan-400 shadow-[0_0_12px_rgba(6,182,212,0.15)] border border-cyan-500/25"
                    : "text-slate-400 hover:text-slate-200 border border-transparent"
                }`}
              >
                <Activity size={14} className="mr-2" /> Supervisor Plane
              </button>
              <button
                type="button"
                onClick={() => { setWorkspaceMode("terminal"); setInspectedFile(null); }}
                className={`px-4 py-1.5 rounded-md text-xs font-medium transition-all duration-200 flex items-center hover:scale-[1.03] active:scale-[0.97] cursor-pointer ${
                  workspaceMode === "terminal"
                    ? "bg-slate-800 text-white shadow-[0_0_12px_rgba(255,255,255,0.05)] border border-white/10"
                    : "text-slate-400 hover:text-slate-200 border border-transparent"
                }`}
              >
                <Terminal size={14} className="mr-2" /> Terminal
              </button>
            </div>
          </div>

          <div className="flex items-center gap-4">
            <div className="flex items-center text-xs font-mono text-emerald-400 bg-emerald-500/10 px-2 py-1 rounded border border-emerald-500/20">
              <ShieldAlert size={12} className="mr-1.5" /> Proxy Active
            </div>
            {activeMachine && (
              <button
                onClick={(e) => openEditEnv(activeMachine, e)}
                className="text-slate-400 hover:text-white transition-colors"
                title="Configure Node"
              >
                <Settings size={16} />
              </button>
            )}
          </div>
        </div>

        <div className="flex-1 flex flex-col md:flex-row min-h-0 overflow-hidden w-full">
          <div
            className={`flex flex-col min-w-0 transition-all duration-300 h-full ${
              inspectedFile && workspaceMode === "supervisor"
                ? "w-full h-1/2 md:w-[55%] md:h-full border-b md:border-b-0 md:border-r border-white/5"
                : "w-full"
            }`}
          >
            {workspaceMode === "supervisor" ? (
              <SupervisorPlane
                activeThreadId={activeThreadId}
                threads={threads}
                messages={messages}
                intercepts={intercepts}
                pendingAssistantText={pendingAssistantText.current}
                supervisorInput={supervisorInput}
                setSupervisorInput={setSupervisorInput}
                onSendDirective={(tid) => void sendDirective(tid)}
                onStopTurn={stopTurn}
                onInspectContext={handleInspectContext}
                onApproveAction={approveAction}
                onRejectAction={rejectAction}
                activeMachineId={activeMachine?.id ?? null}
              />
            ) : (
              <div className="flex-1 bg-[#050508] p-1 overflow-hidden h-full">
                {activeMachine ? (
                  <TerminalTabs machineId={activeMachine.id} host={activeMachine.host} />
                ) : (
                  <div className="flex flex-col justify-center items-center h-full text-slate-500">
                    <Terminal size={32} className="mb-2" />
                    <div>No active target connection node.</div>
                  </div>
                )}
              </div>
            )}
          </div>

          {inspectedFile && workspaceMode === "supervisor" && (
            <CodeInspector
              fileName={inspectedFile.name}
              fileContent={inspectedFile.content}
              onRefresh={() => handleInspectContext(inspectedFile.name)}
              onClose={() => setInspectedFile(null)}
            />
          )}
        </div>
      </div>

      <NewThreadModal
        isOpen={isNewThreadModalOpen}
        onClose={() => setIsNewThreadModalOpen(false)}
        onLaunch={launchThread}
        machineId={activeMachine?.id ?? null}
      />

      <EnvModal
        isOpen={isEnvModalOpen}
        onClose={() => setIsEnvModalOpen(false)}
        initialData={envForm}
        onSave={saveEnv}
        onDelete={(id) => {
          const dummyEvent = { stopPropagation: () => {} } as any;
          return deleteEnv(id, dummyEvent);
        }}
        onBrowseKey={handleBrowseKey}
        onTestConnection={testSshConnection}
      />

      <CommandSelector
        title={selectorConfig?.title ?? ""}
        currentLabel={selectorConfig?.currentLabel}
        options={selectorConfig?.options ?? []}
        isOpen={selectorConfig !== null}
        freeform
        placeholder="Type a value or pick from the list..."
        onSelect={(value) => selectorConfig?.onSelect(value)}
        onClose={() => setSelectorConfig(null)}
      />

      {installPrompt && (
        <div className="fixed inset-0 bg-black/60 backdrop-blur-sm flex items-center justify-center z-50 p-4 select-none">
          <div className="bg-[#0a0a0e] border border-white/10 rounded-2xl w-full max-w-md shadow-2xl overflow-hidden animate-in fade-in zoom-in-95 duration-200">
            <div className="px-6 py-4 border-b border-white/5 flex items-center bg-[#050508]">
              <AlertCircle size={16} className="mr-2 text-amber-400" />
              <h3 className="text-sm font-semibold text-white">
                Install {installPrompt.agentKind} on {installPrompt.machineName}?
              </h3>
            </div>
            <div className="p-6 flex flex-col gap-4">
              <p className="text-xs text-slate-400 leading-relaxed">
                The following official script will be run over SSH to install the
                agent runtime. The remote shell is the same as the worktree's host.
              </p>
              <pre className="bg-[#050508] border border-white/5 rounded-lg p-3 text-[11px] font-mono text-cyan-300 overflow-x-auto whitespace-pre-wrap break-all">
                {installPrompt.installCommand}
              </pre>
              <p className="text-[10px] text-slate-500 font-mono">
                Binary: <span className="text-amber-400">{installPrompt.binary}</span>
              </p>
            </div>
            <div className="px-6 py-4 border-t border-white/5 bg-[#050508] flex justify-end gap-3">
              <button
                type="button"
                onClick={cancelInstall}
                className="px-4 py-2 rounded-lg text-xs font-medium text-slate-400 hover:text-white transition-colors"
              >
                Cancel
              </button>
              <button
                type="button"
                onClick={approveInstall}
                className="px-5 py-2 rounded-lg text-xs font-bold bg-cyan-500 text-slate-950 hover:bg-cyan-400 transition-all"
              >
                Install and continue
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

export default App;
