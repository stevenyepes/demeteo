import { Brain, RotateCw, Pencil, Trash2, Check, X, AlertTriangle } from 'lucide-react';
import { useSettings } from './ProjectSettingsContext';

export function MemoryTab() {
  const s = useSettings();

  return (
    <div className="space-y-6 animate-fadeIn">
      <div className="glass-panel p-6 rounded-xl space-y-4">
        <h3 className="font-outfit text-sm font-semibold text-slate-300 uppercase tracking-wider flex items-center gap-2">
          <Brain className="w-4 h-4 text-violet-400" /> Project Context Memory
        </h3>
        <p className="text-xs text-slate-400 leading-relaxed">
          These key-value entries are injected into the AI agent's system context whenever it works on a feature in this project — useful for persisting architectural decisions, coding conventions, or team norms.
        </p>

        {s.memError && (
          <div className="bg-ruby-500/10 border border-ruby-500/30 p-3 rounded-lg flex items-start gap-3">
            <AlertTriangle className="w-4 h-4 text-ruby-400 shrink-0 mt-0.5" />
            <span className="text-sm text-ruby-200">{s.memError}</span>
          </div>
        )}

        {s.isMemoriesLoading ? (
          <div className="flex items-center justify-center py-8"><RotateCw className="w-5 h-5 text-cyan-400 animate-spin" /></div>
        ) : s.memories.length === 0 ? (
          <p className="text-xs text-slate-500 italic py-2">No project memory entries yet.</p>
        ) : (
          <div className="space-y-2 max-h-[300px] overflow-y-auto pr-1">
            {s.memories.map(entry => (
              <div key={entry.id} className="flex items-start gap-3 p-3 border border-white/5 rounded-lg bg-black/20 group">
                <div className="flex-1 min-w-0">
                  <div className="text-xs font-semibold text-cyan-300 font-mono truncate">{entry.key}</div>
                  <div className="text-xs text-slate-300 mt-0.5 line-clamp-2 leading-relaxed">{entry.value}</div>
                  {entry.source && entry.source !== 'human' && (
                    <span className="mt-1 inline-block text-[9px] font-mono px-1.5 py-0.5 rounded bg-violet-500/10 border border-violet-500/20 text-violet-400 uppercase tracking-wider">{entry.source}</span>
                  )}
                </div>
                <div className="flex gap-1 opacity-0 group-hover:opacity-100 transition-opacity shrink-0">
                  <button onClick={() => s.handleEditMemoryClick(entry)} className="p-1.5 rounded text-slate-400 hover:text-cyan-400 hover:bg-white/5 transition-all" title="Edit">
                    <Pencil className="w-3.5 h-3.5" />
                  </button>
                  <button onClick={() => s.handleDeleteMemory(entry.id)} className="p-1.5 rounded text-slate-400 hover:text-ruby-400 hover:bg-white/5 transition-all" title="Delete">
                    <Trash2 className="w-3.5 h-3.5" />
                  </button>
                </div>
              </div>
            ))}
          </div>
        )}

        {/* Add / Edit Form */}
        <form onSubmit={s.handleSaveMemory} className="border-t border-white/5 pt-4 space-y-3">
          <div className="flex items-center gap-2 mb-1">
            <span className="text-xs font-semibold text-slate-300">{s.editingMemory ? 'Edit Entry' : 'Add Entry'}</span>
            {s.editingMemory && (
              <button type="button" onClick={s.handleCancelEdit} className="flex items-center gap-1 text-[11px] text-slate-500 hover:text-white transition-colors">
                <X className="w-3 h-3" /> Cancel
              </button>
            )}
          </div>
          <div className="flex gap-2">
            <div className="flex-1 min-w-0">
              <label className="block text-[10px] font-mono text-slate-500 mb-1 uppercase tracking-wider">Key</label>
              <input type="text" value={s.newMemKey} onChange={e => s.setNewMemKey(e.target.value)} placeholder="e.g. coding_style" className="w-full bg-black/40 border border-white/10 rounded-lg py-2 px-3 text-xs text-white font-mono focus:outline-none focus:border-cyan-500/50 placeholder-slate-600" required />
            </div>
            <div className="flex-[2] min-w-0">
              <label className="block text-[10px] font-mono text-slate-500 mb-1 uppercase tracking-wider">Value</label>
              <input type="text" value={s.newMemVal} onChange={e => s.setNewMemVal(e.target.value)} placeholder="e.g. Use snake_case for all file names" className="w-full bg-black/40 border border-white/10 rounded-lg py-2 px-3 text-xs text-white focus:outline-none focus:border-cyan-500/50 placeholder-slate-600" required />
            </div>
          </div>
          <div className="flex justify-end">
            <button type="submit" className="px-4 py-2 text-xs font-semibold bg-cyan-600 hover:bg-cyan-500 text-white rounded-lg transition-colors flex items-center gap-1.5">
              <Check className="w-3.5 h-3.5" /> {s.editingMemory ? 'Save Changes' : 'Add Entry'}
            </button>
          </div>
        </form>
      </div>
    </div>
  );
}
