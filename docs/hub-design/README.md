# Demeteo Hub Mockup

Design for a browser control plane over many Demeteo instances — desktops on
macOS, Linux and Windows, and `demeteo-runner` hosts. **Decided, not built.** The
decisions are recorded in [`DECISIONS.md`](../DECISIONS.md#1-the-locked-decisions)
(56–63) and specified in [`HUB.md`](../HUB.md); where this mockup disagrees with
them, they win. The runner half of the design is superseded — see below.

These are the source files of a design canvas
(<https://claude.ai/artifact/Jw4a8KtEapbKaPWDaG54yp>, private to its owner).
Each `.dc.html` needs that canvas's runtime: opened directly in a browser it
shows unfilled `{{…}}` placeholders and no interactivity. `canvas.json` holds
the board layout.

| Artboard | Shows |
| --- | --- |
| [Main](Main.dc.html) | Fleet: every instance, its status, what it is running, and what needs you. |
| [Instance](Instance.dc.html) | One instance: live runs, projects, harnesses, connection, and the scopes it allows. |
| [Dispatch](Dispatch.dc.html) | Start a feature on a chosen instance — `StartFeatureModal`'s fields plus the pipeline graph for per-step overrides. |
| [Runs](Runs.dc.html) | Runs across the fleet, with a gate reviewed and decided from the Hub. |
| [Run](Run.dc.html) | One run's pipeline graph and step inspector. |
| [AddInstance](AddInstance.dc.html) | Pairing a desktop (browser approval). The runner (printed code) half is superseded. |
| [DesktopConnect](DesktopConnect.dc.html) | The new Settings › Hub tab inside the desktop app. |
| [Architecture](Architecture.dc.html) | Trust model: instances dial out, the Hub holds no instance secrets. Shows a runner as a paired instance, which is superseded. |
| [Pairing](Pairing.dc.html) | Sequence: OAuth pairing, then event reporting and dispatch. |

The design reverses `docs/OPEN_QUESTIONS.md` §17 (web companion out of
scope) and lets a human decide gates outside the desktop, which the MCP surface
excludes. Both are decided
([decisions 56 and 60](../DECISIONS.md#1-the-locked-decisions)); neither is built.
The gate decision is a separate surface from MCP's and does not reopen
[decision 50](../MCP_INTEGRATION.md#8-what-is-excluded-and-whether-permanently).

## Superseded by the decisions

The artboards are not edited. Where they disagree with a decision, the decision wins:

| Artboard says | Superseded by |
| --- | --- |
| A `logs` scope that sends transcripts and diffs to the Hub (`DesktopConnect`, `Instance`, `Run`, `Runs`) | [Decision 61](../DECISIONS.md#61--hub-scopes-and-ceilings-detail): scopes are `read`, `spend` and `gates`; transcripts and diffs never leave an instance |
| A runner pairing with a printed code, `demeteo-runner hub pair <url>`, and a refresh token in the runner's credential store (`AddInstance`) | [Decision 58](../DECISIONS.md#58--hub-instances-detail): runners are not paired; a later, separate discovery covers runners over SSH |
| A runner as a paired instance (`Architecture`, `Pairing`) | Decisions 58 and 59 |
| A passkey required only for merge-to-default and over-budget gates (`DesktopConnect`, `Instance`) | [Decision 60](../DECISIONS.md#60--hub-gate-decisions-detail): every Hub gate decision carries an assertion |

The mockup has no place to set the per-run and rolling 24-hour ceilings, and none
for endorsing or revoking a passkey. [`HUB.md`](../HUB.md) §7 and §8 specify both.
