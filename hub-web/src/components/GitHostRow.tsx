import { useState } from 'react';

import { Button, buttonClasses } from '../../../src/components/ui/Button';
import type { GitHost } from '../lib/gitHosts';

interface GitHostRowProps {
  host: GitHost;
  disabled: boolean;
  onRemove: (id: string) => void;
}

/**
 * Removing asks twice: the App stays on GitHub, but connecting again creates
 * a new App rather than bringing this one back, so a stray click costs a
 * reinstall on every repository.
 */
export function GitHostRow({ host, disabled, onRemove }: GitHostRowProps) {
  const [confirming, setConfirming] = useState(false);
  return (
    <li className="flex flex-wrap items-center gap-x-4 gap-y-2 px-4 py-3">
      <div className="min-w-0 flex-1">
        <p className="font-mono text-sm text-slate-100 break-all">{host.appSlug}</p>
        <p className="text-xs text-slate-400">
          {host.account === null ? host.host : `${host.account} on ${host.host}`} · App {host.appId}
        </p>
      </div>
      <a href={host.installUrl} target="_blank" rel="noreferrer" className={buttonClasses('secondary', 'sm')}>
        Choose repositories
      </a>
      {confirming ? (
        <>
          <Button variant="danger" size="sm" disabled={disabled} onClick={() => onRemove(host.id)}>
            Remove from this Hub
          </Button>
          <Button variant="ghost" size="sm" disabled={disabled} onClick={() => setConfirming(false)}>
            Keep
          </Button>
        </>
      ) : (
        <Button variant="danger-subtle" size="sm" disabled={disabled} onClick={() => setConfirming(true)}>
          Remove
        </Button>
      )}
    </li>
  );
}
