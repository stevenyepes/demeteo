//! Ecosystem recipes — the reusable half of project detection (HB3).
//!
//! # What lives here and what does not
//!
//! Detection has two halves with very different lifetimes. The **recipe** —
//! "a `package.json` means npm, its suite is `scripts.test`, a fresh checkout
//! needs an install first" — is knowledge about an ecosystem, true of every
//! repository of that shape, and it belongs in one place. The **command
//! string** it produces is knowledge about *one* repository and belongs to that
//! project's settings. That split is the honest answer to "should there be a
//! global harness config": a global map of `lint → npm run lint` would be wrong
//! for nearly every project, while the recipe that derives it is right for all
//! of them.
//!
//! So this module is pure and synchronous: it takes the evidence an adapter
//! gathered off a filesystem and decides what to emit. Everything reachable
//! from a unit test with no port double, per AGENTS.md ("where a decision is
//! allowed to live").
//!
//! # Why named harnesses rather than one command
//!
//! Detection used to emit a single `test_command`, and for a polyglot repo that
//! was a hand-rolled accumulator:
//!
//! ```text
//! set +e; rc=0; npm test; rc=$((rc||$?)); cargo test; rc=$((rc||$?)); exit $rc
//! ```
//!
//! That string existed **only because there was nowhere to put more than one
//! harness**. It ran every suite, which was the point, but it threw away *which
//! ecosystem failed* — precisely the attribution HB5's plural `HarnessOutcome`
//! exists to recover, and which HB2c's per-gate subtraction now depends on. A
//! baseline cannot record "the Rust suite was already red" about a command that
//! only reports one exit status for both halves of the repo.
//!
//! With `harnesses` plural and gate-selectable, the accumulator has a
//! replacement rather than a fix: `{js-test: "npm test", rust-test: "cargo
//! test"}`, both pre-ticked in `validation_gates`.
//!
//! # The three defects this module is built to not have
//!
//! 1. **Root-only marker stat.** The old loop stat-ed `{repo}/{marker}` and
//!    nothing deeper, so a Tauri app whose `Cargo.toml` lives in `src-tauri/`
//!    matched `package.json` alone and the entire Rust half was invisible.
//!    [`MarkerSite::dir`] carries where the marker actually was, and the
//!    commands are wrapped to run there.
//! 2. **No `prepare_command`.** Detection returned `None` unconditionally, and a
//!    validate worktree is a clean `git worktree add` with no `node_modules` and
//!    no `target/` — so a detected `npm test` failed on a project that works
//!    fine for the human. Every recipe now names its install step.
//! 3. **Watch-mode runners.** `npm test` resolves to whatever `scripts.test`
//!    says, frequently `vitest` or `jest --watch`, which never exit. Since S10
//!    that terminates at the wall-clock cap with remediation naming watch mode —
//!    a far better failure, but still a wasted ceiling on every run.
//!    [`classify_test_script`] reads the script and either corrects it or
//!    declines to emit it.
//!
//! # The bias
//!
//! Emitting nothing is recoverable; emitting a confidently wrong command is
//! what this whole document exists to stop. So a script this module cannot
//! resolve to something that terminates is **not emitted** — the project simply
//! has no JS test gate until a human sets one, and the settings panel (HB6)
//! shows the gap. The one exception is a manifest that could not be read or
//! parsed at all: there the module knows nothing, and falling back to today's
//! `npm test` is strictly better than removing a harness on no evidence.

use std::collections::BTreeMap;

/// One ecosystem Demeteo knows how to drive, identified by a marker file.
pub struct EcosystemRecipe {
    /// Stable id. Only used to group sites of the same ecosystem, so that a
    /// root manifest can shadow the subdirectory ones.
    pub id: &'static str,
    /// The file whose presence in a directory means "a project of this
    /// ecosystem lives here".
    pub marker: &'static str,
    /// Harness name for this ecosystem's test suite.
    pub test_gate: &'static str,
    /// Harness name for its build, when it has one worth running separately.
    /// Emitted into the `harnesses` map but **not** pre-ticked as a gate: a
    /// test run almost always builds first, so ticking both doubles the
    /// wall-clock for one extra signal. Whether that trade is worth it is the
    /// user's call, and the map is what lets them make it.
    pub build_gate: Option<&'static str>,
}

/// Every ecosystem detection recognises, in the order their gates are emitted.
pub const ECOSYSTEMS: &[EcosystemRecipe] = &[
    EcosystemRecipe {
        id: "js",
        marker: "package.json",
        test_gate: "js-test",
        build_gate: Some("js-build"),
    },
    EcosystemRecipe {
        id: "rust",
        marker: "Cargo.toml",
        test_gate: "rust-test",
        build_gate: Some("rust-build"),
    },
    EcosystemRecipe {
        id: "go",
        marker: "go.mod",
        test_gate: "go-test",
        build_gate: Some("go-build"),
    },
    EcosystemRecipe {
        id: "python",
        marker: "requirements.txt",
        test_gate: "python-test",
        build_gate: None,
    },
];

/// The JS lockfiles that name a package manager, most specific first.
pub const JS_LOCKFILES: &[&str] = &[
    "pnpm-lock.yaml",
    "yarn.lock",
    "bun.lockb",
    "bun.lock",
    "package-lock.json",
];

/// Directories never descended into when looking for markers below the root.
///
/// Two reasons, and both matter. `node_modules` and `target` are *full* of
/// marker files — every npm dependency ships a `package.json`, every vendored
/// crate a `Cargo.toml` — so descending would detect hundreds of phantom
/// ecosystems. And listing them is expensive enough over SFTP to be a bug of its
/// own on a repository that has been built once.
pub const SKIPPED_DIRS: &[&str] = &[
    "node_modules",
    "target",
    "vendor",
    "dist",
    "build",
    "out",
    "coverage",
    "__pycache__",
    "venv",
    ".venv",
    "tmp",
];

/// How many immediate subdirectories are examined before the scan stops.
///
/// The depth bound (root plus one level) already keeps this from being a tree
/// walk; this bounds the *breadth* as well, because a monorepo root with two
/// hundred package directories would otherwise turn one detection into two
/// hundred SFTP round trips. Anything past the cap is a repository whose layout
/// a human should be describing in settings anyway.
pub const MAX_SCANNED_SUBDIRS: usize = 24;

/// Whether a directory sitting directly under the repository root is worth
/// looking inside for marker files.
///
/// Dot-directories are excluded wholesale: `.git`, `.github`, `.cargo`, `.venv`
/// and friends hold configuration and history, never the project's own
/// manifest.
pub fn is_scannable_subdir(name: &str) -> bool {
    !name.is_empty() && !name.starts_with('.') && !SKIPPED_DIRS.contains(&name)
}

/// The recipe a marker file name belongs to, if any.
pub fn marker_recipe(marker: &str) -> Option<&'static EcosystemRecipe> {
    ECOSYSTEMS.iter().find(|e| e.marker == marker)
}

/// One marker file an adapter found, plus the evidence needed to resolve that
/// ecosystem's commands.
///
/// Deliberately inert data: the adapter's whole job is to fill this in, and
/// every decision made from it is a synchronous function below.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MarkerSite {
    /// The marker file name, e.g. `package.json`.
    pub marker: String,
    /// Repo-relative directory holding it. Empty means the repository root.
    pub dir: String,
    /// Raw `package.json` text, when the adapter could read it. `None` means
    /// "unknown", not "empty" — see the module header's note on the one case
    /// where this module falls back rather than declining.
    pub manifest: Option<String>,
    /// Lockfile names sitting beside the marker.
    pub lockfiles: Vec<String>,
}

/// Everything detection derives from a repository's markers.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DetectedCommands {
    /// Named harnesses, keyed by gate name. Sorted, so two detections of the
    /// same repository produce byte-identical settings.
    pub harnesses: BTreeMap<String, String>,
    /// The test gates, pre-ticked, in ecosystem order — tier 2 of
    /// `resolve_harnesses`.
    pub validation_gates: Vec<String>,
    /// Tier 3's single command. Set **only** when exactly one test gate was
    /// detected; a polyglot repo has no single command, and inventing one is
    /// what the accumulator was. It is unreachable through the resolution chain
    /// whenever `validation_gates` is non-empty, so its remaining job is to
    /// render `{{test_command}}` in prompts authored before harnesses were
    /// plural.
    pub test_command: Option<String>,
    /// Every detected build, `&&`-chained. Fail-fast is right here in a way it
    /// is not for gates: a build is not a verdict, and there is nothing to
    /// attribute when one half of a polyglot repo will not compile.
    pub build_command: Option<String>,
    /// Every detected install step, `&&`-chained, for the fresh checkout a
    /// harness actually runs in.
    pub prepare_command: Option<String>,
}

/// Compose a repository's detected commands from the markers found in it.
pub fn compose_commands(sites: &[MarkerSite]) -> DetectedCommands {
    let mut out = DetectedCommands::default();

    for recipe in ECOSYSTEMS {
        let mut of_this: Vec<&MarkerSite> =
            sites.iter().filter(|s| s.marker == recipe.marker).collect();
        if of_this.is_empty() {
            continue;
        }

        // A manifest at the repository root is authoritative for its whole
        // ecosystem: a Cargo workspace, npm workspaces and a Go module all
        // describe their members from the root, so also emitting a gate per
        // member would run the same suite twice under two names. Only when
        // there is no root manifest do the subdirectories each stand alone
        // (`frontend/` + `admin/`, a Tauri app's `src-tauri/`).
        if of_this.iter().any(|s| s.dir.is_empty()) {
            of_this.retain(|s| s.dir.is_empty());
        }
        of_this.sort_by(|a, b| a.dir.cmp(&b.dir));

        let multiple = of_this.len() > 1;
        for site in of_this {
            let cmds = site_commands(recipe, site);
            let suffix = if multiple {
                format!("-{}", slug(&site.dir))
            } else {
                String::new()
            };

            if let Some(test) = cmds.test {
                let name = format!("{}{}", recipe.test_gate, suffix);
                out.validation_gates.push(name.clone());
                out.harnesses.insert(name, test);
            }
            if let (Some(gate), Some(build)) = (recipe.build_gate, cmds.build.clone()) {
                out.harnesses.insert(format!("{gate}{suffix}"), build);
            }
            if let Some(build) = cmds.build {
                append_chained(&mut out.build_command, &build);
            }
            if let Some(prepare) = cmds.prepare {
                append_chained(&mut out.prepare_command, &prepare);
            }
        }
    }

    if out.validation_gates.len() == 1 {
        out.test_command = out.harnesses.get(&out.validation_gates[0]).cloned();
    }
    out
}

/// The three commands one detected marker yields.
struct SiteCommands {
    test: Option<String>,
    build: Option<String>,
    prepare: Option<String>,
}

fn site_commands(recipe: &EcosystemRecipe, site: &MarkerSite) -> SiteCommands {
    let (test, build, prepare) = match recipe.id {
        "js" => {
            let pm = PackageManager::from_lockfiles(&site.lockfiles);
            let scripts = Scripts::read(site.manifest.as_deref());
            let test = match scripts.test {
                // Nothing to run, or nothing that terminates. Emitting the
                // manager's `test` script anyway is the defect: `npm test` on a
                // package with no `scripts.test` exits 1 with "Missing script",
                // which reaches validate dressed as this feature's failure.
                TestScript::Missing | TestScript::Uncorrectable => None,
                TestScript::OneShot => Some(pm.test_script()),
                TestScript::Correctable(args) => Some(pm.with_args(&pm.test_script(), args)),
            };
            let build = scripts.has_build.then(|| pm.build_script());
            (test, build, Some(pm.install().to_string()))
        }
        "rust" => (
            Some("cargo test".to_string()),
            Some("cargo build".to_string()),
            Some("cargo fetch".to_string()),
        ),
        "go" => (
            Some("go test ./...".to_string()),
            Some("go build ./...".to_string()),
            Some("go mod download".to_string()),
        ),
        "python" => (
            Some("pytest".to_string()),
            None,
            Some(format!("pip install -r {}", recipe.marker)),
        ),
        _ => (None, None, None),
    };

    SiteCommands {
        test: test.map(|c| in_dir(&site.dir, &c)),
        build: build.map(|c| in_dir(&site.dir, &c)),
        prepare: prepare.map(|c| in_dir(&site.dir, &c)),
    }
}

/// Run `cmd` in `dir`, which is repo-relative and may be the root.
///
/// A subshell rather than a bare `cd … && …` because these commands are
/// chained: a `cd` that leaks into the next link of a `prepare_command` would
/// silently run the second install in the first one's directory.
///
/// The separator is `/`, not [`Path::join`](std::path::Path::join), and that is
/// deliberate. This is not a host path — it is a fragment of a POSIX shell
/// command that will be executed on the *target* machine, which for a remote
/// project is Linux no matter what the desktop is running. Joining it with the
/// host's separator would emit a backslash on a Windows desktop and break the
/// command on every machine it was sent to.
pub fn in_dir(dir: &str, cmd: &str) -> String {
    if dir.is_empty() {
        cmd.to_string()
    } else {
        format!("(cd {} && {})", crate::paths::shell_escape_posix(dir), cmd)
    }
}

/// Append `cmd` to an `&&` chain, starting one if the slot is empty.
fn append_chained(slot: &mut Option<String>, cmd: &str) {
    match slot {
        Some(existing) => {
            existing.push_str(" && ");
            existing.push_str(cmd);
        }
        None => *slot = Some(cmd.to_string()),
    }
}

/// A directory name as a harness-name suffix.
fn slug(dir: &str) -> String {
    let mapped: String = dir
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    mapped.trim_matches('-').to_string()
}

// ── JS: which manager, and does the test script terminate ────────────────────

/// The JS package manager a directory's lockfiles imply.
///
/// Detected for the sake of `prepare_command` above all: `npm ci` in a pnpm
/// repository does not merely fail, it writes a `package-lock.json` that was
/// never meant to exist. The run form follows from the same answer, since a
/// project's scripts are habitually invoked through its own manager.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageManager {
    /// `locked` = a `package-lock.json` is present, so the fresh-checkout
    /// install is `npm ci` (exact, reproducible, and it expects to run against
    /// an empty `node_modules` — which is what a new worktree has).
    Npm {
        locked: bool,
    },
    Pnpm,
    Yarn,
    Bun,
}

impl PackageManager {
    /// The manager the lockfiles beside a `package.json` name.
    pub fn from_lockfiles(lockfiles: &[String]) -> Self {
        let has = |n: &str| lockfiles.iter().any(|l| l == n);
        if has("pnpm-lock.yaml") {
            PackageManager::Pnpm
        } else if has("yarn.lock") {
            PackageManager::Yarn
        } else if has("bun.lockb") || has("bun.lock") {
            PackageManager::Bun
        } else {
            PackageManager::Npm {
                locked: has("package-lock.json"),
            }
        }
    }

    /// The install step for a checkout with no `node_modules` at all.
    pub fn install(&self) -> &'static str {
        match self {
            PackageManager::Npm { locked: true } => "npm ci",
            PackageManager::Npm { locked: false } => "npm install",
            PackageManager::Pnpm => "pnpm install",
            PackageManager::Yarn => "yarn install",
            PackageManager::Bun => "bun install",
        }
    }

    fn test_script(&self) -> String {
        match self {
            PackageManager::Npm { .. } => "npm test".to_string(),
            PackageManager::Pnpm => "pnpm test".to_string(),
            PackageManager::Yarn => "yarn test".to_string(),
            PackageManager::Bun => "bun run test".to_string(),
        }
    }

    fn build_script(&self) -> String {
        match self {
            PackageManager::Npm { .. } => "npm run build".to_string(),
            PackageManager::Pnpm => "pnpm run build".to_string(),
            PackageManager::Yarn => "yarn build".to_string(),
            PackageManager::Bun => "bun run build".to_string(),
        }
    }

    /// Forward extra arguments to the script itself.
    ///
    /// Only npm needs the `--` separator; pnpm, yarn and bun forward trailing
    /// arguments to the script directly, and npm's `--` is inert for them at
    /// best.
    fn with_args(&self, cmd: &str, args: &str) -> String {
        match self {
            PackageManager::Npm { .. } => format!("{cmd} -- {args}"),
            _ => format!("{cmd} {args}"),
        }
    }
}

/// What a package's `scripts.test` entry turns out to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestScript {
    /// Absent, blank, or `npm init`'s `echo "Error: no test specified" && exit
    /// 1` placeholder. There is no suite, so there is no harness to emit —
    /// `npm test` would exit 1 for a reason that has nothing to do with the
    /// code.
    Missing,
    /// Runs once and exits.
    OneShot,
    /// A watch-mode runner with a known one-shot argument, carried here so the
    /// caller can append it in its manager's spelling.
    Correctable(&'static str),
    /// A watch-mode runner with no one-shot form — or one that is not the
    /// *last* command in the script, where appended arguments would land on a
    /// different command entirely. Nothing safe to emit.
    Uncorrectable,
}

/// Read a `scripts.test` entry and decide whether a harness can be built from
/// it.
///
/// The cost this removes is concrete: a watch-mode runner produces no exit
/// status, so S10 abandons it at the wall-clock ceiling — half an hour by
/// default, on every single run, to learn something detection could have read
/// out of `package.json` in a millisecond.
pub fn classify_test_script(script: Option<&str>) -> TestScript {
    let Some(script) = script.map(str::trim).filter(|s| !s.is_empty()) else {
        return TestScript::Missing;
    };
    // `npm init`'s placeholder. It is not a suite; it is a note saying there
    // isn't one, and it always exits 1.
    if script.contains("no test specified") {
        return TestScript::Missing;
    }

    let segments: Vec<TestScript> = split_commands(script).map(classify_segment).collect();
    if segments.is_empty() {
        return TestScript::Missing;
    }
    if segments.contains(&TestScript::Uncorrectable) {
        return TestScript::Uncorrectable;
    }
    // Arguments appended via `npm test -- …` land at the end of the whole
    // script, so they only reach the watcher when the watcher is last. A
    // `vitest && npm run lint` would otherwise be "corrected" into passing
    // `--run` to lint.
    let (last, rest) = segments.split_last().expect("non-empty");
    if rest.iter().any(|s| matches!(s, TestScript::Correctable(_))) {
        return TestScript::Uncorrectable;
    }
    match last {
        TestScript::Correctable(args) => TestScript::Correctable(args),
        _ => TestScript::OneShot,
    }
}

/// Split a script into the individual commands it runs.
fn split_commands(script: &str) -> impl Iterator<Item = &str> {
    script
        .split(['\n', ';'])
        .flat_map(|s| s.split("&&"))
        .flat_map(|s| s.split("||"))
        .flat_map(|s| s.split('|'))
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

/// Wrappers that stand in front of the runner without being it.
const RUNNER_WRAPPERS: &[&str] = &[
    "npm",
    "pnpm",
    "yarn",
    "bun",
    "npx",
    "run",
    "exec",
    "dlx",
    "cross-env",
    "--silent",
    "-s",
];

/// Runners whose entire purpose is to watch. There is no argument that makes
/// them run once, so a script built on one has no correction — only an honest
/// refusal to emit it.
const WATCH_ONLY_RUNNERS: &[&str] = &[
    "nodemon",
    "cargo-watch",
    "watchexec",
    "chokidar",
    "onchange",
    "tsc-watch",
    "concurrently",
    "karma",
];

fn classify_segment(segment: &str) -> TestScript {
    let mut words = segment.split_whitespace().peekable();
    // Step over `VAR=value` assignments and the manager/exec wrappers in front
    // of the actual runner: `cross-env CI=1 npx vitest` runs `vitest`.
    while let Some(w) = words.peek() {
        let is_assignment = w.contains('=') && !w.starts_with('=') && !w.starts_with('-');
        if is_assignment || RUNNER_WRAPPERS.contains(w) {
            words.next();
        } else {
            break;
        }
    }
    let Some(word) = words.next() else {
        return TestScript::OneShot;
    };
    let runner = word.rsplit('/').next().unwrap_or(word);
    let rest: Vec<&str> = words.collect();

    let watch_flag = |args: &[&str]| {
        args.iter().any(|a| {
            (*a == "--watch" || *a == "--watchAll" || a.starts_with("--watch="))
                && !a.ends_with("=false")
        })
    };

    match runner {
        "vitest" => {
            let one_shot = rest.first() == Some(&"run")
                || rest
                    .iter()
                    .any(|a| *a == "--run" || *a == "--no-watch" || *a == "--watch=false");
            if one_shot {
                TestScript::OneShot
            } else {
                // vitest watches by default outside CI, and "outside CI" is
                // exactly where Demeteo runs it.
                TestScript::Correctable("--run")
            }
        }
        "jest" if watch_flag(&rest) => TestScript::Correctable("--watch=false --watchAll=false"),
        r if WATCH_ONLY_RUNNERS.contains(&r) => TestScript::Uncorrectable,
        // A watch flag on a runner with no known off-switch. Guessing one is
        // how a confidently wrong command gets emitted.
        _ if watch_flag(&rest) => TestScript::Uncorrectable,
        _ => TestScript::OneShot,
    }
}

/// The two `package.json` scripts detection cares about.
struct Scripts {
    test: TestScript,
    has_build: bool,
}

impl Scripts {
    /// Read them out of a raw manifest.
    ///
    /// An unreadable or unparseable manifest yields today's behaviour — assume
    /// both scripts exist — rather than the confident "this project has no
    /// tests" that a parse failure would otherwise become. This module declines
    /// to emit when it *knows* a command would not work; it does not decline on
    /// ignorance.
    fn read(manifest: Option<&str>) -> Self {
        let Some(raw) = manifest else {
            return Scripts {
                test: TestScript::OneShot,
                has_build: true,
            };
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(raw) else {
            return Scripts {
                test: TestScript::OneShot,
                has_build: true,
            };
        };
        let script = |name: &str| {
            value
                .get("scripts")
                .and_then(|s| s.get(name))
                .and_then(|s| s.as_str())
        };
        Scripts {
            test: classify_test_script(script("test")),
            has_build: script("build")
                .map(str::trim)
                .is_some_and(|s| !s.is_empty()),
        }
    }
}

#[cfg(test)]
#[path = "../../tests/domain/ecosystem.rs"]
mod tests;
