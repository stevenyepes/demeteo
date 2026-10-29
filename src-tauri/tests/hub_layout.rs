//! Guard over the shape of `crates/demeteo-hub`.
//!
//! The harness runs `cd src-tauri && cargo test`, which scopes cargo to the
//! `demeteo` package: the Hub crate is built only as a Linux dev-dependency
//! of this one, and none of its own tests run. A test inside the Hub crate
//! therefore can never be seen passing; this one lives here so that the
//! structural contract is checked by something the harness runs.
//! It proves file shape, not behaviour.
//!
//! Each detector is a pure function over `cargo metadata` JSON or file text,
//! and `detectors_fire_on_violations` feeds them synthetic violations, so a
//! detector that silently stopped matching fails a test instead of passing.

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;

const HUB: &str = "demeteo-hub";
const FORBIDDEN_DEPS: [&str; 1] = ["demeteo-core"];
const CORE: &str = "demeteo-core";
/// Matched by exact package name, so `demeteo-hub-protocol` — the wire types
/// both ends compile against — stays allowed.
const CORE_FORBIDDEN_DEPS: [&str; 1] = [HUB];
const FORBIDDEN_SQLX_FEATURES: [&str; 3] = ["sqlite", "mysql", "any"];

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("src-tauri has a parent directory")
        .to_path_buf()
}

fn hub_dir() -> PathBuf {
    repo_root().join("crates").join(HUB)
}

fn workspace_metadata() -> Value {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let out = Command::new(cargo)
        .args([
            "metadata",
            "--no-deps",
            "--offline",
            "--format-version",
            "1",
        ])
        .current_dir(repo_root())
        .output()
        .expect("failed to spawn `cargo metadata`");
    assert!(
        out.status.success(),
        "`cargo metadata --no-deps --offline` failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).expect("cargo metadata emitted invalid JSON")
}

fn package<'a>(metadata: &'a Value, name: &str) -> Option<&'a Value> {
    metadata["packages"]
        .as_array()?
        .iter()
        .find(|p| p["name"] == name)
}

fn hub_package(metadata: &Value) -> Option<&Value> {
    package(metadata, HUB)
}

fn is_workspace_member(metadata: &Value) -> bool {
    let Some(pkg) = hub_package(metadata) else {
        return false;
    };
    let id = &pkg["id"];
    metadata["workspace_members"]
        .as_array()
        .is_some_and(|members| members.iter().any(|m| m == id))
}

fn dependencies(pkg: &Value) -> &[Value] {
    pkg["dependencies"]
        .as_array()
        .map_or(&[], |deps| deps.as_slice())
}

/// `cargo metadata` reports `kind` as null for normal dependencies, so every
/// kind (normal, dev, build) is covered by not filtering on it.
/// `name` is the package name even under `package = "…"` renames, and
/// target-specific tables are listed alongside the rest.
fn forbidden_dependency_violations(pkg: &Value, owner: &str, forbidden: &[&str]) -> Vec<String> {
    dependencies(pkg)
        .iter()
        .filter_map(|d| {
            let name = d["name"].as_str()?;
            forbidden.contains(&name).then(|| {
                let kind = d["kind"].as_str().unwrap_or("normal");
                format!("{owner} has a {kind} dependency on `{name}`")
            })
        })
        .collect()
}

fn sqlx_violations(pkg: &Value) -> Vec<String> {
    let sqlx: Vec<&Value> = dependencies(pkg)
        .iter()
        .filter(|d| d["name"] == "sqlx")
        .collect();
    if sqlx.is_empty() {
        return vec![format!("{HUB} declares no `sqlx` dependency")];
    }
    let mut out = Vec::new();
    for dep in sqlx {
        let kind = dep["kind"].as_str().unwrap_or("normal");
        if dep["uses_default_features"] != Value::Bool(false) {
            out.push(format!("sqlx ({kind}) must set `default-features = false`"));
        }
        for feature in dep["features"].as_array().into_iter().flatten() {
            let Some(feature) = feature.as_str() else {
                continue;
            };
            if FORBIDDEN_SQLX_FEATURES.contains(&feature) {
                out.push(format!(
                    "sqlx ({kind}) enables forbidden feature `{feature}`"
                ));
            }
        }
    }
    out
}

/// Needles are assembled from fragments so this file can never match itself,
/// whatever directory it is moved to.
fn compile_time_query_needles() -> Vec<String> {
    ["query", "query_as", "query_scalar", "query_file"]
        .iter()
        .map(|name| format!("{name}{}(", '!'))
        .collect()
}

fn needle_violations(label: &str, text: &str, needles: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for (idx, line) in text.lines().enumerate() {
        for needle in needles {
            if line.contains(needle.as_str()) {
                out.push(format!("{label}:{}: contains `{needle}`", idx + 1));
            }
        }
    }
    out
}

fn collect_files(dir: &Path, into: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) => panic!("cannot read {}: {e}", dir.display()),
    };
    for entry in entries {
        let path = entry.expect("readable dir entry").path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n == "target") {
                continue;
            }
            collect_files(&path, into);
        } else {
            into.push(path);
        }
    }
}

#[test]
fn hub_is_a_workspace_member() {
    let metadata = workspace_metadata();
    assert!(
        is_workspace_member(&metadata),
        "`{HUB}` is not a workspace member: add `crates/{HUB}` to `[workspace] members` in Cargo.toml"
    );
}

#[test]
fn hub_does_not_depend_on_core() {
    let metadata = workspace_metadata();
    let pkg = hub_package(&metadata).unwrap_or_else(|| panic!("`{HUB}` package not found"));
    let violations = forbidden_dependency_violations(pkg, HUB, &FORBIDDEN_DEPS);
    assert!(
        violations.is_empty(),
        "crates/{HUB}/Cargo.toml:\n  - {}",
        violations.join("\n  - ")
    );
}

#[test]
fn core_does_not_depend_on_hub() {
    let metadata = workspace_metadata();
    let pkg = package(&metadata, CORE).unwrap_or_else(|| panic!("`{CORE}` package not found"));
    let violations = forbidden_dependency_violations(pkg, CORE, &CORE_FORBIDDEN_DEPS);
    assert!(
        violations.is_empty(),
        "crates/{CORE}/Cargo.toml:\n  - {}",
        violations.join("\n  - ")
    );
}

#[test]
fn hub_sqlx_is_postgres_only() {
    let metadata = workspace_metadata();
    let pkg = hub_package(&metadata).unwrap_or_else(|| panic!("`{HUB}` package not found"));
    let violations = sqlx_violations(pkg);
    assert!(
        violations.is_empty(),
        "crates/{HUB}/Cargo.toml:\n  - {}",
        violations.join("\n  - ")
    );
}

#[test]
fn hub_uses_no_compile_time_checked_queries() {
    let root = repo_root();
    let dir = hub_dir();
    assert!(dir.is_dir(), "{} does not exist", dir.display());
    let needles = compile_time_query_needles();

    let mut files = Vec::new();
    collect_files(&dir, &mut files);
    let mut violations = Vec::new();
    for path in files {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let label = path
            .strip_prefix(&root)
            .unwrap_or(&path)
            .display()
            .to_string();
        violations.extend(needle_violations(&label, &text, &needles));
    }
    assert!(
        violations.is_empty(),
        "compile-time-checked sqlx macros need DATABASE_URL at build time; use runtime queries:\n  - {}",
        violations.join("\n  - ")
    );
}

/// `domain/` holds no `async fn` (see `crates/demeteo-core/src/domain/mod.rs`);
/// the socket policy is checked here because it is the half of the desktop
/// client most tempting to fold back into the session loop.
const HUB_SOCKET_POLICY: &str = "crates/demeteo-core/src/domain/hub_socket";

fn async_fn_needles() -> Vec<String> {
    vec!["async fn".to_owned()]
}

#[test]
fn hub_socket_policy_has_no_async_fn() {
    let root = repo_root();
    let dir = root.join(HUB_SOCKET_POLICY);
    assert!(dir.is_dir(), "{} does not exist", dir.display());

    let mut files = Vec::new();
    collect_files(&dir, &mut files);
    assert!(!files.is_empty(), "{HUB_SOCKET_POLICY} holds no files");
    let mut violations = Vec::new();
    for path in files {
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
        let label = path
            .strip_prefix(&root)
            .unwrap_or(&path)
            .display()
            .to_string();
        violations.extend(needle_violations(&label, &text, &async_fn_needles()));
    }
    assert!(
        violations.is_empty(),
        "socket policy is synchronous; move the I/O to adapters/hub/socket:\n  - {}",
        violations.join("\n  - ")
    );
}

#[test]
fn detectors_fire_on_violations() {
    let member = serde_json::json!({
        "workspace_members": ["hub-id"],
        "packages": [{ "name": HUB, "id": "hub-id" }]
    });
    assert!(is_workspace_member(&member));
    let orphan = serde_json::json!({
        "workspace_members": ["other-id"],
        "packages": [{ "name": HUB, "id": "hub-id" }]
    });
    assert!(!is_workspace_member(&orphan));
    assert!(!is_workspace_member(&serde_json::json!({
        "workspace_members": [],
        "packages": []
    })));

    for (owner, forbidden) in [(HUB, FORBIDDEN_DEPS), (CORE, CORE_FORBIDDEN_DEPS)] {
        for kind in [Value::Null, "dev".into(), "build".into()] {
            for name in forbidden {
                let pkg = serde_json::json!({ "dependencies": [{ "name": name, "kind": kind }] });
                let hits = forbidden_dependency_violations(&pkg, owner, &forbidden);
                assert_eq!(hits.len(), 1, "{owner} -> {name}");
            }
        }
    }
    let clean = serde_json::json!({ "dependencies": [{ "name": "serde", "kind": null }] });
    assert!(forbidden_dependency_violations(&clean, HUB, &FORBIDDEN_DEPS).is_empty());
    let protocol_only = serde_json::json!({ "dependencies": [
        { "name": "demeteo-hub-protocol", "kind": null }
    ] });
    assert!(forbidden_dependency_violations(&protocol_only, CORE, &CORE_FORBIDDEN_DEPS).is_empty());

    let good = serde_json::json!({ "dependencies": [{
        "name": "sqlx", "kind": null, "uses_default_features": false,
        "features": ["postgres", "runtime-tokio"]
    }] });
    assert!(sqlx_violations(&good).is_empty());
    let defaults_on = serde_json::json!({ "dependencies": [{
        "name": "sqlx", "kind": null, "uses_default_features": true, "features": []
    }] });
    assert_eq!(sqlx_violations(&defaults_on).len(), 1);
    for feature in FORBIDDEN_SQLX_FEATURES {
        let pkg = serde_json::json!({ "dependencies": [{
            "name": "sqlx", "kind": null, "uses_default_features": false,
            "features": ["postgres", feature]
        }] });
        assert_eq!(sqlx_violations(&pkg).len(), 1, "{feature}");
    }
    assert_eq!(sqlx_violations(&clean).len(), 1);

    let needles = compile_time_query_needles();
    assert_eq!(needles.len(), 4);
    for needle in &needles {
        let text = format!("fn ok() {{}}\nlet q = sqlx::{needle}\"select 1\");");
        let hits = needle_violations("src/store.rs", &text, &needles);
        assert_eq!(hits.len(), 1, "{needle}");
        assert!(hits[0].starts_with("src/store.rs:2:"), "{}", hits[0]);
    }
    assert!(needle_violations("x", "sqlx::query(\"select 1\")", &needles).is_empty());

    let async_fn = async_fn_needles();
    for text in [
        "async fn dial() {}",
        "pub async fn run(&self) {}",
        "    pub(crate) async fn x()",
    ] {
        assert_eq!(
            needle_violations("projection.rs", text, &async_fn).len(),
            1,
            "{text}"
        );
    }
    assert!(needle_violations("projection.rs", "pub fn hub_event() {}", &async_fn).is_empty());
}

/// `cargo tree -i libsqlite3-sys` hides inactive optional dependencies and
/// lists only rusqlite, so the coupling is visible only in Cargo.lock.
const SQLITE_LOCKSTEP_RULE: &str = "sqlx-sqlite is resolved whatever sqlx's features are, and its \
     libsqlite3-sys must equal rusqlite's: bump sqlx in crates/demeteo-hub/Cargo.toml and \
     rusqlite in crates/demeteo-core/Cargo.toml together";

/// `(name, dependency names)` per `[[package]]` block; a dependency entry is
/// `"name"` or `"name version"` when the lock holds several versions.
fn lock_packages(lock: &str) -> Vec<(String, Vec<String>)> {
    let mut packages: Vec<(String, Vec<String>)> = Vec::new();
    let mut in_deps = false;
    for line in lock.lines() {
        let line = line.trim();
        if line == "[[package]]" {
            packages.push((String::new(), Vec::new()));
            in_deps = false;
        } else if let Some(name) = line.strip_prefix("name = ") {
            if let Some(last) = packages.last_mut() {
                last.0 = name.trim_matches('"').to_string();
            }
        } else if line == "dependencies = [" {
            in_deps = true;
        } else if line == "]" {
            in_deps = false;
        } else if in_deps {
            if let (Some(last), Some(dep)) = (
                packages.last_mut(),
                line.trim_matches(|c| c == '"' || c == ',')
                    .split(' ')
                    .next(),
            ) {
                last.1.push(dep.to_string());
            }
        }
    }
    packages
}

fn sqlite_lockstep_violations(lock: &str) -> Vec<String> {
    let packages = lock_packages(lock);
    let mut out = Vec::new();
    let copies = packages
        .iter()
        .filter(|(name, _)| name == "libsqlite3-sys")
        .count();
    if copies != 1 {
        out.push(format!(
            "Cargo.lock holds {copies} `libsqlite3-sys` packages, expected exactly 1"
        ));
    }
    for dependent in ["sqlx-sqlite", "rusqlite"] {
        let links = packages
            .iter()
            .any(|(name, deps)| name == dependent && deps.iter().any(|d| d == "libsqlite3-sys"));
        if !links {
            out.push(format!(
                "no `{dependent}` in Cargo.lock depends on `libsqlite3-sys`"
            ));
        }
    }
    out
}

#[test]
fn sqlx_and_rusqlite_share_one_libsqlite3_sys() {
    let path = repo_root().join("Cargo.lock");
    let lock = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    let violations = sqlite_lockstep_violations(&lock);
    assert!(
        violations.is_empty(),
        "{SQLITE_LOCKSTEP_RULE}:\n  - {}",
        violations.join("\n  - ")
    );
}

#[test]
fn sqlite_lockstep_detector_fires_on_violations() {
    let sys = "[[package]]\nname = \"libsqlite3-sys\"\nversion = \"0.37.0\"\ndependencies = [\n \"cc\",\n]\n";
    let dependent = |name: &str, dep: &str| {
        format!("[[package]]\nname = \"{name}\"\nversion = \"1.0.0\"\ndependencies = [\n \"atoi\",\n \"{dep}\",\n]\n")
    };
    let good = format!(
        "{sys}\n{}\n{}",
        dependent("sqlx-sqlite", "libsqlite3-sys"),
        dependent("rusqlite", "libsqlite3-sys 0.37.0")
    );
    assert!(sqlite_lockstep_violations(&good).is_empty());
    let two_copies = format!("{good}\n{}", sys.replace("0.37.0", "0.38.0"));
    assert_eq!(sqlite_lockstep_violations(&two_copies).len(), 1);
    let no_sqlx = format!(
        "{sys}\n{}\n{}",
        dependent("sqlx-sqlite", "log"),
        dependent("rusqlite", "libsqlite3-sys")
    );
    assert_eq!(sqlite_lockstep_violations(&no_sqlx).len(), 1);
    assert_eq!(sqlite_lockstep_violations("").len(), 3);
}

/// The desktop's Hub socket client: the crates only it uses, the default
/// feature that turns them on, and the one core file allowed to name the
/// WebSocket crate (`crates/demeteo-core/src/adapters/hub/socket/ws_transport.rs`).
/// `demeteo-runner` links core with default features off and must keep them
/// off: it never pairs, and its musl binary should not carry a TLS client.
const HUB_SOCKET_FEATURE: &str = "hub-socket";
const HUB_SOCKET_DEPS: [&str; 3] = ["tokio-tungstenite", "rustls", "rustls-platform-verifier"];
const RUNNER: &str = "demeteo-runner";
const CORE_SRC: &str = "crates/demeteo-core/src";

fn ws_crate_needles() -> Vec<String> {
    vec![format!("tokio{}tungstenite", '_')]
}

/// The labels of the `.rs` files among `files` that name the WebSocket crate.
fn ws_crate_files(files: &[(String, String)]) -> Vec<String> {
    let needles = ws_crate_needles();
    files
        .iter()
        .filter(|(label, text)| {
            label.ends_with(".rs") && !needle_violations(label, text, &needles).is_empty()
        })
        .map(|(label, _)| label.clone())
        .collect()
}

fn feature_list<'a>(pkg: &'a Value, feature: &str) -> Vec<&'a str> {
    pkg["features"][feature]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect()
}

fn hub_socket_dep_violations(core: &Value) -> Vec<String> {
    let mut out = Vec::new();
    let defaults = feature_list(core, "default");
    if !defaults.contains(&HUB_SOCKET_FEATURE) {
        out.push(format!(
            "`{HUB_SOCKET_FEATURE}` is not in {CORE}'s `default` features"
        ));
    }
    let enabled = feature_list(core, HUB_SOCKET_FEATURE);
    for name in HUB_SOCKET_DEPS {
        let entries: Vec<&Value> = dependencies(core)
            .iter()
            .filter(|d| d["name"] == name)
            .collect();
        if entries.is_empty() {
            out.push(format!("{CORE} declares no `{name}` dependency"));
        }
        if entries.iter().any(|d| d["optional"] != Value::Bool(true)) {
            out.push(format!("{CORE}'s `{name}` must be `optional = true`"));
        }
        if !enabled.contains(&format!("dep:{name}").as_str()) {
            out.push(format!(
                "`{HUB_SOCKET_FEATURE}` does not enable `dep:{name}`"
            ));
        }
    }
    out
}

/// Every route by which the runner could turn the feature on: core's
/// defaults, the feature named on the dependency, or a runner feature that
/// forwards it.
fn runner_hub_socket_violations(runner: &Value) -> Vec<String> {
    let mut out = Vec::new();
    for dep in dependencies(runner).iter().filter(|d| d["name"] == CORE) {
        let kind = dep["kind"].as_str().unwrap_or("normal");
        if dep["uses_default_features"] != Value::Bool(false) {
            out.push(format!(
                "{RUNNER}'s {kind} `{CORE}` dependency must set `default-features = false`"
            ));
        }
        let features = dep["features"].as_array().into_iter().flatten();
        if features
            .filter_map(Value::as_str)
            .any(|f| f == HUB_SOCKET_FEATURE)
        {
            out.push(format!(
                "{RUNNER}'s {kind} `{CORE}` dependency enables `{HUB_SOCKET_FEATURE}`"
            ));
        }
    }
    let forwards = [
        format!("{CORE}/{HUB_SOCKET_FEATURE}"),
        format!("{CORE}?/{HUB_SOCKET_FEATURE}"),
    ];
    if let Some(features) = runner["features"].as_object() {
        for (name, enables) in features {
            let enables = enables.as_array().into_iter().flatten();
            if enables
                .filter_map(Value::as_str)
                .any(|f| forwards.iter().any(|fw| fw == f))
            {
                out.push(format!(
                    "{RUNNER}'s `{name}` feature enables `{CORE}/{HUB_SOCKET_FEATURE}`"
                ));
            }
        }
    }
    out
}

#[test]
fn only_ws_transport_names_the_websocket_crate() {
    let root = repo_root();
    let mut paths = Vec::new();
    collect_files(&root.join(CORE_SRC), &mut paths);
    let files: Vec<(String, String)> = paths
        .into_iter()
        .filter_map(|path| {
            let text = std::fs::read_to_string(&path).ok()?;
            let label = path
                .strip_prefix(&root)
                .unwrap_or(&path)
                .display()
                .to_string();
            Some((label.replace('\\', "/"), text))
        })
        .collect();
    let naming = ws_crate_files(&files);
    assert_eq!(
        naming,
        vec![format!("{CORE_SRC}/adapters/hub/socket/ws_transport.rs")],
        "the WebSocket crate belongs to ws_transport.rs alone"
    );
}

#[test]
fn hub_socket_deps_are_optional_and_on_by_default() {
    let metadata = workspace_metadata();
    let core = package(&metadata, CORE).unwrap_or_else(|| panic!("`{CORE}` package not found"));
    let violations = hub_socket_dep_violations(core);
    assert!(
        violations.is_empty(),
        "crates/{CORE}/Cargo.toml:\n  - {}",
        violations.join("\n  - ")
    );
}

#[test]
fn the_runner_never_enables_hub_socket() {
    let metadata = workspace_metadata();
    let runner =
        package(&metadata, RUNNER).unwrap_or_else(|| panic!("`{RUNNER}` package not found"));
    let violations = runner_hub_socket_violations(runner);
    assert!(
        violations.is_empty(),
        "crates/{RUNNER}/Cargo.toml:\n  - {}",
        violations.join("\n  - ")
    );
}

#[test]
fn hub_socket_detectors_fire_on_violations() {
    let file = |label: &str, text: &str| (label.to_owned(), text.to_owned());
    let named = format!("use {}::Connector;", ws_crate_needles()[0]);
    let files = [
        file("a/ws_transport.rs", &named),
        file(
            "a/session.rs",
            "use crate::ports::hub::HubSocketConnection;",
        ),
        file("a/notes.md", &named),
    ];
    assert_eq!(ws_crate_files(&files), vec!["a/ws_transport.rs".to_owned()]);
    let leaked = [file("a/ws_transport.rs", &named), file("a/mod.rs", &named)];
    assert_eq!(ws_crate_files(&leaked).len(), 2);

    let dep =
        |name: &str, optional: bool| serde_json::json!({ "name": name, "optional": optional });
    let core = |deps: Vec<Value>, default: Vec<&str>, socket: Vec<&str>| {
        serde_json::json!({
            "dependencies": deps,
            "features": { "default": default, HUB_SOCKET_FEATURE: socket },
        })
    };
    let all_deps = || {
        HUB_SOCKET_DEPS
            .iter()
            .map(|n| dep(n, true))
            .collect::<Vec<_>>()
    };
    let enabling: Vec<String> = HUB_SOCKET_DEPS.iter().map(|n| format!("dep:{n}")).collect();
    let enabling: Vec<&str> = enabling.iter().map(String::as_str).collect();
    let good = core(
        all_deps(),
        vec!["keyring", HUB_SOCKET_FEATURE],
        enabling.clone(),
    );
    assert!(hub_socket_dep_violations(&good).is_empty());
    let off_by_default = core(all_deps(), vec!["keyring"], enabling.clone());
    assert_eq!(hub_socket_dep_violations(&off_by_default).len(), 1);
    for name in HUB_SOCKET_DEPS {
        let required = HUB_SOCKET_DEPS.iter().map(|n| dep(n, *n != name)).collect();
        let pkg = core(required, vec![HUB_SOCKET_FEATURE], enabling.clone());
        assert_eq!(hub_socket_dep_violations(&pkg).len(), 1, "{name} required");
        let unenabled = enabling
            .iter()
            .copied()
            .filter(|e| *e != format!("dep:{name}"));
        let pkg = core(all_deps(), vec![HUB_SOCKET_FEATURE], unenabled.collect());
        assert_eq!(hub_socket_dep_violations(&pkg).len(), 1, "{name} unenabled");
        let missing = HUB_SOCKET_DEPS
            .iter()
            .filter(|n| **n != name)
            .map(|n| dep(n, true));
        let pkg = core(
            missing.collect(),
            vec![HUB_SOCKET_FEATURE],
            enabling.clone(),
        );
        assert_eq!(hub_socket_dep_violations(&pkg).len(), 1, "{name} missing");
    }

    let runner = |uses_default: bool, features: Vec<&str>, own: Value| {
        serde_json::json!({
            "dependencies": [{
                "name": CORE, "kind": null,
                "uses_default_features": uses_default, "features": features,
            }],
            "features": own,
        })
    };
    let clean = runner(false, vec!["vendored-openssl"], serde_json::json!({}));
    assert!(runner_hub_socket_violations(&clean).is_empty());
    let defaults = runner(true, vec![], serde_json::json!({}));
    assert_eq!(runner_hub_socket_violations(&defaults).len(), 1);
    let named = runner(false, vec![HUB_SOCKET_FEATURE], serde_json::json!({}));
    assert_eq!(runner_hub_socket_violations(&named).len(), 1);
    for forward in [
        format!("{CORE}/{HUB_SOCKET_FEATURE}"),
        format!("{CORE}?/{HUB_SOCKET_FEATURE}"),
    ] {
        let own = serde_json::json!({ "desktop": [forward] });
        assert_eq!(
            runner_hub_socket_violations(&runner(false, vec![], own)).len(),
            1
        );
    }
}

const EXPECTED_TABLES: [&str; 12] = [
    "users",
    "passkeys",
    "instances",
    "device_keys",
    "tokens",
    "instance_requests",
    "run_mirror",
    "snapshots",
    "attachments",
    "sessions",
    "passkey_endorsements",
    "passkey_revocations",
];
const FORBIDDEN_TOKEN_COLUMNS: [&str; 4] = ["token", "secret", "plaintext", "private_key"];
const FORBIDDEN_ATTACHMENT_COLUMNS: [&str; 3] = ["data_key", "plaintext", "key"];
/// The CSRF token is derived from the session secret and never stored, so a
/// column for it means that derivation was replaced by a second stored secret.
const FORBIDDEN_SESSION_COLUMNS: [&str; 6] = [
    "token",
    "secret",
    "plaintext",
    "cookie",
    "csrf",
    "csrf_token",
];
/// `(column, type)` for every challenge input and assertion field an instance
/// re-verifies. `expires_at` is the integer that was hashed into the challenge,
/// which a timestamp type would not hand back unchanged; the assertion columns
/// are the base64url text as it arrived, for the same reason.
const ENDORSEMENT_RECORD_COLUMNS: [(&str, &str); 9] = [
    ("instance_id", "TEXT"),
    ("request_id", "TEXT"),
    ("expires_at", "BIGINT"),
    ("new_credential_id", "TEXT"),
    ("new_public_key", "TEXT"),
    ("assertion_credential_id", "TEXT"),
    ("authenticator_data", "TEXT"),
    ("client_data_json", "TEXT"),
    ("signature", "TEXT"),
];
const REVOCATION_RECORD_COLUMNS: [(&str, &str); 8] = [
    ("instance_id", "TEXT"),
    ("request_id", "TEXT"),
    ("expires_at", "BIGINT"),
    ("credential_id", "TEXT"),
    ("assertion_credential_id", "TEXT"),
    ("authenticator_data", "TEXT"),
    ("client_data_json", "TEXT"),
    ("signature", "TEXT"),
];
const INSTANCE_CHILDREN: [&str; 5] = [
    "device_keys",
    "instance_requests",
    "run_mirror",
    "snapshots",
    "attachments",
];
const TABLE_CONSTRAINT_KEYWORDS: [&str; 5] =
    ["PRIMARY", "FOREIGN", "UNIQUE", "CONSTRAINT", "CHECK"];
/// The tables a frame from an instance is written to.
const INSTANCE_FED_TABLES: [&str; 3] = ["run_mirror", "snapshots", "instance_requests"];
/// Transcripts, diffs and local paths never leave an instance (AGENTS.md §2),
/// and the protocol types carry no cost and no gate summary, so a column named
/// for any of them is storage for something no frame may hold. Matched as
/// substrings of the lowercased column name; `summar` is the stem `summary`
/// and `summaries` share.
const CONTENT_STEMS: [&str; 5] = ["transcript", "diff", "path", "cost", "summar"];
/// Every column `0006_instance_socket.sql` adds. The content-column check
/// passes on a file its parser read nothing from, so it also has to find these.
const SOCKET_COLUMNS: [(&str, &str); 9] = [
    ("instances", "install_id"),
    ("instances", "connection_id"),
    ("instances", "connected_at"),
    ("instances", "event_cursor"),
    ("instance_requests", "deliveries"),
    ("instance_requests", "last_sent_at"),
    ("instance_requests", "outcome"),
    ("instance_requests", "refusal_reason"),
    ("instance_requests", "answered_at"),
];

struct Table {
    name: String,
    columns: Vec<(String, String)>,
    constraints: Vec<String>,
}

impl Table {
    fn column(&self, name: &str) -> Option<&str> {
        self.columns
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, def)| def.as_str())
    }
}

/// Relies on the migration layout the spec fixes: `CREATE TABLE <name> (`
/// alone on its line, one column or constraint per line, `);` closing it.
fn parse_tables(sql: &str) -> Vec<Table> {
    let mut tables = Vec::new();
    let mut current: Option<Table> = None;
    for raw in sql.lines() {
        let line = raw.split("--").next().unwrap_or("").trim();
        if let Some(table) = current.as_mut() {
            if line.starts_with(')') {
                tables.extend(current.take());
            } else if !line.is_empty() {
                let line = line.trim_end_matches(',');
                let (head, rest) = line.split_once(' ').unwrap_or((line, ""));
                if TABLE_CONSTRAINT_KEYWORDS.contains(&head.to_ascii_uppercase().as_str()) {
                    table.constraints.push(line.to_string());
                } else {
                    table.columns.push((head.to_string(), rest.to_string()));
                }
            }
        } else if let Some(rest) = line.strip_prefix("CREATE TABLE ") {
            if let Some(name) = rest.strip_suffix('(') {
                current = Some(Table {
                    name: name.trim().to_string(),
                    columns: Vec::new(),
                    constraints: Vec::new(),
                });
            }
        }
    }
    tables
}

fn migration_files() -> Vec<PathBuf> {
    let dir = hub_dir().join("migrations");
    let entries =
        std::fs::read_dir(&dir).unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()));
    let mut files: Vec<PathBuf> = entries
        .map(|e| e.expect("readable dir entry").path())
        .collect();
    files.sort();
    files
}

fn all_tables() -> Vec<Table> {
    migration_files()
        .iter()
        .flat_map(|p| {
            let sql = std::fs::read_to_string(p)
                .unwrap_or_else(|e| panic!("cannot read {}: {e}", p.display()));
            parse_tables(&sql)
        })
        .collect()
}

fn table_set_violations(tables: &[Table]) -> Vec<String> {
    let mut found: Vec<&str> = tables.iter().map(|t| t.name.as_str()).collect();
    found.sort_unstable();
    let mut expected = EXPECTED_TABLES.to_vec();
    expected.sort_unstable();
    if found == expected {
        return Vec::new();
    }
    let missing: Vec<&str> = expected
        .iter()
        .copied()
        .filter(|n| !found.contains(n))
        .collect();
    let extra: Vec<&str> = found
        .iter()
        .copied()
        .filter(|n| !expected.contains(n))
        .collect();
    vec![format!(
        "table set differs from the expected one: missing {missing:?}, unexpected {extra:?}"
    )]
}

fn tenancy_violations(tables: &[Table]) -> Vec<String> {
    tables
        .iter()
        .filter(|t| t.name != "users")
        .filter_map(|t| match t.column("user_id") {
            None => Some(format!("{} has no user_id column", t.name)),
            Some(def) if !def.contains("NOT NULL") => {
                Some(format!("{}.user_id is not NOT NULL", t.name))
            }
            Some(_) => None,
        })
        .collect()
}

fn secret_column_violations(tables: &[Table]) -> Vec<String> {
    let mut out = Vec::new();
    for (table, required, forbidden) in [
        ("tokens", "token_hash", &FORBIDDEN_TOKEN_COLUMNS[..]),
        (
            "attachments",
            "wrapped_key",
            &FORBIDDEN_ATTACHMENT_COLUMNS[..],
        ),
        ("sessions", "token_hash", &FORBIDDEN_SESSION_COLUMNS[..]),
    ] {
        let Some(t) = tables.iter().find(|t| t.name == table) else {
            out.push(format!("{table} is missing"));
            continue;
        };
        if t.column(required).is_none() {
            out.push(format!("{table} has no {required} column"));
        }
        for name in forbidden {
            if t.column(name).is_some() {
                out.push(format!("{table} has forbidden column `{name}`"));
            }
        }
    }
    out
}

/// A record names passkeys by credential id value, never by a foreign key to
/// `passkeys`: it must be verifiable from its own row alone, and a cascade
/// from that table must not be able to delete it.
fn chain_record_violations(tables: &[Table]) -> Vec<String> {
    let mut out = Vec::new();
    for (name, required) in [
        ("passkey_endorsements", &ENDORSEMENT_RECORD_COLUMNS[..]),
        ("passkey_revocations", &REVOCATION_RECORD_COLUMNS[..]),
    ] {
        let Some(t) = tables.iter().find(|t| t.name == name) else {
            out.push(format!("{name} is missing"));
            continue;
        };
        for (column, ty) in required {
            match t.column(column) {
                None => out.push(format!("{name} has no {column} column")),
                Some(def) if def.split_whitespace().next() != Some(*ty) => {
                    out.push(format!("{name}.{column} is not {ty}"));
                }
                Some(_) => {}
            }
        }
        for (column, def) in &t.columns {
            if !def.contains("NOT NULL") && !def.contains("PRIMARY KEY") {
                out.push(format!("{name}.{column} is nullable"));
            }
        }
        let references_passkeys = t
            .columns
            .iter()
            .map(|(_, def)| def)
            .chain(&t.constraints)
            .any(|text| text.contains("REFERENCES passkeys"));
        if references_passkeys {
            out.push(format!("{name} has a foreign key to passkeys"));
        }
    }
    out
}

fn composite_fk_violations(tables: &[Table]) -> Vec<String> {
    let needle = "FOREIGN KEY (user_id, instance_id) REFERENCES instances (user_id, id)";
    let mut out = Vec::new();
    for name in INSTANCE_CHILDREN {
        let ok = tables
            .iter()
            .find(|t| t.name == name)
            .is_some_and(|t| t.constraints.iter().any(|c| c.starts_with(needle)));
        if !ok {
            out.push(format!(
                "{name} lacks a composite FK to instances (user_id, id)"
            ));
        }
    }
    let instances_unique = tables
        .iter()
        .find(|t| t.name == "instances")
        .is_some_and(|t| t.constraints.iter().any(|c| c == "UNIQUE (user_id, id)"));
    if !instances_unique {
        out.push("instances lacks UNIQUE (user_id, id)".to_string());
    }
    out
}

fn run_mirror_violations(tables: &[Table]) -> Vec<String> {
    let Some(t) = tables.iter().find(|t| t.name == "run_mirror") else {
        return vec!["run_mirror is missing".to_string()];
    };
    let mut out = Vec::new();
    if t.column("event_offset").is_none() {
        out.push("run_mirror has no event_offset column".to_string());
    }
    if t.column("offset").is_some() {
        out.push("run_mirror has a column named the reserved word `offset`".to_string());
    }
    out
}

/// Splits on `sep` outside parentheses and single-quoted literals, so the
/// commas of `CHECK (x IN ('a', 'b'))` do not end an `ALTER TABLE` action.
fn split_top_level(text: &str, sep: char) -> Vec<&str> {
    let mut parts = Vec::new();
    let (mut depth, mut quoted, mut start) = (0usize, false, 0);
    for (i, c) in text.char_indices() {
        match c {
            '\'' => quoted = !quoted,
            '(' if !quoted => depth += 1,
            ')' if !quoted => depth = depth.saturating_sub(1),
            c if c == sep && !quoted && depth == 0 => {
                parts.push(&text[start..i]);
                start = i + c.len_utf8();
            }
            _ => {}
        }
    }
    parts.push(&text[start..]);
    parts
}

fn strip_keywords<'a, 'b>(words: &'a [&'b str], keywords: &[&str]) -> Option<&'a [&'b str]> {
    let (head, rest) = words.split_at_checked(keywords.len())?;
    head.iter()
        .zip(keywords)
        .all(|(word, keyword)| word.eq_ignore_ascii_case(keyword))
        .then_some(rest)
}

/// Postgres lets `COLUMN` be left out, so `ADD notes TEXT` adds a column too;
/// without the keyword only a table-constraint word says it does not.
fn added_column<'a>(action: &[&'a str]) -> Option<&'a str> {
    let rest = strip_keywords(action, &["ADD"])?;
    let explicit = strip_keywords(rest, &["COLUMN"]);
    let rest = explicit.unwrap_or(rest);
    let rest = strip_keywords(rest, &["IF", "NOT", "EXISTS"]).unwrap_or(rest);
    let name = *rest.first()?;
    let is_constraint = TABLE_CONSTRAINT_KEYWORDS.contains(&name.to_ascii_uppercase().as_str());
    (explicit.is_some() || !is_constraint).then_some(name)
}

/// `(table, column)` for every column an `ALTER TABLE` statement adds, which
/// `parse_tables` never sees. Statement-wise, not line-wise like that parser:
/// one `ALTER TABLE` may sit on a single line or carry an action per line.
/// Comments are cut first because an apostrophe in one would open a literal
/// that never closes.
fn added_columns(sql: &str) -> Vec<(String, String)> {
    let text: Vec<&str> = sql
        .lines()
        .map(|line| line.split("--").next().unwrap_or(""))
        .collect();
    let text = text.join("\n");
    let mut out = Vec::new();
    for statement in split_top_level(&text, ';') {
        let mut actions = split_top_level(statement, ',')
            .into_iter()
            .map(|action| action.split_whitespace().collect::<Vec<&str>>());
        let Some(first) = actions.next() else {
            continue;
        };
        let Some(rest) = strip_keywords(&first, &["ALTER", "TABLE"]) else {
            continue;
        };
        let rest = strip_keywords(rest, &["IF", "EXISTS"]).unwrap_or(rest);
        let rest = strip_keywords(rest, &["ONLY"]).unwrap_or(rest);
        let Some((table, first_action)) = rest.split_first() else {
            continue;
        };
        let added = added_column(first_action)
            .into_iter()
            .chain(actions.filter_map(|action| added_column(&action)))
            .map(|column| (table.to_string(), column.to_string()));
        out.extend(added);
    }
    out
}

/// Takes one migration's text because half of what it reads is not a `Table`:
/// the columns `INSTANCE_FED_TABLES` are created with, and every column any
/// `ALTER TABLE` adds to any table.
fn content_column_violations(sql: &str) -> Vec<String> {
    let created = parse_tables(sql)
        .into_iter()
        .filter(|t| INSTANCE_FED_TABLES.contains(&t.name.as_str()))
        .flat_map(|t| {
            let Table { name, columns, .. } = t;
            columns
                .into_iter()
                .map(move |(column, _)| (name.clone(), column))
        });
    created
        .chain(added_columns(sql))
        .filter_map(|(table, column)| {
            let lowered = column.to_ascii_lowercase();
            let stem = CONTENT_STEMS.iter().find(|stem| lowered.contains(**stem))?;
            Some(format!("{table}.{column} is `{stem}`-shaped"))
        })
        .collect()
}

/// `^[0-9]{4}_[a-z0-9_]+\.sql$`, hand-checked so the guard needs no regex dep.
fn is_migration_filename(name: &str) -> bool {
    let Some(stem) = name.strip_suffix(".sql") else {
        return false;
    };
    let Some((version, desc)) = stem.split_once('_') else {
        return false;
    };
    version.len() == 4
        && version.bytes().all(|b| b.is_ascii_digit())
        && !desc.is_empty()
        && desc
            .bytes()
            .all(|b| b.is_ascii_digit() || b.is_ascii_lowercase() || b == b'_')
}

#[test]
fn migrations_create_exactly_the_expected_tables() {
    let violations = table_set_violations(&all_tables());
    assert!(violations.is_empty(), "{}", violations.join("\n"));
}

/// Every statement of `sql` with its comments dropped and its whitespace
/// collapsed to single spaces, so that a statement is matched whatever its
/// line breaks.
fn statements(sql: &str) -> Vec<String> {
    let code: Vec<&str> = sql
        .lines()
        .map(|line| line.split("--").next().unwrap_or(""))
        .collect();
    code.join(" ")
        .split(';')
        .map(|statement| statement.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|statement| !statement.is_empty())
        .collect()
}

/// The queue is read after every `hello` and on every wake-up, and its rows
/// are never deleted. The predicate is the literal `list_queued_requests`
/// spells, which is what lets Postgres use the index at all.
#[test]
fn the_request_queue_read_has_an_index() {
    let serving = |statement: &str| {
        statement.starts_with("CREATE INDEX ")
            && statement.ends_with(
                " ON instance_requests (user_id, instance_id, queue_seq) WHERE status = 'queued'",
            )
    };
    let found: Vec<String> = migration_files()
        .iter()
        .flat_map(|path| {
            let sql = std::fs::read_to_string(path)
                .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
            statements(&sql)
        })
        .filter(|statement| serving(statement))
        .collect();
    assert_eq!(found.len(), 1, "{found:?}");
}

#[test]
fn every_table_but_users_has_user_id_not_null() {
    let violations = tenancy_violations(&all_tables());
    assert!(
        violations.is_empty(),
        "tenancy column violations:\n  - {}",
        violations.join("\n  - ")
    );
}

#[test]
fn secret_bearing_columns_are_hashed_or_wrapped() {
    let violations = secret_column_violations(&all_tables());
    assert!(
        violations.is_empty(),
        "plaintext secret columns:\n  - {}",
        violations.join("\n  - ")
    );
}

#[test]
fn chain_records_keep_every_field_an_instance_reverifies() {
    let violations = chain_record_violations(&all_tables());
    assert!(
        violations.is_empty(),
        "endorsement and revocation record violations:\n  - {}",
        violations.join("\n  - ")
    );
}

#[test]
fn child_tables_have_composite_fk_to_instances() {
    let violations = composite_fk_violations(&all_tables());
    assert!(
        violations.is_empty(),
        "tenant-FK violations:\n  - {}",
        violations.join("\n  - ")
    );
}

#[test]
fn run_mirror_avoids_the_reserved_offset_column() {
    let violations = run_mirror_violations(&all_tables());
    assert!(violations.is_empty(), "{}", violations.join("\n"));
}

#[test]
fn no_column_is_shaped_to_hold_instance_content() {
    let mut added = Vec::new();
    let mut violations = Vec::new();
    for path in migration_files() {
        let sql = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
        let file = path.file_name().unwrap_or_default().to_string_lossy();
        added.extend(added_columns(&sql));
        violations.extend(
            content_column_violations(&sql)
                .into_iter()
                .map(|v| format!("{file}: {v}")),
        );
    }
    assert!(
        violations.is_empty(),
        "the Hub stores what the protocol types carry, never a transcript, diff, path, cost or summary:\n  - {}",
        violations.join("\n  - ")
    );
    for (table, column) in SOCKET_COLUMNS {
        assert!(
            added.contains(&(table.to_string(), column.to_string())),
            "no `ALTER TABLE {table} … ADD COLUMN {column}` was read from the migrations: {added:?}"
        );
    }
}

#[test]
fn migration_filenames_match_sqlx_pattern() {
    let files = migration_files();
    assert!(!files.is_empty(), "crates/{HUB}/migrations is empty");
    let bad: Vec<String> = files
        .iter()
        .filter_map(|p| p.file_name()?.to_str().map(str::to_string))
        .filter(|n| !is_migration_filename(n))
        .collect();
    assert!(
        bad.is_empty(),
        "migration files must match ^[0-9]{{4}}_[a-z0-9_]+\\.sql$: {bad:?}"
    );
}

#[test]
fn build_script_reruns_when_migrations_change() {
    let path = hub_dir().join("build.rs");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    assert!(
        text.contains("cargo:rerun-if-changed=migrations"),
        "{} must emit `cargo:rerun-if-changed=migrations` so `sqlx::migrate!` re-embeds new files",
        path.display()
    );
}

#[test]
fn schema_detectors_fire_on_violations() {
    let good = "CREATE TABLE users (\n  id UUID PRIMARY KEY\n);\n\
                CREATE TABLE tokens (\n  id UUID PRIMARY KEY,\n  user_id UUID NOT NULL, -- c\n  token_hash BYTEA NOT NULL\n);\n";
    let tables = parse_tables(good);
    assert_eq!(tables.len(), 2);
    assert_eq!(tables[1].columns.len(), 3);
    assert!(tenancy_violations(&tables).is_empty());
    assert_eq!(table_set_violations(&tables).len(), 1);

    let nullable = parse_tables("CREATE TABLE tokens (\n  user_id UUID,\n  token_hash BYTEA\n);\n");
    assert_eq!(tenancy_violations(&nullable).len(), 1);
    let absent = parse_tables("CREATE TABLE tokens (\n  id UUID\n);\n");
    assert_eq!(tenancy_violations(&absent).len(), 1);

    let mut exact = EXPECTED_TABLES
        .map(|n| format!("CREATE TABLE {n} (\n  id UUID\n);\n"))
        .join("");
    assert!(table_set_violations(&parse_tables(&exact)).is_empty());
    let one_short = exact.replace("CREATE TABLE sessions (\n  id UUID\n);\n", "");
    assert_eq!(table_set_violations(&parse_tables(&one_short)).len(), 1);
    exact.push_str("CREATE TABLE extra (\n  id UUID\n);\n");
    assert_eq!(table_set_violations(&parse_tables(&exact)).len(), 1);

    let secrets = "CREATE TABLE tokens (\n  token_hash BYTEA\n);\n\
                   CREATE TABLE attachments (\n  wrapped_key TEXT\n);\n\
                   CREATE TABLE sessions (\n  token_hash BYTEA\n);\n";
    assert!(secret_column_violations(&parse_tables(secrets)).is_empty());
    for (table, required, forbidden) in [
        ("tokens", "token_hash BYTEA", &FORBIDDEN_TOKEN_COLUMNS[..]),
        (
            "attachments",
            "wrapped_key TEXT",
            &FORBIDDEN_ATTACHMENT_COLUMNS[..],
        ),
        (
            "sessions",
            "token_hash BYTEA",
            &FORBIDDEN_SESSION_COLUMNS[..],
        ),
    ] {
        let good = format!("CREATE TABLE {table} (\n  {required}\n);\n");
        for bad in forbidden {
            let plaintext = format!("CREATE TABLE {table} (\n  {required},\n  {bad} TEXT\n);\n");
            let sql = secrets.replace(&good, &plaintext);
            assert_eq!(
                secret_column_violations(&parse_tables(&sql)).len(),
                1,
                "{table}.{bad}"
            );
        }
        let unhashed = secrets.replace(&good, &format!("CREATE TABLE {table} (\n  id UUID\n);\n"));
        assert_eq!(
            secret_column_violations(&parse_tables(&unhashed)).len(),
            1,
            "{table} without {required}"
        );
        let absent = secrets.replace(&good, "");
        assert_eq!(
            secret_column_violations(&parse_tables(&absent)).len(),
            1,
            "no {table}"
        );
    }

    let record = |name: &str, columns: &[(&str, &str)]| {
        let body: String = columns
            .iter()
            .map(|(column, ty)| format!("  {column} {ty} NOT NULL,\n"))
            .collect();
        format!(
            "CREATE TABLE {name} (\n  id UUID PRIMARY KEY,\n  user_id UUID NOT NULL REFERENCES users (id),\n{body}  UNIQUE (user_id, request_id)\n);\n"
        )
    };
    let records = format!(
        "{}{}",
        record("passkey_endorsements", &ENDORSEMENT_RECORD_COLUMNS),
        record("passkey_revocations", &REVOCATION_RECORD_COLUMNS)
    );
    assert!(chain_record_violations(&parse_tables(&records)).is_empty());
    for (name, columns) in [
        ("passkey_endorsements", &ENDORSEMENT_RECORD_COLUMNS[..]),
        ("passkey_revocations", &REVOCATION_RECORD_COLUMNS[..]),
    ] {
        let good = record(name, columns);
        for (column, ty) in columns {
            let line = format!("  {column} {ty} NOT NULL,\n");
            for (what, replacement) in [
                ("missing", String::new()),
                ("nullable", format!("  {column} {ty},\n")),
                ("retyped", format!("  {column} JSONB NOT NULL,\n")),
            ] {
                let sql = records.replace(&good, &good.replace(&line, &replacement));
                assert_eq!(
                    chain_record_violations(&parse_tables(&sql)).len(),
                    1,
                    "{name}.{column} {what}"
                );
            }
        }
        let column_fk = good.replace(
            "signature TEXT NOT NULL",
            "signature TEXT NOT NULL REFERENCES passkeys (credential_id)",
        );
        let table_fk = good.replace(
            "  UNIQUE (user_id, request_id)\n",
            "  UNIQUE (user_id, request_id),\n  FOREIGN KEY (request_id) REFERENCES passkeys (id)\n",
        );
        for (what, bad) in [("column", column_fk), ("table", table_fk)] {
            let sql = records.replace(&good, &bad);
            assert_eq!(
                chain_record_violations(&parse_tables(&sql)).len(),
                1,
                "{name} {what}-level foreign key to passkeys"
            );
        }
        let absent = records.replace(&good, "");
        assert_eq!(
            chain_record_violations(&parse_tables(&absent)).len(),
            1,
            "no {name}"
        );
    }

    let offset = parse_tables("CREATE TABLE run_mirror (\n  offset BIGINT\n);\n");
    assert_eq!(run_mirror_violations(&offset).len(), 2);

    let no_fk = parse_tables("CREATE TABLE snapshots (\n  user_id UUID NOT NULL\n);\n");
    assert_eq!(
        composite_fk_violations(&no_fk).len(),
        INSTANCE_CHILDREN.len() + 1
    );

    assert!(is_migration_filename("0001_init.sql"));
    for bad in [
        "1_init.sql",
        "0001init.sql",
        "0001_Init.sql",
        "0001_.sql",
        "0001_init.SQL",
        "00001_x.sql",
        "0001_a-b.sql",
    ] {
        assert!(!is_migration_filename(bad), "{bad}");
    }
}

#[test]
fn content_column_detector_fires_on_violations() {
    let clean = "-- the Hub's cursor; `0` is none\n\
                 CREATE TABLE run_mirror (\n  event JSONB NOT NULL,\n  PRIMARY KEY (event)\n);\n\
                 ALTER TABLE instance_requests\n  ADD COLUMN outcome TEXT CHECK (outcome IN ('a, b', 'c;d')),\n  \
                 ADD COLUMN answered_at TIMESTAMPTZ,\n  \
                 ADD CONSTRAINT cost_check CHECK (outcome IS NOT NULL),\n  ADD CHECK (answered_at IS NOT NULL);\n\
                 CREATE INDEX path_idx ON run_mirror (event);\n";
    assert!(content_column_violations(clean).is_empty());
    assert_eq!(
        added_columns(clean),
        [
            ("instance_requests", "outcome"),
            ("instance_requests", "answered_at")
        ]
        .map(|(table, column)| (table.to_string(), column.to_string()))
    );

    for name in [
        "transcript",
        "diff_text",
        "worktree_path",
        "COST_USD",
        "gate_summary",
        "summaries",
    ] {
        for table in INSTANCE_FED_TABLES {
            let created = format!("CREATE TABLE {table} (\n  id UUID,\n  {name} TEXT\n);\n");
            assert_eq!(
                content_column_violations(&created).len(),
                1,
                "CREATE TABLE {table} ({name})"
            );
        }
        for added in [
            format!("ALTER TABLE users\n  ADD COLUMN ok TEXT,\n  ADD COLUMN {name} TEXT;\n"),
            format!("alter table only users add column if not exists {name} text;"),
            format!("ALTER TABLE IF EXISTS users ADD {name} TEXT DEFAULT 'a,b';"),
            format!("{clean}ALTER TABLE users ADD COLUMN {name} TEXT;\n"),
        ] {
            assert_eq!(content_column_violations(&added).len(), 1, "{added}");
        }
    }

    let elsewhere = "CREATE TABLE attachments (\n  id UUID,\n  transcript TEXT\n);\n";
    assert!(content_column_violations(elsewhere).is_empty());
}

const EXPECTED_SERVICES: [&str; 4] = ["hub", "postgres", "openbao", "caddy"];
const STORAGE_BACKENDS: [&str; 3] = ["file", "raft", "postgresql"];
/// The image's entrypoint chowns only `/openbao/{config,logs,file}` before
/// dropping to the `openbao` user; a volume mounted anywhere else is created
/// root-owned, and `bao operator init` fails writing its keyring there.
const OPENBAO_FILE_STORAGE: &str = "/openbao/file";
const README_KEYWORDS: [&str; 8] = [
    "bao operator init",
    "bao operator unseal",
    "bao secrets enable transit",
    "transit/keys/",
    "HTTPS",
    "RP ID",
    "localhost",
    "--ignored",
];
/// The Hub never renews its periodic token, and compose interpolates the raw
/// password into `DATABASE_URL`: both are operator duties only the README states.
const README_OPERATOR_DUTIES: [&str; 3] = ["bao token renew", "bao token lookup", "URL-safe"];
/// The hub exits at startup without `OPENBAO_TOKEN`, and `docker compose restart`
/// keeps a container's old environment: the README must start the whole stack
/// once the token is in `.env`, and recreate `hub` after any later `.env` change.
const README_FULL_STACK_UP: &str = "docker compose up -d";
const README_RECREATE_HUB: &str = "docker compose up -d hub";
/// Nothing in the stack bounds a client, the Hub is one replica and a restart
/// drops its ceremonies. The rustdoc on `Ceremonies` says so to whoever reads
/// the code, and an operator does not: the README is where they are told.
const README_CEREMONY_LIMITS: [&str; 4] = [
    "too_many_ceremonies",
    "rate limit",
    "replica",
    "ceremony_expired",
];
/// Each has a row in the README's table, whose value cell is compared with the
/// declaration: a budget raised in the Hub and not in the README is a number
/// an operator sizes a limiter from and gets wrong.
const CEREMONY_BUDGETS: [&str; 4] = [
    "CHALLENGE_TTL_SECS",
    "MAX_LIVE_CEREMONIES",
    "MAX_LIVE_PER_HOLDER",
    "MAX_LIVE_SESSION_BOUND",
];
const CEREMONY_BUDGET_SOURCES: [&str; 2] = ["challenge.rs", "ceremonies.rs"];

fn deploy_dir() -> PathBuf {
    repo_root().join("deploy").join("hub")
}

fn read_deploy(rel: &str) -> String {
    let path = deploy_dir().join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

fn indent_of(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

fn is_blank_or_comment(line: &str) -> bool {
    let t = line.trim();
    t.is_empty() || t.starts_with('#')
}

/// Relies on canonical 2-space block style: top-level keys at column 0,
/// service names at column 2, their keys at column 4.
struct Service {
    name: String,
    lines: Vec<String>,
}

impl Service {
    fn has_key(&self, key: &str) -> bool {
        let prefix = format!("{key}:");
        self.lines
            .iter()
            .any(|l| indent_of(l) == 4 && l.trim_start().starts_with(&prefix))
    }

    fn key_text(&self, key: &str) -> Option<String> {
        let prefix = format!("{key}:");
        let start = self
            .lines
            .iter()
            .position(|l| indent_of(l) == 4 && l.trim_start().starts_with(&prefix))?;
        let mut text = self.lines[start].clone();
        for line in &self.lines[start + 1..] {
            if indent_of(line) <= 4 {
                break;
            }
            text.push('\n');
            text.push_str(line);
        }
        Some(text)
    }
}

fn parse_services(compose: &str) -> Vec<Service> {
    let mut services: Vec<Service> = Vec::new();
    let mut in_services = false;
    for line in compose.lines().filter(|l| !is_blank_or_comment(l)) {
        let indent = indent_of(line);
        if indent == 0 {
            in_services = line.trim_end() == "services:";
        } else if in_services && indent == 2 {
            if let Some(name) = line.trim().strip_suffix(':') {
                services.push(Service {
                    name: name.to_string(),
                    lines: Vec::new(),
                });
            }
        } else if in_services {
            if let Some(last) = services.last_mut() {
                last.lines.push(line.to_string());
            }
        }
    }
    services
}

fn service_set_violations(compose: &str) -> Vec<String> {
    let mut out = Vec::new();
    if compose.lines().any(|l| l.starts_with("version:")) {
        out.push("compose.yaml has a top-level `version:` key".to_string());
    }
    let mut found: Vec<String> = parse_services(compose)
        .into_iter()
        .map(|s| s.name)
        .collect();
    found.sort();
    let mut expected: Vec<String> = EXPECTED_SERVICES.iter().map(|s| s.to_string()).collect();
    expected.sort();
    if found != expected {
        out.push(format!(
            "services must be exactly {EXPECTED_SERVICES:?}, found {found:?}"
        ));
    }
    out
}

fn port_violations(compose: &str) -> Vec<String> {
    parse_services(compose)
        .iter()
        .filter(|s| s.name != "caddy" && s.has_key("ports"))
        .map(|s| format!("service `{}` publishes `ports:`; only caddy may", s.name))
        .collect()
}

fn image_violations(compose: &str) -> Vec<String> {
    let mut out = Vec::new();
    for service in parse_services(compose) {
        let Some(image) = service.key_text("image") else {
            if service.name != "hub" {
                out.push(format!("service `{}` has no `image:`", service.name));
            }
            continue;
        };
        let reference = image
            .trim_start()
            .trim_start_matches("image:")
            .trim()
            .trim_matches(|c| c == '"' || c == '\'');
        let last_segment = reference.rsplit('/').next().unwrap_or(reference);
        match last_segment.split_once(':') {
            None => out.push(format!("`{}` image `{reference}` has no tag", service.name)),
            Some((_, tag)) if tag.is_empty() || tag == "latest" => {
                out.push(format!(
                    "`{}` image `{reference}` is not pinned",
                    service.name
                ));
            }
            Some(_) => {}
        }
    }
    out
}

fn hub_build_violations(compose: &str) -> Vec<String> {
    let services = parse_services(compose);
    let Some(hub) = services.iter().find(|s| s.name == "hub") else {
        return vec!["hub service is missing".to_string()];
    };
    let build = hub.key_text("build").unwrap_or_default();
    if build
        .lines()
        .any(|l| l.trim() == "dockerfile: deploy/hub/Dockerfile")
    {
        Vec::new()
    } else {
        vec!["hub `build:` must set `dockerfile: deploy/hub/Dockerfile`".to_string()]
    }
}

fn toolchain_channel() -> String {
    let toml = std::fs::read_to_string(repo_root().join("rust-toolchain.toml"))
        .expect("rust-toolchain.toml is readable");
    toml.lines()
        .find_map(|l| {
            let (key, value) = l.split_once('=')?;
            (key.trim() == "channel").then(|| value.trim().trim_matches('"').to_string())
        })
        .expect("rust-toolchain.toml sets a channel")
}

/// Joins `\`-continued lines, so a `RUN` split for readability is one line.
fn logical_lines(dockerfile: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut pending = String::new();
    for line in dockerfile.lines().map(str::trim) {
        match line.strip_suffix('\\') {
            Some(head) => {
                pending.push_str(head.trim_end());
                pending.push(' ');
            }
            None => {
                pending.push_str(line);
                out.push(std::mem::take(&mut pending));
            }
        }
    }
    if !pending.is_empty() {
        out.push(pending);
    }
    out
}

/// A source of `.` copies the whole context, which is how `deploy/hub/.env`
/// reached the builder layer; the Dockerfile must name what it copies.
fn copies_whole_context(line: &str) -> bool {
    let Some(args) = line.strip_prefix("COPY ") else {
        return false;
    };
    let operands: Vec<&str> = args
        .split_whitespace()
        .filter(|t| !t.starts_with("--"))
        .collect();
    if args.contains("--from=") || operands.len() < 2 {
        return false;
    }
    operands[..operands.len() - 1]
        .iter()
        .any(|s| matches!(*s, "." | "./"))
}

fn cache_mount_targets(line: &str) -> Vec<&str> {
    line.split_whitespace()
        .filter_map(|t| t.strip_prefix("--mount="))
        .filter(|m| m.split(',').any(|kv| kv == "type=cache"))
        .filter_map(|m| m.split(',').find_map(|kv| kv.strip_prefix("target=")))
        .collect()
}

/// Whether an `apt-get install` of the last stage names `libssl3`. Why it has
/// to is on that `RUN` in `deploy/hub/Dockerfile`.
fn runtime_installs_libssl(lines: &[&str]) -> bool {
    let runtime = lines
        .iter()
        .rposition(|l| l.starts_with("FROM "))
        .unwrap_or(0);
    lines[runtime..]
        .iter()
        .any(|l| l.contains("apt-get install") && l.split_whitespace().any(|t| t == "libssl3"))
}

fn dockerfile_violations(dockerfile: &str, channel: &str) -> Vec<String> {
    let logical = logical_lines(dockerfile);
    let lines: Vec<&str> = logical.iter().map(String::as_str).collect();
    let mut out = Vec::new();
    if lines.iter().any(|l| copies_whole_context(l)) {
        out.push("Dockerfile copies the whole build context (`COPY . .`)".to_string());
    }
    let target_dir = lines
        .iter()
        .find_map(|l| l.strip_prefix("ENV CARGO_TARGET_DIR="))
        .map(str::trim);
    if let Some(build) = lines.iter().find(|l| l.contains("cargo build")) {
        let mounts = cache_mount_targets(build);
        if !mounts.contains(&"/usr/local/cargo/registry") {
            out.push("`cargo build` has no cache mount on the cargo registry".to_string());
        }
        if target_dir.is_some_and(|dir| !mounts.contains(&dir)) {
            out.push("`cargo build` has no cache mount on `CARGO_TARGET_DIR`".to_string());
        }
        for mount in mounts {
            let reads_mount = lines.iter().any(|l| {
                l.starts_with("COPY --from=") && l.split_whitespace().any(|t| t.starts_with(mount))
            });
            if reads_mount {
                out.push(format!(
                    "a later stage copies from cache mount `{mount}`, which is not in the layer"
                ));
            }
        }
    }
    let builder = format!("FROM rust:{channel}");
    if !lines.iter().any(|l| l.starts_with(&builder)) {
        out.push(format!("builder stage must start `{builder}`"));
    }
    if !lines.iter().any(|l| l.starts_with("ENV CARGO_TARGET_DIR=")) {
        out.push("builder must set `ENV CARGO_TARGET_DIR=` explicitly".to_string());
    }
    let builds_hub = lines.iter().any(|l| {
        l.contains("cargo build")
            && l.contains("--locked")
            && l.contains("--release")
            && l.contains("-p demeteo-hub")
    });
    if !builds_hub {
        out.push("no `cargo build --locked --release -p demeteo-hub` line".to_string());
    }
    match lines.iter().rposition(|l| l.starts_with("USER ")) {
        None => out.push("runtime stage sets no `USER`".to_string()),
        Some(i) => {
            let user = lines[i].trim_start_matches("USER ").trim();
            if user == "root" || user == "0" {
                out.push("runtime `USER` must not be root".to_string());
            }
        }
    }
    if !lines.iter().any(|l| l.starts_with("ENTRYPOINT")) {
        out.push("runtime stage sets no `ENTRYPOINT`".to_string());
    }
    if !runtime_installs_libssl(&lines) {
        out.push("runtime stage does not `apt-get install` `libssl3`".to_string());
    }
    out
}

fn openbao_command_violations(compose: &str) -> Vec<String> {
    let services = parse_services(compose);
    let Some(bao) = services.iter().find(|s| s.name == "openbao") else {
        return vec!["openbao service is missing".to_string()];
    };
    let Some(command) = bao.key_text("command") else {
        return vec!["openbao has no `command:`".to_string()];
    };
    let mut out = Vec::new();
    if !command.contains("server") {
        out.push("openbao command lacks `server`".to_string());
    }
    if command.contains("-dev") {
        out.push("openbao command contains `-dev`".to_string());
    }
    // The image entrypoint already rewrites `server` to
    // `bao server -config=/openbao/config`; a second `-config` loads the file
    // twice and the duplicated listener fails to bind 8200.
    if command.contains("-config") {
        out.push("openbao command passes `-config`; the entrypoint already does".to_string());
    }
    out
}

/// `:?` makes compose refuse to interpolate rather than start Postgres on a
/// known default credential.
fn postgres_password_violations(compose: &str) -> Vec<String> {
    let mut out = Vec::new();
    if compose.contains("change-me") {
        out.push("compose.yaml still carries a `change-me` default".to_string());
    }
    let refs: Vec<&str> = compose
        .match_indices("${POSTGRES_PASSWORD")
        .map(|(i, _)| &compose[i..])
        .collect();
    if refs.is_empty() {
        out.push("compose.yaml never interpolates `${POSTGRES_PASSWORD`".to_string());
    }
    for r in refs
        .iter()
        .filter(|r| !r.starts_with("${POSTGRES_PASSWORD:?"))
    {
        let shown: String = r.chars().take_while(|c| *c != '}').collect();
        out.push(format!("`{shown}}}` is not `:?`-guarded"));
    }
    out
}

/// The README's first run is `docker compose up -d openbao`, before any token
/// exists, and compose interpolates the whole file for every service.
fn openbao_token_violations(compose: &str) -> Vec<String> {
    if compose.contains("${OPENBAO_TOKEN:?") || compose.contains("${OPENBAO_TOKEN?") {
        vec!["`OPENBAO_TOKEN` is `?`-guarded; first-run `up -d openbao` would fail".to_string()]
    } else {
        Vec::new()
    }
}

/// Postgres must be healthy before the hub runs migrations; OpenBao must not
/// gate startup, because it stays sealed until an operator acts.
fn hub_dependency_violations(compose: &str) -> Vec<String> {
    let services = parse_services(compose);
    let Some(hub) = services.iter().find(|s| s.name == "hub") else {
        return vec!["hub service is missing".to_string()];
    };
    let depends = hub.key_text("depends_on").unwrap_or_default();
    let lines: Vec<&str> = depends.lines().map(str::trim).collect();
    let mut out = Vec::new();
    let healthy = lines
        .windows(2)
        .any(|w| w[0] == "postgres:" && w[1] == "condition: service_healthy");
    if !healthy {
        out.push("hub must depend on `postgres` with `condition: service_healthy`".to_string());
    }
    if depends.contains("openbao") {
        out.push("hub must not depend on `openbao`".to_string());
    }
    out
}

fn dockerignore_violations(ignore: &str) -> Vec<String> {
    let entries: Vec<&str> = ignore.lines().map(str::trim).collect();
    ["**/.env", "deploy/"]
        .iter()
        .filter(|p| !entries.contains(p))
        .map(|p| format!("Dockerfile.dockerignore does not exclude `{p}`"))
        .collect()
}

/// OpenBao refuses to start when mlock is on without `IPC_LOCK`, and the
/// capability is dead weight when mlock is off: exactly one must hold.
fn mlock_violations(hcl: &str, compose: &str) -> Vec<String> {
    let mlock_disabled = hcl
        .lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .any(|l| l.split_whitespace().collect::<String>() == "disable_mlock=true");
    let ipc_lock = parse_services(compose)
        .iter()
        .find(|s| s.name == "openbao")
        .and_then(|s| s.key_text("cap_add"))
        .is_some_and(|c| c.contains("IPC_LOCK"));
    match (mlock_disabled, ipc_lock) {
        (true, true) => vec!["`disable_mlock = true` contradicts `cap_add: IPC_LOCK`".to_string()],
        (false, false) => vec!["mlock is on but openbao lacks `cap_add: IPC_LOCK`".to_string()],
        _ => Vec::new(),
    }
}

fn storage_violations(hcl: &str) -> Vec<String> {
    let has_storage = hcl.lines().any(|l| {
        let t = l.trim();
        STORAGE_BACKENDS
            .iter()
            .any(|b| t.starts_with(&format!("storage \"{b}\"")))
    });
    if has_storage {
        Vec::new()
    } else {
        vec![format!(
            "config.hcl has no `storage` stanza of {STORAGE_BACKENDS:?}"
        )]
    }
}

fn file_storage_path(hcl: &str) -> Option<String> {
    hcl.lines()
        .map(str::trim)
        .filter(|l| !l.starts_with('#'))
        .skip_while(|l| !l.starts_with("storage \"file\""))
        .take_while(|l| *l != "}")
        .find_map(|l| {
            let joined: String = l.split_whitespace().collect();
            joined
                .strip_prefix("path=\"")?
                .strip_suffix('"')
                .map(str::to_string)
        })
}

fn openbao_storage_path_violations(hcl: &str, compose: &str) -> Vec<String> {
    let Some(path) = file_storage_path(hcl) else {
        return vec!["config.hcl has no `path` in a `storage \"file\"` stanza".to_string()];
    };
    let mut out = Vec::new();
    if path != OPENBAO_FILE_STORAGE {
        out.push(format!(
            "file storage path `{path}` is not `{OPENBAO_FILE_STORAGE}`"
        ));
    }
    let volumes = parse_services(compose)
        .into_iter()
        .find(|s| s.name == "openbao")
        .and_then(|s| s.key_text("volumes"))
        .unwrap_or_default();
    let mounted = volumes.lines().skip(1).any(|l| {
        let entry = l.trim().trim_start_matches('-').trim().trim_matches('"');
        let mut parts = entry.split(':');
        let (Some(source), Some(target)) = (parts.next(), parts.next()) else {
            return false;
        };
        let named = !source.starts_with('.') && !source.starts_with('/');
        named && target == path && parts.next() != Some("ro")
    });
    if !mounted {
        out.push(format!(
            "openbao mounts no writable named volume at `{path}`"
        ));
    }
    out
}

fn readme_violations(readme: &str) -> Vec<String> {
    let mut out: Vec<String> = README_KEYWORDS
        .iter()
        .chain(README_OPERATOR_DUTIES.iter())
        .chain(README_CEREMONY_LIMITS.iter())
        .filter(|k| !readme.contains(*k))
        .map(|k| format!("deploy/hub/README.md never mentions `{k}`"))
        .collect();
    let unseal_heading = readme.lines().any(|l| {
        let l = l.to_ascii_lowercase();
        l.starts_with('#') && l.contains("unseal") && l.contains("restart")
    });
    if !unseal_heading {
        out.push("deploy/hub/README.md has no heading naming unseal after restart".into());
    }
    let full_stack_up = readme.lines().any(|l| {
        l.trim()
            .strip_prefix(README_FULL_STACK_UP)
            .is_some_and(|rest| rest.trim().is_empty())
    });
    if !full_stack_up {
        out.push(format!(
            "deploy/hub/README.md has no `{README_FULL_STACK_UP}` line that starts the whole stack"
        ));
    }
    if !readme.contains(README_RECREATE_HUB) {
        out.push(format!(
            "deploy/hub/README.md never says to recreate the hub with `{README_RECREATE_HUB}`"
        ));
    }
    out
}

fn declared_const(source: &str, name: &str) -> Option<u64> {
    let declaration = format!("pub const {name}:");
    source.lines().find_map(|l| {
        let (_, value) = l.trim().strip_prefix(&declaration)?.split_once('=')?;
        value
            .trim()
            .trim_end_matches(';')
            .replace('_', "")
            .parse()
            .ok()
    })
}

/// The second cell of the table row whose first cell is `` `name` ``. A
/// mention in a sentence is not read: the number beside it there is as often
/// a rate or a count derived from the constant as the constant itself.
fn documented_const(readme: &str, name: &str) -> Option<u64> {
    let first_cell = format!("`{name}`");
    readme.lines().find_map(|l| {
        let mut cells = l.trim().strip_prefix('|')?.split('|').map(str::trim);
        if cells.next()? != first_cell {
            return None;
        }
        cells.next()?.replace(',', "").parse().ok()
    })
}

fn ceremony_budget_violations(readme: &str, source: &str) -> Vec<String> {
    CEREMONY_BUDGETS
        .iter()
        .filter_map(|name| {
            let Some(declared) = declared_const(source, name) else {
                return Some(format!(
                    "the Hub declares no `pub const {name}` with an integer literal"
                ));
            };
            match documented_const(readme, name) {
                None => Some(format!(
                    "deploy/hub/README.md has no table row giving `{name}` a value"
                )),
                Some(documented) if documented != declared => Some(format!(
                    "deploy/hub/README.md gives `{name}` as {documented}; the Hub declares {declared}"
                )),
                Some(_) => None,
            }
        })
        .collect()
}

fn ceremony_budget_sources() -> String {
    let auth = hub_dir().join("src").join("auth");
    CEREMONY_BUDGET_SOURCES
        .iter()
        .map(|file| {
            let path = auth.join(file);
            std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
        })
        .collect()
}

#[test]
fn compose_has_exactly_the_four_services() {
    let violations = service_set_violations(&read_deploy("compose.yaml"));
    assert!(violations.is_empty(), "{}", violations.join("\n"));
}

#[test]
fn only_caddy_publishes_ports() {
    let violations = port_violations(&read_deploy("compose.yaml"));
    assert!(violations.is_empty(), "{}", violations.join("\n"));
}

#[test]
fn compose_images_are_pinned() {
    let violations = image_violations(&read_deploy("compose.yaml"));
    assert!(violations.is_empty(), "{}", violations.join("\n"));
}

#[test]
fn hub_builds_from_the_hub_dockerfile() {
    let violations = hub_build_violations(&read_deploy("compose.yaml"));
    assert!(violations.is_empty(), "{}", violations.join("\n"));
}

#[test]
fn hub_dockerfile_exists_and_builds_the_hub_as_non_root() {
    let dockerfile = deploy_dir().join("Dockerfile");
    assert!(dockerfile.is_file(), "{} is missing", dockerfile.display());
    let violations = dockerfile_violations(&read_deploy("Dockerfile"), &toolchain_channel());
    assert!(violations.is_empty(), "{}", violations.join("\n"));
    assert!(
        deploy_dir().join("Dockerfile.dockerignore").is_file(),
        "deploy/hub/Dockerfile.dockerignore is missing"
    );
}

#[test]
fn openbao_runs_in_server_mode_with_a_storage_stanza() {
    let violations = openbao_command_violations(&read_deploy("compose.yaml"));
    assert!(violations.is_empty(), "{}", violations.join("\n"));
    let violations = storage_violations(&read_deploy("openbao/config.hcl"));
    assert!(violations.is_empty(), "{}", violations.join("\n"));
}

#[test]
fn openbao_file_storage_is_the_mounted_image_volume() {
    let violations = openbao_storage_path_violations(
        &read_deploy("openbao/config.hcl"),
        &read_deploy("compose.yaml"),
    );
    assert!(violations.is_empty(), "{}", violations.join("\n"));
}

#[test]
fn readme_documents_the_operator_steps() {
    let readme = read_deploy("README.md");
    let mut violations = readme_violations(&readme);
    violations.extend(ceremony_budget_violations(
        &readme,
        &ceremony_budget_sources(),
    ));
    assert!(violations.is_empty(), "{}", violations.join("\n"));
}

#[test]
fn env_example_says_the_postgres_password_must_be_url_safe() {
    let example = read_deploy(".env.example");
    let flagged = example
        .lines()
        .take_while(|l| !l.starts_with("POSTGRES_PASSWORD="))
        .last()
        .is_some_and(|l| l.starts_with('#') && l.contains("URL-safe"));
    assert!(
        flagged,
        "deploy/hub/.env.example: the line above POSTGRES_PASSWORD must say it is URL-safe"
    );
}

#[test]
fn readme_detector_fires_on_violations() {
    let good = format!(
        "# Hub\n\n## Unseal after every restart\n\n{}\n{}\n{}\n{README_FULL_STACK_UP}\n{README_RECREATE_HUB}\n",
        README_KEYWORDS.join("\n"),
        README_OPERATOR_DUTIES.join("\n"),
        README_CEREMONY_LIMITS.join("\n")
    );
    assert!(readme_violations(&good).is_empty());
    for keyword in README_KEYWORDS
        .iter()
        .chain(README_OPERATOR_DUTIES.iter())
        .chain(README_CEREMONY_LIMITS.iter())
    {
        let missing = good.replace(keyword, "");
        assert_eq!(readme_violations(&missing).len(), 1, "{keyword}");
    }
    let no_heading = good.replace("## Unseal after every restart", "## Notes");
    assert_eq!(readme_violations(&no_heading).len(), 1);
    let body_only = good.replace("## Unseal after every restart", "Unseal after restart");
    assert_eq!(readme_violations(&body_only).len(), 1);
    let one_service_only = good.replace(
        &format!("\n{README_FULL_STACK_UP}\n"),
        &format!("\n{README_FULL_STACK_UP} openbao\n"),
    );
    assert_eq!(readme_violations(&one_service_only).len(), 1);
    let no_recreate = good.replace(README_RECREATE_HUB, "docker compose restart hub");
    assert_eq!(readme_violations(&no_recreate).len(), 1);
}

#[test]
fn ceremony_budget_detector_fires_on_violations() {
    let source = "pub const CHALLENGE_TTL_SECS: u64 = 300;\npub const MAX_LIVE_CEREMONIES: usize = 4096;\npub const MAX_LIVE_PER_HOLDER: usize = 8;\npub const MAX_LIVE_SESSION_BOUND: usize = 16_384;\n";
    let readme = "| Constant | Value | Bounds |\n|---|---|---|\n| `CHALLENGE_TTL_SECS` | 300 | a |\n| `MAX_LIVE_CEREMONIES` | 4096 | b |\n| `MAX_LIVE_PER_HOLDER` | 8 | c |\n| `MAX_LIVE_SESSION_BOUND` | 16,384 | d |\n";
    assert!(ceremony_budget_violations(readme, source).is_empty());

    let raised = source.replace("= 4096;", "= 8192;");
    assert_eq!(ceremony_budget_violations(readme, &raised).len(), 1);
    let stale = readme.replace("16,384", "16,385");
    assert_eq!(ceremony_budget_violations(&stale, source).len(), 1);
    let no_row = readme.replace("| `MAX_LIVE_PER_HOLDER` | 8 | c |\n", "");
    assert_eq!(ceremony_budget_violations(&no_row, source).len(), 1);
    let prose_only = readme.replace(
        "| `MAX_LIVE_PER_HOLDER` | 8 | c |",
        "`MAX_LIVE_PER_HOLDER` is 8",
    );
    assert_eq!(ceremony_budget_violations(&prose_only, source).len(), 1);
    let undeclared = source.replace("pub const CHALLENGE_TTL_SECS: u64 = 300;\n", "");
    assert_eq!(ceremony_budget_violations(readme, &undeclared).len(), 1);
    let computed = source.replace("= 300;", "= 5 * 60;");
    assert_eq!(ceremony_budget_violations(readme, &computed).len(), 1);
    assert_eq!(ceremony_budget_violations("", source).len(), 4);
}

#[test]
fn deploy_detectors_fire_on_violations() {
    let good = "name: x\n\nservices:\n  hub:\n    build:\n      context: .\n      dockerfile: deploy/hub/Dockerfile\n  postgres:\n    image: postgres:17\n  openbao:\n    image: openbao/openbao:2.4.1\n    command: [\"server\"]\n  caddy:\n    image: caddy:2\n    ports:\n      - \"443:443\"\nvolumes:\n  data:\n";
    assert!(service_set_violations(good).is_empty());
    assert!(port_violations(good).is_empty());
    assert!(image_violations(good).is_empty());
    assert!(hub_build_violations(good).is_empty());
    assert!(openbao_command_violations(good).is_empty());

    let fifth = good.replace("volumes:\n", "  sidecar:\n    image: busybox:1\nvolumes:\n");
    assert_eq!(service_set_violations(&fifth).len(), 1);
    let versioned = format!("version: \"3\"\n{good}");
    assert_eq!(service_set_violations(&versioned).len(), 1);

    let ported = good.replace(
        "    image: postgres:17\n",
        "    image: postgres:17\n    ports:\n      - \"5432:5432\"\n",
    );
    assert_eq!(port_violations(&ported).len(), 1);

    let latest = good.replace("postgres:17", "postgres:latest");
    assert_eq!(image_violations(&latest).len(), 1);
    let untagged = good.replace("image: caddy:2", "image: caddy");
    assert_eq!(image_violations(&untagged).len(), 1);
    let registry_port = good.replace("image: caddy:2", "image: reg:5000/caddy");
    assert_eq!(image_violations(&registry_port).len(), 1);

    let wrong_build = good.replace("deploy/hub/Dockerfile", "Dockerfile");
    assert_eq!(hub_build_violations(&wrong_build).len(), 1);

    let dev = good.replace("\"server\"", "\"server\", \"-dev\"");
    assert_eq!(openbao_command_violations(&dev).len(), 1);
    let no_server = good.replace("[\"server\"]", "[\"bao\"]");
    assert_eq!(openbao_command_violations(&no_server).len(), 1);
    let config_twice = good.replace(
        "\"server\"",
        "\"server\", \"-config=/openbao/config/config.hcl\"",
    );
    assert_eq!(openbao_command_violations(&config_twice).len(), 1);

    for backend in STORAGE_BACKENDS {
        let hcl = format!("storage \"{backend}\" {{\n  path = \"/x\"\n}}\n");
        assert!(storage_violations(&hcl).is_empty(), "{backend}");
    }
    assert_eq!(
        storage_violations("# storage \"file\" {\nui = false\n").len(),
        1
    );
}

#[test]
fn dockerfile_detector_fires_on_violations() {
    let good = "FROM rust:1.97.0-bookworm AS builder\nENV CARGO_TARGET_DIR=/build/target\nCOPY Cargo.toml Cargo.lock ./\nCOPY crates ./crates\nRUN --mount=type=cache,target=/usr/local/cargo/registry \\\n    --mount=type=cache,target=/build/target \\\n    cargo build --locked --release -p demeteo-hub \\\n    && cp /build/target/release/demeteo-hub /usr/local/bin/demeteo-hub\nFROM debian:bookworm-slim\nRUN apt-get update \\\n    && apt-get install -y --no-install-recommends ca-certificates libssl3\nCOPY --from=builder /usr/local/bin/demeteo-hub /usr/local/bin/demeteo-hub\nUSER hub\nENTRYPOINT [\"/usr/local/bin/demeteo-hub\"]\n";
    assert!(dockerfile_violations(good, "1.97.0").is_empty());
    assert_eq!(dockerfile_violations(good, "1.98.0").len(), 1);
    assert_eq!(
        dockerfile_violations(
            &good.replace("ENV CARGO_TARGET_DIR=/build/target\n", ""),
            "1.97.0"
        )
        .len(),
        1
    );
    assert_eq!(
        dockerfile_violations(&good.replace("--locked ", ""), "1.97.0").len(),
        1
    );
    assert_eq!(
        dockerfile_violations(&good.replace("--release ", ""), "1.97.0").len(),
        1
    );
    assert_eq!(
        dockerfile_violations(&good.replace("USER hub", "USER root"), "1.97.0").len(),
        1
    );
    assert_eq!(
        dockerfile_violations(&good.replace("USER hub\n", ""), "1.97.0").len(),
        1
    );
    assert_eq!(
        dockerfile_violations(&good.replace("ENTRYPOINT", "CMD"), "1.97.0").len(),
        1
    );
    for whole in ["COPY . .\n", "COPY . /src\n", "COPY --chown=1:1 ./ ./\n"] {
        let copied = good.replace("COPY crates ./crates\n", whole);
        assert_eq!(dockerfile_violations(&copied, "1.97.0").len(), 1, "{whole}");
    }
    assert_eq!(
        dockerfile_violations(
            &good.replace("--mount=type=cache,target=/usr/local/cargo/registry ", ""),
            "1.97.0"
        )
        .len(),
        1
    );
    assert_eq!(
        dockerfile_violations(
            &good.replace(
                "type=cache,target=/build/target",
                "type=cache,target=/elsewhere"
            ),
            "1.97.0"
        )
        .len(),
        1
    );
    let from_mount = good.replace(
        "COPY --from=builder /usr/local/bin/demeteo-hub",
        "COPY --from=builder /build/target/release/demeteo-hub",
    );
    assert_eq!(dockerfile_violations(&from_mount, "1.97.0").len(), 1);
    assert_eq!(
        dockerfile_violations(&good.replace(" libssl3", ""), "1.97.0").len(),
        1
    );
    let builder_only = good.replace(" libssl3", "").replace(
        "COPY crates ./crates\n",
        "COPY crates ./crates\nRUN apt-get install -y libssl3\n",
    );
    assert_eq!(dockerfile_violations(&builder_only, "1.97.0").len(), 1);
}

#[test]
fn compose_requires_a_postgres_password_but_not_an_openbao_token() {
    let compose = read_deploy("compose.yaml");
    let mut violations = postgres_password_violations(&compose);
    violations.extend(openbao_token_violations(&compose));
    assert!(violations.is_empty(), "{}", violations.join("\n"));
}

#[test]
fn hub_waits_for_healthy_postgres_and_never_for_openbao() {
    let violations = hub_dependency_violations(&read_deploy("compose.yaml"));
    assert!(violations.is_empty(), "{}", violations.join("\n"));
}

#[test]
fn docker_build_context_excludes_env_and_deploy() {
    let violations = dockerignore_violations(&read_deploy("Dockerfile.dockerignore"));
    assert!(violations.is_empty(), "{}", violations.join("\n"));
}

#[test]
fn openbao_mlock_and_ipc_lock_agree() {
    let violations = mlock_violations(
        &read_deploy("openbao/config.hcl"),
        &read_deploy("compose.yaml"),
    );
    assert!(violations.is_empty(), "{}", violations.join("\n"));
}

#[test]
fn deploy_hardening_detectors_fire_on_violations() {
    let good = "services:\n  hub:\n    environment:\n      DATABASE_URL: postgres://u:${POSTGRES_PASSWORD:?set it}@postgres/db\n      OPENBAO_TOKEN: ${OPENBAO_TOKEN:-}\n    depends_on:\n      postgres:\n        condition: service_healthy\n  postgres:\n    environment:\n      POSTGRES_PASSWORD: ${POSTGRES_PASSWORD:?set it}\n  openbao:\n    cap_add:\n      - IPC_LOCK\n";
    assert!(postgres_password_violations(good).is_empty());
    assert!(openbao_token_violations(good).is_empty());
    assert!(hub_dependency_violations(good).is_empty());

    let defaulted = good.replacen(
        "${POSTGRES_PASSWORD:?set it}",
        "${POSTGRES_PASSWORD:-change-me}",
        1,
    );
    assert_eq!(postgres_password_violations(&defaulted).len(), 2);
    let unguarded = good.replacen("${POSTGRES_PASSWORD:?set it}", "${POSTGRES_PASSWORD}", 1);
    assert_eq!(postgres_password_violations(&unguarded).len(), 1);
    let literal = good.replace("${POSTGRES_PASSWORD:?set it}", "hunter2");
    assert_eq!(postgres_password_violations(&literal).len(), 1);
    let comment_default = format!("# POSTGRES_PASSWORD=change-me\n{good}");
    assert_eq!(postgres_password_violations(&comment_default).len(), 1);

    let token_guarded = good.replace("${OPENBAO_TOKEN:-}", "${OPENBAO_TOKEN:?set it}");
    assert_eq!(openbao_token_violations(&token_guarded).len(), 1);

    let short_form = good.replace(
        "      postgres:\n        condition: service_healthy\n",
        "      - postgres\n",
    );
    assert_eq!(hub_dependency_violations(&short_form).len(), 1);
    let started = good.replace("service_healthy", "service_started");
    assert_eq!(hub_dependency_violations(&started).len(), 1);
    let waits_on_bao = good.replace(
        "        condition: service_healthy\n",
        "        condition: service_healthy\n      openbao:\n        condition: service_started\n",
    );
    assert_eq!(hub_dependency_violations(&waits_on_bao).len(), 1);

    let ignore = ".git\n**/target\n**/.env\n**/.env.*\n!**/.env.example\ndeploy/\n";
    assert!(dockerignore_violations(ignore).is_empty());
    assert_eq!(
        dockerignore_violations(&ignore.replace("**/.env\n", "")).len(),
        1
    );
    assert_eq!(
        dockerignore_violations(&ignore.replace("deploy/\n", "")).len(),
        1
    );
    assert_eq!(dockerignore_violations("# **/.env\n# deploy/\n").len(), 2);

    let hcl = "ui = false\n\nstorage \"file\" {\n  path = \"/x\"\n}\n";
    assert!(mlock_violations(hcl, good).is_empty());
    let both = format!("disable_mlock = true\n{hcl}");
    assert_eq!(mlock_violations(&both, good).len(), 1);
    let aligned = format!("disable_mlock   =   true\n{hcl}");
    assert_eq!(mlock_violations(&aligned, good).len(), 1);
    let no_cap = good.replace("    cap_add:\n      - IPC_LOCK\n", "");
    assert_eq!(mlock_violations(hcl, &no_cap).len(), 1);
    assert!(mlock_violations(&both, &no_cap).is_empty());
    let commented = format!("# disable_mlock = true\n{hcl}");
    assert!(mlock_violations(&commented, good).is_empty());

    let bao_hcl = |path: &str| format!("storage \"file\" {{\n  path = \"{path}\"\n}}\n");
    let bao_compose = |target: &str| {
        format!(
            "services:\n  openbao:\n    volumes:\n      - ./openbao/config.hcl:/openbao/config/config.hcl:ro\n      - openbao-data:{target}\n"
        )
    };
    let image_volume = bao_hcl("/openbao/file");
    assert!(
        openbao_storage_path_violations(&image_volume, &bao_compose("/openbao/file")).is_empty()
    );
    assert_eq!(
        openbao_storage_path_violations(&bao_hcl("/openbao/data"), &bao_compose("/openbao/data"))
            .len(),
        1
    );
    assert_eq!(
        openbao_storage_path_violations(&image_volume, &bao_compose("/openbao/data")).len(),
        1
    );
    let read_only = bao_compose("/openbao/file:ro");
    assert_eq!(
        openbao_storage_path_violations(&image_volume, &read_only).len(),
        1
    );
    let bind = bao_compose("/openbao/file").replace("openbao-data:", "./data:");
    assert_eq!(
        openbao_storage_path_violations(&image_volume, &bind).len(),
        1
    );
    assert_eq!(
        openbao_storage_path_violations("storage \"file\" {\n}\n", &bao_compose("/openbao/file"))
            .len(),
        1
    );
}
