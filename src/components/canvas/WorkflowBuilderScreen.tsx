/**
 * The design-mode **route** (task P3.6): loads a workflow, hands its graph to
 * `WorkflowBuilder`, and persists what comes back. Replaces `WorkflowEditor`,
 * the stacked form the app shipped with.
 *
 * P3.1–P3.4 deliberately built the canvas, config panel, lint surface and
 * version drawer as IPC-free components, and P3.3 left `onSave` as a prop
 * because storage was still v1-only. This file is where that seam is finally
 * connected: `workflow_save` (V34) stores the schema-v2 document verbatim —
 * positions, joins, per-class retry and edge guards intact — alongside the v1
 * projection the runner still reads.
 *
 * Loading is two reads rather than a bespoke command: `workflow_get` for the
 * row (name, description, latest version) and `workflow_version_graph` for
 * that version's definition. The second is the same command the version drawer
 * already uses, so the graph an author opens is the graph the drawer diffs.
 *
 * A *new* workflow starts from a template (`TemplatePicker`) or a clone of an
 * existing one, and has no id until its first save — `workflow_save` creates
 * the row then, so an abandoned "new workflow" never leaves a husk behind.
 */
import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { RefreshCw } from 'lucide-react';

import { formatError } from '../../lib/errors';
import { WorkflowBuilder, type WorkflowSaveRequest } from './WorkflowBuilder';
import { TemplatePicker, type TemplateChoice } from './TemplatePicker';
import type { WorkflowDefinitionV2 } from './types';

/** Serde shape of the Rust `WorkflowWithSteps`. */
interface WorkflowRow {
  id: string;
  name: string;
  description: string;
  is_starter: boolean;
  version: number;
  version_id: string;
}

export interface WorkflowBuilderScreenProps {
  /** `null` opens the template picker for a brand-new workflow. */
  workflowId: string | null;
  onBack: () => void;
}

interface Loaded {
  workflowId: string | null;
  definition: WorkflowDefinitionV2;
  name: string;
  description: string;
  version: number;
  isStarter: boolean;
}

export function WorkflowBuilderScreen({ workflowId, onBack }: WorkflowBuilderScreenProps) {
  const [loaded, setLoaded] = useState<Loaded | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(workflowId != null);

  const loadWorkflow = useCallback(async (id: string): Promise<Loaded> => {
    const row = await invoke<WorkflowRow>('workflow_get', { workflowId: id });
    const definition = await invoke<WorkflowDefinitionV2>('workflow_version_graph', {
      workflowId: id,
      versionId: row.version_id,
    });
    return {
      workflowId: id,
      definition,
      name: row.name,
      description: row.description,
      version: row.version,
      isStarter: row.is_starter,
    };
  }, []);

  useEffect(() => {
    if (workflowId == null) {
      setLoaded(null);
      setLoading(false);
      return;
    }
    let live = true;
    setLoading(true);
    loadWorkflow(workflowId)
      .then((next) => {
        if (live) {
          setLoaded(next);
          setError(null);
        }
      })
      .catch((err) => live && setError(formatError(err)))
      .finally(() => live && setLoading(false));
    return () => {
      live = false;
    };
  }, [workflowId, loadWorkflow]);

  /** A template pick, or a clone of an existing workflow's graph. */
  const startFrom = useCallback(
    async (choice: TemplateChoice) => {
      if (choice.kind === 'template') {
        setLoaded({
          workflowId: null,
          definition: choice.template.build(),
          name: '',
          description: '',
          version: 0,
          isStarter: false,
        });
        return;
      }
      setLoading(true);
      try {
        const source = await loadWorkflow(choice.workflowId);
        // A clone is a *new* workflow: drop the id and the version history, and
        // say so in the name so two entries in the library are tellable apart.
        setLoaded({
          ...source,
          workflowId: null,
          version: 0,
          isStarter: false,
          name: `${source.name} (copy)`,
        });
        setError(null);
      } catch (err) {
        setError(formatError(err));
      } finally {
        setLoading(false);
      }
    },
    [loadWorkflow],
  );

  const save = useCallback(
    async ({ definition, name, description }: WorkflowSaveRequest) => {
      const row = await invoke<WorkflowRow>('workflow_save', {
        workflowId: loaded?.workflowId ?? null,
        name,
        description,
        definition,
        note: null,
      });
      // A first save mints the workflow row, so adopt its id — the version
      // drawer, the draft slot, and the next save all key off it.
      setLoaded((prev) =>
        prev
          ? { ...prev, workflowId: row.id, version: row.version, name, description }
          : prev,
      );
    },
    [loaded?.workflowId],
  );

  if (loading) {
    return (
      <div className="flex h-full items-center justify-center bg-[#0b0d12]">
        <RefreshCw className="h-8 w-8 animate-spin text-violet-500" />
      </div>
    );
  }

  if (error) {
    return (
      <div className="flex h-full flex-col items-center justify-center gap-3 bg-[#0b0d12] text-sm text-rose-300">
        <span>{error}</span>
        <button
          type="button"
          onClick={onBack}
          className="rounded-lg border border-slate-700/60 px-3 py-1.5 text-xs text-slate-300 hover:border-slate-600 hover:text-white"
        >
          Back to workflows
        </button>
      </div>
    );
  }

  if (!loaded) {
    return <TemplatePicker onPick={startFrom} onCancel={onBack} />;
  }

  return (
    <WorkflowBuilder
      // Remounting on identity change resets history and the draft slot, which
      // is what "opened a different workflow" should mean.
      key={loaded.workflowId ?? 'new'}
      workflowId={loaded.workflowId}
      definition={loaded.definition}
      name={loaded.name}
      description={loaded.description}
      version={loaded.version}
      isStarter={loaded.isStarter}
      onSave={save}
      onWorkflowReplaced={({ version, name, description }) =>
        setLoaded((prev) => (prev ? { ...prev, version, name, description } : prev))
      }
      onClose={onBack}
    />
  );
}
