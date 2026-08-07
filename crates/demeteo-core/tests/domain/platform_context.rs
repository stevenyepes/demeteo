//! What each platform is told about the machine it is standing on.
//!
//! No doubles: the block is pure over a [`Platform`], and the platform itself
//! is a value the execution port already resolved. Every assertion here holds
//! on every host, so the Windows arm is covered from the macOS and Linux
//! machines this is developed and gated on — the `cfg`-free-logic rule in
//! AGENTS.md §7.

use super::*;

use crate::domain::models::Platform;

#[test]
fn windows_forbids_translating_the_prompt_s_commands() {
    let block = platform_context_section(Platform::Windows);
    assert!(
        block.contains("MUST NOT translate"),
        "the do-not-translate instruction is the load-bearing sentence, got: {block}"
    );
    assert!(
        block.contains("PowerShell") && block.contains("cmd.exe"),
        "both shells an agent would reach for must be named, got: {block}"
    );
    assert!(
        block.contains("POSIX bodies"),
        "the block must say what the quoted gate commands actually are, got: {block}"
    );
}

#[test]
fn windows_names_the_os_the_shell_and_the_separator() {
    let block = platform_context_section(Platform::Windows);
    assert!(block.contains("Windows"), "the OS must be named: {block}");
    assert!(
        block.contains("Git for") && block.contains("bash"),
        "the agent must be told which interpreter runs a POSIX body: {block}"
    );
    assert!(
        block.contains("IS runnable"),
        "the agent must be told a POSIX command runs here at all: {block}"
    );
    assert!(
        block.contains("`\\` is this machine's path separator"),
        "the separator must be stated, not implied: {block}"
    );
}

/// The block corrects a default; on Linux and macOS the default is already
/// right, so a render on either is byte-for-byte what it was before this
/// module existed.
#[test]
fn a_posix_render_is_unchanged() {
    for platform in [Platform::Linux, Platform::MacOS] {
        assert_eq!(
            platform_context_section(platform),
            "",
            "{platform} needs no correction"
        );
        let placed = place_platform_context(Some(platform), "do the work");
        assert_eq!(placed.bound, "", "{platform} needs no correction");
        assert_eq!(
            render(&placed, "do the work"),
            "do the work",
            "{platform} must render exactly the prompt it renders today"
        );
    }
}

/// A machine whose OS the port could not name is not a machine assumed POSIX —
/// it is one the block says nothing about, whatever the template asked for.
#[test]
fn an_unresolved_platform_places_nothing_either_way() {
    for template in ["do the work", "intro\n\n{{platform_context}}\n\noutro"] {
        let placed = place_platform_context(None, template);
        assert_eq!(placed.bound, "");
        assert_eq!(placed.prefix, "");
    }
}

// ── the block appears exactly once, either way ──────────────────────────
//
// The two placements are one decision, and this is the decision the prompt
// builders actually run: `bound` is what `{{platform_context}}` renders as and
// `prefix` is what precedes the result, so applying both — as every builder
// does, unconditionally — is the whole of what an agent receives. Asserting
// against a hand-rolled `replace` here would be asserting against a second
// implementation of the renderer rather than against this one.

fn render(placed: &PlatformPlacement, template: &str) -> String {
    format!(
        "{}{}",
        placed.prefix,
        template.replace("{{platform_context}}", &placed.bound)
    )
}

fn occurrences(rendered: &str) -> usize {
    rendered.matches("## Platform — Windows").count()
}

#[test]
fn a_template_that_never_names_the_token_still_gets_the_block_once() {
    let template = "Implement the feature.";

    let placed = place_platform_context(Some(Platform::Windows), template);
    let rendered = render(&placed, template);
    assert_eq!(
        occurrences(&rendered),
        1,
        "the safety net must reach a template that cannot know the token: {rendered}"
    );
    assert!(
        rendered.starts_with("## Platform — Windows"),
        "the block reframes command text that follows it, so it goes first: {rendered}"
    );
    assert!(
        rendered.ends_with(template),
        "the prompt must survive intact after the block, got: {rendered}"
    );
}

#[test]
fn a_template_that_names_the_token_gets_the_block_once_where_it_asked() {
    let template = "intro\n\n{{platform_context}}\n\noutro";

    let placed = place_platform_context(Some(Platform::Windows), template);
    assert_eq!(
        placed.prefix, "",
        "a template that placed the block must not also be prefixed with it"
    );

    let rendered = render(&placed, template);
    assert_eq!(
        occurrences(&rendered),
        1,
        "the template's own placement must be the only one: {rendered}"
    );
    assert!(
        rendered.find("intro").unwrap() < rendered.find("## Platform — Windows").unwrap(),
        "an opting-in template places the block itself: {rendered}"
    );
    assert!(
        !rendered.contains("{{platform_context}}"),
        "the token must be fully substituted: {rendered}"
    );
}

#[test]
fn a_near_miss_token_does_not_count_as_opting_in() {
    for template in ["{{platform}}", "{{platform_contexts}}", ""] {
        let placed = place_platform_context(Some(Platform::Windows), template);
        assert_eq!(
            placed.bound, "",
            "`{template}` does not name the token, so nothing binds to it"
        );
        assert_eq!(
            occurrences(&placed.prefix),
            1,
            "`{template}` gets the safety net instead"
        );
    }
}
