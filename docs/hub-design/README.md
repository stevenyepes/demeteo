# Demeteo Hub Mockup

Design for a browser control plane over many Demeteo instances — desktops on
macOS, Linux and Windows, and `demeteo-runner` hosts. Designed, not built.

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
| [AddInstance](AddInstance.dc.html) | Pairing a desktop (browser approval) or a runner (printed code). |
| [DesktopConnect](DesktopConnect.dc.html) | The new Settings › Hub tab inside the desktop app. |
| [Architecture](Architecture.dc.html) | Trust model: instances dial out, the Hub holds no secrets. |
| [Pairing](Pairing.dc.html) | Sequence: OAuth pairing, then event reporting and dispatch. |

The design reverses `docs/OPEN_QUESTIONS.md` §17 (web companion out of
scope) and lets a human decide gates outside the desktop, which the MCP
surface excludes. Neither is decided; both need a `docs/DECISIONS.md` entry
before any of this is built.
