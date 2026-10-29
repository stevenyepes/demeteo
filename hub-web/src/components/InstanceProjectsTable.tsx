import type { ProjectSummary, RunShapeDefaults, WorkflowChoice } from '../lib/fleet';
import { redactUserinfo } from '../lib/fleetSummary';

interface InstanceProjectsTableProps {
  projects: ProjectSummary[];
  workflows: WorkflowChoice[] | null;
  runShape: RunShapeDefaults[] | null;
}

export function InstanceProjectsTable({ projects, workflows, runShape }: InstanceProjectsTableProps) {
  return (
    <div className="overflow-x-auto">
      <table className="w-full text-left text-sm">
        <thead className="text-xs text-slate-400">
          <tr className="border-b border-hairline">
            <th scope="col" className="py-2 pr-4 font-medium">Project</th>
            <th scope="col" className="py-2 pr-4 font-medium">Remote</th>
            <th scope="col" className="py-2 font-medium">Default workflow</th>
          </tr>
        </thead>
        <tbody>
          {projects.map((project) => (
            <tr key={project.id} className="border-b border-hairline last:border-b-0">
              <td className="py-2.5 pr-4 font-medium text-slate-200 break-all">{project.name}</td>
              <td className="py-2.5 pr-4">
                {/* Text, never an `href`: the remote is whatever the machine's git config says. */}
                {project.remoteUrl === null ? (
                  <span className="text-xs text-slate-400">No remote</span>
                ) : (
                  <span className="font-mono text-xs text-slate-400 break-all">{redactUserinfo(project.remoteUrl)}</span>
                )}
              </td>
              <td className="py-2.5 font-mono text-xs text-slate-300">
                {defaultWorkflowText(project.id, workflows, runShape)}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

/** The newest version reported under the project's default id; the run itself pins its own. */
function defaultWorkflowText(
  projectId: string,
  workflows: WorkflowChoice[] | null,
  runShape: RunShapeDefaults[] | null,
): string {
  const workflowId = runShape?.find((shape) => shape.projectId === projectId)?.workflowId ?? null;
  if (workflowId === null) return 'None set';
  const versions = (workflows ?? []).filter((workflow) => workflow.workflowId === workflowId);
  if (versions.length === 0) return 'Not reported';
  const newest = versions.reduce((a, b) => (b.version > a.version ? b : a));
  return `${newest.name} v${newest.version}`;
}
