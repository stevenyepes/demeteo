import React, { useState, useEffect, useRef, useCallback } from 'react';
import { Search, ArrowRight } from 'lucide-react';

interface CommandEntry {
  id: string;
  label: string;
  description: string;
  category: 'project' | 'feature' | 'workflow' | 'action' | 'settings';
  icon: React.ReactNode;
  onSelect: () => void;
}

interface CommandPaletteProps {
  isOpen: boolean;
  onClose: () => void;
  entries: CommandEntry[];
}

function fuzzyMatch(text: string, query: string): boolean {
  const lower = text.toLowerCase();
  const q = query.toLowerCase();
  let qi = 0;
  for (let i = 0; i < lower.length && qi < q.length; i++) {
    if (lower[i] === q[qi]) qi++;
  }
  return qi === q.length;
}

const CommandPalette: React.FC<CommandPaletteProps> = ({ isOpen, onClose, entries }) => {
  const [query, setQuery] = useState('');
  const [selectedIndex, setSelectedIndex] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);

  const filtered = query
    ? entries.filter(e => fuzzyMatch(`${e.label} ${e.description} ${e.category}`, query))
    : entries;

  useEffect(() => {
    if (isOpen) {
      setQuery('');
      setSelectedIndex(0);
      setTimeout(() => inputRef.current?.focus(), 50);
    }
  }, [isOpen]);

  useEffect(() => {
    setSelectedIndex(0);
  }, [query]);

  const handleKeyDown = useCallback((e: React.KeyboardEvent) => {
    if (e.key === 'Escape') {
      onClose();
      return;
    }
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      setSelectedIndex(prev => Math.min(prev + 1, filtered.length - 1));
      return;
    }
    if (e.key === 'ArrowUp') {
      e.preventDefault();
      setSelectedIndex(prev => Math.max(prev - 1, 0));
      return;
    }
    if (e.key === 'Enter' && filtered[selectedIndex]) {
      e.preventDefault();
      filtered[selectedIndex].onSelect();
      onClose();
      return;
    }
  }, [filtered, selectedIndex, onClose]);

  useEffect(() => {
    if (listRef.current) {
      const el = listRef.current.children[selectedIndex] as HTMLElement;
      el?.scrollIntoView({ block: 'nearest' });
    }
  }, [selectedIndex]);

  if (!isOpen) return null;

  return (
    <div className="fixed inset-0 z-[60] flex items-start justify-center pt-[15vh]" onClick={onClose}>
      <div className="absolute inset-0 bg-black/60 backdrop-blur-sm" />
      <div
        className="relative w-full max-w-xl glass-panel border border-white/10 rounded-xl shadow-2xl overflow-hidden"
        onClick={e => e.stopPropagation()}
      >
        <div className="flex items-center gap-3 px-5 py-3.5 border-b border-white/5 bg-[#0d0f14]/80">
          <Search className="w-5 h-5 text-slate-400 shrink-0" />
          <input
            ref={inputRef}
            type="text"
            value={query}
            onChange={e => setQuery(e.target.value)}
            onKeyDown={handleKeyDown}
            placeholder="Search projects, features, workflows, settings..."
            className="flex-1 bg-transparent text-sm text-white placeholder-slate-500 focus:outline-none"
          />
          <span className="text-[10px] font-mono text-slate-600 border border-white/10 px-1.5 py-0.5 rounded">Esc</span>
        </div>
        <div
          ref={listRef}
          className="max-h-[60vh] overflow-y-auto py-2 space-y-0.5"
        >
          {filtered.length === 0 ? (
            <div className="px-5 py-8 text-center text-sm text-slate-500">
              No results for <span className="text-slate-300 font-mono">"{query}"</span>
            </div>
          ) : (
            filtered.map((entry, idx) => (
              <div
                key={entry.id}
                onClick={() => {
                  entry.onSelect();
                  onClose();
                }}
                className={`flex items-center gap-3 px-5 py-2.5 cursor-pointer transition-all mx-1.5 rounded-lg ${
                  idx === selectedIndex
                    ? 'bg-violet-500/15 text-white border border-violet-500/20'
                    : 'text-slate-300 hover:bg-white/5 hover:text-white'
                }`}
              >
                <span className="shrink-0 opacity-70">{entry.icon}</span>
                <div className="flex-1 min-w-0">
                  <div className="text-sm font-medium truncate">{entry.label}</div>
                  <div className={`text-[11px] truncate ${idx === selectedIndex ? 'text-slate-400' : 'text-slate-500'}`}>
                    {entry.description}
                  </div>
                </div>
                <span className={`text-[10px] font-mono uppercase tracking-wider shrink-0 ${
                  idx === selectedIndex ? 'text-violet-400' : 'text-slate-600'
                }`}>
                  {entry.category}
                </span>
                {idx === selectedIndex && (
                  <ArrowRight className="w-3.5 h-3.5 text-violet-400 shrink-0" />
                )}
              </div>
            ))
          )}
        </div>
      </div>
    </div>
  );
};

export type { CommandEntry };
export default CommandPalette;
