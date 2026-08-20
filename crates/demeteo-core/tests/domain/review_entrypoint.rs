//! What a project's entrypoint does to the prompt that carries it.
//! `super` is `crate::domain::review_entrypoint`.

use super::*;

use crate::domain::prompt_context::PromptContext;

/// The shipped starter is the template under test on purpose: the binding is
/// only correct relative to where the token sits, and a render against a
/// hand-written stub would pass over a starter that had moved it.
fn starter_template() -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../src-tauri/workflows/code-review.json");
    let raw = std::fs::read_to_string(path).expect("the code-review starter ships in-tree");
    let doc: serde_json::Value = serde_json::from_str(&raw).unwrap();
    let step = doc["steps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|step| step["id"] == "s-review")
        .expect("the review step is the whole workflow");
    step["prompt_template"].as_str().unwrap().to_string()
}

fn render(configured: Option<&str>) -> String {
    PromptContext::new()
        .set("review_entrypoint", review_entrypoint_binding(configured))
        .render(&starter_template())
}

/// A heading whose section is empty reads as an instruction that was meant to
/// say something — the shape an unset entrypoint must not leave behind.
fn heading_with_nothing_under_it(rendered: &str) -> Option<String> {
    let mut open: Option<&str> = None;
    for line in rendered.lines() {
        if line.starts_with('#') {
            if let Some(previous) = open {
                return Some(previous.to_string());
            }
            open = Some(line);
        } else if !line.trim().is_empty() {
            open = None;
        }
    }
    None
}

#[test]
fn a_project_that_names_nothing_binds_nothing() {
    assert_eq!(review_entrypoint_binding(None), "");
    assert_eq!(review_entrypoint_binding(Some("")), "");
    assert_eq!(review_entrypoint_binding(Some("  \n ")), "");
}

#[test]
fn a_named_entrypoint_is_bound_as_written() {
    assert_eq!(
        review_entrypoint_binding(Some("/code-review")),
        "/code-review"
    );
    assert_eq!(
        review_entrypoint_binding(Some("  /review --deep  ")),
        "/review --deep"
    );
}

#[test]
fn an_unset_entrypoint_leaves_the_starter_prompt_untouched() {
    let rendered = render(None);

    assert!(
        !rendered.contains("{{"),
        "the token survived into the prompt"
    );
    assert_eq!(
        heading_with_nothing_under_it(&rendered),
        None,
        "an empty entrypoint left a section for the reviewer to obey"
    );
}

/// The wrapper test. A sentence around the value — "Start by running X", "The
/// project prefers X" — is Demeteo authoring review vocabulary, which is the
/// one thing this workflow exists not to do. Both halves catch it: the render
/// grows by exactly the value, and the value stands on its own line.
#[test]
fn a_named_entrypoint_reaches_the_prompt_verbatim() {
    const ENTRYPOINT: &str = "/code-review";
    let rendered = render(Some(ENTRYPOINT));

    assert_eq!(
        rendered.len() - render(None).len(),
        ENTRYPOINT.len(),
        "binding an entrypoint added bytes that are not the entrypoint"
    );
    assert!(
        rendered.lines().any(|line| line == ENTRYPOINT),
        "the entrypoint is not the whole of any line — something wraps it"
    );
}
