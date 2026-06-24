import React, { useState, useEffect, useCallback } from 'react';
import { X, Terminal } from 'lucide-react';
import { invoke } from '@tauri-apps/api/core';
import { TerminalWindow } from './TerminalWindow';
import { writeTerminalSession } from '../lib/terminal';

interface AgentEntry {
  kind: string;
  binary: string;
  label: string;
}

const AGENT_CLI: Record<string, { binary: string; label: string }> = {
  'claude-code': { binary: 'claude', label: 'Claude' },
  'opencode':    { binary: 'opencode', label: 'OpenCode' },
  'hermes':      { binary: 'hermes', label: 'Hermes' },
  'antigravity': { binary: 'agy', label: 'Antigravity' },
};

export interface AgentTerminalDrawerProps {
  isOpen: boolean;
  onClose: () => void;
  /** machineId for agent config lookup and PTY routing */
  machineId: string;
  /** Pre-resolved absolute path (feature worktree). Skips resolveRepoDir. */
  absoluteWorkDir?: string;
  /** Project-relative repo path. Used when absoluteWorkDir is absent. */
  repoPath?: string;
  projectId: string;
  computeType: string;
  remoteHost: string | null;
  /** Sidebar width in px so the drawer doesn't overlap it. Default 240. */
  sidebarWidth?: number;
}

export const AgentTerminalDrawer: React.FC<AgentTerminalDrawerProps> = ({
  isOpen,
  onClose,
  machineId,
  absoluteWorkDir,
  repoPath,
  projectId,
  computeType,
  remoteHost,
  sidebarWidth = 240,
}) => {
  const [sessionId, setSessionId] = useState<string | null>(null);
  const [agents, setAgents] = useState<AgentEntry[]>([]);
  const [launching, setLaunching] = useState<string | null>(null);

  useEffect(() => {
    if (!isOpen) {
      setSessionId(null);
      setLaunching(null);
      return;
    }
    (async () => {
      try {
        const configs = await invoke<Array<{ kind: string; enabled: boolean }>>('get_agent_configs', { machineId });
        const found = (configs || [])
          .filter(c => c.enabled && AGENT_CLI[c.kind])
          .map(c => ({ kind: c.kind, ...AGENT_CLI[c.kind] }));
        setAgents(found.length > 0 ? found : defaultAgents());
      } catch {
        setAgents(defaultAgents());
      }
    })();
  }, [isOpen, machineId]);

  const handleLaunchAgent = useCallback(async (agent: AgentEntry) => {
    if (!sessionId || launching) return;
    setLaunching(agent.kind);
    try {
      await writeTerminalSession(sessionId, agent.binary + '\r');
    } finally {
      setLaunching(null);
    }
  }, [sessionId, launching]);

  const pathLabel = absoluteWorkDir
    ? absoluteWorkDir.split('/').slice(-2).join('/')
    : repoPath || projectId;

  if (!isOpen) return null;

  return (
    <>
      {/* Scrim — only covers the main content area, not the sidebar */}
      <div
        className="fixed inset-y-0 bottom-0 z-30 bg-black/30 backdrop-blur-[2px]"
        style={{ left: sidebarWidth, right: 0 }}
        onClick={onClose}
      />

      {/* Drawer */}
      <div
        className="fixed bottom-0 z-40 flex flex-col bg-[#050608] border-t border-white/10 shadow-[0_-12px_60px_rgba(0,0,0,0.7)]"
        style={{ left: sidebarWidth, right: 0, height: '52vh' }}
      >
        {/* Toolbar */}
        <div className="flex items-center gap-3 px-4 py-2 border-b border-white/5 bg-[#0c0d12] shrink-0">
          <Terminal className="w-3.5 h-3.5 text-cyan-400 shrink-0" />
          <span className="text-[11px] font-mono text-slate-400 truncate max-w-[240px]">
            {pathLabel}
          </span>

          <div className="h-3.5 w-px bg-white/10 mx-1 shrink-0" />

          {/* Agent launch buttons */}
          <div className="flex items-center gap-2 flex-1 min-w-0">
            <span className="text-[10px] text-slate-500 uppercase font-bold tracking-wider shrink-0">
              Launch
            </span>
            {agents.map(agent => (
              <button
                key={agent.kind}
                onClick={() => handleLaunchAgent(agent)}
                disabled={!sessionId || launching !== null}
                title={`Run ${agent.binary} in this terminal`}
                className={`px-3 py-1 rounded text-[10px] font-bold uppercase tracking-wider border transition-all shrink-0
                  disabled:opacity-30 disabled:cursor-not-allowed
                  ${launching === agent.kind
                    ? 'bg-cyan-500/20 border-cyan-400/60 text-cyan-300 animate-pulse'
                    : 'bg-white/5 border-white/10 text-slate-300 hover:bg-cyan-500/15 hover:border-cyan-500/40 hover:text-cyan-300'
                  }`}
              >
                {agent.label}
              </button>
            ))}
            {!sessionId && (
              <span className="text-[10px] text-slate-600 font-mono italic">
                connecting…
              </span>
            )}
          </div>

          <button
            onClick={onClose}
            className="ml-auto p-1.5 rounded-lg text-slate-500 hover:text-white hover:bg-white/5 transition shrink-0"
            title="Close terminal"
          >
            <X className="w-4 h-4" />
          </button>
        </div>

        {/* Terminal body */}
        <div className="flex-1 min-h-0">
          <TerminalWindow
            key={absoluteWorkDir || repoPath || projectId}
            projectId={projectId}
            computeType={computeType}
            remoteHost={remoteHost}
            repoPath={repoPath || ''}
            workDir={absoluteWorkDir}
            onSessionStarted={setSessionId}
          />
        </div>
      </div>
    </>
  );
};

function defaultAgents(): AgentEntry[] {
  return [
    { kind: 'claude-code', ...AGENT_CLI['claude-code'] },
    { kind: 'opencode', ...AGENT_CLI['opencode'] },
  ];
}
