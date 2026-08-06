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
        assert_eq!(
            inject_platform_context("do the work", platform),
            "do the work",
            "{platform} must render exactly the prompt it renders today"
        );
    }
}

#[test]
fn the_windows_block_is_prepended_whole() {
    let out = inject_platform_context("do the work", Platform::Windows);
    assert!(
        out.ends_with("do the work"),
        "the prompt must survive intact after the block, got: {out}"
    );
    assert!(
        out.starts_with("## Platform — Windows"),
        "the block reframes command text that follows it, so it goes first: {out}"
    );
}

// ── the block appears exactly once, either way ──────────────────────────
//
// The two placements are the caller's branch: a template naming the token
// gets the section substituted in place and no prepend; one that does not
// gets the prepend. Both are asserted against the same counting helper so a
// divergence between them cannot read as a pass.

fn occurrences(rendered: &str) -> usize {
    rendered.matches("## Platform — Windows").count()
}

#[test]
fn a_template_that_never_names_the_token_still_gets_the_block_once() {
    let template = "Implement the feature.";
    assert!(!template_uses_platform_context(template));

    let rendered = inject_platform_context(template, Platform::Windows);
    assert_eq!(
        occurrences(&rendered),
        1,
        "the safety net must reach a template that cannot know the token: {rendered}"
    );
}

#[test]
fn a_template_that_names_the_token_gets_the_block_once_where_it_asked() {
    let template = "intro\n\n{{platform_context}}\n\noutro";
    assert!(template_uses_platform_context(template));

    let section = platform_context_section(Platform::Windows);
    let rendered = template.replace("{{platform_context}}", &section);
    assert_eq!(
        occurrences(&rendered),
        1,
        "substitution alone must place exactly one block: {rendered}"
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
    assert!(!template_uses_platform_context("{{platform}}"));
    assert!(!template_uses_platform_context("{{platform_contexts}}"));
    assert!(!template_uses_platform_context(""));
}
