/**
 * "New workflow" starting point (task P3.6, PRD §6.3): one of three shapes, or
 * a clone of a workflow that already exists.
 *
 * Clone-a-starter leads, deliberately. The bundled starters are the only
 * definitions in the app that have been tuned against real runs — prompts,
 * capabilities, verifier wiring, retry budgets — so "start from one and change
 * the part you care about" is a better first move than assembling a pipeline
 * from an empty canvas, and it is the path the J3 script takes.
 */
import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Copy, LayoutTemplate, RefreshCw, X } from 'lucide-react';

import { formatError } from '../../lib/errors';
import { WORKFLOW_TEMPLATES, type WorkflowTemplate } from './templates';

export type TemplateChoice =
  | { kind: 'template'; template: WorkflowTemplate }
  | { kind: 'clone'; workflowId: string; name: string };

interface WorkflowRow {
  id: string;
  name: string;
  description: string;
  is_starter: boolean;
}

export interface TemplatePickerProps {
  onPick: (choice: TemplateChoice) => void;
  onCancel: () => void;
}

export function TemplatePicker({ onPick, onCancel }: TemplatePickerProps) {
  const [rows, setRows] = useState<WorkflowRow[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    let live = true;
    invoke<WorkflowRow[]>('workflow_list')
      .then((list) => live && setRows(list))
      .catch((err) => live && setError(formatError(err)))
      .finally(() => live && setLoading(false));
    return () => {
      live = false;
    };
  }, []);

  return (
    <div
      className="flex h-full flex-col overflow-y-auto bg-[#0b0d12] px-8 py-10"
      data-testid="template-picker"
    >
      <div className="mx-auto w-full max-w-3xl space-y-8">
        <header className="flex items-start justify-between">
          <div>
            <h1 className="font-display text-2xl font-bold text-white">New workflow</h1>
            <p className="mt-1 text-sm text-slate-400">
              Start from a pipeline that already works, or from a shape.
            </p>
          </div>
          <button
            type="button"
            onClick={onCancel}
            aria-label="Cancel"
            className="rounded-lg border border-slate-700/60 p-1.5 text-slate-300 transition-colors hover:border-slate-600 hover:text-white"
          >
            <X className="h-4 w-4" />
          </button>
        </header>

        <section className="space-y-3">
          <h2 className="text-xs font-bold uppercase tracking-wider text-slate-500">
            Clone an existing pipeline
          </h2>
          {loading ? (
            <div className="flex items-center gap-2 text-sm text-slate-400">
              <RefreshCw className="h-4 w-4 animate-spin" /> Loading…
            </div>
          ) : error ? (
            <p className="text-sm text-rose-300">{error}</p>
          ) : rows.length === 0 ? (
            <p className="text-sm text-slate-500">No workflows to clone yet.</p>
          ) : (
            <div className="grid gap-2 sm:grid-cols-2">
              {rows.map((row) => (
                <button
                  key={row.id}
                  type="button"
                  onClick={() => onPick({ kind: 'clone', workflowId: row.id, name: row.name })}
                  className="flex items-start gap-3 rounded-lg border border-white/5 bg-white/[0.02] p-4 text-left transition-colors hover:border-violet-500/40 hover:bg-violet-500/5"
                >
                  <Copy className="mt-0.5 h-4 w-4 shrink-0 text-violet-400" />
                  <span className="min-w-0">
                    <span className="flex items-center gap-2">
                      <span className="truncate font-medium text-white">{row.name}</span>
                      {row.is_starter && (
                        <span className="shrink-0 rounded-full border border-emerald-500/20 bg-emerald-500/10 px-1.5 text-[9px] font-bold uppercase tracking-wider text-emerald-400">
                          Starter
                        </span>
                      )}
                    </span>
                    <span className="mt-1 line-clamp-2 block text-xs text-slate-400">
                      {row.description}
                    </span>
                  </span>
                </button>
              ))}
            </div>
          )}
        </section>

        <section className="space-y-3">
          <h2 className="text-xs font-bold uppercase tracking-wider text-slate-500">
            Start from a shape
          </h2>
          <div className="grid gap-2">
            {WORKFLOW_TEMPLATES.map((template) => (
              <button
                key={template.id}
                type="button"
                onClick={() => onPick({ kind: 'template', template })}
                className="flex items-start gap-3 rounded-lg border border-white/5 bg-white/[0.02] p-4 text-left transition-colors hover:border-cyan-500/40 hover:bg-cyan-500/5"
              >
                <LayoutTemplate className="mt-0.5 h-4 w-4 shrink-0 text-cyan-400" />
                <span>
                  <span className="block font-medium text-white">{template.label}</span>
                  <span className="mt-1 block text-xs text-slate-400">{template.summary}</span>
                </span>
              </button>
            ))}
          </div>
        </section>
      </div>
    </div>
  );
}
