/**
 * Graph | Timeline toggle for the run column (PRD §6.1).
 *
 * List stays the default; the graph is the same pinned-version definition with
 * the live `run_events`-driven overlay. It lives in its own file because it is
 * also *chrome* — one of the two elements `useRunColumnLayout` measures to work
 * out how much height is left for the graph box — so the `ref` it takes is the
 * hook's, not decoration.
 */
import { List, Network } from 'lucide-react';

export type RunViewMode = 'graph' | 'timeline';

interface RunViewToggleProps {
  mode: RunViewMode;
  onSelect: (mode: RunViewMode) => void;
  /** `setToggleChromeEl` from `useRunColumnLayout`. */
  chromeRef: (el: HTMLDivElement | null) => void;
}

const TAB = 'flex items-center gap-1.5 rounded-md px-3 py-1.5 text-xs font-semibold transition';
const SELECTED = 'bg-cyan-500/15 text-cyan-300 shadow-[0_0_10px_rgba(6,182,212,0.15)]';
const IDLE = 'text-slate-400 hover:text-slate-200';

export function RunViewToggle({ mode, onSelect, chromeRef }: RunViewToggleProps) {
  return (
    <div
      ref={chromeRef}
      className="mb-6 inline-flex shrink-0 self-start items-center gap-1 rounded-lg border border-white/10 bg-white/[0.02] p-1"
    >
      <button onClick={() => onSelect('graph')} className={`${TAB} ${mode === 'graph' ? SELECTED : IDLE}`}>
        <Network className="h-3.5 w-3.5" /> Graph
      </button>
      <button onClick={() => onSelect('timeline')} className={`${TAB} ${mode === 'timeline' ? SELECTED : IDLE}`}>
        <List className="h-3.5 w-3.5" /> Timeline
      </button>
    </div>
  );
}
