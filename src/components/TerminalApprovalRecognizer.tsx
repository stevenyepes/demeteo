import { useEffect, useRef } from 'react';
import { Channel } from '@tauri-apps/api/core';
import { Terminal } from '@xterm/headless';

import {
  attachTerminalSession,
  detachTerminalSession,
  reportScreenActivity,
} from '../lib/terminal';
import { getLastTerminalSize } from '../lib/terminalViewport';
import { DEFAULT_COMPILED_PACKS } from '../lib/terminalActivity/rulePacks';
import { readBottomRows, recognizerTick } from '../lib/terminalActivity/recognizer';
import { ScreenApprovalDebouncer } from '../lib/terminalActivity/screenApprovalMonitor';
import type { CompiledPack } from '../lib/terminalActivity/rulePacks';
import type { TerminalTabDescriptor } from '../types';

// Phase 3 "Option B" — background on-screen approval recognition for every
// agent session, not just the focused tab.
//
// Only one `TerminalSurface` (the focused tab) is ever mounted, so recognizing
// off the visible grid could never catch a BACKGROUNDED agent hitting an
// approval gate — which is the entire point of "needs a decision". Instead this
// component (mounted once, high in the tree, for the app's whole lifetime)
// keeps a lightweight **headless** xterm buffer per agent session: it attaches a
// second broadcast subscriber to the session, feeds every PTY chunk into a
// renderer-less `Terminal`, and scans that buffer's bottom rows. A debounced
// approval reading is reported to the backend, which folds it into the same
// activity state as the Claude hooks (screen-sourced == hook-sourced;
// TERMINAL_ACTIVITY §Phase 3). Because the source is just "a buffer", the
// recognizer, debouncer, and backend feed are reused verbatim from the
// focused-surface wiring — this component only supplies the background buffers.

/** How long to coalesce a burst of PTY writes before scanning. A prompt is
 *  drawn across several chunks; waiting for the burst to settle avoids scanning
 *  a half-drawn frame. */
const SCAN_DEBOUNCE_MS = 150;

interface SessionEntry {
  /** Headless buffer fed the session's PTY bytes. No renderer, no DOM. */
  term: Terminal;
  /** Id of the broadcast channel we attached, for a clean detach. */
  channelId: number;
  /** Agent kind this entry was built for — re-attach if it changes. */
  kind: string;
  /** The agent's compiled approval rule pack. */
  pack: CompiledPack;
  /** Presence/confirmation debounce over the per-scan boolean. */
  debouncer: ScreenApprovalDebouncer;
  /** Pending scan timer, or null when idle. */
  timer: ReturnType<typeof setTimeout> | null;
}

export interface TerminalApprovalRecognizerProps {
  /** Current terminal tabs (the provider's `state.tabs`). */
  tabs: TerminalTabDescriptor[];
}

/**
 * Reconciles a headless recognition buffer against the set of live agent
 * sessions that have a rule pack. Renders nothing.
 */
export function TerminalApprovalRecognizer({
  tabs,
}: TerminalApprovalRecognizerProps): null {
  const entriesRef = useRef<Map<string, SessionEntry>>(new Map());

  // The sessions that SHOULD be under recognition right now: a live backend
  // session (running/disconnected — a closed/connecting tab has no stream), an
  // agent kind that has an approval rule pack (Claude has none — it hooks). Keyed
  // by session id → agent kind.
  const wanted = new Map<string, string>();
  for (const t of tabs) {
    if (!t.sessionId) continue;
    if (t.phase !== 'running' && t.phase !== 'disconnected') continue;
    if (t.agentKind && DEFAULT_COMPILED_PACKS.has(t.agentKind)) {
      wanted.set(t.sessionId, t.agentKind);
    }
  }
  // Reconcile only when the (session → kind) set actually changes, not on every
  // unrelated tab re-render.
  const wantedKey = Array.from(wanted.entries())
    .map(([sid, kind]) => `${sid}=${kind}`)
    .sort()
    .join(',');

  useEffect(() => {
    const entries = entriesRef.current;

    const teardown = (sid: string, entry: SessionEntry) => {
      if (entry.timer !== null) clearTimeout(entry.timer);
      void detachTerminalSession(sid, entry.channelId).catch(() => {});
      // Retract any latched approval so the backend doesn't hold a stale
      // "needs a decision" for a session we've stopped watching.
      if (entry.debouncer.reset(false) !== null) {
        void reportScreenActivity(sid, false).catch(() => {});
      }
      entry.term.dispose();
    };

    // 1. Drop entries that are no longer wanted, or whose agent kind changed
    //    (a different pack means a fresh buffer + debounce).
    for (const [sid, entry] of Array.from(entries)) {
      if (wanted.get(sid) !== entry.kind) {
        teardown(sid, entry);
        entries.delete(sid);
      }
    }

    // 2. Attach newly-wanted sessions.
    for (const [sid, kind] of wanted) {
      if (entries.has(sid)) continue;
      const pack = DEFAULT_COMPILED_PACKS.get(kind);
      if (!pack) continue;

      const size = getLastTerminalSize();
      const term = new Terminal({
        cols: size?.cols ?? 80,
        rows: size?.rows ?? 24,
        // We only ever read the on-screen viewport; history would just cost
        // memory across N background buffers.
        scrollback: 0,
        allowProposedApi: true,
      });
      const entry: SessionEntry = {
        term,
        channelId: 0,
        kind,
        pack,
        debouncer: new ScreenApprovalDebouncer(),
        timer: null,
      };

      const runScan = () => {
        entry.timer = null;
        const present = recognizerTick(() => readBottomRows(term), pack);
        const change = entry.debouncer.observe(present);
        if (change !== null) {
          void reportScreenActivity(sid, change).catch(() => {});
          return;
        }
        // Still accumulating toward a flip (the debouncer needs consecutive
        // confirmations) but the agent has gone quiet at its prompt — no more
        // writes will arrive to re-trigger a scan. Keep scanning on our own so
        // the pending transition can actually commit.
        if (present !== entry.debouncer.state) {
          scheduleScan();
        }
      };
      const scheduleScan = () => {
        if (entry.timer !== null) return;
        entry.timer = setTimeout(runScan, SCAN_DEBOUNCE_MS);
      };

      const channel = new Channel<Uint8Array | number[]>();
      entry.channelId = channel.id;
      channel.onmessage = (chunk: Uint8Array | number[]) => {
        const bytes = chunk instanceof Uint8Array ? chunk : new Uint8Array(chunk);
        term.write(bytes);
        scheduleScan();
      };

      entries.set(sid, entry);
      // On attach the backend replays the session's scrollback, so a tab that
      // was already sitting at an approval prompt before we attached is still
      // recognized. A failed attach drops the entry so a later reconcile retries.
      void attachTerminalSession(sid, channel).catch(() => {
        entries.delete(sid);
        if (entry.timer !== null) clearTimeout(entry.timer);
        term.dispose();
      });
    }
  }, [wantedKey]); // eslint-disable-line react-hooks/exhaustive-deps

  // Tear everything down on unmount (app shutdown / provider remount).
  useEffect(() => {
    const entries = entriesRef.current;
    return () => {
      for (const [sid, entry] of entries) {
        if (entry.timer !== null) clearTimeout(entry.timer);
        void detachTerminalSession(sid, entry.channelId).catch(() => {});
        entry.term.dispose();
      }
      entries.clear();
    };
  }, []);

  return null;
}
