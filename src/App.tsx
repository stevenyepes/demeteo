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
  StreamEvent,
  InterceptPayload,
  ExecutionResult,
  AgentEvent,
  ThreadStatusChangedEvent,
} from "./types";
import TerminalTabs from "./components/TerminalTabs";
import Sidebar from "./components/Sidebar";
import SupervisorPlane from "./components/SupervisorPlane";
import CodeInspector from "./components/CodeInspector";
import NewThreadModal from "./components/NewThreadModal";
import EnvModal from "./components/EnvModal";
import {
  agentSessionRegistry,
  agentStart,
  agentInstallAndStart,
  agentPrompt,
  agentCancel,
  loadWorkingMemory,
} from "./agentSessionRegistry";

/** Display label used in the EnvModal toggle, keyed by the lowercase
 *  adapter kind stored in the backend. Keep in sync with
 *  EnvModal's `["Claude Code", "OpenCode", "Hermes"]` chip list. */
const DISPLAY_LABEL: Record<string, string> = {
  opencode: "OpenCode",
  hermes: "Hermes",
};

const mapMachineToFrontend = (m: Machine): FrontendMachine => {
  return {
    ...m,
    type: m.auth_type === "local" ? "local" : "server",
    status: "connected",
    user: `${m.username}@${m.host}`,
  };
};

/** Parse the structured `NOT_FOUND:binary:install_command` error
 *  emitted by the `agent_start` Tauri command on a missing binary.
 *  Returns `{ binary, install_command }` or `null` if the error
 *  doesn't match the marker. */
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
    id: "",
    name: "",
    connection: "",
    authType: "key",
    keyPath: "",
    secret: "",
    agents: [] as string[],
  });

  const [isNewThreadModalOpen, setIsNewThreadModalOpen] = useState(false);

  // Install-consent modal state. When a thread's launch hits
  // `agent_start` with a missing binary, we surface the
  // install_command in this modal. The user clicks "Install and
  // continue" to invoke `agent_install_and_start`.
  const [installPrompt, setInstallPrompt] = useState<{
    threadId: string;
    agentKind: string;
    binary: string;
    installCommand: string;
    machineName: string;
  } | null>(null);

  const [workspaceMode, setWorkspaceMode] = useState<string>("supervisor");
  const [inspectedFile, setInspectedFile] = useState<{ name: string; content: string } | null>(null);

  const [streams, setStreams] = useState<Record<string, StreamEvent[]>>({});
  const [supervisorInput, setSupervisorInput] = useState("");

  // Per-thread sequence counter for ordering persisted events.
  // seqRef tracks the highest seq seen (from DB or in-flight) so that
  // new events get seqRef + 1 and sort correctly after restored history.
  const seqRef = useRef<Record<string, number>>({});

  // Tracks the next seq to assign. Updated whenever seqRef is updated.
  const maxSeqRef = useRef<Record<string, number>>({});

  // Persistable event types. `text` deltas are included because they
  // are the agent's actual responses — there is no other way to recover
  // them after a restart. `tool_call_update` is excluded as it is
  // transient UI feedback that has no lasting value.
  const PERSISTABLE_TYPES = new Set(["info", "directive", "agent_error", "intercept", "tool_call", "plan", "turn_complete", "text"]);

  const setActiveThreadIdAndPersist = (id: string | null) => {
    setActiveThreadId(id);
    if (id) {
      invoke("set_app_session", { key: "active_thread_id", value: id }).catch(() => {});
    }
  };

  // Wraps setStreams: fires setStreams as normal, then asynchronously
  // persists any new non-text events to the backend.
  const setStreamsAndPersist = (
    updater: React.SetStateAction<Record<string, StreamEvent[]>>,
  ) => {
    setStreams((prev) => {
      const next = typeof updater === "function" ? updater(prev) : updater;
      for (const [threadId, nextEvents] of Object.entries(next)) {
        const prevEvents = prev[threadId] || [];
        if (nextEvents.length > prevEvents.length) {
          const newEvents = nextEvents.slice(prevEvents.length);
          for (const ev of newEvents) {
            if (!PERSISTABLE_TYPES.has(ev.type)) continue;
            const newSeq = (maxSeqRef.current[threadId] ?? 0) + 1;
            maxSeqRef.current[threadId] = newSeq;
            seqRef.current[threadId] = newSeq;
            const json = JSON.stringify(ev);
            invoke("append_thread_event", { id: ev.id, threadId, eventJson: json, seq: newSeq })
              .catch(() => {});
          }
        }
      }
      return next;
    });
  };

  // Per-thread bookkeeping for the auto-inspector rule (§8.6):
  // - `inspectedFile` is already in state; we treat a non-null
  //   inspectedFile as "open" and a null as "dismissed".
  // - `lastStreamEventAt[threadId]` is the timestamp of the last
  //   event we observed. The rule fires when the gap is > 5s
  //   AND there's no currently-open inspector.
  const lastStreamEventAt = useRef<Record<string, number>>({});
  // The previous agent event's tool_call_id is not used directly,
  // but we keep `lastInspectedPath` so we can update the inspector
  // (not re-open) when the agent re-reads the same file.
  const lastInspectedPath = useRef<string | null>(null);

  useEffect(() => {
    loadMachines();
  }, []);

  // Install the global agent_event listener once.
  useEffect(() => {
    agentSessionRegistry
      .ensureInstalled()
      .catch(console.error);
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

  useEffect(() => {
    const unlistens: Array<Promise<() => void>> = [];

    unlistens.push(
      listen<InterceptPayload>("permission_requested", (e) => {
        const p = e.payload;
        if (!p?.intercept_id) return;
        setStreamsAndPersist((prev) => {
          const list = prev[p.thread_id] || [];
          if (list.some((ev) => ev.payload?.intercept_id === p.intercept_id)) return prev;
          const message =
            p.action === "run_bash"
              ? `Intercepted: agent wants to run \`${p.target}\``
              : p.action === "edit"
              ? `Intercepted: agent wants to edit \`${p.target}\``
              : p.action === "write"
              ? `Intercepted: agent wants to write \`${p.target}\``
              : `Intercepted: agent wants to read \`${p.target}\``;
          return {
            ...prev,
            [p.thread_id]: [
              ...list,
              {
                id: crypto.randomUUID(),
                type: "intercept",
                message,
                timestamp: new Date().toLocaleTimeString(),
                payload: {
                  intercept_id: p.intercept_id,
                  action: p.action,
                  path: p.target,
                  code: p.preview ?? "",
                  created_at: p.created_at,
                  tool_call_id: p.tool_call_id,
                },
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
          setStreamsAndPersist((prev) => {
            // Remove the matching intercept card (if any). This covers both
            // the auto-approve path (intercept_id is null) and the escalated
            // path where the user approved/rejected — the card becomes stale
            // the moment execution completes.
            const baseList = intercept_id
              ? (prev[thread_id] || []).filter(
                  (ev) => !(ev.type === "intercept" && ev.payload?.intercept_id === intercept_id),
                )
              : prev[thread_id] || [];

            let event: StreamEvent;
            if (result.kind === "bash") {
              event = {
                id: crypto.randomUUID(),
                type: "auto_approve",
                message: result.output || "(no output)",
                timestamp: new Date().toLocaleTimeString(),
              };
            } else if (result.kind === "file_changed") {
              event = {
                id: crypto.randomUUID(),
                type: "info",
                message: `Edited \`${result.path}\` (+${result.lines_added} -${result.lines_removed})`,
                timestamp: new Date().toLocaleTimeString(),
              };
            } else if (result.kind === "file_read") {
              event = {
                id: crypto.randomUUID(),
                type: "info",
                message: `Read \`${result.path}\` (${result.content_preview.split("\n").length} lines)`,
                timestamp: new Date().toLocaleTimeString(),
              };
            } else {
              return intercept_id ? { ...prev, [thread_id]: baseList } : prev;
            }
            return { ...prev, [thread_id]: [...baseList, event] };
          });
          invoke("update_thread_status", { id: thread_id, status: "running" }).catch(console.error);
        },
      ),
    );

    // `thread_status_changed` is the backend's authoritative status
    // update (it can correct the frontend's optimistic state). We
    // update both the in-memory thread and the registry mirror.
    unlistens.push(
      listen<ThreadStatusChangedEvent>("thread_status_changed", (e) => {
        const { thread_id, status } = e.payload;
        if (!thread_id || !status) return;
        setThreads((prev) =>
          prev.map((t) => (t.id === thread_id ? { ...t, status } : t)),
        );
        agentSessionRegistry.setStatus(thread_id, status as any);
        // On turn complete (running -> idle), refresh working memory
        // for the active thread in case the agent read files we
        // should track.
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

  // Subscribe to per-thread agent events. Whenever a new AgentEvent
  // comes in for a thread, we dispatch it: append to the stream,
  // trigger the auto-inspector on first Read, refresh working memory
  // on ToolCall, flip status on TurnComplete.
  useEffect(() => {
    const unsubs: Array<() => void> = [];
    for (const t of threads) {
      const u = agentSessionRegistry.subscribe(t.id, (ev) =>
        handleAgentEvent(t.id, t.agent_kind ?? null, ev),
      );
      unsubs.push(u);
    }
    return () => {
      unsubs.forEach((u) => u());
    };
    // We intentionally only re-subscribe when the set of thread ids
    // changes; the per-event handler is stable.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [threads.map((t) => t.id).join("|")]);

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

      const events: Record<string, StreamEvent[]> = {};
      const loadedSeqs: Record<string, number> = { ...seqRef.current };
      const loadedMaxSeqs: Record<string, number> = { ...maxSeqRef.current };
      await Promise.all(
        threadList.map(async (t) => {
          try {
            const raw: [StreamEvent, number][] = await invoke("get_thread_events", { threadId: t.id });
            if (raw.length > 0) {
              const [eventList, maxSeq] = raw.reduce(
                ([evts, max], [evt, seq]) => {
                  evts.push(evt);
                  return [evts, Math.max(max, seq)];
                },
                [[] as StreamEvent[], 0] as [StreamEvent[], number],
              );
              events[t.id] = eventList;
              loadedSeqs[t.id] = maxSeq;
              loadedMaxSeqs[t.id] = maxSeq;
            }
          } catch {}
        }),
      );
      if (Object.keys(events).length > 0) {
        seqRef.current = { ...seqRef.current, ...loadedSeqs };
        maxSeqRef.current = { ...maxSeqRef.current, ...loadedMaxSeqs };
        setStreams((prev) => ({ ...prev, ...events }));
      }
    } catch (err) {
      console.error(err);
    }
  };

  // Working memory is populated from the DB; refresh on thread switch.
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

  const openAddEnv = () => {
    setEnvForm({
      id: "",
      name: "",
      connection: "ubuntu@localhost:22",
      authType: "key",
      keyPath: "~/.ssh/id_rsa",
      secret: "",
      agents: [],
    });
    setIsEnvModalOpen(true);
    setShowMachineSelector(false);
  };

  const openEditEnv = (m: FrontendMachine, e: React.MouseEvent) => {
    e.stopPropagation();
    // The stored `agents` field is a JSON array of {kind, enabled}
    // records. Normalize both shapes we may encounter on disk (legacy
    // bare strings, structured objects) into the display labels the
    // EnvModal toggle UI keys on. Disabled entries are dropped — the
    // UI only surfaces the "on" state, and re-enabling writes a
    // fresh {kind, enabled:true} record on save.
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
      id: m.id,
      name: m.name,
      connection: `${m.username}@${m.host}:${m.port}`,
      authType: m.auth_type,
      keyPath: m.key_path || "",
      secret: "",
      agents: enabledLabels,
    });
    setIsEnvModalOpen(true);
    setShowMachineSelector(false);
  };

  const handleBrowseKey = async (): Promise<string | null> => {
    try {
      const selected = await open({
        multiple: false,
        directory: false,
      });
      if (selected && typeof selected === "string") {
        return selected;
      }
    } catch (err) {
      console.error("Failed to select key file:", err);
    }
    return null;
  };

  const deleteEnv = async (id: string, e: React.MouseEvent) => {
    e.stopPropagation();
    const ok = await ask("Are you sure you want to remove this connection profile?", {
      title: "Confirm Delete",
      kind: "warning",
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
      host,
      port,
      username,
      auth_type: form.authType,
      key_path: form.authType === "key" ? form.keyPath : undefined,
      agents: JSON.stringify(
        (form.agents ?? [])
          .map((name: string) => {
            // Resolve the display label back to the canonical lowercase
            // kind. Unknown labels (e.g. "Claude Code", which has no
            // registered adapter) are dropped at this layer — the
            // backend migration does the same pass on read.
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
      mode: mode,
      branch: mode === "worktree" ? branch : undefined,
      repo_path: mode === "worktree" ? repoPath : undefined,
      sandbox_path: sandboxPath,
      status: "spawning",
      agent_kind: agentKind,
    };

    try {
      await invoke("add_thread_session", { thread: threadData });
      setIsNewThreadModalOpen(false);

      setStreamsAndPersist((prev) => ({
        ...prev,
        [id]: [
          {
            id: crypto.randomUUID(),
            type: "info",
            message: `Workspace sandbox provisioned. Mode: ${mode.toUpperCase()}`,
            timestamp: new Date().toLocaleTimeString(),
          },
        ],
      }));

      const threadList: ThreadSession[] = await invoke("get_thread_sessions", {
        machineId: activeMachine.id,
      });
      setThreads(threadList);
      setActiveThreadId(id);

      // Eagerly start the agent if the user picked one. Per spec §5.3,
      // this is the moment we hit NotFound and surface the install
      // consent flow.
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
            // Surface the install consent modal.
            setInstallPrompt({
              threadId: id,
              agentKind,
              binary: parsed.binary,
              installCommand: parsed.install_command,
              machineName: activeMachine.name,
            });
            return;
          }
          // Some other error — surface in the stream and mark as
          // error state. The user can restart from the supervisor.
          agentSessionRegistry.setStatus(id, "error");
          setThreads((prev) =>
            prev.map((t) => (t.id === id ? { ...t, status: "error" } : t)),
          );
          setStreamsAndPersist((prev) => ({
            ...prev,
            [id]: [
              ...(prev[id] || []),
              {
                id: crypto.randomUUID(),
                type: "agent_error",
                message: `agent_start failed: ${msg}`,
                timestamp: new Date().toLocaleTimeString(),
              },
            ],
          }));
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
      setStreamsAndPersist((prev) => {
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
        host,
        port,
        username,
        authType: form.authType,
        keyPath: form.authType === "key" ? form.keyPath || null : null,
        secret: form.secret || null,
      });
      return "ok";
    } catch (err) {
      return String(err);
    }
  };

  const handleInspectContext = async (path: string) => {
    if (!activeMachine) return;
    try {
      const content = await invoke<string>("sftp_read_file", {
        machineId: activeMachine.id,
        path,
      });
      setInspectedFile({ name: path, content });
      lastInspectedPath.current = path;
    } catch (e) {
      console.warn("Could not read remote file:", path, e);
      setInspectedFile(null);
    }
  };

  const approveAction = async (threadId: string, eventId: string) => {
    const stream = streams[threadId] || [];
    const event = stream.find((e) => e.id === eventId);
    const interceptId = event?.payload?.intercept_id;
    if (!interceptId) {
      console.warn("No intercept_id on event", eventId);
      return;
    }
    try {
      await invoke("approve_intercept", { interceptId });
      setStreamsAndPersist((prev) => {
        const list = prev[threadId] || [];
        return {
          ...prev,
          [threadId]: list.map((e) =>
            e.id === eventId
              ? { ...e, type: "info" as const, message: `${e.message} (Approved by Supervisor)` }
              : e,
          ),
        };
      });
    } catch (err) {
      console.error("approve_intercept failed:", err);
    }
  };

  const rejectAction = async (threadId: string, eventId: string, feedback: string) => {
    const stream = streams[threadId] || [];
    const event = stream.find((e) => e.id === eventId);
    const interceptId = event?.payload?.intercept_id;
    if (!interceptId) {
      console.warn("No intercept_id on event", eventId);
      return;
    }
    try {
      await invoke("reject_intercept", { interceptId, feedback });
      await invoke("update_thread_status", { id: threadId, status: "running" });
      if (activeMachine) {
        const list: ThreadSession[] = await invoke("get_thread_sessions", {
          machineId: activeMachine.id,
        });
        setThreads(list);
      }
      setStreamsAndPersist((prev) => {
        const list = prev[threadId] || [];
        return {
          ...prev,
          [threadId]: [
            ...list.map((e) =>
              e.id === eventId
                ? { ...e, type: "info" as const, message: `${e.message} (Rejected by Supervisor)` }
                : e,
            ),
            {
              id: crypto.randomUUID(),
              type: "directive",
              message: feedback
                ? `Action rejected. Feedback returned: ${feedback}`
                : "Action rejected by Supervisor.",
              timestamp: new Date().toLocaleTimeString(),
            },
          ],
        };
      });
    } catch (err) {
      console.error("reject_intercept failed:", err);
    }
  };

  /**
   * Send a directive to the agent. Implements the §8.4 implicit
   * cancel + redirect on Enter during a running turn.
   */
  const sendDirective = async (threadId: string) => {
    if (!supervisorInput.trim()) return;
    const thread = threads.find((t) => t.id === threadId);
    if (!thread) return;
    const text = supervisorInput;
    setSupervisorInput("");

    // Optimistic: append the directive to the visible stream so the
    // user sees what they sent.
    setStreamsAndPersist((prev) => ({
      ...prev,
      [threadId]: [
        ...(prev[threadId] || []),
        {
          id: crypto.randomUUID(),
          type: "directive",
          message: text,
          timestamp: new Date().toLocaleTimeString(),
        },
      ],
    }));

    // Implicit cancel + redirect: if the agent is mid-turn, we
    // first call agent_cancel (idempotent), then send the new
    // directive.
    if (thread.status === "running") {
      try {
        await agentCancel(threadId);
      } catch (e) {
        console.error("agent_cancel failed:", e);
      }
    }

    if (!thread.agent_kind) {
      // No agent selected; we just leave the directive in the stream.
      return;
    }

    // Optimistic status: the spec says "Frontend optimistic status,
    // backend confirms".
    agentSessionRegistry.setStatus(threadId, "running");
    setThreads((prev) =>
      prev.map((t) => (t.id === threadId ? { ...t, status: "running" } : t)),
    );
    lastStreamEventAt.current[threadId] = Date.now();
    try {
      await agentPrompt(threadId, thread.agent_kind, text);
    } catch (e) {
      const msg = String(e);
      setStreamsAndPersist((prev) => ({
        ...prev,
        [threadId]: [
          ...(prev[threadId] || []),
          {
            id: crypto.randomUUID(),
            type: "agent_error",
            message: `agent_prompt failed: ${msg}`,
            timestamp: new Date().toLocaleTimeString(),
          },
        ],
      }));
      agentSessionRegistry.setStatus(threadId, "error");
      setThreads((prev) =>
        prev.map((t) => (t.id === threadId ? { ...t, status: "error" } : t)),
      );
    }
  };

  const stopTurn = async (threadId: string) => {
    try {
      await agentCancel(threadId);
    } catch (e) {
      console.error("agent_cancel failed:", e);
    }
  };

  /**
   * Per-thread AgentEvent dispatcher. The event arrives via the
   * global Tauri event bus; we apply it to the right stream and
   * trigger the auto-inspector rule for the first Read of a turn.
   */
  const handleAgentEvent = async (
    threadId: string,
    _agentKind: string | null,
    event: AgentEvent,
  ) => {
    const now = Date.now();
    lastStreamEventAt.current[threadId] = now;
    setStreamsAndPersist((prev) => {
      const list = prev[threadId] || [];
      let next: StreamEvent[] = list;
      switch (event.kind) {
        case "text": {
          // Per spec §6.4: append to the most recent text block.
          const lastEvent = list[list.length - 1];
          if (lastEvent && lastEvent.type === "text") {
            next = [
              ...list.slice(0, -1),
              {
                ...lastEvent,
                message: lastEvent.message + event.delta,
                timestamp: new Date().toLocaleTimeString(),
              },
            ];
          } else {
            next = [
              ...list,
              {
                id: crypto.randomUUID(),
                type: "text",
                message: event.delta,
                timestamp: new Date().toLocaleTimeString(),
              },
            ];
          }
          break;
        }
        case "tool_call": {
          // NOTE: The ACP runtime's ToolBridge already routes this tool call
          // through PolicyEnforcedExecutionPort.submit_agent internally.
          // By the time this event arrives on the frontend, the backend has
          // already executed or escalated the action (emitting
          // `permission_requested` if escalation is needed, or
          // `command_executed` if auto-approved). Calling `request_action`
          // again from here would create a *second* intercept for the same
          // action, producing duplicate approval cards.
          //
          // Auto-inspector: on the first Read of a turn (or after a
          // 5s+ gap), open the inspector at the file the agent is
          // about to read. Per spec §8.6.
          if (event.action === "read") {
            const lastAt = lastStreamEventAt.current[threadId] ?? 0;
            const isFirstOrAfterPause = !inspectedFile || (now - lastAt > 5000);
            if (isFirstOrAfterPause) {
              void handleInspectContext(event.target);
            } else if (inspectedFile) {
              void handleInspectContext(event.target);
            }
          }
          break;
        }
        case "tool_call_update": {
          // Forward to the matching intercept card if the user has
          // it open. v1: we don't render the tool_call_update as a
          // separate stream event; it lives on the underlying
          // intercept card.
          break;
        }
        case "plan": {
          const planMsg = `Plan: ${event.entries.map((e) => `${e.step} (${e.status})`).join(" → ")}`;
          const lastInfo = list.filter((e) => e.type === "info").pop();
          if (lastInfo?.message === planMsg) break;
          next = [
            ...list,
            {
              id: crypto.randomUUID(),
              type: "info",
              message: planMsg,
              timestamp: new Date().toLocaleTimeString(),
            },
          ];
          break;
        }
        case "usage": {
          // Stub in v1: we don't show token counts yet. Phase 7f
          // wires the sidebar indicator.
          break;
        }
        case "error": {
          const errKey = `${event.code}:${event.message}`;
          const lastErr = list.filter((e) => e.type === "agent_error").pop();
          if (lastErr?.message === errKey) break;
          next = [
            ...list,
            {
              id: crypto.randomUUID(),
              type: "agent_error",
              message: errKey,
              timestamp: new Date().toLocaleTimeString(),
            },
          ];
          break;
        }
        case "turn_complete": {
          const reason = event.stop_reason;
          const msg =
            reason === "cancelled"
              ? "[cancelled by user]"
              : reason === "max_tokens"
              ? "Turn complete: max tokens reached."
              : reason === "error"
              ? "Turn complete: error."
              : "Turn complete.";
          const lastInfo = list.filter((e) => e.type === "info").pop();
          if (lastInfo?.message === msg) break;
          next = [
            ...list,
            {
              id: crypto.randomUUID(),
              type: "info",
              message: msg,
              timestamp: new Date().toLocaleTimeString(),
            },
          ];
          break;
        }
      }
      return { ...prev, [threadId]: next };
    });
  };

  // Approve the install of a missing agent and start the session.
  const approveInstall = async () => {
    if (!installPrompt) return;
    const { threadId, agentKind, machineName } = installPrompt;
    setInstallPrompt(null);
    agentSessionRegistry.setStatus(threadId, "installing");
    setThreads((prev) =>
      prev.map((t) => (t.id === threadId ? { ...t, status: "installing" } : t)),
    );
    setStreamsAndPersist((prev) => ({
      ...prev,
      [threadId]: [
        ...(prev[threadId] || []),
        {
          id: crypto.randomUUID(),
          type: "info",
          message: `Installing ${agentKind} on ${machineName}…`,
          timestamp: new Date().toLocaleTimeString(),
        },
      ],
    }));
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
      setStreamsAndPersist((prev) => ({
        ...prev,
        [threadId]: [
          ...(prev[threadId] || []),
          {
            id: crypto.randomUUID(),
            type: "agent_error",
            message: `install failed: ${e}`,
            timestamp: new Date().toLocaleTimeString(),
          },
        ],
      }));
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
    setStreamsAndPersist((prev) => ({
      ...prev,
      [threadId]: [
        ...(prev[threadId] || []),
        {
          id: crypto.randomUUID(),
          type: "agent_error",
          message: `Install of ${agentKind} declined. Thread is in error state; use "Restart thread" to retry.`,
          timestamp: new Date().toLocaleTimeString(),
        },
      ],
    }));
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
                onClick={() => {
                  setWorkspaceMode("supervisor");
                }}
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
                onClick={() => {
                  setWorkspaceMode("terminal");
                  setInspectedFile(null);
                }}
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
                streams={streams}
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
                  <TerminalTabs
                    machineId={activeMachine.id}
                    host={activeMachine.host}
                  />
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

      {/* Install consent modal (AGENT_INTEGRATION §5.3). */}
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
                agent runtime. The remote shell is the same as the worktree's
                host.
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
