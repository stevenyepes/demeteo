import { useEffect, useMemo, useRef, useState } from 'react';

import { readableOutput } from '../../../lib/rawOutput';

/**
 * What the failing command said, in a box the size of a paragraph.
 *
 * A blocked-at-`verify` sync stores the project's entire check run — 2552 lines
 * in the incident this was built for — and the old fixed `<pre>` showed the
 * first fifteen of them, top-anchored, with no sign there were more. Those
 * fifteen were biome warnings; the four compile errors that actually withheld
 * the push were 2470 lines below. The reading a user took from that pane was
 * that the checks had never run.
 *
 * So: head and tail, the middle behind a press, and the scroll parked at the
 * bottom, because the bottom is where a command puts its verdict.
 * `lib/rawOutput.ts` owns which lines those are and why.
 */
export function SyncRawOutput({ text }: { text: string }) {
  const [expanded, setExpanded] = useState(false);
  const boxRef = useRef<HTMLDivElement | null>(null);
  // Memoised against the size this exists for: `raw_error` reaches 188 KB, and
  // the pane re-renders on every parent state change.
  const output = useMemo(() => readableOutput(text), [text]);
  const shown =
    output.kind === 'whole' ? null : expanded ? output.full : `${output.head}\n${output.tail}`;

  useEffect(() => {
    const box = boxRef.current;
    if (shown === null || box === null) return;
    box.scrollTop = box.scrollHeight;
  }, [shown]);

  if (output.kind === 'whole') {
    return (
      <pre
        data-testid="sync-raw-error"
        className="max-h-40 overflow-y-auto whitespace-pre-wrap break-words rounded-xl border border-white/5 bg-black/30 p-3 font-mono text-[11px] leading-relaxed text-slate-300"
      >
        {output.text}
      </pre>
    );
  }

  return (
    <div>
      <div
        ref={boxRef}
        data-testid="sync-raw-error"
        data-elided={expanded ? 'false' : 'true'}
        className="max-h-64 overflow-y-auto rounded-xl border border-white/5 bg-black/30 p-3 font-mono text-[11px] leading-relaxed text-slate-300"
      >
        {expanded ? (
          <pre className="whitespace-pre-wrap break-words">{output.full}</pre>
        ) : (
          <>
            <pre className="whitespace-pre-wrap break-words">{output.head}</pre>
            <p
              data-testid="sync-raw-error-elision"
              className="my-2 select-none text-center font-sans text-[10px] font-bold uppercase tracking-widest text-slate-500"
            >
              ··· {output.hiddenLines} lines hidden ···
            </p>
            <pre className="whitespace-pre-wrap break-words">{output.tail}</pre>
          </>
        )}
      </div>
      <button
        type="button"
        data-testid="sync-raw-error-toggle"
        aria-expanded={expanded}
        onClick={() => setExpanded((was) => !was)}
        className="mt-2 rounded-lg px-2 py-1 text-[10px] font-bold uppercase tracking-widest text-slate-400 transition hover:bg-white/5 hover:text-slate-200"
      >
        {expanded ? 'Show the ends only' : `Show all ${output.totalLines} lines`}
      </button>
    </div>
  );
}

export default SyncRawOutput;
