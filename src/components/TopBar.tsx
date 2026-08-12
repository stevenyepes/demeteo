import { useEffect, useState } from 'react';
import { Search, Sliders, Globe, Inbox, TerminalSquare } from 'lucide-react';
import { NotificationBell } from './NotificationBell';
import { AccountMenu } from './AccountMenu';
import { HeaderNavItem } from './ui/HeaderNavItem';
import { bucketFor } from '../lib/remoteRunBuckets';
import { useHeaderDensity } from '../hooks/useHeaderDensity';
import { useNavigation, useUIState, useTerminalPanel } from '../context';
import { listMirroredRuns } from '../lib/remoteRuns';
import type { Provider } from '../types';

interface TopBarProps {
  connectedProvider: Provider | null;
}

function TopBar({ connectedProvider }: TopBarProps) {
  const { navigate, view } = useNavigation();
  const { uiDispatch } = useUIState();
  const { state: terminalState } = useTerminalPanel();
  const { setHeaderEl, density } = useHeaderDensity();
  const terminalsActive = view.kind === 'terminals';

  // Ambient badge for the Remote Runs entry point — without this, a
  // parked/failed/needs-credentials run gives zero passive signal that
  // it needs attention unless the user thinks to open the inbox
  // (docs/REMOTE_EXECUTION.md M6.2/§8). Polls independently of
  // whether the inbox itself is mounted.
  const [actionableCount, setActionableCount] = useState(0);
  const [runningCount, setRunningCount] = useState(0);
  useEffect(() => {
    let cancelled = false;
    const poll = async () => {
      try {
        const runs = await listMirroredRuns();
        if (cancelled) return;
        const buckets = runs.map((r) => bucketFor(r.status));
        setActionableCount(buckets.filter((b) => b === 'parked' || b === 'needs_credentials' || b === 'failed').length);
        setRunningCount(buckets.filter((b) => b === 'running').length);
      } catch {
        // best-effort ambient badge — a transient failure just leaves
        // the last known count in place
      }
    };
    poll();
    const interval = setInterval(poll, 10000);
    return () => { cancelled = true; clearInterval(interval); };
  }, []);

  const liveTerminals = terminalState.tabs.length;

  // The base header titled the cyan running dot itself; `HeaderNavItem` renders
  // that dot as 8px of decoration with no title of its own, so the in-progress
  // count rides the button's title — the same recovery the Terminals entry below
  // makes for its session count.
  const runsTitle =
    actionableCount > 0
      ? `Runs — ${actionableCount} run${actionableCount === 1 ? '' : 's'} need attention`
      : runningCount > 0
        ? `Runs — ${runningCount} remote run${runningCount === 1 ? '' : 's'} in progress`
        : 'Runs — every run launched on a remote machine';

  return (
    <header
      ref={setHeaderEl}
      className="h-14 border-b border-white/5 bg-[#0d0f14]/80 backdrop-blur-md grid grid-cols-[minmax(0,1fr)_auto_minmax(0,1fr)] items-center gap-4 px-6 z-20 relative shrink-0"
    >
      <div data-testid="topbar-brand" className="flex items-center gap-4 min-w-0">
        <img src="/icon.png" alt="Demeteo" className="w-8 h-8 rounded-lg shrink-0" />
        <h1 className="font-heading font-bold tracking-wide text-lg text-white truncate">demeteo</h1>
      </div>

      {/* The centre track's width is the nav labels' whole budget — it comes
          out of both side tracks, and `24vw` is what leaves the labelled
          cluster room at the 1440 default window. Re-measure before widening
          it (src/lib/headerLayout.ts). */}
      <div data-testid="topbar-center" className="flex items-center justify-center">
        {/* A real `<button>`, not `role="button"` + `tabIndex` + `onKeyDown`:
            Enter/Space activation, tab order and the focus ring come from the
            platform. It takes no `aria-label` — its contents already name it,
            and a label would override them (WCAG 2.5.3). */}
        <button
          type="button"
          data-testid="topbar-search"
          className="flex items-center px-3 py-1.5 glass-panel rounded-md text-sm text-slate-400 w-[clamp(13rem,24vw,28rem)] hover:border-white/20 transition-colors cursor-pointer"
          onClick={() => uiDispatch({ type: 'SET_COMMAND_PALETTE', open: true })}
        >
          <Search className="w-4 h-4 mr-2 opacity-50" />
          <span className="truncate">Search workspace...</span>
          <span className="ml-auto pl-2 text-[10px] font-mono border border-white/10 px-1.5 py-0.5 rounded opacity-50">⌘K</span>
        </button>
      </div>

      <div data-testid="topbar-nav" className="flex items-center gap-2 min-w-0 justify-self-end">
        <HeaderNavItem
          icon={Sliders}
          label="Workflows"
          density={density}
          accent="violet"
          active={view.kind === 'workflows'}
          title="Templates Hub"
          onClick={() => navigate({ kind: 'workflows' })}
        />
        <HeaderNavItem
          icon={Globe}
          label="Providers"
          density={density}
          accent="cyan"
          active={view.kind === 'providers'}
          title="Source Providers"
          onClick={() => navigate({ kind: 'providers' })}
        />
        <HeaderNavItem
          icon={Inbox}
          label="Runs"
          density={density}
          accent="amber"
          active={view.kind === 'remote-inbox'}
          count={actionableCount}
          activity={runningCount > 0}
          title={runsTitle}
          testId="topbar-runs"
          pulseTestId="topbar-runs-pulse"
          onClick={() => navigate({ kind: 'remote-inbox' })}
        />
        {/* Terminal panel toggle (spec §3 (f)). The pulse indicator is
            visible only while the panel is collapsed but at least one
            session is alive — gives the user a passive signal that
            hiding the panel did not kill anything (spec §1 AC #6). */}
        <HeaderNavItem
          icon={TerminalSquare}
          label="Terminals"
          density={density}
          accent="cyan"
          active={terminalsActive}
          pulse={liveTerminals > 0 && !terminalsActive}
          title={liveTerminals > 0 ? `Open the Terminals view — ${liveTerminals} live session${liveTerminals === 1 ? '' : 's'}` : 'Open the Terminals view — sessions stay alive as you navigate'}
          ariaLabel="Open terminals view"
          testId="topbar-terminal-toggle"
          pulseTestId="topbar-terminal-pulse"
          onClick={() => navigate({ kind: 'terminals' })}
        />
        <NotificationBell />
        <AccountMenu
          connectedProvider={connectedProvider}
          onNavigateSettings={() => navigate({ kind: 'settings' })}
        />
      </div>
    </header>
  );
};

export default TopBar;
