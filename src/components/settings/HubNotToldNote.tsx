import { Info } from 'lucide-react';
import type { HubUnpairNotice } from '../../lib/hub';
import { unpairNoticeCopy } from '../../lib/hubPresentation';

interface HubNotToldNoteProps {
  notice: HubUnpairNotice | null | undefined;
}

/** Shown after a disconnect the Hub did not hear about. Neutral, not amber:
 *  the pairing here is already gone, and nothing on this machine waits on
 *  the user. */
export function HubNotToldNote({ notice }: HubNotToldNoteProps) {
  const copy = unpairNoticeCopy(notice);
  if (!copy) return null;
  return (
    <div role="status" aria-label="Hub not told" className="panel p-4 flex items-start gap-2">
      <Info className="w-4 h-4 text-slate-400 shrink-0 mt-0.5" aria-hidden="true" />
      <div className="space-y-1">
        <p className="text-xs text-slate-300 leading-relaxed">{copy.message}</p>
        {copy.reason && <p className="text-xs text-slate-400 break-words">{copy.reason}</p>}
      </div>
    </div>
  );
}

export default HubNotToldNote;
