//! Pure decisions over ecosystem detection (HB3).
//!
//! Every test here reaches the policy directly: no port doubles, no I/O, no
//! filesystem. What the *adapter* finds on a real repository — the depth bound,
//! the skipped directories, the listing — is covered in
//! `tests/infrastructure/worktree/git_ops/strategy.rs` against a real
//! `LocalSubprocessAdapter`, which is the opposite of a double that answers
//! everything `Ok("")`.

use super::*;

fn site(marker: &str, dir: &str) -> MarkerSite {
    MarkerSite {
        marker: marker.to_string(),
        dir: dir.to_string(),
        manifest: None,
        lockfiles: Vec::new(),
    }
}

/// A `package.json` whose `scripts` are exactly what is passed.
fn manifest(scripts: &str) -> String {
    format!(r#"{{"name":"x","scripts":{{{scripts}}}}}"#)
}

fn js_site(dir: &str, scripts: &str, lockfiles: &[&str]) -> MarkerSite {
    MarkerSite {
        marker: "package.json".to_string(),
        dir: dir.to_string(),
        manifest: Some(manifest(scripts)),
        lockfiles: lockfiles.iter().map(|l| l.to_string()).collect(),
    }
}

// ── The headline: named harnesses, and no accumulator anywhere ───────────────

/// The Stratosbar/Tauri layout, which is the shape this task exists for: a root
/// `package.json` and a `Cargo.toml` that lives only under `src-tauri/`.
/// Detection used to see the JS half alone.
#[test]
fn a_tauri_layout_detects_both_ecosystems_as_separate_named_gates() {
    let out = compose_commands(&[
        js_site("", r#""test":"vitest run","build":"vite build""#, &[]),
        site("Cargo.toml", "src-tauri"),
    ]);

    assert_eq!(
        out.harnesses.get("js-test").map(String::as_str),
        Some("npm test")
    );
    assert_eq!(
        out.harnesses.get("rust-test").map(String::as_str),
        Some("(cd src-tauri && cargo test)")
    );
    // Both pre-ticked, in ecosystem order: a gate nobody selects is dead
    // config, which is the whole reason HB5 added tier 2.
    assert_eq!(out.validation_gates, vec!["js-test", "rust-test"]);
}

/// The string this task deletes. It ran every suite, which was right, but it
/// reported one exit status for the whole repo — so a baseline can never say
/// *which* half was already red, and HB2c's per-gate subtraction has nothing to
/// subtract.
#[test]
fn no_rc_accumulator_appears_in_any_emitted_command() {
    let out = compose_commands(&[
        js_site("", r#""test":"vitest run","build":"vite build""#, &[]),
        site("Cargo.toml", "src-tauri"),
        site("go.mod", "svc"),
        site("requirements.txt", "ml"),
    ]);

    let mut all: Vec<String> = out.harnesses.values().cloned().collect();
    all.extend(out.test_command.clone());
    all.extend(out.build_command.clone());
    all.extend(out.prepare_command.clone());
    for cmd in &all {
        assert!(
            !cmd.contains("rc=") && !cmd.contains("set +e") && !cmd.contains("exit $rc"),
            "the accumulator must be gone, not relocated; got: {cmd}"
        );
    }
    assert!(!all.is_empty());
}

/// A single-ecosystem repo — still the common case — must read cleanly: one
/// gate, and tier 3's `test_command` set to the same command so a workflow
/// binding `{{test_command}}` keeps rendering.
#[test]
fn a_single_ecosystem_repo_still_produces_one_plain_command() {
    let out = compose_commands(&[site("Cargo.toml", "")]);

    assert_eq!(out.validation_gates, vec!["rust-test"]);
    assert_eq!(out.test_command.as_deref(), Some("cargo test"));
    assert_eq!(out.build_command.as_deref(), Some("cargo build"));
    assert_eq!(out.prepare_command.as_deref(), Some("cargo fetch"));
}

/// A polyglot repo has no single command, so tier 3 stays empty rather than
/// being handed one ecosystem's command (which would claim the other half was
/// covered) or a re-invented accumulator.
#[test]
fn a_polyglot_repo_leaves_the_single_test_command_unset() {
    let out = compose_commands(&[site("Cargo.toml", ""), site("go.mod", "svc")]);
    assert!(out.validation_gates.len() > 1);
    assert_eq!(out.test_command, None);
}

// ── prepare_command ─────────────────────────────────────────────────────────

/// The validate worktree is a clean `git worktree add`: no `node_modules`, no
/// `target/`. Detection returned `prepare_command: None` unconditionally, so a
/// detected `npm test` failed on a project that works fine for the human.
#[test]
fn every_detected_ecosystem_contributes_an_install_step() {
    let out = compose_commands(&[
        js_site("", r#""test":"vitest run""#, &["package-lock.json"]),
        site("Cargo.toml", "src-tauri"),
    ]);

    assert_eq!(
        out.prepare_command.as_deref(),
        Some("npm ci && (cd src-tauri && cargo fetch)")
    );
}

/// `npm ci` in a pnpm repository does not merely fail — it writes a
/// `package-lock.json` that was never meant to exist.
#[test]
fn the_install_step_follows_the_lockfile_that_is_actually_there() {
    let pnpm = compose_commands(&[js_site("", r#""test":"vitest run""#, &["pnpm-lock.yaml"])]);
    assert_eq!(pnpm.prepare_command.as_deref(), Some("pnpm install"));
    assert_eq!(
        pnpm.harnesses.get("js-test").map(String::as_str),
        Some("pnpm test")
    );

    let yarn = compose_commands(&[js_site("", r#""test":"vitest run""#, &["yarn.lock"])]);
    assert_eq!(yarn.prepare_command.as_deref(), Some("yarn install"));

    // No lockfile at all: `npm ci` would refuse, so the reproducible form is
    // not available and `npm install` is the honest one.
    let bare = compose_commands(&[js_site("", r#""test":"vitest run""#, &[])]);
    assert_eq!(bare.prepare_command.as_deref(), Some("npm install"));
}

// ── Watch mode ──────────────────────────────────────────────────────────────

/// `"test": "vitest"` — the Stratosbar case, and vitest's own default. It never
/// exits, so S10 abandons it at the wall-clock ceiling: half an hour burned on
/// every run to learn something readable out of `package.json`.
#[test]
fn a_watch_mode_test_script_is_corrected_rather_than_emitted_bare() {
    let out = compose_commands(&[js_site("", r#""test":"vitest""#, &[])]);
    assert_eq!(
        out.harnesses.get("js-test").map(String::as_str),
        Some("npm test -- --run")
    );
}

#[test]
fn an_already_one_shot_script_is_left_alone() {
    for script in ["vitest run", "vitest --run", "jest", "jest --ci"] {
        assert_eq!(
            classify_test_script(Some(script)),
            TestScript::OneShot,
            "{script} runs once already"
        );
    }
}

#[test]
fn jest_is_corrected_only_when_it_was_actually_told_to_watch() {
    assert_eq!(
        classify_test_script(Some("jest --watch")),
        TestScript::Correctable("--watch=false --watchAll=false")
    );
    assert_eq!(
        classify_test_script(Some("jest --watchAll")),
        TestScript::Correctable("--watch=false --watchAll=false")
    );
    assert_eq!(
        classify_test_script(Some("jest --watchAll=false")),
        TestScript::OneShot
    );
}

/// Appended arguments land at the *end* of the whole script, so correcting a
/// watcher that is not last would pass `--run` to something else entirely.
#[test]
fn a_watcher_that_is_not_the_last_command_cannot_be_corrected() {
    assert_eq!(
        classify_test_script(Some("vitest && npm run lint")),
        TestScript::Uncorrectable
    );
    // …but last is fine, and the earlier commands are untouched.
    assert_eq!(
        classify_test_script(Some("npm run lint && vitest")),
        TestScript::Correctable("--run")
    );
}

/// There is no argument that makes `nodemon` run once. Emitting it anyway is
/// exactly the confidently wrong command this task removes, so the gate is not
/// emitted at all — the settings panel shows the gap, and a human fills it.
#[test]
fn a_runner_with_no_one_shot_form_yields_no_gate_rather_than_a_hang() {
    assert_eq!(
        classify_test_script(Some("nodemon --exec mocha")),
        TestScript::Uncorrectable
    );
    assert_eq!(
        classify_test_script(Some("tsc --watch")),
        TestScript::Uncorrectable
    );

    let out = compose_commands(&[js_site("", r#""test":"nodemon --exec mocha""#, &[])]);
    assert!(!out.harnesses.contains_key("js-test"));
    assert!(out.validation_gates.is_empty());
    // The rest of the recipe still applies: the install step and the build are
    // unaffected by a broken test script.
    assert_eq!(out.prepare_command.as_deref(), Some("npm install"));
}

/// `npm test` on a package with no `scripts.test` exits 1 with "Missing
/// script", which arrives at validate wearing the costume of this feature's
/// failure. `npm init`'s placeholder does the same thing on purpose.
#[test]
fn a_package_with_no_real_test_script_gets_no_test_gate() {
    assert_eq!(classify_test_script(None), TestScript::Missing);
    assert_eq!(classify_test_script(Some("  ")), TestScript::Missing);
    assert_eq!(
        classify_test_script(Some(r#"echo "Error: no test specified" && exit 1"#)),
        TestScript::Missing
    );

    let out = compose_commands(&[js_site("", r#""build":"vite build""#, &[])]);
    assert!(!out.harnesses.contains_key("js-test"));
    assert_eq!(
        out.harnesses.get("js-build").map(String::as_str),
        Some("npm run build")
    );
}

/// The one place ignorance is not treated as absence. A manifest that could not
/// be read or parsed says nothing about the project, and dropping its harness
/// on no evidence is worse than today's behaviour.
#[test]
fn an_unreadable_manifest_falls_back_to_todays_behaviour() {
    let unread = MarkerSite {
        marker: "package.json".to_string(),
        dir: String::new(),
        manifest: None,
        lockfiles: Vec::new(),
    };
    let out = compose_commands(&[unread]);
    assert_eq!(
        out.harnesses.get("js-test").map(String::as_str),
        Some("npm test")
    );

    let corrupt = MarkerSite {
        manifest: Some("{ this is not json".to_string()),
        ..js_site("", "", &[])
    };
    let out = compose_commands(&[corrupt]);
    assert_eq!(
        out.harnesses.get("js-test").map(String::as_str),
        Some("npm test")
    );
}

#[test]
fn wrappers_and_env_assignments_do_not_hide_the_runner() {
    for script in [
        "npx vitest",
        "cross-env CI=1 npx vitest",
        "pnpm exec vitest",
        "./node_modules/.bin/vitest",
    ] {
        assert_eq!(
            classify_test_script(Some(script)),
            TestScript::Correctable("--run"),
            "{script} is vitest in watch mode"
        );
    }
}

// ── Where a command runs ────────────────────────────────────────────────────

/// A `cd` that leaks would run the second install in the first one's directory.
#[test]
fn a_subdirectory_command_is_wrapped_so_the_cd_cannot_leak() {
    assert_eq!(in_dir("", "cargo test"), "cargo test");
    assert_eq!(
        in_dir("src-tauri", "cargo test"),
        "(cd src-tauri && cargo test)"
    );

    let out = compose_commands(&[site("Cargo.toml", "engine"), site("go.mod", "svc")]);
    assert_eq!(
        out.prepare_command.as_deref(),
        Some("(cd engine && cargo fetch) && (cd svc && go mod download)")
    );
}

/// A root manifest describes its whole ecosystem — a Cargo workspace, npm
/// workspaces, a Go module. Emitting a gate per member as well would run the
/// same suite twice under two names.
#[test]
fn a_root_manifest_shadows_the_same_ecosystem_below_it() {
    let out = compose_commands(&[
        site("Cargo.toml", ""),
        site("Cargo.toml", "crates"),
        site("Cargo.toml", "src-tauri"),
    ]);
    assert_eq!(out.validation_gates, vec!["rust-test"]);
    assert_eq!(
        out.harnesses.get("rust-test").map(String::as_str),
        Some("cargo test")
    );
}

/// Two independent packages with no root manifest each get their own gate, and
/// the names have to distinguish them or the map silently loses one.
#[test]
fn sibling_packages_of_one_ecosystem_get_distinct_gate_names() {
    let out = compose_commands(&[
        js_site("admin", r#""test":"vitest run""#, &[]),
        js_site("frontend", r#""test":"vitest run""#, &[]),
    ]);
    assert_eq!(
        out.validation_gates,
        vec!["js-test-admin", "js-test-frontend"]
    );
    assert_eq!(
        out.harnesses.get("js-test-frontend").map(String::as_str),
        Some("(cd frontend && npm test)")
    );
}

// ── The bounded scan's own rules ────────────────────────────────────────────

#[test]
fn the_directories_full_of_other_peoples_manifests_are_never_scanned() {
    assert!(!is_scannable_subdir("node_modules"));
    assert!(!is_scannable_subdir("target"));
    assert!(!is_scannable_subdir(".git"));
    assert!(!is_scannable_subdir(".venv"));
    assert!(is_scannable_subdir("src-tauri"));
    assert!(is_scannable_subdir("crates"));
}

#[test]
fn an_unrecognised_marker_contributes_nothing() {
    let out = compose_commands(&[site("Gemfile", "")]);
    assert_eq!(out, DetectedCommands::default());
}
