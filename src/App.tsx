import React, { useState, useEffect } from "react";
import { Settings, Terminal, Activity, ShieldAlert, Menu } from "lucide-react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import "./App.css";

import { Machine, FrontendMachine, ThreadSession, FileReference, StreamEvent } from "./types";
import SSHTerminal from "./components/SSHTerminal";
import Sidebar from "./components/Sidebar";
import SupervisorPlane from "./components/SupervisorPlane";
import CodeInspector from "./components/CodeInspector";
import NewThreadModal from "./components/NewThreadModal";
import EnvModal from "./components/EnvModal";

const mapMachineToFrontend = (m: Machine): FrontendMachine => {
  return {
    ...m,
    type: m.auth_type === "local" ? "local" : "server",
    status: m.id === "staging-api" ? "offline" : "connected",
    user: `${m.username}@${m.host}`
  };
};

function App() {
  const [machinesList, setMachinesList] = useState<FrontendMachine[]>([]);
  const [activeMachine, setActiveMachine] = useState<FrontendMachine | null>(null);
  const [showMachineSelector, setShowMachineSelector] = useState(false);
  const [isSidebarCollapsed, setIsSidebarCollapsed] = useState(false);

  // Active Threads & Working Memory
  const [threads, setThreads] = useState<ThreadSession[]>([]);
  const [activeThreadId, setActiveThreadId] = useState<string | null>(null);
  const [workingMemory, setWorkingMemory] = useState<FileReference[]>([]);

  // Modal States
  const [isEnvModalOpen, setIsEnvModalOpen] = useState(false);
  const [envForm, setEnvForm] = useState({
    id: "",
    name: "",
    connection: "", // username@host:port format
    authType: "key",
    keyPath: "",
    secret: "",
    agents: [] as string[],
    autoApprovedRules: '["^git status$", "^cat .*"]'
  });

  const [isNewThreadModalOpen, setIsNewThreadModalOpen] = useState(false);

  // Workspace Planes
  const [workspaceMode, setWorkspaceMode] = useState<string>("supervisor"); // supervisor | terminal
  const [inspectedFile, setInspectedFile] = useState<{ name: string; content: string } | null>(null);

  // Supervisor Streams per thread ID
  const [streams, setStreams] = useState<Record<string, StreamEvent[]>>({});
  const [supervisorInput, setSupervisorInput] = useState("");

  useEffect(() => {
    loadMachines();
  }, []);

  const loadMachines = async () => {
    try {
      // Idempotent: only seeds on the first run when the demo machine is missing
      await invoke("seed_demo_data").catch((err) =>
        console.warn("Demo seed (safe to ignore):", err)
      );

      const list: Machine[] = await invoke("get_machines");
      const mapped = list.map(mapMachineToFrontend);
      setMachinesList(mapped);
      if (mapped.length > 0 && !activeMachine) {
        const demo = mapped.find((m) => m.id === "prod-db-cluster");
        handleMachineSelect(demo || mapped[0]);
      }
    } catch (err) {
      console.error("Failed to load nodes:", err);
    }
  };

  const handleMachineSelect = async (m: FrontendMachine) => {
    setActiveMachine(m);
    setShowMachineSelector(false);

    try {
      // Fetch thread sessions for the selected node
      const threadList: ThreadSession[] = await invoke("get_thread_sessions", { machineId: m.id });
      setThreads(threadList);

      // Seed default threads and telemetry stream if none exist
      if (threadList.length === 0) {
        const mockT1: ThreadSession = {
          id: "t1_" + m.id,
          machine_id: m.id,
          title: "Implement OAuth2",
          mode: "worktree",
          branch: "feature/agent-oauth",
          repo_path: "/home/ubuntu/project",
          sandbox_path: `/home/ubuntu/project/.demeteo/worktrees/feature-agent-oauth`,
          status: "pending_approval"
        };
        const mockT2: ThreadSession = {
          id: "t2_" + m.id,
          machine_id: m.id,
          title: "Analyze syslog memory leak",
          mode: "adhoc",
          status: "idle"
        };

        await invoke("add_thread_session", { thread: mockT1 });
        await invoke("add_thread_session", { thread: mockT2 });

        const freshThreads: ThreadSession[] = await invoke("get_thread_sessions", { machineId: m.id });
        setThreads(freshThreads);
        setActiveThreadId(mockT1.id);

        // Seed events
        setStreams(prev => ({
          ...prev,
          [mockT1.id]: [
            {
              id: "e1",
              type: "directive",
              message: "Set up the initial OAuth2 routes using Actix-web in src/oauth.rs and update Cargo.toml",
              timestamp: new Date(Date.now() - 3600000).toLocaleTimeString()
            },
            {
              id: "e2",
              type: "auto_approve",
              message: "Agent executed git status",
              timestamp: new Date(Date.now() - 3500000).toLocaleTimeString()
            },
            {
              id: "e3",
              type: "auto_approve",
              message: "Agent executed cat Cargo.toml",
              timestamp: new Date(Date.now() - 3400000).toLocaleTimeString()
            },
            {
              id: "e4",
              type: "intercept",
              message: "Intercepted Action: File Write",
              timestamp: new Date(Date.now() - 3300000).toLocaleTimeString(),
              payload: {
                path: "src/oauth.rs",
                additions: 42,
                code: `// Actix-web OAuth2 login controller\nuse actix_web::{get, web, HttpResponse, Responder};\nuse reqwest::Client;\n\n#[get("/login")]\npub async fn login() -> impl Responder {\n    HttpResponse::Ok().body("OAuth implementation pending")\n}`
              }
            }
          ],
          [mockT2.id]: [
            {
              id: "e5",
              type: "directive",
              message: "Find memory leaks inside syslog files",
              timestamp: new Date().toLocaleTimeString()
            },
            {
              id: "e6",
              type: "info",
              message: "Syslog diagnostic listener bound directly to remote kernel socket.",
              timestamp: new Date().toLocaleTimeString()
            }
          ]
        }));
      } else {
        setActiveThreadId(threadList[0].id);
      }

      if (m.id === "prod-db-cluster") {
        seedDemoStreams(threadList);
      }
    } catch (err) {
      console.error(err);
    }
  };

  const seedDemoStreams = (threadList: ThreadSession[]) => {
    const t1 = threadList.find((t) => t.id === "t1_prod-db-cluster");
    const t2 = threadList.find((t) => t.id === "t2_prod-db-cluster");
    const t3 = threadList.find((t) => t.id === "t3_prod-db-cluster");

    setStreams((prev) => {
      const next = { ...prev };
      if (t1 && !next[t1.id]) {
        next[t1.id] = [
          {
            id: "e1",
            type: "directive",
            message:
              "Set up the initial OAuth2 routes using Actix-web in src/oauth.rs and update Cargo.toml",
            timestamp: new Date(Date.now() - 3600000).toLocaleTimeString(),
          },
          {
            id: "e2",
            type: "auto_approve",
            message: "Agent executed git status",
            timestamp: new Date(Date.now() - 3500000).toLocaleTimeString(),
          },
          {
            id: "e3",
            type: "auto_approve",
            message: "Agent executed cat Cargo.toml",
            timestamp: new Date(Date.now() - 3400000).toLocaleTimeString(),
          },
          {
            id: "e4",
            type: "intercept",
            message: "Intercepted Action: File Write",
            timestamp: new Date(Date.now() - 3300000).toLocaleTimeString(),
            payload: {
              path: "src/oauth.rs",
              additions: 42,
              code: `// Actix-web OAuth2 login controller\nuse actix_web::{get, web, HttpResponse, Responder};\nuse reqwest::Client;\n\n#[get("/login")]\npub async fn login() -> impl Responder {\n    HttpResponse::Ok().body("OAuth implementation pending")\n}`,
            },
          },
        ];
      }
      if (t2 && !next[t2.id]) {
        next[t2.id] = [
          {
            id: "e5",
            type: "directive",
            message: "Find memory leaks inside syslog files",
            timestamp: new Date().toLocaleTimeString(),
          },
          {
            id: "e6",
            type: "info",
            message: "Syslog diagnostic listener bound directly to remote kernel socket.",
            timestamp: new Date().toLocaleTimeString(),
          },
        ];
      }
      if (t3 && !next[t3.id]) {
        next[t3.id] = [
          {
            id: "e7",
            type: "directive",
            message: "Bump the rustc base image to slim-bookworm and prune apt caches",
            timestamp: new Date(Date.now() - 7200000).toLocaleTimeString(),
          },
          {
            id: "e8",
            type: "info",
            message: "Build pipeline linked to feature/docker-fix worktree.",
            timestamp: new Date(Date.now() - 7100000).toLocaleTimeString(),
          },
        ];
      }
      return next;
    });
  };

  // Seed / update working memory files list based on active thread
  useEffect(() => {
    if (!activeThreadId) {
      setWorkingMemory([]);
      return;
    }
    if (activeThreadId.startsWith("t1")) {
      setWorkingMemory([
        { name: "src/main.rs", lines: 142, type: "rust" },
        { name: "Cargo.toml", lines: 34, type: "toml" },
        { name: "src/oauth.rs", lines: 89, type: "rust", isNew: true }
      ]);
    } else if (activeThreadId.startsWith("t3")) {
      setWorkingMemory([
        { name: "Dockerfile", lines: 48, type: "dockerfile" },
        { name: ".dockerignore", lines: 21, type: "text" },
        { name: "docker-compose.yml", lines: 76, type: "yaml", isNew: true }
      ]);
    } else {
      setWorkingMemory([
        { name: "var/log/syslog", lines: 4500, type: "text" },
        { name: "src/main.rs", lines: 142, type: "rust" }
      ]);
    }
    setInspectedFile(null);
  }, [activeThreadId]);

  const openAddEnv = () => {
    setEnvForm({
      id: "",
      name: "",
      connection: "ubuntu@localhost:22",
      authType: "key",
      keyPath: "~/.ssh/id_rsa",
      secret: "",
      agents: [] as string[],
      autoApprovedRules: '["^git status$", "^cat .*"]'
    });
    setIsEnvModalOpen(true);
    setShowMachineSelector(false);
  };

  const openEditEnv = (m: FrontendMachine, e: React.MouseEvent) => {
    e.stopPropagation();
    setEnvForm({
      id: m.id,
      name: m.name,
      connection: `${m.username}@${m.host}:${m.port}`,
      authType: m.auth_type,
      keyPath: m.key_path || "",
      secret: "",
      agents: JSON.parse(m.agents || "[]"),
      autoApprovedRules: m.auto_approved_rules || '["^git status$", "^cat .*"]'
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
    if (!confirm("Are you sure you want to remove this connection profile?")) return;
    try {
      await invoke("delete_machine", { id });
      await invoke("delete_machine_secret", { machineId: id }).catch(console.error);
      const updated = machinesList.filter(m => m.id !== id);
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
      agents: JSON.stringify(form.agents),
      auto_approved_rules: form.autoApprovedRules
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

  const launchThread = async (title: string, mode: string, branch: string, repoPath: string) => {
    if (!activeMachine) return;
    const id = "t_" + Date.now();
    const sandboxPath = mode === "worktree"
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
      status: "idle"
    };

    try {
      await invoke("add_thread_session", { thread: threadData });
      setIsNewThreadModalOpen(false);

      // Seed new stream
      setStreams(prev => ({
        ...prev,
        [id]: [
          {
            id: crypto.randomUUID(),
            type: "info",
            message: `Workspace sandbox provisioned. Mode: ${mode.toUpperCase()}`,
            timestamp: new Date().toLocaleTimeString()
          }
        ]
      }));

      // Reload
      const threadList: ThreadSession[] = await invoke("get_thread_sessions", { machineId: activeMachine.id });
      setThreads(threadList);
      setActiveThreadId(id);
    } catch (err) {
      alert("Failed to provision worktree sandbox: " + err);
    }
  };

  const handleInspectContext = async (path: string) => {
    if (!activeMachine) return;
    try {
      const content = await invoke<string>("sftp_read_file", { machineId: activeMachine.id, path });
      setInspectedFile({ name: path, content });
    } catch (e) {
      if (path.endsWith("oauth.rs")) {
        setInspectedFile({
          name: path,
          content: `// OAuth Implementation Proposal\nuse actix_web::{get, web, HttpResponse, Responder};\nuse reqwest::Client;\n\n#[get("/login")]\npub async fn login() -> impl Responder {\n    HttpResponse::Ok().body("OAuth implementation pending")\n}`
        });
      } else {
        setInspectedFile({
          name: path,
          content: `// Local file context for ${path} not loaded.`
        });
      }
    }
  };

  // Supervisor Approvals
  const approveAction = async (threadId: string, eventId: string) => {
    try {
      await invoke("update_thread_status", { id: threadId, status: "running" });
      if (activeMachine) {
        const list: ThreadSession[] = await invoke("get_thread_sessions", { machineId: activeMachine.id });
        setThreads(list);
      }

      setStreams(prev => {
        const list = prev[threadId] || [];
        const updated = list.map(e => e.id === eventId ? { ...e, type: "info" as const, message: `${e.message} (Approved by Supervisor)` } : e);
        return {
          ...prev,
          [threadId]: [
            ...updated,
            {
              id: crypto.randomUUID(),
              type: "info",
              message: "Approval signature signed. Release SSH write lock. Starting compile task...",
              timestamp: new Date().toLocaleTimeString()
            }
          ]
        };
      });
    } catch (err) {
      console.error(err);
    }
  };

  const rejectAction = async (threadId: string, eventId: string, feedback: string) => {
    try {
      await invoke("update_thread_status", { id: threadId, status: "idle" });
      if (activeMachine) {
        const list: ThreadSession[] = await invoke("get_thread_sessions", { machineId: activeMachine.id });
        setThreads(list);
      }

      setStreams(prev => {
        const list = prev[threadId] || [];
        const updated = list.map(e => e.id === eventId ? { ...e, type: "info" as const, message: `${e.message} (Rejected by Supervisor)` } : e);
        return {
          ...prev,
          [threadId]: [
            ...updated,
            {
              id: crypto.randomUUID(),
              type: "directive",
              message: feedback ? `Action Intercepted & Cancelled. Feedback returned: ${feedback}` : "Action Intercepted & Cancelled by Supervisor.",
              timestamp: new Date().toLocaleTimeString()
            }
          ]
        };
      });
    } catch (err) {
      console.error(err);
    }
  };

  const sendDirective = (threadId: string) => {
    if (!supervisorInput.trim()) return;

    setStreams(prev => ({
      ...prev,
      [threadId]: [
        ...(prev[threadId] || []),
        {
          id: crypto.randomUUID(),
          type: "directive",
          message: supervisorInput,
          timestamp: new Date().toLocaleTimeString()
        }
      ]
    }));
    setSupervisorInput("");
  };

  return (
    <div className="flex h-screen bg-[#050508] text-slate-300 font-sans selection:bg-cyan-500/30 w-full overflow-hidden">
      
      {/* LEFT SIDEBAR: Environment & Threads */}
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
        onThreadSelect={setActiveThreadId}
        setWorkspaceMode={setWorkspaceMode}
        onNewThreadClick={() => setIsNewThreadModalOpen(true)}
        workingMemory={workingMemory}
        inspectedFileName={inspectedFile?.name}
        onInspectFile={handleInspectContext}
      />

      {/* CENTER PANEL: The Orchestrator Stream & Inspector */}
      <div className="flex-1 flex flex-col min-w-0 bg-slate-950/40 relative shadow-2xl z-20 h-full">
        
        {/* Header Tabs */}
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
                onClick={() => { setWorkspaceMode('supervisor'); }}
                className={`px-4 py-1.5 rounded-md text-xs font-medium transition-all duration-200 flex items-center hover:scale-[1.03] active:scale-[0.97] cursor-pointer ${
                  workspaceMode === 'supervisor' ? 'bg-cyan-500/20 text-cyan-400 shadow-[0_0_12px_rgba(6,182,212,0.15)] border border-cyan-500/25' : 'text-slate-400 hover:text-slate-200 border border-transparent'
                }`}
              >
                <Activity size={14} className="mr-2" /> Supervisor Plane
              </button>
              <button 
                type="button"
                onClick={() => { setWorkspaceMode('terminal'); setInspectedFile(null); }}
                className={`px-4 py-1.5 rounded-md text-xs font-medium transition-all duration-200 flex items-center hover:scale-[1.03] active:scale-[0.97] cursor-pointer ${
                  workspaceMode === 'terminal' ? 'bg-slate-800 text-white shadow-[0_0_12px_rgba(255,255,255,0.05)] border border-white/10' : 'text-slate-400 hover:text-slate-200 border border-transparent'
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

        {/* Dynamic Workspace Container */}
        <div className="flex-1 flex flex-col md:flex-row min-h-0 overflow-hidden w-full">
          
          {/* Main Stream (Shrinks if Inspector is open) */}
          <div className={`flex flex-col min-w-0 transition-all duration-300 h-full ${inspectedFile && workspaceMode === 'supervisor' ? 'w-full h-1/2 md:w-[55%] md:h-full border-b md:border-b-0 md:border-r border-white/5' : 'w-full'}`}>
            
            {workspaceMode === 'supervisor' ? (
              <SupervisorPlane
                activeThreadId={activeThreadId}
                threads={threads}
                streams={streams}
                supervisorInput={supervisorInput}
                setSupervisorInput={setSupervisorInput}
                onSendDirective={sendDirective}
                onInspectContext={handleInspectContext}
                onApproveAction={approveAction}
                onRejectAction={rejectAction}
              />
            ) : (
              /* INTERACTIVE TERMINAL WORKSPACE */
              <div className="flex-1 bg-[#050508] p-1 overflow-hidden h-full">
                {activeMachine ? (
                  <SSHTerminal machineId={activeMachine.id} />
                ) : (
                  <div className="flex flex-col justify-center items-center h-full text-slate-500">
                    <Terminal size={32} className="mb-2" />
                    <div>No active target connection node.</div>
                  </div>
                )}
              </div>
            )}
          </div>

          {/* CODE INSPECTOR PANEL (Sliding Split View) */}
          {inspectedFile && workspaceMode === 'supervisor' && (
            <CodeInspector
              fileName={inspectedFile.name}
              fileContent={inspectedFile.content}
              onRefresh={() => handleInspectContext(inspectedFile.name)}
              onClose={() => setInspectedFile(null)}
            />
          )}
        </div>
      </div>

      {/* MODAL: New Thread (Dual-Mode Execution) */}
      <NewThreadModal
        isOpen={isNewThreadModalOpen}
        onClose={() => setIsNewThreadModalOpen(false)}
        onLaunch={launchThread}
      />

      {/* Environment Config Modal */}
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
      />
    </div>
  );
}

export default App;
