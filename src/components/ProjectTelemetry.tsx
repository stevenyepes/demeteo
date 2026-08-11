/**
 * The project view's telemetry cluster, as one `MetricStrip`
 * (docs/UI_REDESIGN_PLAN.md §6, Phase 4). It replaces two hand-built stat
 * columns whose numbers were reduced inline in `ProjectHome`'s render — the
 * same component that holds the composer's text, so every keystroke re-summed
 * every feature in the project.
 *
 * That is why the prop is the feature list and not the three totals: totals
 * would push the reduce back into the caller's render body, and the fresh
 * object it returns would defeat the memo below in the same breath. With the
 * list as the only input, the sums run when a fetch or a status event lands
 * and at no other time, and `summarizeProjectTelemetry` stays a pure function
 * a test can call without mounting anything.
 *
 * Cost is here because it is the number users actually watch (ux-audit
 * *Opportunity 5*) and the cluster this replaces omitted it.
 */

import { memo, useMemo, type ReactElement } from 'react';

import { featureRunStatus, runStatusMeta } from '../lib/runStatus';
import { formatCost, formatTokens } from '../lib/utils';
import type { Feature } from '../types';
import { Metric, MetricStrip } from './ui/MetricStrip';

export interface ProjectTelemetrySummary {
  /** Runs still changing on their own, per `runStatusMeta().active`. */
  active: number;
  total: number;
  tokens: number;
  costUsd: number;
}

/**
 * "Still moving" is `runStatusMeta().active`, not `status === 'running'`.
 * A queued or bootstrapping run is fleet the user is waiting on just as much
 * as a running one, and a second spelling of which statuses count is how the
 * F27 drift starts over in a numeric vocabulary.
 */
export function summarizeProjectTelemetry(
  features: readonly Feature[],
): ProjectTelemetrySummary {
  return features.reduce<ProjectTelemetrySummary>(
    (acc, feature) => ({
      active: acc.active + (runStatusMeta(featureRunStatus(feature)).active ? 1 : 0),
      total: acc.total + 1,
      tokens: acc.tokens + (feature.tokens ?? 0),
      costUsd: acc.costUsd + (feature.total_cost || 0),
    }),
    { active: 0, total: 0, tokens: 0, costUsd: 0 },
  );
}

export interface ProjectTelemetryProps {
  features: readonly Feature[];
  className?: string;
}

function ProjectTelemetryInner({
  features,
  className = '',
}: ProjectTelemetryProps): ReactElement {
  const { active, total, tokens, costUsd } = useMemo(
    () => summarizeProjectTelemetry(features),
    [features],
  );

  return (
    <MetricStrip className={className}>
      <Metric
        label="Fleet Active"
        value={String(active)}
        tone={active > 0 ? 'emerald' : 'slate'}
        tooltip={`${active} of ${total} pipelines still moving`}
      />
      <Metric
        label="Cost"
        value={formatCost(costUsd)}
        tone="emerald"
        tooltip={`${costUsd.toFixed(4)} USD across ${total} pipelines`}
      />
      <Metric label="Tokens" value={formatTokens(tokens)} tone="cyan" />
    </MetricStrip>
  );
}

export const ProjectTelemetry = memo(ProjectTelemetryInner);
