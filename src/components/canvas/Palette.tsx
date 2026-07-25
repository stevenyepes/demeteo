/**
 * The builder's node palette and its two pickers (task P3.1, PRD §6.3).
 *
 * Every entry is rendered from a `NodeTypeInfo` handed over by
 * `node_types_list` — nothing here enumerates kinds, so a node type
 * registered in Rust (P3.5's `command`, later `subworkflow`) shows up with no
 * edit to this file. The only per-kind frontend knowledge is the lucide icon
 * in `types.ts`, which already falls back for an unrecognised kind.
 *
 * Three surfaces, one list renderer:
 *
 * - `Palette` — the always-visible rail. Drag an entry onto the canvas.
 * - `NodeTypePicker` — the overlay used both for Cmd+K in-canvas search and
 *   for the "what can connect here" list when a drag from an output handle
 *   ends on empty canvas.
 *
 * Entries at their instance cap (`max_instances`, e.g. one `finalize` per
 * workflow) render disabled with the reason rather than disappearing, so the
 * author learns the rule instead of wondering where the entry went.
 */
import { useEffect, useMemo, useRef, useState } from 'react';
import { Search } from 'lucide-react';

import { nodeTypeMeta } from './types';
import { TONE_CHIP, TONE_TEXT } from '../../lib/runStatus';
import type { NodeTypeInfo } from './nodeCatalog';

/** The drag payload key the canvas's drop handler reads. */
export const NODE_TYPE_MIME = 'application/x-demeteo-node-type';

export interface PaletteEntry {
  type: NodeTypeInfo;
  /** Disabled with this explanation when set (instance cap reached). */
  disabledReason?: string;
}

function matches(type: NodeTypeInfo, query: string): boolean {
  if (!query) return true;
  const q = query.toLowerCase();
  return (
    type.label.toLowerCase().includes(q) ||
    type.kind.toLowerCase().includes(q) ||
    type.summary.toLowerCase().includes(q)
  );
}

interface NodeTypeRowProps {
  entry: PaletteEntry;
  draggable: boolean;
  active?: boolean;
  onSelect: (type: NodeTypeInfo) => void;
}

function NodeTypeRow({ entry, draggable, active, onSelect }: NodeTypeRowProps) {
  const { type, disabledReason } = entry;
  const meta = nodeTypeMeta(type.kind);
  const Icon = meta.icon;
  const disabled = Boolean(disabledReason);

  return (
    <button
      type="button"
      role="option"
      aria-selected={Boolean(active)}
      disabled={disabled}
      draggable={draggable && !disabled}
      onDragStart={(evt) => {
        evt.dataTransfer.setData(NODE_TYPE_MIME, type.kind);
        evt.dataTransfer.effectAllowed = 'copy';
      }}
      onClick={() => !disabled && onSelect(type)}
      title={disabledReason ?? type.summary}
      className={[
        'flex w-full items-start gap-2.5 rounded-lg border px-2.5 py-2 text-left transition-colors',
        disabled
          ? 'cursor-not-allowed border-slate-800/60 bg-slate-900/40 opacity-50'
          : active
            ? 'border-cyan-400/60 bg-slate-800/70'
            : 'border-slate-700/50 bg-slate-900/60 hover:border-slate-600 hover:bg-slate-800/60',
      ].join(' ')}
    >
      <span
        className={[
          'mt-0.5 flex h-7 w-7 shrink-0 items-center justify-center rounded-md border',
          TONE_CHIP[meta.tone],
        ].join(' ')}
      >
        <Icon className={`h-3.5 w-3.5 ${TONE_TEXT[meta.tone]}`} aria-hidden />
      </span>
      <span className="min-w-0 flex-1">
        <span className="block text-sm font-medium text-slate-100">{type.label}</span>
        <span className="block text-[11px] leading-snug text-slate-400">
          {disabledReason ?? type.summary}
        </span>
      </span>
    </button>
  );
}

export interface PaletteProps {
  entries: PaletteEntry[];
  /** Click-to-add (drag is the primary gesture; click is the a11y path). */
  onSelect: (type: NodeTypeInfo) => void;
  className?: string;
}

export function Palette({ entries, onSelect, className = '' }: PaletteProps) {
  const [query, setQuery] = useState('');
  const shown = useMemo(() => entries.filter((e) => matches(e.type, query)), [entries, query]);

  return (
    <div
      className={`flex w-56 flex-col gap-2 rounded-xl border border-slate-700/60 bg-slate-900/85 p-2.5 backdrop-blur-sm ${className}`}
      data-testid="node-palette"
    >
      <div className="relative">
        <Search
          className="pointer-events-none absolute left-2 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-slate-500"
          aria-hidden
        />
        <input
          type="search"
          value={query}
          onChange={(evt) => setQuery(evt.target.value)}
          placeholder="Search nodes"
          aria-label="Search node types"
          className="w-full rounded-lg border border-slate-700/60 bg-slate-950/60 py-1.5 pl-7 pr-2 text-xs text-slate-200 placeholder:text-slate-500 focus:border-cyan-500/50 focus:outline-none"
        />
      </div>

      <div role="listbox" aria-label="Node types" className="flex flex-col gap-1.5">
        {shown.map((entry) => (
          <NodeTypeRow
            key={entry.type.kind}
            entry={entry}
            draggable
            onSelect={onSelect}
          />
        ))}
        {shown.length === 0 && (
          <p className="px-1 py-2 text-[11px] text-slate-500">No node type matches “{query}”.</p>
        )}
      </div>
    </div>
  );
}

export interface NodeTypePickerProps {
  /** Heading — states *why* this list is what it is. */
  title: string;
  entries: PaletteEntry[];
  onSelect: (type: NodeTypeInfo) => void;
  onDismiss: () => void;
  /** Absolute position within the canvas, for the connect-drop picker. */
  anchor?: { x: number; y: number } | null;
}

/**
 * Overlay list with type-ahead and arrow-key navigation. Used for Cmd+K
 * search (`anchor` unset → centred) and for the connect-drop picker
 * (`anchor` set → placed where the edge was dropped).
 */
export function NodeTypePicker({
  title,
  entries,
  onSelect,
  onDismiss,
  anchor,
}: NodeTypePickerProps) {
  const [query, setQuery] = useState('');
  const [cursor, setCursor] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);

  const shown = useMemo(() => entries.filter((e) => matches(e.type, query)), [entries, query]);

  useEffect(() => {
    inputRef.current?.focus();
  }, []);

  // Keep the cursor inside the filtered list as the query narrows it.
  useEffect(() => {
    setCursor((c) => (c >= shown.length ? 0 : c));
  }, [shown.length]);

  const commit = (index: number) => {
    const entry = shown[index];
    if (entry && !entry.disabledReason) onSelect(entry.type);
  };

  return (
    <div
      className="absolute inset-0 z-20"
      // Clicking the backdrop dismisses; the panel itself stops propagation.
      onClick={onDismiss}
      data-testid="node-type-picker-backdrop"
    >
      <div
        className={
          anchor
            ? 'absolute w-64 rounded-xl border border-slate-700/60 bg-slate-900/95 p-2.5 shadow-2xl shadow-black/50 backdrop-blur-sm'
            : 'absolute left-1/2 top-24 w-80 -translate-x-1/2 rounded-xl border border-slate-700/60 bg-slate-900/95 p-2.5 shadow-2xl shadow-black/50 backdrop-blur-sm'
        }
        style={anchor ? { left: anchor.x, top: anchor.y } : undefined}
        onClick={(evt) => evt.stopPropagation()}
        role="dialog"
        aria-label={title}
        data-testid="node-type-picker"
      >
        <p className="px-1 pb-2 text-[11px] font-medium uppercase tracking-wide text-slate-400">
          {title}
        </p>
        <input
          ref={inputRef}
          type="text"
          value={query}
          onChange={(evt) => setQuery(evt.target.value)}
          onKeyDown={(evt) => {
            if (evt.key === 'Escape') {
              evt.preventDefault();
              onDismiss();
            } else if (evt.key === 'ArrowDown') {
              evt.preventDefault();
              setCursor((c) => Math.min(c + 1, shown.length - 1));
            } else if (evt.key === 'ArrowUp') {
              evt.preventDefault();
              setCursor((c) => Math.max(c - 1, 0));
            } else if (evt.key === 'Enter') {
              evt.preventDefault();
              commit(cursor);
            }
          }}
          placeholder="Search node types"
          aria-label="Search node types"
          className="mb-2 w-full rounded-lg border border-slate-700/60 bg-slate-950/60 px-2 py-1.5 text-xs text-slate-200 placeholder:text-slate-500 focus:border-cyan-500/50 focus:outline-none"
        />
        <div role="listbox" aria-label={title} className="flex max-h-72 flex-col gap-1.5 overflow-y-auto">
          {shown.map((entry, i) => (
            <NodeTypeRow
              key={entry.type.kind}
              entry={entry}
              draggable={false}
              active={i === cursor}
              onSelect={onSelect}
            />
          ))}
          {shown.length === 0 && (
            <p className="px-1 py-2 text-[11px] text-slate-500">
              Nothing here can accept that connection.
            </p>
          )}
        </div>
      </div>
    </div>
  );
}
