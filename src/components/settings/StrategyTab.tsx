import { GitBranch, Zap, Settings, FileText, Activity, Check, RotateCw, Trash2, Plus } from 'lucide-react';
import { useSettings } from './ProjectSettingsContext';

export function StrategyTab() {
  const s = useSettings();

  return (
    <div className="grid grid-cols-1 md:grid-cols-2 gap-6">
      {/* Git Isolation */}
      <div className="glass-panel p-6 rounded-xl space-y-4">
        <h3 className="font-outfit text-sm font-semibold text-slate-300 uppercase tracking-wider mb-2 flex items-center gap-2">
          <GitBranch className="w-4 h-4 text-violet-400" /> Git Isolation & Strategy
        </h3>
        {[
          { label: 'Default Branch', value: s.defaultBranch, onChange: s.setDefaultBranch },
          { label: 'Branch Prefix', value: s.branchPrefix, onChange: s.setBranchPrefix },
          { label: 'Default Test Command', value: s.testCommand, onChange: s.setTestCommand, placeholder: 'e.g. npm test or cargo test' },
        ].map(({ label, value, onChange, placeholder }) => (
          <div key={label}>
            <label className="block text-xs font-mono text-slate-400 mb-1.5 uppercase tracking-wider">{label}</label>
            <input type="text" value={value} onChange={e => onChange(e.target.value)} placeholder={placeholder} className="w-full bg-black/40 border border-white/10 rounded-lg py-2 px-3 text-sm text-white focus:outline-none focus:border-cyan-500/50 placeholder-slate-600" />
          </div>
        ))}
      </div>

      {/* Named Test Harnesses */}
      <div className="glass-panel p-6 rounded-xl space-y-4">
        <h3 className="font-outfit text-sm font-semibold text-slate-300 uppercase tracking-wider mb-2 flex items-center gap-2">
          <Zap className="w-4 h-4 text-cyan-400" /> Named Test Harnesses
        </h3>
        <p className="text-xs text-slate-400 leading-relaxed">
          Define named test harness commands to verify agent-generated code (e.g., key: <code>lint</code>, command: <code>npm run lint</code>).
        </p>
        <div className="space-y-3">
          {Object.entries(s.harnesses).map(([name, cmd]) => (
            <div key={name} className="flex gap-2 items-center">
              <div className="flex-1 font-mono text-xs bg-black/40 border border-white/10 rounded-lg p-2 text-white truncate">
                <span className="text-cyan-400">{name}</span>: <span className="text-slate-300">{cmd}</span>
              </div>
              <button type="button" onClick={() => { const copy = { ...s.harnesses }; delete copy[name]; s.setHarnesses(copy); }} className="p-2 text-slate-500 hover:text-ruby-400 bg-white/5 rounded-lg border border-white/5 hover:bg-white/10 shrink-0" title="Delete harness">
                <Trash2 className="w-3.5 h-3.5" />
              </button>
            </div>
          ))}
          <div className="border-t border-white/5 pt-3 flex gap-2">
            <input type="text" placeholder="Name" id="new-harness-name" className="w-1/3 bg-black/40 border border-white/10 rounded-lg py-1.5 px-3 text-xs text-white placeholder-slate-600 focus:outline-none focus:border-cyan-500/50 font-mono" />
            <input type="text" placeholder="Command" id="new-harness-cmd" className="flex-1 bg-black/40 border border-white/10 rounded-lg py-1.5 px-3 text-xs text-white placeholder-slate-600 focus:outline-none focus:border-cyan-500/50 font-mono" />
            <button type="button" onClick={() => {
              const nameEl = document.getElementById('new-harness-name') as HTMLInputElement;
              const cmdEl = document.getElementById('new-harness-cmd') as HTMLInputElement;
              if (nameEl && cmdEl) {
                const name = nameEl.value.trim(); const cmd = cmdEl.value.trim();
                if (name && cmd) { s.setHarnesses({ ...s.harnesses, [name]: cmd }); nameEl.value = ''; cmdEl.value = ''; }
              }
            }} className="px-3 py-1.5 text-xs bg-cyan-600 hover:bg-cyan-500 text-white rounded-lg transition-colors flex items-center gap-1 font-semibold shrink-0">
              <Plus className="w-3 h-3" /> Add
            </button>
          </div>
        </div>
      </div>

      {/* Automation Policies */}
      <div className="glass-panel p-6 rounded-xl space-y-4">
        <h3 className="font-outfit text-sm font-semibold text-slate-300 uppercase tracking-wider mb-2 flex items-center gap-2">
          <Settings className="w-4 h-4 text-cyan-400" /> Automation Policies
        </h3>
        <div>
          <label className="block text-xs font-mono text-slate-400 mb-1.5 uppercase tracking-wider">Conflict Resolution Policy</label>
          <select value={s.conflictPolicy} onChange={e => s.setConflictPolicy(e.target.value)} className="w-full bg-[#08090c] border border-white/10 rounded-lg py-2.5 px-3 text-sm text-white focus:outline-none focus:border-cyan-500/50">
            <option value="always_gate">Always Gate (Requires approval)</option>
            <option value="auto_agent">Auto Agent First (Cascade to manual)</option>
            <option value="auto_human">Immediate Manual Merge</option>
          </select>
        </div>
        <div>
          <label className="block text-xs font-mono text-slate-400 mb-1.5 uppercase tracking-wider">Completed Feature Lifecycle</label>
          <select value={s.featureLifecycle} onChange={e => s.setFeatureLifecycle(e.target.value)} className="w-full bg-[#08090c] border border-white/10 rounded-lg py-2.5 px-3 text-sm text-white focus:outline-none focus:border-cyan-500/50">
            <option value="archive">Archive by default</option>
            <option value="keep">Keep active</option>
            <option value="auto_delete">Auto delete branch after MR merge</option>
          </select>
        </div>
      </div>

      {/* Default AI Executor */}
      <div className="glass-panel p-6 rounded-xl space-y-4">
        <h3 className="font-outfit text-sm font-semibold text-slate-300 uppercase tracking-wider mb-2 flex items-center gap-2">
          <Zap className="w-4 h-4 text-violet-400" /> Default AI Executor Settings
        </h3>
        <div>
          <label className="block text-xs font-mono text-slate-400 mb-1.5 uppercase tracking-wider">Default Coding Agent</label>
          <select value={s.defaultAgentKind} onChange={e => { s.setDefaultAgentKind(e.target.value); s.setDefaultModel(''); }} className="w-full bg-[#08090c] border border-white/10 rounded-lg py-2.5 px-3 text-sm text-white focus:outline-none focus:border-cyan-500/50 capitalize">
            <option value="">No default (Prompt on feature creation)</option>
            {s.agentConfigs.filter(a => a.enabled && a.available && a.kind !== 'antigravity').map(a => <option key={a.kind} value={a.kind}>{a.kind.replace(/-/g, ' ')}</option>)}
          </select>
        </div>
        <div>
          <label className="block text-xs font-mono text-slate-400 mb-1.5 uppercase tracking-wider">Default Model</label>
          {s.isLoadingModelsForDefault ? (
            <div className="w-full bg-[#08090c]/40 border border-white/10 rounded-lg py-2.5 px-3 text-sm text-slate-400 flex items-center gap-2">
              <RotateCw className="w-3.5 h-3.5 animate-spin text-cyan-400" /><span>Probing available models...</span>
            </div>
          ) : (
            <div className="flex gap-2">
              <select value={s.defaultModel} onChange={e => s.setDefaultModel(e.target.value)} className="flex-1 min-w-0 bg-[#08090c] border border-white/10 rounded-lg py-2.5 px-3 text-sm text-white focus:outline-none focus:border-cyan-500/50" disabled={!s.defaultAgentKind}>
                <option value="">No default model</option>
                {s.availableModelsForDefault.map(m => <option key={m.value} value={m.value}>{m.name}</option>)}
                {s.defaultModel && !s.availableModelsForDefault.some(m => m.value === s.defaultModel) && <option value={s.defaultModel}>{s.defaultModel} (custom)</option>}
              </select>
              <input type="text" value={s.defaultModel} onChange={e => s.setDefaultModel(e.target.value)} placeholder="Custom override" className="w-1/3 shrink-0 min-w-[140px] bg-black/40 border border-white/10 rounded-lg py-2.5 px-3 text-sm text-white focus:outline-none focus:border-cyan-500/50 font-mono placeholder-slate-600" disabled={!s.defaultAgentKind} />
            </div>
          )}
          <div className="mt-4">
            <label className="block text-xs font-mono text-slate-400 mb-1.5 uppercase tracking-wider">Default Loop Iterations</label>
            <input type="number" min={1} max={10} value={s.defaultLoopIterations} onChange={e => s.setDefaultLoopIterations(e.target.value)} placeholder="3 (engine default)" className="w-40 bg-[#08090c] border border-white/10 rounded-lg py-2.5 px-3 text-sm text-white focus:outline-none focus:border-cyan-500/50 font-mono placeholder-slate-600" />
            <p className="text-[11px] text-slate-500 mt-1.5 leading-relaxed">How many times a validation step may loop back to implementation before giving up. Leave blank to use the engine default (3). Overridable per run.</p>
          </div>
        </div>
      </div>

      {/* Artifact Handling */}
      <div className="glass-panel p-6 rounded-xl space-y-4">
        <h3 className="font-outfit text-sm font-semibold text-slate-300 uppercase tracking-wider mb-2 flex items-center gap-2">
          <FileText className="w-4 h-4 text-cyan-400" /> Artifact Handling
        </h3>
        <p className="text-xs text-slate-400 leading-relaxed">Each workflow step produces a report (<code className="text-slate-300">research-report.md</code>, <code className="text-slate-300">critic-review.md</code>, …). By default these land in a subfolder and stay out of the PR — view them in demeteo's artifact panel. Toggle the commit switch to ship them with the feature branch instead.</p>
        <div>
          <label className="block text-xs font-mono text-slate-400 mb-1.5 uppercase tracking-wider">Artifact Subfolder</label>
          <input type="text" value={s.artifactSubdir} onChange={e => s.setArtifactSubdir(e.target.value)} placeholder="artifacts/" className="w-full bg-black/40 border border-white/10 rounded-lg py-2 px-3 text-sm text-white focus:outline-none focus:border-cyan-500/50 font-mono placeholder-slate-600" />
          <p className="text-[10px] font-mono text-slate-500 mt-1.5 leading-relaxed">Repo-relative path. The orchestrator injects this as <code>{'{{artifact_dir}}'}</code> into every step's prompt and excludes it from <code>git add</code> when the commit switch is off.</p>
        </div>
        <label className="flex items-start gap-3 p-3 rounded-lg border border-white/5 bg-black/20 cursor-pointer hover:border-cyan-500/30 transition-colors">
          <input type="checkbox" checked={s.commitArtifacts} onChange={e => s.setCommitArtifacts(e.target.checked)} className="mt-0.5 w-4 h-4 rounded border-white/20 bg-black/40 text-cyan-500 focus:ring-cyan-500/40 focus:ring-offset-0" />
          <div className="flex-1">
            <div className="text-xs font-semibold text-slate-200">Commit artifacts to the feature branch</div>
            <div className="text-[11px] text-slate-400 mt-0.5 leading-relaxed">When off (default), the orchestrator runs <code>git add -A -- ':!&lt;artifact_subfolder&gt;'</code> so the reports stay as untracked files in the worktree. The UI viewer still shows them.</div>
          </div>
        </label>
      </div>

      {s.prTemplate && (
        <div className="glass-panel p-6 rounded-xl md:col-span-2 space-y-2">
          <h3 className="font-outfit text-sm font-semibold text-slate-300 uppercase tracking-wider">Detected PR Template</h3>
          <div className="w-full bg-black/40 border border-white/5 rounded-lg p-4 font-mono text-xs text-slate-400 max-h-[160px] overflow-y-auto leading-relaxed">{s.prTemplate}</div>
        </div>
      )}

      {/* Coding Agent Configurations */}
      <div className="glass-panel p-6 rounded-xl md:col-span-2 space-y-4">
        <div className="flex items-center justify-between border-b border-white/5 pb-2">
          <h3 className="font-outfit text-sm font-semibold text-slate-300 uppercase tracking-wider flex items-center gap-2">
            <Activity className="w-4 h-4 text-cyan-400 animate-pulse" /> Coding Agent Configuration
          </h3>
          <button type="button" onClick={() => s.fetchAgentConfigs(true)} disabled={s.isRefreshingAgents} className="p-1 rounded text-slate-500 hover:text-cyan-400 hover:bg-white/5 transition-all disabled:opacity-50" title="Re-check agent availability">
            <RotateCw className={`w-3.5 h-3.5 ${s.isRefreshingAgents ? 'animate-spin text-cyan-400' : ''}`} />
          </button>
        </div>
        <p className="text-xs text-slate-400">Enable or disable specific AI coding agents for this workspace. Demeteo validates if these agents' CLI binaries are available on the selected compute server.</p>
        <div className="grid grid-cols-1 md:grid-cols-2 gap-4 mt-2">
          {s.agentConfigs.filter(a => a.kind !== 'antigravity').length === 0 ? (
            <div className="md:col-span-2 text-xs text-slate-500 italic p-2">No agents found on target machine.</div>
          ) : s.agentConfigs.filter(a => a.kind !== 'antigravity').map(agent => (
            <div key={agent.kind} className={`flex items-start justify-between p-4 rounded-lg border transition-all ${agent.enabled ? 'bg-violet-500/5 border-violet-500/25 shadow-[0_0_15px_rgba(139,92,246,0.05)]' : 'bg-black/20 border-white/5 opacity-60'}`}>
              <div className="flex gap-3 w-full">
                <div className="pt-0.5">
                  <button type="button" onClick={() => s.setAgentConfigs(s.agentConfigs.map(a => a.kind === agent.kind ? { ...a, enabled: !a.enabled } : a))} className={`w-4 h-4 rounded border flex items-center justify-center transition-all ${agent.enabled ? 'bg-violet-500 border-violet-500 text-white' : 'border-slate-600 hover:border-slate-500'}`}>
                    {agent.enabled && <Check className="w-3 h-3 stroke-[3]" />}
                  </button>
                </div>
                <div className="flex-1 min-w-0">
                  <div className="flex items-center gap-2 flex-wrap">
                    <span className="text-sm font-semibold text-white font-outfit capitalize">{agent.kind.replace(/-/g, ' ')}</span>
                    {agent.available ? (
                      <span className="flex items-center gap-1 px-1.5 py-0.5 text-[9px] rounded bg-emerald-500/10 border border-emerald-500/20 text-emerald-400 font-mono"><span className="w-1.5 h-1.5 rounded-full bg-emerald-400" />Available</span>
                    ) : (
                      <span className="flex items-center gap-1 px-1.5 py-0.5 text-[9px] rounded bg-ruby-500/10 border border-ruby-500/20 text-ruby-400 font-mono"><span className="w-1.5 h-1.5 rounded-full bg-ruby-400" />Missing</span>
                    )}
                  </div>
                  <p className="text-[11px] text-slate-400 mt-1 leading-relaxed">
                    {agent.kind === 'opencode' && 'Local open-source developer agent.'}
                    {agent.kind === 'hermes' && 'Autonomic codebase planner and execution agent.'}
                    {agent.kind === 'claude-code' && 'Claude Code agent for complex tasks.'}
                    {!['opencode', 'hermes', 'claude-code', 'antigravity'].includes(agent.kind) && 'Additional configured coding agent.'}
                  </p>
                  {!agent.available && agent.install_command && (
                    <div className="mt-2.5 p-2 bg-black/40 border border-white/5 rounded font-mono text-[9px] text-slate-300 flex items-center justify-between gap-2 select-all overflow-x-auto">
                      <span>{agent.install_command}</span>
                    </div>
                  )}
                </div>
              </div>
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}
