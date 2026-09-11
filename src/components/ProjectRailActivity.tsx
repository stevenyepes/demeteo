import RailBadge from './ui/RailBadge';
import type { ProjectActivity } from '../lib/projectActivity';

export interface ProjectRailActivityProps {
  activity: ProjectActivity;
  collapsed?: boolean;
}

function attentionLabel(needsYou: number): string {
  return `${needsYou} needs your attention`;
}

function activeLabel(active: number): string {
  return `${active} active`;
}

export function ProjectRailActivity({
  activity,
  collapsed = false,
}: ProjectRailActivityProps): React.ReactElement | null {
  const { active, needsYou } = activity;
  if (active <= 0 && needsYou <= 0) return null;

  const attention = needsYou > 0 && (
    <RailBadge
      testId="rail-project-attention"
      tone="amber"
      count={needsYou}
      label={attentionLabel(needsYou)}
      corner={collapsed ? 'left' : undefined}
      pulse
    />
  );
  const running = active > 0 && (
    <RailBadge
      testId="rail-project-active"
      tone="cyan"
      count={active}
      label={activeLabel(active)}
      corner={collapsed ? 'right' : undefined}
    />
  );

  if (collapsed) {
    return (
      <>
        {attention}
        {running}
      </>
    );
  }

  return (
    <span className="flex items-center gap-1.5">
      {attention}
      {running}
    </span>
  );
}

export default ProjectRailActivity;
