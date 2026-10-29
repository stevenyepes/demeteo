//! No host, port or filesystem-root literal may live in the hub's `src/` outside
//! `config.rs` — every deployment-shaped value is read from the environment
//! there, so a container image never bakes in an address or a path.
//!
//! The scan is textual and std-only. `#[cfg(test)]` items are blanked before
//! scanning (line numbers are preserved so a report names the real line), and
//! full-line comments are skipped. It does not understand raw strings: a
//! `r#"…"#` literal containing an unbalanced brace inside a `#[cfg(test)]`
//! block would end the strip early, so keep such fixtures in `tests/`.
//!
//! URL routes (`/healthz`, `/v1/…`) are not filesystem roots and stay allowed.
//! The needles are assembled from fragments so this file never matches itself.

use std::fs;
use std::path::{Path, PathBuf};

fn forbidden_words() -> Vec<String> {
    vec![
        ["local", "host"].concat(),
        ["127", "0", "0", "1"].join("."),
        ["0", "0", "0", "0"].join("."),
    ]
}

fn forbidden_roots() -> Vec<String> {
    ["srv", "var", "etc", "tmp", "app"]
        .iter()
        .map(|dir| format!("\"/{dir}"))
        .collect()
}

/// Blank every `#[cfg(test)]` item, keeping newlines so line numbers survive.
fn strip_cfg_test(src: &str) -> String {
    let marker = ["#[cfg(", "test)]"].concat();
    let bytes = src.as_bytes();
    let mut out = bytes.to_vec();
    let mut from = 0;
    while let Some(rel) = src[from..].find(&marker) {
        let start = from + rel;
        let end = item_end(bytes, start + marker.len());
        for b in &mut out[start..end] {
            if *b != b'\n' {
                *b = b' ';
            }
        }
        from = end.max(start + marker.len());
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Index just past the item following an attribute: through the matching `}`
/// of its first block, or through the `;` of a block-less item (`use`, `const`).
fn item_end(bytes: &[u8], mut i: usize) -> usize {
    let mut depth = 0usize;
    let mut in_string = false;
    while i < bytes.len() {
        let c = bytes[i];
        if in_string {
            match c {
                b'\\' => i += 1,
                b'"' => in_string = false,
                _ => {}
            }
        } else if c == b'/' && bytes.get(i + 1) == Some(&b'/') {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        } else if c == b'\'' && bytes.get(i + 2) == Some(&b'\'') {
            i += 2;
        } else {
            match c {
                b'"' => in_string = true,
                b'{' => depth += 1,
                b'}' => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        return i + 1;
                    }
                }
                b';' if depth == 0 => return i + 1,
                _ => {}
            }
        }
        i += 1;
    }
    bytes.len()
}

/// A `:<2-5 digits>` that sits inside a string literal and is not followed by
/// more alphanumerics — `"host:8080"`, `":9000"`, `"http://x:80/p"`.
fn has_quoted_port(line: &str) -> bool {
    let bytes = line.as_bytes();
    for (i, &c) in bytes.iter().enumerate() {
        if c != b':' {
            continue;
        }
        let digits = bytes[i + 1..]
            .iter()
            .take_while(|b| b.is_ascii_digit())
            .count();
        let after = bytes.get(i + 1 + digits);
        let ends_cleanly = after.is_none_or(|b| !b.is_ascii_alphanumeric());
        let inside_string = bytes[..i].iter().filter(|&&b| b == b'"').count() % 2 == 1;
        if (2..=5).contains(&digits) && ends_cleanly && inside_string {
            return true;
        }
    }
    false
}

/// `file:line: <what>` for every literal in `text`, which is one source file.
fn violations(file: &str, text: &str) -> Vec<String> {
    let words = forbidden_words();
    let roots = forbidden_roots();
    let mut found = Vec::new();
    for (idx, line) in strip_cfg_test(text).lines().enumerate() {
        if line.trim_start().starts_with("//") {
            continue;
        }
        let at = idx + 1;
        for word in &words {
            if line.contains(word.as_str()) {
                found.push(format!("{file}:{at}: host literal `{word}`"));
            }
        }
        for root in &roots {
            if line.contains(root.as_str()) {
                found.push(format!("{file}:{at}: filesystem root literal `{root}`"));
            }
        }
        if has_quoted_port(line) {
            found.push(format!("{file}:{at}: port literal in `{}`", line.trim()));
        }
    }
    found
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<PathBuf> = fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
        .map(|entry| {
            entry
                .unwrap_or_else(|e| panic!("entry in {}: {e}", dir.display()))
                .path()
        })
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

fn scan_src(src: &Path) -> Vec<String> {
    let mut files = Vec::new();
    rust_files(src, &mut files);
    let config = src.join("config.rs");
    let mut found = Vec::new();
    for path in files.into_iter().filter(|p| *p != config) {
        let text =
            fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        let name = path
            .strip_prefix(src)
            .unwrap_or(&path)
            .display()
            .to_string();
        found.extend(violations(&format!("src/{name}"), &text));
    }
    found
}

#[test]
fn hub_src_has_no_host_port_or_path_literals_outside_config() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let found = scan_src(&src);
    assert!(
        found.is_empty(),
        "literals belong in config.rs, read from the environment:\n  {}",
        found.join("\n  ")
    );
}

#[test]
fn scanner_flags_each_literal_kind_with_its_line() {
    let host = ["local", "host"].concat();
    let ip = ["127", "0", "0", "1"].join(".");
    let any = ["0", "0", "0", "0"].join(".");
    let cases = [
        format!("let a = \"{host}\";"),
        format!("let a = \"{ip}\";"),
        format!("let a = \"{any}\";"),
        "let a = \"db:5432\";".to_string(),
        "let a = \":8080\";".to_string(),
        "let a = \"/srv/web\";".to_string(),
        "let a = \"/var/lib\";".to_string(),
        "let a = \"/etc/hub\";".to_string(),
        "let a = \"/tmp/x\";".to_string(),
        "let a = \"/app/web\";".to_string(),
    ];
    for case in cases {
        let text = format!("fn ok() {{}}\n{case}\n");
        let found = violations("routes.rs", &text);
        assert_eq!(found.len(), 1, "`{case}` -> {found:?}");
        assert!(found[0].starts_with("routes.rs:2:"), "{found:?}");
    }
}

#[test]
fn scanner_allows_routes_comments_and_format_placeholders() {
    let host = ["local", "host"].concat();
    let text = format!(
        "// connect to {host} in dev\n\
         const A: &str = \"/healthz\";\n\
         const B: &str = \"/v1/instances\";\n\
         const C: &str = \"{{host}}:{{port}}\";\n\
         let m = map.get(key:80);\n"
    );
    assert_eq!(violations("routes.rs", &text), Vec::<String>::new());
}

#[test]
fn scanner_ignores_cfg_test_modules_but_not_code_after_them() {
    let host = ["local", "host"].concat();
    let text = format!(
        "fn real() {{}}\n\
         #[cfg(test)]\n\
         mod tests {{\n\
         \x20   const A: &str = \"{host}\";\n\
         \x20   const B: &str = \"}}\";\n\
         \x20   fn t() {{ if true {{ }} }}\n\
         }}\n\
         const AFTER: &str = \"{host}\";\n"
    );
    let found = violations("x.rs", &text);
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].starts_with("x.rs:8:"), "{found:?}");
}

#[test]
fn scanner_strips_blockless_cfg_test_items() {
    let host = ["local", "host"].concat();
    let text = format!("#[cfg(test)]\nconst A: &str = \"{host}\";\nconst B: &str = \"{host}\";\n");
    let found = violations("x.rs", &text);
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].starts_with("x.rs:3:"), "{found:?}");
}

#[test]
fn scan_src_skips_config_and_names_the_offending_file() {
    let root = std::env::temp_dir().join(format!("hub-no-literals-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("adapters")).unwrap_or_else(|e| panic!("mkdir: {e}"));
    let host = ["local", "host"].concat();
    let write = |rel: &str, body: &str| {
        fs::write(root.join(rel), body).unwrap_or_else(|e| panic!("write {rel}: {e}"));
    };
    write("config.rs", &format!("const D: &str = \"{host}:5432\";\n"));
    write("routes.rs", "const R: &str = \"/healthz\";\n");
    write(
        "adapters/openbao.rs",
        &format!("const U: &str = \"http://{host}\";\n"),
    );

    let found = scan_src(&root);
    let _ = fs::remove_dir_all(&root);

    assert_eq!(found.len(), 1, "{found:?}");
    assert!(
        found[0].starts_with("src/adapters/openbao.rs:1:"),
        "{found:?}"
    );
}
