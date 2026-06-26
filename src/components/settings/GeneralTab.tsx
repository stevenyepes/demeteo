import { Globe, Box, HardDrive, Server, AlertTriangle, Activity, CircleAlert, GitBranch, Check, X, Plus, RotateCw, RefreshCw, ChevronDown, ChevronUp, Trash2, Settings } from 'lucide-react';
import { useSettings } from './ProjectSettingsContext';
import { useNavigation } from '../../context';

export function GeneralTab() {
  const s = useSettings();
  const { navigate } = useNavigation();

  return (
    <div className="grid grid-cols-1 md:grid-cols-2 gap-6">
      {/* General Configuration */}
      <div className="glass-panel p-6 rounded-xl space-y-4">
        <h3 className="font-outfit text-sm font-semibold text-slate-300 uppercase tracking-wider mb-2 flex items-center gap-2">
          <Globe className="w-4 h-4 text-violet-400" /> General Configuration
        </h3>

        <div>
          <label className="block text-xs font-mono text-slate-400 mb-1.5 uppercase tracking-wider">Workspace Name</label>
          <input type="text" value={s.projectName} onChange={e => s.setProjectName(e.target.value)} className="w-full bg-black/40 border border-white/10 rounded-lg py-2 px-3 text-sm text-white focus:outline-none focus:border-cyan-500/50" />
        </div>

        <div>
          <label className="block text-xs font-mono text-slate-400 mb-1.5 uppercase tracking-wider">Environment / Target Server</label>
          <div className="flex gap-2">
            <button onClick={() => s.setComputeType('local')} className={`flex-1 flex items-center justify-center gap-2 border rounded-lg py-2 px-3 text-sm transition-all ${s.computeType === 'local' ? 'bg-violet-500/10 border-violet-500/50 text-violet-300' : 'bg-black/40 border-white/5 text-slate-400'}`}>
              <HardDrive className="w-4 h-4" /> Local Compute
            </button>
            <button onClick={() => s.setComputeType('remote')} className={`flex-1 flex items-center justify-center gap-2 border rounded-lg py-2 px-3 text-sm transition-all ${s.computeType === 'remote' ? 'bg-cyan-500/10 border-cyan-500/50 text-cyan-300' : 'bg-black/40 border-white/5 text-slate-400'}`}>
              <Server className="w-4 h-4" /> Remote SSH
            </button>
          </div>
          {s.computeType === 'remote' && (
            <div className="mt-3 flex gap-2">
              <select value={s.remoteHost} onChange={e => s.setRemoteHost(e.target.value)} className="flex-1 bg-black/40 border border-white/10 rounded-lg py-2 px-3 text-sm text-white font-mono focus:outline-none focus:border-cyan-500/50">
                <option value="">{s.machines.length === 0 ? 'No machines configured' : 'Select a machine…'}</option>
                {s.machines.map(m => <option key={m.id} value={m.id}>{m.name} ({m.username}@{m.host}:{m.port})</option>)}
              </select>
              <button type="button" onClick={s.handleTestConnection} disabled={!s.remoteHost || s.isTestingConnection} className="px-4 py-2 text-xs font-semibold rounded-lg bg-white/5 border border-white/10 hover:bg-white/10 text-white disabled:opacity-40 flex items-center gap-1.5 transition-all shrink-0">
                {s.isTestingConnection ? <RotateCw className="w-3.5 h-3.5 animate-spin text-cyan-400" /> : s.connectionStatus === 'success' ? <Check className="w-3.5 h-3.5 text-emerald-400" /> : s.connectionStatus === 'error' ? <X className="w-3.5 h-3.5 text-ruby-400" /> : null}
                {s.isTestingConnection ? 'Testing...' : s.connectionStatus === 'success' ? 'Connected' : s.connectionStatus === 'error' ? 'Failed' : 'Test'}
              </button>
              <button type="button" onClick={() => navigate({ kind: 'settings' })} className="px-3 py-2 text-xs rounded-lg bg-violet-500/10 border border-violet-500/30 hover:bg-violet-500/20 text-violet-300 transition-all shrink-0" title="Manage machines in Settings">
                <Settings className="w-3.5 h-3.5" />
              </button>
            </div>
          )}
        </div>
      </div>

      {/* Repositories */}
      <div className="glass-panel p-6 rounded-xl space-y-4">
        <h3 className="font-outfit text-sm font-semibold text-slate-300 uppercase tracking-wider mb-2 flex items-center gap-2">
          <Box className="w-4 h-4 text-cyan-400" /> Repositories Mapped ({s.selectedRepos.length})
        </h3>
        <div className="space-y-2 max-h-[190px] overflow-y-auto pr-1">
          {s.selectedRepos.length === 0 ? (
            <p className="text-xs text-slate-500 italic py-2">No repositories configured.</p>
          ) : s.selectedRepos.map(repo => (
            <div key={repo.path} className="flex items-center gap-2 p-2 border border-white/5 rounded-lg bg-black/20">
              <Box className="w-3.5 h-3.5 text-cyan-400 shrink-0" />
              <span className="text-xs text-slate-300 truncate w-4/5">{repo.path}</span>
              <button onClick={() => s.toggleRepo(repo)} className="ml-auto text-slate-500 hover:text-ruby-400 p-0.5 rounded hover:bg-white/5"><X className="w-3.5 h-3.5" /></button>
            </div>
          ))}
        </div>
        <button onClick={() => { s.setIsRepoModalOpen(true); s.fetchAllReposFromProviders(); }} className="w-full flex items-center justify-center gap-1.5 p-2 rounded-lg border border-dashed border-white/10 text-slate-400 hover:text-white hover:bg-white/5 transition-all text-xs">
          <Plus className="w-3.5 h-3.5" /> Manage Workspace Repositories
        </button>
      </div>

      {/* Workspace Health Panel */}
      <div className="md:col-span-2">
        {s.healthError && (
          <div className="rounded-xl border border-ruby-500/30 bg-ruby-500/5 p-4 mb-3">
            <div className="flex items-center gap-2 mb-2">
              <AlertTriangle className="w-4 h-4 text-ruby-400" />
              <span className="font-outfit text-sm font-semibold text-ruby-300 uppercase tracking-wider">Workspace Health Check Failed</span>
            </div>
            <pre className="font-mono text-xs text-ruby-200/80 whitespace-pre-wrap break-words max-h-40 overflow-y-auto">{s.healthError}</pre>
            <div className="mt-3 flex gap-2">
              <button onClick={s.fetchWorkspaceHealth} disabled={s.isLoadingHealth} className="px-3 py-1.5 text-xs rounded-md border border-ruby-500/30 text-ruby-200 hover:bg-ruby-500/10 transition-all flex items-center gap-1.5">
                <RefreshCw className={`w-3 h-3 ${s.isLoadingHealth ? 'animate-spin' : ''}`} /> Retry
              </button>
              <button onClick={s.proceedWithReBootstrap} className="px-3 py-1.5 text-xs rounded-md bg-cyan-600 hover:bg-cyan-500 text-white transition-all font-medium">Re-run Bootstrap</button>
            </div>
          </div>
        )}
        {!s.showHealthPanel ? (
          <button onClick={s.fetchWorkspaceHealth} disabled={s.isLoadingHealth} className="w-full flex items-center justify-center gap-2 p-3 rounded-xl border border-dashed border-white/10 text-slate-400 hover:text-cyan-400 hover:border-cyan-500/30 hover:bg-cyan-500/5 transition-all text-sm">
            {s.isLoadingHealth ? <RotateCw className="w-4 h-4 animate-spin" /> : <Activity className="w-4 h-4" />}
            {s.isLoadingHealth ? 'Checking workspace health...' : 'Check Workspace Health'}
          </button>
        ) : (
          <div className="glass-panel rounded-xl border border-white/5 overflow-hidden">
            <div role="button" tabIndex={0} onClick={() => s.setHealthExpanded(!s.healthExpanded)} onKeyDown={e => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); s.setHealthExpanded(!s.healthExpanded); } }} className="w-full flex items-center justify-between px-5 py-3.5 bg-white/[0.02] hover:bg-white/[0.04] transition-colors cursor-pointer">
              <div className="flex items-center gap-2.5">
                <Activity className="w-4 h-4 text-cyan-400" />
                <span className="font-outfit text-sm font-semibold text-slate-200 uppercase tracking-wider">Workspace Health</span>
                {s.healthData && s.healthData.length > 0 && (() => {
                  const hasError = s.healthData!.some(r => !r.is_cloned);
                  const hasDirty = s.healthData!.some(r => r.has_uncommitted || r.has_unpushed);
                  if (hasError) return <span className="px-2 py-0.5 text-[10px] rounded-full bg-ruby-500/15 border border-ruby-500/25 text-ruby-400 font-mono">DEGRADED</span>;
                  if (hasDirty) return <span className="px-2 py-0.5 text-[10px] rounded-full bg-amber-500/15 border border-amber-500/25 text-amber-400 font-mono">DIRTY</span>;
                  return <span className="px-2 py-0.5 text-[10px] rounded-full bg-emerald-500/15 border border-emerald-500/25 text-emerald-400 font-mono">HEALTHY</span>;
                })()}
              </div>
              <div className="flex items-center gap-2">
                <button onClick={e => { e.stopPropagation(); s.fetchWorkspaceHealth(); }} disabled={s.isLoadingHealth} className="p-1.5 rounded-md text-slate-400 hover:text-cyan-400 hover:bg-white/5 transition-all" title="Refresh health">
                  <RefreshCw className={`w-3.5 h-3.5 ${s.isLoadingHealth ? 'animate-spin' : ''}`} />
                </button>
                {s.healthExpanded ? <ChevronUp className="w-4 h-4 text-slate-500" /> : <ChevronDown className="w-4 h-4 text-slate-500" />}
              </div>
            </div>
            {s.healthExpanded && (
              <div className="p-4 space-y-3">
                {s.isLoadingHealth && !s.healthData ? (
                  <div className="flex items-center gap-2 text-sm text-slate-400 py-2"><RotateCw className="w-4 h-4 animate-spin text-cyan-400" />Scanning repositories...</div>
                ) : s.healthData && s.healthData.length > 0 ? s.healthData.map(repo => {
                  const repoName = repo.repo_path.split('/').pop() ?? repo.repo_path;
                  const activeWorktrees = repo.worktrees.slice(1);
                  const isDirty = repo.has_uncommitted || repo.has_unpushed;
                  return (
                    <div key={repo.repo_path} className={`rounded-lg border p-3.5 transition-all ${repo.is_cloned ? (isDirty ? 'border-amber-500/20 bg-amber-500/5' : 'border-white/5 bg-black/20') : 'border-ruby-500/20 bg-ruby-500/5'}`}>
                      <div className="flex items-center gap-3 flex-wrap">
                        <span className={`w-2 h-2 rounded-full shrink-0 ${repo.is_cloned ? (isDirty ? 'bg-amber-400' : 'bg-emerald-400') : 'bg-ruby-400 animate-pulse'}`} />
                        <div className="flex flex-col min-w-0 flex-1">
                          <span className="text-sm text-white font-medium truncate">{repoName}</span>
                          <span className="text-[11px] text-slate-500 truncate">{repo.repo_path}</span>
                        </div>
                        <div className="flex items-center gap-1.5 flex-wrap justify-end">
                          {repo.is_cloned ? <span className="px-2 py-0.5 text-[10px] font-mono rounded-md bg-emerald-500/10 border border-emerald-500/20 text-emerald-400">CLONED</span> : <span className="px-2 py-0.5 text-[10px] font-mono rounded-md bg-ruby-500/10 border border-ruby-500/20 text-ruby-400">MISSING</span>}
                          {repo.head_branch && <span className="flex items-center gap-1 px-2 py-0.5 text-[10px] font-mono rounded-md bg-cyan-500/10 border border-cyan-500/20 text-cyan-400"><GitBranch className="w-2.5 h-2.5" />{repo.head_branch}</span>}
                          {activeWorktrees.length > 0 && <span className="flex items-center gap-1 px-2 py-0.5 text-[10px] font-mono rounded-md bg-violet-500/10 border border-violet-500/20 text-violet-400"><GitBranch className="w-2.5 h-2.5" />{activeWorktrees.length} worktree{activeWorktrees.length !== 1 ? 's' : ''}</span>}
                          {repo.has_uncommitted && <span className="px-2 py-0.5 text-[10px] font-mono rounded-md bg-amber-500/10 border border-amber-500/20 text-amber-400">Uncommitted</span>}
                          {repo.has_unpushed && <span className="px-2 py-0.5 text-[10px] font-mono rounded-md bg-orange-500/10 border border-orange-500/20 text-orange-400">Unpushed</span>}
                        </div>
                      </div>
                      {activeWorktrees.length > 0 && (
                        <div className="mt-2.5 pl-5 space-y-1">
                          {activeWorktrees.map(wt => (
                            <div key={wt.path} className="flex items-center gap-2 text-[10px] text-slate-400">
                              <span className="w-1 h-1 rounded-full bg-violet-400 shrink-0" />
                              <span className="font-mono truncate">{wt.branch ?? wt.path.split('/').pop()}</span>
                              {wt.is_locked && <span className="text-amber-400">(locked)</span>}
                            </div>
                          ))}
                        </div>
                      )}
                      {!repo.is_cloned && (
                        <div className="mt-3 flex items-center gap-2 p-2.5 rounded-lg bg-ruby-500/5 border border-ruby-500/15">
                          <CircleAlert className="w-4 h-4 text-ruby-400 shrink-0" />
                          <p className="text-[11px] text-slate-400 flex-1">This repository clone is missing. Re-run bootstrap to restore it.</p>
                          <button onClick={s.proceedWithReBootstrap} className="px-3 py-1.5 text-[11px] font-semibold bg-ruby-600/80 hover:bg-ruby-500 text-white rounded-md transition-all flex items-center gap-1.5 shrink-0">
                            <RotateCw className="w-3 h-3" /> Re-bootstrap
                          </button>
                        </div>
                      )}
                    </div>
                  );
                }) : <p className="text-sm text-slate-500 italic py-1">No repository data available.</p>}
              </div>
            )}
          </div>
        )}
      </div>

      {/* Danger zone */}
      <div className="glass-panel p-6 rounded-xl border border-ruby-500/10 md:col-span-2 flex flex-col md:flex-row items-center justify-between gap-4">
        <div className="flex gap-3">
          <Trash2 className="w-10 h-10 text-ruby-400 shrink-0" />
          <div>
            <h4 className="font-outfit font-bold text-white text-base">Danger Zone: Destroy Workspace</h4>
            <p className="text-xs text-slate-400 mt-1 max-w-xl">Deleting a workspace will remove its configuration records and permanently delete all local repository clones. This action is irreversible.</p>
          </div>
        </div>
        <button onClick={s.handleDeleteClick} className="bg-ruby-600 hover:bg-ruby-500 text-white font-semibold text-xs px-4 py-2.5 rounded-lg transition-all shrink-0 shadow-[0_0_15px_rgba(239,68,68,0.2)]">
          Delete Workspace
        </button>
      </div>
    </div>
  );
}
