/**
 * One row of the project view's feature-pipeline list.
 *
 * Memoized because `ProjectHome` holds the composer's text, the staged
 * attachments and the feature rows in one component: without this every
 * keystroke re-rendered every card. The memo only holds while every prop is
 * stable across an unrelated parent render, which is why the click handler
 * takes the feature's id and title as arguments instead of the parent closing
 * over the row — see `PipelineCard.test.tsx`, which counts renders rather than
 * asserting the memo exists.
 *
 * Everything the card shows is derived by `pipelineCardMeta`; nothing is
 * decided here. The `data-tier` attributes are what keeps that grouping
 * enforceable: the row rendered all three tiers at one weight for as long as
 * the grouping existed only in the type, and a test asserting a field is
 * *present* passes on exactly that layout. Asserting which tier it landed in
 * does not.
 */

import { Clock, Cpu, Zap, ChevronRight } from 'lucide-react';
import { memo, useCallback, useMemo } from 'react';

import { Chip } from './ui/Chip';
import { DEFAULT_DENSITY, pipelineDensityClasses, type PipelineDensityClasses } from '../lib/density';
import { pipelineCardMeta } from '../lib/pipelineCard';
import { type RunStatusTone } from '../lib/runStatus';
import type { WorkflowMeta } from '../lib/workflowBadge';
import type { Feature } from '../types';

/**
 * Left accent bar per tone. Local to this component (the way StatusBadge
 * keeps its own TONE_DOT) because the glow is specific to these cards —
 * the shared registry only carries the flat `TONE_BORDER_L` border.
 */
const TONE_ACCENT: Record<RunStatusTone, string> = {
    emerald: 'bg-emerald-500 shadow-[0_0_10px_rgba(16,185,129,0.8)]',
    cyan:    'bg-cyan-500 shadow-[0_0_10px_rgba(6,182,212,0.8)]',
    violet:  'bg-violet-500 shadow-[0_0_10px_rgba(139,92,246,0.8)]',
    amber:   'bg-amber-500 shadow-[0_0_10px_rgba(245,158,11,0.8)]',
    ruby:    'bg-ruby-500 shadow-[0_0_10px_rgba(239,68,68,0.8)]',
    slate:   'bg-slate-600 shadow-[0_0_10px_rgba(100,116,139,0.6)]',
};

const COMFORTABLE = pipelineDensityClasses(DEFAULT_DENSITY);

export interface PipelineCardProps {
    feature: Feature;
    workflowById: ReadonlyMap<string, WorkflowMeta>;
    /** Run is owned by the runner, whatever the project's compute type says. */
    detached: boolean;
    computeType: string | undefined;
    remoteHost: string | null | undefined;
    /**
     * Resolved by `pipelineDensityClasses` — one stable object per density, or
     * the memo below sees a changed prop on every parent render. A caller that
     * offers no density control gets comfortable.
     */
    density?: PipelineDensityClasses;
    onOpen: (featureId: string, featureTitle: string) => void;
}

function PipelineCardInner({
    feature,
    workflowById,
    detached,
    computeType,
    remoteHost,
    density = COMFORTABLE,
    onOpen,
}: PipelineCardProps) {
    const { scan, context, detail } = useMemo(
        () => pipelineCardMeta({ feature, workflowById, detached, computeType, remoteHost }),
        [feature, workflowById, detached, computeType, remoteHost],
    );

    const handleOpen = useCallback(() => {
        onOpen(feature.id, feature.title);
    }, [onOpen, feature.id, feature.title]);

    return (
        <div
            onClick={handleOpen}
            className={`pipeline-card glass-panel glass-panel-hover rounded-xl cursor-pointer relative overflow-hidden group ${density.card} ${scan.needsYou ? 'ring-1 ring-amber-500/40' : ''}`}
        >
            <div className={`absolute left-0 top-0 bottom-0 w-1 ${TONE_ACCENT[scan.status.tone]}`}></div>

            <div data-tier="scan" className="flex items-start justify-between gap-4">
                <h3
                    className={`min-w-0 flex-1 font-heading text-white line-clamp-2 break-words ${density.title}`}
                    title={scan.title}
                >
                    {scan.title}
                </h3>
                <div className="flex shrink-0 items-center gap-3">
                    <Chip tone={scan.status.tone} pulse={scan.status.active} size="sm">
                        {scan.status.label}
                    </Chip>
                    <span
                        className={`flex items-center gap-1 font-mono font-medium text-white ${density.elapsed}`}
                        title="Elapsed"
                    >
                        <Clock className="w-3 h-3 text-slate-500" />
                        {scan.elapsed}
                    </span>
                    <ChevronRight className="w-4 h-4 text-slate-500 opacity-0 group-hover:opacity-100 transition-opacity" />
                </div>
            </div>

            <div
                data-tier="context"
                className={`mt-2 flex flex-wrap items-center gap-x-3 gap-y-2 font-mono text-slate-400 ${density.meta}`}
            >
                {context.workflow.variant === 'fallback' ? (
                    <Chip tone="slate" size="sm" title="Workflow reference missing">
                        Workflow: unknown
                    </Chip>
                ) : (
                    <Chip
                        tone="violet"
                        size="sm"
                        maxWidth="220px"
                        title={`Workflow: ${context.workflow.name} (${context.workflow.is_starter ? 'starter' : 'custom'})`}
                    >
                        {context.workflow.name}
                    </Chip>
                )}
                <Chip
                    tone={context.transport.tone}
                    size="sm"
                    icon={<Cpu className="w-3 h-3" />}
                    title={context.transport.title}
                >
                    {context.transport.label}
                </Chip>
                <span className="text-slate-300" title="Cost so far">{context.cost}</span>
                <span className="flex items-center gap-1" title="Tokens">
                    <Zap className="w-3 h-3 text-cyan-400" />
                    {context.tokens}
                </span>
            </div>

            <div
                data-tier="detail"
                className={`mt-2 flex items-center gap-2 text-slate-500 ${density.meta}`}
            >
                <span className="font-mono shrink-0">{detail.featureId}</span>
                {detail.description && (
                    <p className="min-w-0 truncate" title={detail.description}>
                        {detail.description}
                    </p>
                )}
            </div>
        </div>
    );
}

export const PipelineCard = memo(PipelineCardInner);
