import { StatusBadge } from 'demeteo';

const Row = ({ children }: { children: React.ReactNode }) => (
  <div className="flex flex-wrap items-center gap-2">{children}</div>
);

/** Every tone in the canonical run-status vocabulary (lib/runStatus.ts). */
export const Pills = () => (
  <div className="flex flex-col gap-3">
    <Row>
      <StatusBadge status="running" variant="pill" />
      <StatusBadge status="verifying" variant="pill" />
      <StatusBadge status="pending" variant="pill" />
    </Row>
    <Row>
      <StatusBadge status="gated" variant="pill" />
      <StatusBadge status="needs_credentials" variant="pill" />
      <StatusBadge status="interrupted" variant="pill" />
    </Row>
    <Row>
      <StatusBadge status="completed" variant="pill" />
      <StatusBadge status="awaiting_mr" variant="pill" />
      <StatusBadge status="published" variant="pill" />
    </Row>
    <Row>
      <StatusBadge status="failed" variant="pill" />
      <StatusBadge status="over-budget" variant="pill" />
      <StatusBadge status="cancelled" variant="pill" />
    </Row>
  </div>
);

/** The dot variant — same tone mapping, no label, for dense list rows. */
export const Dots = () => (
  <Row>
    {['running', 'verifying', 'gated', 'completed', 'failed', 'cancelled'].map((s) => (
      <span key={s} className="flex items-center gap-2 text-xs font-mono text-slate-400">
        <StatusBadge status={s} />
        {s}
      </span>
    ))}
  </Row>
);

/** `label` overrides the humanized status — used where the row already says why. */
export const CustomLabel = () => (
  <Row>
    <StatusBadge status="gated" variant="pill" label="Waiting on review" />
    <StatusBadge status="over-budget" variant="pill" label="3.2M tokens" />
    <StatusBadge status="running" variant="pill" label="Step 4 of 9" />
  </Row>
);

/** In place: the dot leading a feature row, the pill closing it. */
export const InAFeatureRow = () => (
  <div className="flex flex-col gap-1 w-full max-w-xl">
    {[
      { title: 'Add SSH connection pooling', status: 'running' },
      { title: 'Windows path derivation for worktrees', status: 'gated' },
      { title: 'Retry budget for harness triage', status: 'completed' },
      { title: 'Runner artifact upload', status: 'failed' },
    ].map((f) => (
      <div
        key={f.title}
        className="flex items-center gap-3 px-3 py-2.5 rounded-lg bg-white/[0.02] border border-white/5"
      >
        <StatusBadge status={f.status} />
        <span className="text-sm text-slate-200 flex-1 truncate">{f.title}</span>
        <StatusBadge status={f.status} variant="pill" />
      </div>
    ))}
  </div>
);
