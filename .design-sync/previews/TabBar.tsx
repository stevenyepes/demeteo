import { TabBar } from 'demeteo';
import { Activity, GitBranch, Settings, Terminal, FileText } from 'lucide-react';

/** The canonical use — icons plus labels, cyan marking the active tab. */
export const Default = () => (
  <div className="w-full max-w-xl">
    <TabBar
      activeTab="pipeline"
      onChange={() => {}}
      tabs={[
        { key: 'pipeline', label: 'Pipeline', icon: <Activity className="w-4 h-4" /> },
        { key: 'diff', label: 'Changes', icon: <GitBranch className="w-4 h-4" /> },
        { key: 'logs', label: 'Logs', icon: <Terminal className="w-4 h-4" /> },
        { key: 'settings', label: 'Settings', icon: <Settings className="w-4 h-4" /> },
      ]}
    />
  </div>
);

/** Selection moves the cyan underline — the last tab active. */
export const LastTabActive = () => (
  <div className="w-full max-w-xl">
    <TabBar
      activeTab="settings"
      onChange={() => {}}
      tabs={[
        { key: 'pipeline', label: 'Pipeline', icon: <Activity className="w-4 h-4" /> },
        { key: 'diff', label: 'Changes', icon: <GitBranch className="w-4 h-4" /> },
        { key: 'logs', label: 'Logs', icon: <Terminal className="w-4 h-4" /> },
        { key: 'settings', label: 'Settings', icon: <Settings className="w-4 h-4" /> },
      ]}
    />
  </div>
);

/** Labels only — the icon is optional. */
export const WithoutIcons = () => (
  <div className="w-full max-w-md">
    <TabBar
      activeTab="workflows"
      onChange={() => {}}
      tabs={[
        { key: 'features', label: 'Features' },
        { key: 'workflows', label: 'Workflows' },
        { key: 'machines', label: 'Machines' },
      ]}
    />
  </div>
);

/** Over a panel, which is where it sits in the product. */
export const OverAPanel = () => (
  <div className="w-full max-w-xl glass-panel p-5">
    <TabBar
      activeTab="artifacts"
      onChange={() => {}}
      tabs={[
        { key: 'artifacts', label: 'Artifacts', icon: <FileText className="w-4 h-4" /> },
        { key: 'logs', label: 'Logs', icon: <Terminal className="w-4 h-4" /> },
      ]}
    />
    <p className="mt-4 text-sm text-slate-400">
      Artifacts produced by this Step, newest first.
    </p>
  </div>
);
