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
 * decided here.
 */

import { Clock, Cpu, Zap, ChevronRight } from 'lucide-react';
import { memo, useCallback, useMemo } from 'react';

import { pipelineCardMeta } from '../lib/pipelineCard';
import { TONE_CHIP, type RunStatusTone } from '../lib/runStatus';
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

/**
 * Chip classes for the transport badge, which is not a run status.
 *
 * `slate` here is deliberately weaker than `TONE_CHIP.slate`: a local run is
 * the unremarkable case and must not compete with the status chip beside it.
 * Only the two tones `pipelineCardMeta` can produce for a transport are listed.
 */
const TRANSPORT_CHIP: Record<'cyan' | 'slate', string> = {
    cyan:  'bg-cyan-500/10 text-cyan-400 border-cyan-500/20',
    slate: 'bg-white/5 text-slate-500 border-white/10',
};

function transportChipClass(tone: RunStatusTone): string {
    return tone === 'cyan' ? TRANSPORT_CHIP.cyan : TRANSPORT_CHIP.slate;
}

export interface PipelineCardProps {
    feature: Feature;
    workflowById: ReadonlyMap<string, WorkflowMeta>;
    /** Run is owned by the runner, whatever the project's compute type says. */
    detached: boolean;
    computeType: string | undefined;
    remoteHost: string | null | undefined;
    onOpen: (featureId: string, featureTitle: string) => void;
}

function PipelineCardInner({
    feature,
    workflowById,
    detached,
    computeType,
    remoteHost,
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
            className="glass-panel glass-panel-hover rounded-xl p-5 cursor-pointer relative overflow-hidden group"
        >
            <div className={`absolute left-0 top-0 bottom-0 w-1 ${TONE_ACCENT[scan.status.tone]}`}></div>

            <div className="flex justify-between items-start gap-4">
                <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-3 mb-1 flex-wrap">
                        <span className={`px-2 py-0.5 rounded text-[10px] font-mono border uppercase flex items-center gap-1 ${TONE_CHIP[scan.status.tone]}`}>
                            {scan.status.active && (
                                <span className="w-1.5 h-1.5 rounded-full bg-current animate-pulse"></span>
                            )}
                            {scan.status.label}
                        </span>
                        {context.workflow.variant === 'fallback' ? (
                            <span
                                className="px-2 py-0.5 rounded text-[10px] font-mono bg-white/5 border border-white/10 text-slate-500 uppercase"
                                title="Workflow reference missing"
                            >
                                Workflow: unknown
                            </span>
                        ) : (
                            <span
                                className="px-2 py-0.5 rounded text-[10px] font-mono bg-violet-500/10 border border-violet-500/30 text-violet-300 font-outfit truncate max-w-[220px] inline-flex items-center gap-1"
                                title={`Workflow: ${context.workflow.name}`}
                            >
                                <span className="text-violet-400/80">Workflow:</span>
                                <span className="truncate">{context.workflow.name}</span>
                                <span className="text-[9px] px-1 rounded bg-violet-500/20 text-violet-300 font-medium font-mono uppercase">
                                    {context.workflow.is_starter ? 'Starter' : 'Custom'}
                                </span>
                            </span>
                        )}
                        <span
                            className={`px-2 py-0.5 rounded text-[10px] font-mono uppercase border inline-flex items-center gap-1 ${transportChipClass(context.transport.tone)}`}
                            title={context.transport.title}
                        >
                            <Cpu className="w-3 h-3" /> {context.transport.label}
                        </span>
                        <span className="text-xs text-slate-500 font-mono truncate">{detail.featureId}</span>
                    </div>
                    <h3 className="text-lg font-outfit text-white line-clamp-2 break-words" title={scan.title}>{scan.title}</h3>
                    {detail.description && (
                        <p
                            className="mt-1 text-xs text-slate-400 leading-relaxed line-clamp-2 break-words"
                            title={detail.description}
                        >
                            {detail.description}
                        </p>
                    )}
                </div>

                <div className="flex gap-6 text-right shrink-0 pt-1">
                    <div>
                        <div className="text-xs text-slate-500 font-mono flex items-center gap-1 justify-end"><Clock className="w-3 h-3" /> Duration</div>
                        <div className="text-sm font-medium text-white">{scan.elapsed}</div>
                    </div>
                    <div>
                        <div className="text-xs text-slate-500 font-mono flex items-center gap-1 justify-end"><Zap className="w-3 h-3 text-cyan-400 animate-pulse" /> Tokens</div>
                        <div className="text-sm font-medium text-white">{context.tokens}</div>
                    </div>
                    <ChevronRight className="w-5 h-5 text-slate-500 mt-2 opacity-0 group-hover:opacity-100 transition-opacity" />
                </div>
            </div>
        </div>
    );
}

export const PipelineCard = memo(PipelineCardInner);
