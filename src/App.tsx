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
    status: "connected",
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
      const list: Machine[] = await invoke("get_machines");
      const mapped = list.map(mapMachineToFrontend);
      setMachinesList(mapped);
      if (mapped.length > 0 && !activeMachine) {
        handleMachineSelect(mapped[0]);
      }
    } catch (err) {
      console.error("Failed to load nodes:", err);
    }
  };

  const handleMachineSelect = async (m: FrontendMachine) => {
    setActiveMachine(m);
    setShowMachineSelector(false);

    try {
      const threadList: ThreadSession[] = await invoke("get_thread_sessions", { machineId: m.id });
      setThreads(threadList);
      if (threadList.length > 0) {
        setActiveThreadId(threadList[0].id);
      } else {
        setActiveThreadId(null);
      }
    } catch (err) {
      console.error(err);
    }
  };


  // Working memory is populated by real agent events; start empty
  useEffect(() => {
    setWorkingMemory([]);
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

  const deleteThread = async (threadId: string) => {
    if (!confirm("Remove this thread session?")) return;
    try {
      await invoke("delete_thread_session", { id: threadId });
      setStreams(prev => { const next = { ...prev }; delete next[threadId]; return next; });
      if (activeMachine) {
        const list: ThreadSession[] = await invoke("get_thread_sessions", { machineId: activeMachine.id });
        setThreads(list);
        setActiveThreadId(list.length > 0 ? list[0].id : null);
      }
    } catch (err) {
      console.error(err);
    }
  };

  const testSshConnection = async (form: any): Promise<string> => {
    try {
      // Parse host/port/username from the connection string (same logic as saveEnv)
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
        keyPath: form.authType === "key" ? (form.keyPath || null) : null,
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
      const content = await invoke<string>("sftp_read_file", { machineId: activeMachine.id, path });
      setInspectedFile({ name: path, content });
    } catch (e) {
      console.warn("Could not read remote file:", path, e);
      setInspectedFile(null);
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
        onDeleteThread={deleteThread}
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
        onTestConnection={testSshConnection}
      />
    </div>
  );
}

export default App;
