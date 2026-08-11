## Building with Demeteo

Demeteo is a **dark-only** design system for an agent-orchestration desktop app.
There is no light theme and no theme switch — do not build one, and never place
these components on a light surface.

### Setup

No provider or root wrapper. `styles.css` sets the page ground itself
(`background-color: var(--bg-app)` on `body`, Inter at 14–16px, `#f3f4f6`
text), so importing it is the whole setup. Build your layout directly on that
ground; use `var(--bg-sidebar)` for rails and `var(--bg-well)` for terminal or
code wells sunk into a panel.

### The one hard constraint: the utility set is fixed

Styling is Tailwind-flavoured utility classes, **but there is no Tailwind at
runtime.** `_ds_bundle.css` is a pre-compiled stylesheet, so a class that isn't
already in it produces *no rule at all* — silently, with no error. This bites
hardest on opacity and shade variants: `bg-violet-500/10` exists,
`bg-violet-500/80` does not.

So: **grep `_ds/<folder>/_ds_bundle.css` for a class before relying on it.** When
what you need isn't there, write the property directly with a token —
`style={{ background: 'var(--bg-panel)' }}` always works — rather than inventing
a class name.

Shipped opacity steps are sparse and differ per colour. `/5 /10 /15` exist on
all of `bg-{violet,cyan,emerald,ruby,amber}-500`; `/20` exists on violet, cyan,
emerald and ruby but **not** amber; violet adds `/30`, emerald adds `/30 /40`.
Borders are similarly limited (`border-white/5`, `border-white/10`). Assume
nothing outside that and check the stylesheet.

### Colour is semantic — match meaning, not taste

| Tone | Means | Example classes |
|---|---|---|
| **violet** | active connection, primary action, agent working | `bg-violet-500/10` `border-violet-500/50` `text-violet-300` |
| **cyan** | in motion, terminal streams, interactive/selected | `bg-cyan-500/10` `text-cyan-400` |
| **emerald** | healthy, completed, running agent | `bg-emerald-500/10` `text-emerald-400` |
| **ruby** | error, failure, stopped (this DS's red alias) | `bg-ruby-500/10` `text-ruby-400` |
| **amber** | needs a human — gates, credentials, interruptions | `bg-amber-500/10` `text-amber-400` |
| **slate** | inert — queued, cancelled, disabled | `text-slate-300` `text-slate-500` |

Never use raw `red-*`; the red alias here is **ruby**.

### Surfaces and type

- **`glass-panel`** is the signature card: `var(--bg-panel)` +
  `backdrop-filter: blur(12px)` + a hairline `var(--border-glass)` + a soft drop
  shadow. Reach for it before hand-rolling a card.
- Fonts: **`font-heading`** (Outfit) for headings, **`font-mono`** (Fira Code)
  for identifiers, paths, statuses and counts — this UI uses mono heavily for
  anything machine-generated — and the Inter default for prose.
  ⚠ The app's own source writes `font-outfit`, which is **not** a real class and
  produces nothing. Use `font-heading`.
- Small labels are uppercase mono with wide tracking: `text-[10px] font-mono
  text-slate-400 uppercase tracking-widest`. Prefer the `FieldLabel` component.
- `animate-pulse-glow` marks something live. Use it sparingly — it means
  "changing on its own".

### Reach for a component first

`SectionCard` (titled glass panel) · `StatusBadge` (run status, dot or pill —
pass a real status like `running`/`gated`/`failed` and it resolves the tone) ·
`PhaseBadge` · `AgentBadge` · `MachineDot` · `ActivityIndicator` · `Modal`
(portalled, backdrop included) · `OverlayPortal` · `TabBar` · `RailNavItem` ·
`ScrollArea` · `FieldLabel` · `TimeoutField` · `HarnessModelPicker`. The
`CreateZero*` set are whole wizard steps, not primitives.

Read the component's `.prompt.md` and `.d.ts` before composing it — they carry
the real prop contract.

### Idiomatic example

```jsx
<SectionCard title="Execution" icon={<Server className="w-4 h-4 text-cyan-400" />}>
  <div className="flex items-center justify-between gap-4">
    <div className="min-w-0">
      <div className="text-sm text-slate-200 font-medium">build-01</div>
      <div className="text-xs font-mono text-slate-500 truncate">ssh://build-01.internal:22</div>
    </div>
    <StatusBadge status="running" variant="pill" />
  </div>
</SectionCard>
```
