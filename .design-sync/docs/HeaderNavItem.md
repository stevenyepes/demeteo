---
category: Primitives
---

One nav entry in the horizontal top header bar: an icon, an optional label, and
at most one corner badge — an amber actionable count, a cyan activity dot, or an
emerald `animate-pulse-glow` dot, in that precedence.

It renders at one of two densities, and which one is not its decision:
`src/lib/headerLayout.ts` derives it from the measured header width and the bar
passes it down. At `labels` the label is a text node beside the icon; at `icons`
that node is **not rendered at all** rather than hidden with CSS, so the control
does not keep announcing a label the eye cannot see. `aria-label` is what carries
the accessible name across both densities; `title`, once a name is published, is
only the accessible *description*, so it cannot stand in for one. That is also why
the name has to fold in the badge count — `aria-label` overrides the element's
contents, leaving the count unreachable however it is marked up.

**Why `RailNavItem` could not take this job** — rail-sized variants and a
different badge vocabulary — is recorded once, in the header comment of
`src/components/ui/HeaderNavItem.tsx`, next to the code it constrains. Read it
there. AGENTS.md §7 asks for a repo-wide rationale to be cited rather than
copied, and a justification pasted into two files is two things to update and
one that will not be.
