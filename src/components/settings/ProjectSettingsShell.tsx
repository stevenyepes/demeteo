import { AlertTriangle, RotateCw, Check, X, Search, Box, Save, Trash2, ShieldAlert, Activity } from 'lucide-react';
import { Modal } from '../ui/Modal';
import { TabBar } from '../ui/TabBar';
import type { TabDef } from '../ui/TabBar';
import { ProjectSettingsProvider, useSettings } from './ProjectSettingsContext';
import { GeneralTab } from './GeneralTab';
import { StrategyTab } from './StrategyTab';
import { OverridesTab } from './OverridesTab';
import { MemoryTab } from './MemoryTab';

function Shell() {
  const s = useSettings();

  if (s.isLoading) {
    return <div className="flex-1 flex items-center justify-center p-8"><RotateCw className="w-8 h-8 text-cyan-400 animate-spin" /></div>;
  }

  if (s.bootstrapStep === 'bootstrapping') {
    return (
      <div className="flex-1 flex flex-col items-center justify-center p-8 relative overflow-hidden bg-[#08090c]">
        <div className="absolute top-1/4 left-1/2 -translate-x-1/2 w-[600px] h-[300px] bg-violet-600/10 rounded-full blur-[120px] pointer-events-none" />
        <div className="glass-panel max-w-lg w-full p-8 rounded-xl flex flex-col items-center text-center relative border border-white/10 shadow-2xl">
          <RotateCw className="w-12 h-12 text-cyan-400 animate-spin mb-6" />
          <h2 className="text-2xl font-outfit font-bold text-white mb-2">Workspace Re-bootstrap In Progress</h2>
          <p className="text-sm text-slate-400 mb-6 leading-relaxed">Demeteo is updating your cloned repositories and analyzing codebase strategies.</p>
          <div className="w-full bg-black/40 border border-white/5 rounded-lg p-4 font-mono text-left text-xs space-y-2.5 text-slate-300">
            <div className="flex items-center gap-2"><span className="w-2 h-2 rounded-full bg-cyan-400 animate-pulse" /><span>Validating Credentials...</span></div>
            <div className="flex items-center gap-2"><span className="w-2 h-2 rounded-full bg-cyan-400 animate-pulse" /><span>Syncing and Cloning Repository directories...</span></div>
            <div className="flex items-center gap-2"><span className="w-2 h-2 rounded-full bg-slate-600" /><span className="text-slate-500">Pruning unconfigured repository folders...</span></div>
          </div>
        </div>
      </div>
    );
  }

  if (s.bootstrapStep === 'error') {
    return (
      <div className="flex-1 flex flex-col items-center justify-center p-8 relative overflow-hidden bg-[#08090c]">
        <div className="glass-panel max-w-lg w-full p-8 rounded-xl flex flex-col items-center text-center relative border border-ruby-500/20 shadow-2xl">
          <AlertTriangle className="w-12 h-12 text-ruby-400 mb-4" />
          <h2 className="text-2xl font-outfit font-bold text-white mb-2">Re-bootstrap Failed</h2>
          <p className="text-sm text-slate-400 mb-6">An error occurred while re-building the project workspace.</p>
          <div className="w-full bg-black/40 border border-ruby-500/10 rounded-lg p-4 font-mono text-left text-xs text-ruby-300 overflow-x-auto mb-6">{s.bootstrapError}</div>
          <div className="flex gap-3">
            <button onClick={() => { s.setBootstrapStep('form'); }} className="px-5 py-2.5 text-sm bg-white/5 border border-white/10 hover:bg-white/10 text-white rounded-lg transition-all">Back to Settings</button>
            <button onClick={s.proceedWithReBootstrap} className="px-5 py-2.5 text-sm bg-ruby-600 hover:bg-ruby-500 text-white rounded-lg transition-all font-medium">Retry Build</button>
          </div>
        </div>
      </div>
    );
  }

  if (s.bootstrapStep === 'bootstrap_success') {
    return (
      <div className="flex-1 flex flex-col items-center justify-center p-8 relative overflow-hidden bg-[#08090c]">
        <div className="absolute top-1/4 left-1/2 -translate-x-1/2 w-[600px] h-[300px] bg-emerald-600/10 rounded-full blur-[120px] pointer-events-none" />
        <div className="absolute bottom-1/4 left-1/4 w-[300px] h-[300px] bg-violet-600/10 rounded-full blur-[100px] pointer-events-none" />
        <div className="glass-panel max-w-lg w-full p-8 rounded-2xl flex flex-col items-center text-center relative border border-emerald-500/20 shadow-2xl">
          <div className="relative mb-6">
            <div className="w-20 h-20 rounded-full bg-emerald-500/10 border border-emerald-500/20 flex items-center justify-center animate-pulse">
              <div className="w-14 h-14 rounded-full bg-emerald-500/20 border border-emerald-500/30 flex items-center justify-center">
                <Check className="w-8 h-8 text-emerald-400 stroke-[2.5]" />
              </div>
            </div>
          </div>
          <h2 className="text-2xl font-outfit font-bold text-white mb-2">Workspace Ready</h2>
          <p className="text-sm text-slate-400 mb-6 leading-relaxed">{s.selectedRepos.length} repositor{s.selectedRepos.length !== 1 ? 'ies' : 'y'} bootstrapped successfully.</p>
          <div className="flex gap-3 w-full">
            <button onClick={() => { s.setBootstrapStep('form'); s.fetchWorkspaceHealth(); }} className="flex-1 px-5 py-2.5 text-sm font-medium bg-white/5 border border-white/10 hover:bg-white/10 text-white rounded-lg transition-all flex items-center justify-center gap-2">
              <Activity className="w-4 h-4 text-cyan-400" /> View Workspace Health
            </button>
            <button onClick={() => s.navigate({ kind: 'home' })} className="flex-1 px-6 py-2.5 text-sm font-medium bg-emerald-600 hover:bg-emerald-500 text-white rounded-lg shadow-[0_0_20px_rgba(16,185,129,0.3)] transition-all flex items-center justify-center gap-2">
              <Check className="w-4 h-4" /> Go to Project
            </button>
          </div>
        </div>
      </div>
    );
  }

  if (s.bootstrapStep === 'strategy_proposal') {
    return (
      <div className="flex-1 overflow-y-auto p-8 relative flex items-center justify-center">
        <div className="absolute top-0 left-1/2 -translate-x-1/2 w-[800px] h-[400px] bg-violet-600/10 rounded-full blur-[120px] pointer-events-none" />
        <div className="glass-panel max-w-xl w-full p-6 rounded-xl flex flex-col border-white/10 shadow-2xl">
          <div className="mb-6 border-b border-white/5 pb-4">
            <h3 className="font-outfit font-semibold text-cyan-400 uppercase tracking-widest text-xs mb-1">STRATEGY UPDATED</h3>
            <h2 className="text-xl font-bold text-white">Approve Detected Worktree Strategy</h2>
          </div>
          <div className="space-y-4 max-h-[400px] overflow-y-auto pr-1">
            {[
              { label: 'Default Branch', value: s.defaultBranch, onChange: s.setDefaultBranch },
              { label: 'Branch Prefix', value: s.branchPrefix, onChange: s.setBranchPrefix },
              { label: 'Default Test Command', value: s.testCommand, onChange: s.setTestCommand, placeholder: 'e.g. npm test or cargo test' },
            ].map(({ label, value, onChange, placeholder }) => (
              <div key={label}>
                <label className="block text-[11px] font-mono text-slate-400 mb-1.5 uppercase tracking-wider">{label}</label>
                <input type="text" value={value} onChange={e => onChange(e.target.value)} placeholder={placeholder} className="w-full bg-black/40 border border-white/10 rounded-lg p-2.5 text-xs text-white focus:outline-none focus:border-cyan-500/50 placeholder-slate-600" />
              </div>
            ))}
            <div>
              <label className="block text-[11px] font-mono text-slate-400 mb-1.5 uppercase tracking-wider">Conflict Resolution Policy</label>
              <select value={s.conflictPolicy} onChange={e => s.setConflictPolicy(e.target.value)} className="w-full bg-[#08090c] border border-white/10 rounded-lg p-2.5 text-xs text-white focus:outline-none focus:border-cyan-500/50">
                <option value="always_gate">Always Gate (Requires approval)</option>
                <option value="auto_agent">Auto Agent First (Cascade to manual)</option>
                <option value="auto_human">Immediate Manual Merge</option>
              </select>
            </div>
            <div>
              <label className="block text-[11px] font-mono text-slate-400 mb-1.5 uppercase tracking-wider">Completed Feature Lifecycle</label>
              <select value={s.featureLifecycle} onChange={e => s.setFeatureLifecycle(e.target.value)} className="w-full bg-[#08090c] border border-white/10 rounded-lg p-2.5 text-xs text-white focus:outline-none focus:border-cyan-500/50">
                <option value="archive">Archive by default</option>
                <option value="keep">Keep active</option>
                <option value="auto_delete">Auto delete branch after MR merge</option>
              </select>
            </div>
            {s.prTemplate && (
              <div>
                <label className="block text-[11px] font-mono text-slate-400 mb-1.5 uppercase tracking-wider">Detected PR Template</label>
                <div className="w-full bg-black/40 border border-white/5 rounded-lg p-3 font-mono text-[10px] text-slate-400 max-h-[100px] overflow-y-auto leading-relaxed">{s.prTemplate}</div>
              </div>
            )}
          </div>
          <div className="mt-6 flex justify-end gap-3 border-t border-white/5 pt-4">
            <button onClick={() => s.setBootstrapStep('form')} className="px-5 py-2.5 text-sm font-medium text-slate-400 hover:text-white transition-colors">Back</button>
            <button onClick={s.handleApproveStrategy} className="px-6 py-2.5 text-sm font-medium bg-emerald-600 hover:bg-emerald-500 text-white rounded-lg shadow-[0_0_15px_rgba(16,185,129,0.3)] transition-all flex items-center gap-2">
              <Check className="w-4 h-4" /> Approve & Build Workspace
            </button>
          </div>
        </div>
      </div>
    );
  }

  return (
    <div className="flex-1 overflow-y-auto p-8 relative flex flex-col justify-start max-w-4xl mx-auto w-full">
      <div className="absolute top-0 left-1/2 -translate-x-1/2 w-[800px] h-[300px] bg-violet-600/5 rounded-full blur-[120px] pointer-events-none" />

      {/* Repo Selection Modal */}
      {s.isRepoModalOpen && (
        <Modal onClose={() => s.setIsRepoModalOpen(false)} className="glass-panel w-[500px] border border-white/10 rounded-xl overflow-hidden shadow-2xl flex flex-col">
          <div className="p-4 border-b border-white/5 flex justify-between items-center bg-[#0d0f14]">
            <h3 className="font-outfit font-semibold text-white">Select Repositories</h3>
            <button onClick={() => s.setIsRepoModalOpen(false)} className="text-slate-400 hover:text-white p-1 rounded hover:bg-white/5"><X className="w-5 h-5" /></button>
          </div>
          <div className="p-4 border-b border-white/5 bg-[#08090c]">
            <div className="relative">
              <Search className="w-4 h-4 absolute left-3 top-3 text-slate-500" />
              <input type="text" value={s.repoSearch} onChange={e => s.setRepoSearch(e.target.value)} placeholder="Search repositories..." className="w-full bg-black/40 border border-white/10 rounded-lg py-2.5 pl-9 pr-4 text-sm text-white focus:outline-none focus:border-cyan-500/50" />
            </div>
          </div>
          <div className="overflow-y-auto max-h-[300px] p-2 space-y-1 bg-[#08090c]">
            {s.isLoadingRepos ? (
              <div className="p-4 text-center text-sm text-slate-500">Fetching repositories from connected providers...</div>
            ) : s.availableRepos.length === 0 ? (
              <div className="p-4 text-center text-sm text-slate-500">No repositories found. Make sure providers are connected.</div>
            ) : s.availableRepos.filter(r => r.path.toLowerCase().includes(s.repoSearch.toLowerCase())).map(repo => {
              const isSelected = s.selectedRepos.some(r => r.path === repo.path);
              return (
                <div key={repo.path} onClick={() => s.toggleRepo(repo)} className={`flex items-center gap-3 p-3 rounded-lg cursor-pointer transition-all ${isSelected ? 'bg-cyan-500/10 border border-cyan-500/30' : 'hover:bg-white/5 border border-transparent'}`}>
                  <div className={`w-4 h-4 rounded border flex items-center justify-center ${isSelected ? 'bg-cyan-500 border-cyan-500 text-black' : 'border-slate-600'}`}>
                    {isSelected && <Check className="w-3 h-3 stroke-[3]" />}
                  </div>
                  <Box className={`w-4 h-4 ${isSelected ? 'text-cyan-400' : 'text-slate-500'}`} />
                  <span className={isSelected ? 'text-white' : 'text-slate-300'}>{repo.path}</span>
                </div>
              );
            })}
          </div>
          <div className="p-4 border-t border-white/5 flex justify-end gap-3 bg-[#0d0f14]">
            <button onClick={() => s.setIsRepoModalOpen(false)} className="px-4 py-2 text-sm font-medium bg-cyan-600 hover:bg-cyan-500 text-white rounded-md transition-colors">Done</button>
          </div>
        </Modal>
      )}

      {/* Dirty Warning Modal */}
      {s.dirtyWarningRepos.length > 0 && (
        <Modal onClose={() => { s.setDirtyWarningRepos([]); s.setPendingActionAfterConfirm(null); }} className="glass-panel w-[500px] border border-ruby-500/20 rounded-xl overflow-hidden shadow-2xl flex flex-col p-6 space-y-4">
          <div className="flex items-center gap-3 text-ruby-400"><AlertTriangle className="w-8 h-8 shrink-0 animate-pulse" /><h3 className="font-outfit font-bold text-lg text-white">Potential Data Loss Warning</h3></div>
          <p className="text-sm text-slate-300 leading-relaxed">{s.pendingActionAfterConfirm === 'delete' ? 'The workspace has repositories with uncommitted changes or unpushed commits. Deleting the workspace will permanently erase these directories:' : 'You are about to remove the following repositories, but they contain uncommitted changes or unpushed commits on the local server. Removing them will permanently erase these folders:'}</p>
          <div className="bg-black/40 border border-white/5 rounded-lg p-3 max-h-[200px] overflow-y-auto space-y-2">
            {s.dirtyWarningRepos.map(repo => (
              <div key={repo.repo_path} className="text-xs font-mono p-2 border border-white/5 rounded bg-[#0a0c10]">
                <div className="text-white font-medium truncate mb-1">{repo.repo_path}</div>
                <div className="flex gap-2">
                  {repo.has_uncommitted && <span className="px-1.5 py-0.5 rounded bg-ruby-500/10 border border-ruby-500/20 text-ruby-400">Uncommitted Changes</span>}
                  {repo.has_unpushed && <span className="px-1.5 py-0.5 rounded bg-violet-500/10 border border-violet-500/20 text-violet-400">Unpushed Commits</span>}
                </div>
              </div>
            ))}
          </div>
          <p className="text-xs text-slate-400">Are you absolutely sure you want to proceed and permanently delete these files?</p>
          <div className="flex justify-end gap-3 pt-2">
            <button onClick={() => { s.setDirtyWarningRepos([]); s.setPendingActionAfterConfirm(null); }} className="px-4 py-2 text-sm bg-white/5 border border-white/10 hover:bg-white/10 text-white rounded-lg transition-all">Cancel</button>
            <button onClick={async () => { if (s.pendingActionAfterConfirm === 'delete') { await s.proceedWithDelete(); } else { s.setDirtyWarningRepos([]); s.setPendingActionAfterConfirm(null); await s.proceedWithReBootstrap(); } }} className="px-4 py-2 text-sm bg-ruby-600 hover:bg-ruby-500 text-white rounded-lg font-medium transition-all">Proceed Anyway</button>
          </div>
        </Modal>
      )}

      {/* Delete Confirm Modal */}
      {s.showDeleteConfirm && (
        <Modal onClose={() => s.setShowDeleteConfirm(false)} className="glass-panel w-[450px] border border-ruby-500/20 rounded-xl overflow-hidden shadow-2xl flex flex-col p-6 space-y-4">
          <div className="flex items-center gap-3 text-ruby-400"><Trash2 className="w-7 h-7 shrink-0" /><h3 className="font-outfit font-bold text-lg text-white">Delete Workspace</h3></div>
          <p className="text-sm text-slate-300 leading-relaxed">Are you sure you want to delete <span className="text-white font-semibold">{s.projectName}</span>? This will permanently delete the project record and remove all local workspace clones.</p>
          <div className="flex justify-end gap-3 pt-2">
            <button onClick={() => s.setShowDeleteConfirm(false)} className="px-4 py-2 text-sm bg-white/5 border border-white/10 hover:bg-white/10 text-white rounded-lg transition-all">Cancel</button>
            <button onClick={s.proceedWithDelete} className="px-4 py-2 text-sm bg-ruby-600 hover:bg-ruby-500 text-white rounded-lg font-medium transition-all">Delete Permanently</button>
          </div>
        </Modal>
      )}

      {/* Header */}
      <div className="flex justify-between items-center mb-8 border-b border-white/5 pb-4 z-10">
        <div>
          <h1 className="text-2xl font-outfit font-bold text-white mb-2">Workspace Settings</h1>
          <p className="text-sm text-slate-400">Configure code isolation rules, repositories and environment configurations for <span className="text-white font-medium">{s.activeProject.name}</span>.</p>
        </div>
        <button onClick={s.handleSave} disabled={s.status === 'saving'} className="bg-cyan-600 hover:bg-cyan-500 disabled:bg-cyan-600/50 text-white font-medium text-sm px-4 py-2 rounded-lg transition-all shadow-[0_0_15px_rgba(6,182,212,0.3)] flex items-center gap-2">
          {s.status === 'saving' ? <RotateCw className="w-4 h-4 animate-spin" /> : <Save className="w-4 h-4" />}
          Save Changes
        </button>
      </div>

      {s.activeProject.status === 'error' && (
        <div className="mb-6 bg-ruby-500/10 border border-ruby-500/20 rounded-xl p-4 flex items-start gap-3 shadow-lg z-10">
          <AlertTriangle className="w-5 h-5 text-ruby-400 shrink-0 mt-0.5 animate-pulse" />
          <div>
            <h4 className="font-outfit font-bold text-white text-sm">Workspace Bootstrap Failed</h4>
            <p className="text-xs text-slate-300 mt-1">The build for this workspace could not complete. Verify target compute availability, credentials, and mapped repository paths, then click <strong>Save Changes</strong> to retry the build.</p>
          </div>
        </div>
      )}

      <TabBar className="mb-6 z-10" tabs={[
        { key: 'general', label: 'General & Repositories' },
        { key: 'strategy', label: 'Agent Strategy & Policies' },
        { key: 'overrides', label: 'Workflow Overrides' },
        { key: 'memory', label: 'Project Memory' },
      ] satisfies TabDef[]} activeTab={s.activeTab} onChange={k => s.setActiveTab(k as typeof s.activeTab)} />

      <div className="z-10">
        {s.activeTab === 'general' ? <GeneralTab />
          : s.activeTab === 'strategy' ? <StrategyTab />
          : s.activeTab === 'overrides' ? <OverridesTab />
          : <MemoryTab />}

        {s.status === 'error' && (
          <div className="bg-ruby-500/10 border border-ruby-500/30 p-3 rounded-lg flex items-start gap-3 mt-6">
            <ShieldAlert className="w-5 h-5 text-ruby-400 shrink-0" />
            <span className="text-sm text-ruby-200">{s.errorMsg}</span>
          </div>
        )}
        {s.status === 'success' && (
          <div className="bg-emerald-500/10 border border-emerald-500/30 p-3 rounded-lg flex items-center gap-3 mt-6">
            <div className="w-6 h-6 rounded-full bg-emerald-500 flex items-center justify-center shrink-0"><Check className="w-4 h-4 text-black stroke-[3]" /></div>
            <span className="text-sm text-emerald-300 font-medium">Strategy settings saved. No structural changes detected — workspace remains healthy.</span>
          </div>
        )}
      </div>

      <div className="mt-8 flex justify-end gap-3 z-10 border-t border-white/5 pt-4">
        <button onClick={() => s.navigate({ kind: 'home' })} className="px-5 py-2.5 rounded-lg text-sm text-slate-400 hover:text-white hover:bg-white/5 transition-all">Back to Project</button>
      </div>
    </div>
  );
}

export default function ProjectSettingsView() {
  return (
    <ProjectSettingsProvider>
      <Shell />
    </ProjectSettingsProvider>
  );
}
