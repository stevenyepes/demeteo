//! Source-level guard on what the wire types can carry (HUB.md §3).
//!
//! The other test files prove the shapes that exist; this one fails when a
//! shape that must not exist is added. It reads the crate's own source, so a
//! `path` field or a `Logs` variant on any type — including a type added after
//! this file — turns this red without anyone remembering to extend a fixture.
//!
//! Every guard has a self-test that plants the violation it exists to catch,
//! so none of them can pass by matching nothing.

use std::path::Path;

const SOURCES: &[(&str, &str)] = &[
    ("canonical.rs", include_str!("../src/canonical.rs")),
    ("lib.rs", include_str!("../src/lib.rs")),
    ("message.rs", include_str!("../src/message.rs")),
    ("request.rs", include_str!("../src/request.rs")),
    ("scope.rs", include_str!("../src/scope.rs")),
    ("snapshot.rs", include_str!("../src/snapshot.rs")),
];

const OTHER_TESTS: &[(&str, &str)] = &[
    ("messages.rs", include_str!("messages.rs")),
    ("requests.rs", include_str!("requests.rs")),
    ("signing.rs", include_str!("signing.rs")),
    ("snapshot.rs", include_str!("snapshot.rs")),
];

const MANIFEST: &str = include_str!("../Cargo.toml");

/// Matched per `_`-separated word of a declared name (after `CamelCase` is
/// split), with or without a trailing `s`: `worktree_path`, `LocalPath` and
/// `api_tokens` are the same leak as `path` and `token`.
const FORBIDDEN_WORDS: &[&str] = &[
    "path",
    "worktree",
    "transcript",
    "output",
    "diff",
    "content",
    "contents",
    "logs",
    "token",
    "secret",
    "password",
];

const FORBIDDEN_TEXT: &[&str] = &[
    "serde_json::Value",
    "HashMap",
    "BTreeMap",
    "serde(flatten",
    "serde(other",
];

/// Only this type may omit `deny_unknown_fields`; see its rustdoc.
const LENIENT: &str = "RequestHeader";

/// Functions outside `canonical.rs` that produce signed bytes.
const ENCODER_FNS: &[&str] = &[
    "signing_bytes",
    "put_payload",
    "put_assignment",
    "put_assertion",
];

fn source(file: &str) -> &'static str {
    SOURCES
        .iter()
        .find(|(name, _)| *name == file)
        .map(|(_, text)| *text)
        .unwrap_or_else(|| panic!("{file} is not in SOURCES"))
}

/// The line with its `//` comment cut, ignoring `//` inside a string literal.
fn strip_comment(line: &str) -> &str {
    let mut in_string = false;
    let mut escaped = false;
    let mut prev = ' ';
    for (at, c) in line.char_indices() {
        if in_string {
            match (escaped, c) {
                (true, _) => escaped = false,
                (false, '\\') => escaped = true,
                (false, '"') => in_string = false,
                _ => {}
            }
        } else if c == '"' {
            in_string = true;
        } else if c == '/' && prev == '/' {
            return &line[..at - 1];
        }
        prev = c;
    }
    line
}

/// The line with every string literal's contents blanked, so text inside a
/// string is never read as an identifier.
fn without_strings(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut in_string = false;
    let mut escaped = false;
    for c in line.chars() {
        if in_string {
            match (escaped, c) {
                (true, _) => escaped = false,
                (false, '\\') => escaped = true,
                (false, '"') => {
                    in_string = false;
                    out.push('"');
                }
                _ => {}
            }
        } else {
            if c == '"' {
                in_string = true;
            }
            out.push(c);
        }
    }
    out
}

fn brace_delta(line: &str) -> i32 {
    without_strings(line)
        .chars()
        .map(|c| match c {
            '{' => 1,
            '}' => -1,
            _ => 0,
        })
        .sum()
}

/// Index one past the end of the item that starts at `start`: the line where
/// its braces close, or the first `;` before any brace opens.
fn item_end(lines: &[String], start: usize) -> usize {
    let mut depth = 0;
    let mut opened = false;
    for (at, line) in lines.iter().enumerate().skip(start) {
        let delta = brace_delta(line);
        opened |= without_strings(line).contains('{');
        depth += delta;
        if (opened && depth <= 0) || (!opened && line.ends_with(';')) {
            return at + 1;
        }
    }
    lines.len()
}

/// Production, non-comment lines, trimmed: `//` comments removed and every
/// `#[cfg(test)]` item skipped wherever it sits in the file.
fn code_lines(source: &str) -> Vec<String> {
    let lines: Vec<String> = source
        .lines()
        .map(|line| strip_comment(line).trim().to_string())
        .filter(|line| !line.is_empty())
        .collect();
    let mut kept = Vec::new();
    let mut at = 0;
    while at < lines.len() {
        if lines[at].starts_with("#[cfg(test)]") {
            let rest = lines[at]["#[cfg(test)]".len()..].trim();
            let item = if rest.is_empty() { at + 1 } else { at };
            at = item_end(&lines, item);
        } else {
            kept.push(lines[at].clone());
            at += 1;
        }
    }
    kept
}

fn snake_words(name: &str) -> Vec<String> {
    let mut snake = String::new();
    for (at, c) in name.chars().enumerate() {
        if c.is_uppercase() && at > 0 {
            snake.push('_');
        }
        snake.extend(c.to_lowercase());
    }
    snake
        .split('_')
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect()
}

fn is_forbidden(name: &str) -> bool {
    snake_words(name).iter().any(|word| {
        FORBIDDEN_WORDS.contains(&word.as_str())
            || word
                .strip_suffix('s')
                .is_some_and(|w| FORBIDDEN_WORDS.contains(&w))
    })
}

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Every name the line declares or puts on the wire: item names, `name:`
/// fields and parameters (whatever their visibility, including inside an enum
/// variant), enum variants, and `rename`/`alias` strings.
fn declared_names(line: &str) -> Vec<String> {
    let code = without_strings(line);
    let chars: Vec<char> = code.chars().collect();
    let mut names = Vec::new();
    let mut words: Vec<String> = Vec::new();
    let mut at = 0;
    while at < chars.len() {
        if !is_ident_char(chars[at]) {
            at += 1;
            continue;
        }
        let start = at;
        while at < chars.len() && is_ident_char(chars[at]) {
            at += 1;
        }
        let word: String = chars[start..at].iter().collect();
        let before = start.checked_sub(1).map(|i| chars[i]);
        let next = chars.get(at).copied();
        let after_next = chars.get(at + 1).copied();
        let is_field = next == Some(':') && after_next != Some(':') && before != Some(':');
        let is_item = words.last().is_some_and(|kw| {
            [
                "struct", "enum", "type", "trait", "union", "mod", "const", "static",
            ]
            .contains(&kw.as_str())
        });
        let is_variant = start == 0
            && word.starts_with(|c: char| c.is_uppercase())
            && matches!(next, None | Some(',' | ' ' | '{' | '('));
        if is_field || is_item || is_variant {
            names.push(word.clone());
        }
        words.push(word);
    }
    for key in ["rename = \"", "alias = \""] {
        let mut rest = line;
        while let Some(found) = rest.find(key) {
            rest = &rest[found + key.len()..];
            let end = rest.find('"').unwrap_or(rest.len());
            names.push(rest[..end].to_string());
        }
    }
    names
}

fn forbidden_name_hits(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .flat_map(|line| declared_names(line))
        .filter(|name| is_forbidden(name))
        .collect()
}

fn forbidden_text_hits(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .filter(|line| {
            let code = without_strings(line);
            FORBIDDEN_TEXT.iter().any(|needle| code.contains(needle))
        })
        .cloned()
        .collect()
}

fn struct_name(line: &str) -> Option<&str> {
    let at = line.find("struct ")?;
    let preamble = &line[..at];
    if !(preamble.is_empty() || preamble.starts_with("pub")) {
        return None;
    }
    let rest = &line[at + "struct ".len()..];
    let end = rest.find(|c: char| !is_ident_char(c)).unwrap_or(rest.len());
    Some(&rest[..end])
}

/// The attribute text above line `at`: every line back to where the previous
/// item ended, so a `#[derive(...)]` rustfmt split over lines is read whole.
fn attributes_above(lines: &[String], at: usize) -> String {
    let mut attrs: Vec<&str> = lines[..at]
        .iter()
        .rev()
        .take_while(|l| !(l.ends_with('}') || l.ends_with(';') || l.ends_with('{')))
        .map(String::as_str)
        .collect();
    attrs.reverse();
    attrs.join(" ")
}

fn lenient_deserialize_structs(lines: &[String]) -> Vec<String> {
    let mut hits = Vec::new();
    for (at, line) in lines.iter().enumerate() {
        let Some(name) = struct_name(line) else {
            continue;
        };
        let attrs = attributes_above(lines, at);
        let derives_deserialize = attrs.contains("derive(") && attrs.contains("Deserialize");
        let denies = attrs.contains("deny_unknown_fields");
        if derives_deserialize && !denies && name != LENIENT {
            hits.push(name.to_string());
        }
    }
    hits
}

fn manifest_problems(manifest: &str) -> Vec<String> {
    let expected_deps = [
        ("serde", "{version=\"1\",features=[\"derive\"]}"),
        ("serde_json", "\"1\""),
    ];
    let required = [
        ("package", "version.workspace", "true"),
        ("package", "edition", "\"2021\""),
        ("lints", "workspace", "true"),
    ];
    let mut problems = Vec::new();
    let mut section = String::new();
    let mut entries: Vec<(String, String, String)> = Vec::new();
    for line in manifest.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(header) = line.strip_prefix('[') {
            section = header.trim_end_matches(']').trim().to_string();
            if !["package", "dependencies", "lints"].contains(&section.as_str()) {
                problems.push(format!("unexpected section [{section}]"));
            }
        } else if let Some((key, value)) = line.split_once('=') {
            let value: String = value.chars().filter(|c| !c.is_whitespace()).collect();
            entries.push((section.clone(), key.trim().to_string(), value));
        }
    }
    let deps: Vec<(&str, &str)> = entries
        .iter()
        .filter(|(s, _, _)| s == "dependencies")
        .map(|(_, k, v)| (k.as_str(), v.as_str()))
        .collect();
    if deps != expected_deps {
        problems.push(format!("dependencies are {deps:?}"));
    }
    for (s, k, v) in required {
        if !entries.iter().any(|e| e.0 == s && e.1 == k && e.2 == v) {
            problems.push(format!("[{s}] lacks {k} = {v}"));
        }
    }
    problems
}

fn serde_hits(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .filter(|line| without_strings(line).contains("serde"))
        .cloned()
        .collect()
}

/// The lines of every `fn` in `names`, signature through closing brace.
fn fn_bodies(lines: &[String], names: &[&str]) -> Vec<String> {
    let mut body = Vec::new();
    for (at, line) in lines.iter().enumerate() {
        let declares = names.iter().any(|name| {
            let code = without_strings(line);
            code.contains(&format!("fn {name}(")) || code.contains(&format!("fn {name}<"))
        });
        if declares {
            body.extend_from_slice(&lines[at..item_end(lines, at)]);
        }
    }
    body
}

fn encoder_serde_hits(canonical: &str, request: &str) -> Vec<String> {
    let mut hits = serde_hits(&code_lines(canonical));
    hits.extend(serde_hits(&fn_bodies(&code_lines(request), ENCODER_FNS)));
    hits
}

/// A `TODO`/`FIXME`, or a comment citing a line number (`line 12`, `foo.rs:12`,
/// `L12`) — the tree has exactly one, and it moves.
fn hygiene_hits(text: &str) -> Vec<String> {
    let cites_line = |line: &str| {
        let lower = line.to_lowercase();
        let digit_after = |s: &str, key: &str| {
            s.match_indices(key)
                .any(|(at, _)| s[at + key.len()..].starts_with(|c: char| c.is_ascii_digit()))
        };
        digit_after(&lower, "line ")
            || digit_after(&lower, "lines ")
            || digit_after(&lower, ".rs:")
            || digit_after(line, "#L")
    };
    text.lines()
        .filter(|line| line.contains("TODO") || line.contains("FIXME") || cites_line(line))
        .map(str::to_string)
        .collect()
}

#[test]
fn sources_list_every_file_in_src() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut on_disk: Vec<String> = std::fs::read_dir(&src)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "rs"))
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    on_disk.sort();
    let mut listed: Vec<&str> = SOURCES.iter().map(|(name, _)| *name).collect();
    listed.sort();
    assert_eq!(on_disk, listed, "add every src/*.rs to SOURCES");
    assert!(
        !src.join("canonical").exists(),
        "a canonical/ module escapes SOURCES"
    );
}

#[test]
fn no_type_field_or_variant_is_named_after_something_that_must_not_leave_an_instance() {
    for (file, text) in SOURCES {
        let hits = forbidden_name_hits(&code_lines(text));
        assert!(hits.is_empty(), "{file}: {hits:?}");
    }
}

#[test]
fn no_untyped_or_open_ended_shape_in_production_code() {
    for (file, text) in SOURCES {
        let hits = forbidden_text_hits(&code_lines(text));
        assert!(hits.is_empty(), "{file}: {hits:?}");
    }
}

#[test]
fn every_deserialize_struct_denies_unknown_fields() {
    for (file, text) in SOURCES {
        let hits = lenient_deserialize_structs(&code_lines(text));
        assert!(hits.is_empty(), "{file}: {hits:?}");
    }
}

#[test]
fn the_lenient_exception_still_exists_and_still_derives_deserialize() {
    let lines = code_lines(source("message.rs"));
    let at = lines
        .iter()
        .position(|l| struct_name(l) == Some(LENIENT))
        .expect("LENIENT names a struct that no longer exists");
    assert!(attributes_above(&lines, at).contains("Deserialize"));
}

#[test]
fn manifest_depends_on_serde_and_serde_json_only() {
    let problems = manifest_problems(MANIFEST);
    assert!(problems.is_empty(), "{problems:?}");
}

#[test]
fn canonical_encoder_uses_no_serde() {
    let request_encoder = fn_bodies(&code_lines(source("request.rs")), ENCODER_FNS);
    for name in ENCODER_FNS {
        assert!(
            request_encoder
                .iter()
                .any(|l| l.contains(&format!("fn {name}("))),
            "ENCODER_FNS names {name}, which request.rs no longer defines"
        );
    }
    let hits = encoder_serde_hits(source("canonical.rs"), source("request.rs"));
    assert!(hits.is_empty(), "{hits:?}");
}

#[test]
fn no_todo_or_line_number_citation_in_the_crate() {
    for (file, text) in SOURCES.iter().chain(OTHER_TESTS) {
        let hits = hygiene_hits(text);
        assert!(hits.is_empty(), "{file}: {hits:?}");
    }
}

#[test]
fn every_spec_public_name_is_importable_from_the_crate_root() {
    use demeteo_hub_protocol::{
        endorsement_challenge, gate_challenge, revocation_challenge, AttachmentRef, CancelFeature,
        CursorAck, Decision, Effort, EncodeError, GateDecision, Hello, PasskeyEndorsement,
        PasskeyRevocation, ProjectSnapshot, RefusalReason, RequestHeader, RequestOutcome,
        RequestPayload, RequestResult, RunEvent, RunEventKind, RunEventPayload, RunShapeSettings,
        Scope, SetStepAssignment, SignedRequest, Snapshot, StartFeature, StepOverride,
        WebAuthnAssertion, WorkflowVersionSnapshot, PROTOCOL_VERSION,
    };
    use std::any::type_name;

    let types = [
        type_name::<Scope>(),
        type_name::<Decision>(),
        type_name::<Effort>(),
        type_name::<StepOverride>(),
        type_name::<AttachmentRef>(),
        type_name::<WebAuthnAssertion>(),
        type_name::<Hello>(),
        type_name::<RunEventKind>(),
        type_name::<RunEventPayload>(),
        type_name::<RunEvent>(),
        type_name::<CursorAck>(),
        type_name::<ProjectSnapshot>(),
        type_name::<WorkflowVersionSnapshot>(),
        type_name::<RunShapeSettings>(),
        type_name::<Snapshot>(),
        type_name::<RefusalReason>(),
        type_name::<RequestOutcome>(),
        type_name::<RequestResult>(),
        type_name::<RequestHeader>(),
        type_name::<SignedRequest>(),
        type_name::<RequestPayload>(),
        type_name::<StartFeature>(),
        type_name::<CancelFeature>(),
        type_name::<SetStepAssignment>(),
        type_name::<GateDecision>(),
        type_name::<PasskeyEndorsement>(),
        type_name::<PasskeyRevocation>(),
        type_name::<EncodeError>(),
    ];
    assert!(types
        .iter()
        .all(|t| t.starts_with("demeteo_hub_protocol::")));

    type Bytes = Result<Vec<u8>, EncodeError>;
    let gate: fn(&str, &str, &str, Decision, &str, u64) -> Bytes = gate_challenge;
    let endorse: fn(&str, &str, u64, &str, &str) -> Bytes = endorsement_challenge;
    let revoke: fn(&str, &str, u64, &str) -> Bytes = revocation_challenge;
    let sign: fn(&SignedRequest) -> Bytes = SignedRequest::signing_bytes;
    let _ = (gate, endorse, revoke, sign);
    assert_eq!(PROTOCOL_VERSION, 1);
}

fn lines(text: &str) -> Vec<String> {
    code_lines(text)
}

#[test]
fn guard_flags_a_path_field() {
    let bad = lines("pub struct ProjectSnapshot {\n    pub path: String,\n}");
    assert_eq!(forbidden_name_hits(&bad), ["path"]);
    let private = lines("struct P {\n    local_path: String,\n}");
    assert_eq!(forbidden_name_hits(&private), ["local_path"]);
    let compound = lines("pub struct P {\n    pub worktree_path: String,\n}");
    assert_eq!(forbidden_name_hits(&compound), ["worktree_path"]);
    let plural = lines("pub struct P {\n    pub api_tokens: Vec<String>,\n}");
    assert_eq!(forbidden_name_hits(&plural), ["api_tokens"]);
}

#[test]
fn guard_flags_a_forbidden_type_name() {
    assert_eq!(
        forbidden_name_hits(&lines("pub struct Transcript {}")),
        ["Transcript"]
    );
    assert_eq!(
        forbidden_name_hits(&lines("pub struct LocalPath(String);")),
        ["LocalPath"]
    );
    assert_eq!(
        forbidden_name_hits(&lines("type AgentOutput = String;")),
        ["AgentOutput"]
    );
}

#[test]
fn guard_flags_a_forbidden_variant_or_variant_field() {
    let scope = lines("pub enum Scope {\n    Read,\n    Logs,\n    Gates,\n}");
    assert_eq!(forbidden_name_hits(&scope), ["Logs"]);
    let inline = lines("pub enum O {\n    Refused { reason: R, path: String },\n}");
    assert_eq!(forbidden_name_hits(&inline), ["path"]);
    let tuple = lines("pub enum P {\n    Diff(String),\n}");
    assert_eq!(forbidden_name_hits(&tuple), ["Diff"]);
}

#[test]
fn guard_flags_a_forbidden_wire_key() {
    let bad = lines("pub struct P {\n    #[serde(rename = \"path\")]\n    pub dir: String,\n}");
    assert_eq!(forbidden_name_hits(&bad), ["path"]);
    let alias = lines("pub struct P {\n    #[serde(alias = \"secret\")]\n    pub k: String,\n}");
    assert_eq!(forbidden_name_hits(&alias), ["secret"]);
}

#[test]
fn guard_passes_ordinary_names() {
    let ok = lines(
        "pub struct ProjectSnapshot {\n    pub id: String,\n    pub remote_url: Option<String>,\n}\n\
         pub enum Scope {\n    Read,\n    Spend,\n}\nfn f(enc: &mut Encoder) -> Result<(), E> {\n    \
         enc.put_str(\"path\")?;\n    Ok(())\n}",
    );
    assert!(
        forbidden_name_hits(&ok).is_empty(),
        "{:?}",
        forbidden_name_hits(&ok)
    );
}

#[test]
fn guard_ignores_comments_and_test_modules_but_not_code_after_them() {
    let ok = lines(
        "/// pub path: String\n// HashMap\npub id: String,\n#[cfg(test)]\nmod t {\n    \
         pub path: u8,\n    fn f() { let m: HashMap<u8, u8>; }\n}",
    );
    assert!(forbidden_name_hits(&ok).is_empty());
    assert!(forbidden_text_hits(&ok).is_empty());

    let after =
        lines("#[cfg(test)]\nmod t {\n    fn f() {}\n}\npub struct P {\n    pub path: String,\n}");
    assert_eq!(forbidden_name_hits(&after), ["path"]);
    let one_line = lines("#[cfg(test)] mod t;\npub struct Diff;");
    assert_eq!(forbidden_name_hits(&one_line), ["Diff"]);
}

#[test]
fn guard_flags_open_shapes() {
    for bad in [
        "pub extra: serde_json::Value,",
        "pub m: HashMap<String, String>,",
        "pub m: std::collections::BTreeMap<String, String>,",
        "#[serde(flatten)]",
        "#[serde(other)]",
        "#[serde(other, rename = \"x\")]",
    ] {
        assert_eq!(forbidden_text_hits(&lines(bad)).len(), 1, "{bad}");
    }
}

#[test]
fn guard_flags_a_lenient_deserialize_struct() {
    let bad = lines("#[derive(Debug, Deserialize)]\npub struct Open {\n}");
    assert_eq!(lenient_deserialize_structs(&bad), ["Open"]);
    let private = lines("use x;\n#[derive(Deserialize)]\nstruct Hidden {\n}");
    assert_eq!(lenient_deserialize_structs(&private), ["Hidden"]);
    let split = lines("}\n#[derive(\n    Debug,\n    Deserialize,\n)]\npub struct Split {\n}");
    assert_eq!(lenient_deserialize_structs(&split), ["Split"]);
    let ok = lines(
        "#[derive(Debug, Deserialize)]\n#[serde(deny_unknown_fields)]\npub struct Closed {\n}",
    );
    assert!(lenient_deserialize_structs(&ok).is_empty());
    let write_only = lines("#[derive(Debug, Serialize)]\npub struct Out {\n}");
    assert!(lenient_deserialize_structs(&write_only).is_empty());
    let exempt = lines("#[derive(Deserialize)]\npub struct RequestHeader {\n}");
    assert!(lenient_deserialize_structs(&exempt).is_empty());
}

#[test]
fn guard_flags_extra_manifest_dependencies_and_sections() {
    let base = "[package]\nversion.workspace = true\nedition = \"2021\"\n\
                [dependencies]\nserde = { version = \"1\", features = [\"derive\"] }\n\
                serde_json = \"1\"\n[lints]\nworkspace = true\n";
    assert!(
        manifest_problems(base).is_empty(),
        "{:?}",
        manifest_problems(base)
    );

    let extra = base.replace(
        "serde_json = \"1\"\n",
        "serde_json = \"1\"\nrand = \"0.8\"\n",
    );
    assert_eq!(manifest_problems(&extra).len(), 1);
    let no_derive = base.replace(", features = [\"derive\"]", "");
    assert_eq!(manifest_problems(&no_derive).len(), 1);
    let more_features = base.replace("[\"derive\"]", "[\"derive\", \"rc\"]");
    assert_eq!(manifest_problems(&more_features).len(), 1);
    let dev = format!("{base}[dev-dependencies]\nx = \"1\"\n");
    assert_eq!(manifest_problems(&dev).len(), 1);
    let build = format!("{base}[build-dependencies]\nx = \"1\"\n");
    assert_eq!(manifest_problems(&build).len(), 1);
    let features = format!("{base}[features]\n");
    assert_eq!(manifest_problems(&features).len(), 1);
    let target = format!("{base}[target.'cfg(unix)'.dependencies]\nlibc = \"0.2\"\n");
    assert_eq!(manifest_problems(&target).len(), 1);
    let pinned = base.replace("version.workspace = true", "version = \"0.1.0\"");
    assert_eq!(manifest_problems(&pinned).len(), 1);
    let no_lints = base.replace("[lints]\nworkspace = true\n", "");
    assert_eq!(manifest_problems(&no_lints).len(), 1);
}

#[test]
fn guard_flags_serde_in_the_encoder() {
    let canonical = "//! Never serde.\nuse serde::Serialize;\n";
    assert_eq!(encoder_serde_hits(canonical, "").len(), 1);

    let request = "use serde::{Deserialize, Serialize};\n\
                   #[derive(Serialize, Deserialize)]\npub struct S {}\n\
                   impl S {\n    pub fn signing_bytes(&self) -> Vec<u8> {\n        \
                   serde_json::to_vec(self).unwrap_or_default()\n    }\n}";
    assert_eq!(encoder_serde_hits("", request).len(), 1);

    let clean = "use serde::Serialize;\nfn put_payload(e: &mut E) {\n    e.put_u8(1);\n}";
    assert!(encoder_serde_hits("", clean).is_empty());
}

#[test]
fn guard_flags_todos_and_line_citations() {
    assert_eq!(hygiene_hits("// TODO: drop this").len(), 1);
    assert_eq!(hygiene_hits("/// FIXME later").len(), 1);
    assert_eq!(hygiene_hits("/// see line 12 above").len(), 1);
    assert_eq!(hygiene_hits("/// as in canonical.rs:40").len(), 1);
    assert!(hygiene_hits("/// the line where its braces close").is_empty());
}
