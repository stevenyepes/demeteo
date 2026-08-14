//! What each platform is told about the machine it is standing on.
//!
//! No doubles: the block is pure over a [`Platform`] and a
//! [`WindowsAgentShell`], both of them values something else already resolved —
//! the execution port and the agent's own runtime. Every assertion here holds
//! on every host, so the Windows arm is covered from the macOS and Linux
//! machines this is developed and gated on — the `cfg`-free-logic rule in
//! AGENTS.md §7.

use super::*;

use crate::domain::models::{Platform, WindowsAgentShell};

/// A resolved interpreter path, in the shape `posix_shell::resolve` answers
/// with — spaces included, since `C:\Program Files` is the default install root
/// and quoting it is the thing a wrapper has to get right.
const BASH: Option<&str> = Some(r"C:\Program Files\Git\bin\bash.exe");

/// Every shell an agent may be declared to run, so a new variant cannot be
/// added without deciding what it is told. The invariants below hold across all
/// of them; only the syntax promise differs.
const ALL_SHELLS: [WindowsAgentShell; 3] = [
    WindowsAgentShell::GitBash,
    WindowsAgentShell::PowerShell,
    WindowsAgentShell::Unknown,
];

#[test]
fn a_powershell_agent_distinguishes_its_own_shell_from_demeteos_posix_harness() {
    let block = platform_context_section(Platform::Windows, WindowsAgentShell::PowerShell, BASH);

    assert!(
        block.contains("own command tool") && block.contains("PowerShell"),
        "the agent must be told which shell executes commands it authors, got: {block}"
    );
    assert!(
        block.contains("Demeteo") && block.contains("harness") && block.contains("POSIX"),
        "the separate Demeteo harness lane must remain explicit, got: {block}"
    );
    assert!(
        block.contains("printf") && block.contains("sed") && block.contains("find"),
        "the observed bare-POSIX failure modes must be named, got: {block}"
    );
}

/// The prohibition is what every Windows block is *for*, so it is asserted
/// across all of them rather than through whichever one a caller happened to
/// pick. Pinned to one shell, this passed while the block for another shell
/// carried no prohibition at all.
#[test]
fn every_windows_block_forbids_translating_the_prompt_s_commands() {
    for shell in ALL_SHELLS {
        let block = platform_context_section(Platform::Windows, shell, BASH);
        assert!(
            block.contains("translate"),
            "{shell:?} must carry the do-not-translate instruction, got: {block}"
        );
        assert!(
            block.contains("PowerShell"),
            "{shell:?} must name the shell an agent would translate into: {block}"
        );
        assert!(
            block.contains("POSIX"),
            "{shell:?} must say what the quoted gate commands actually are: {block}"
        );
    }
}

/// What holds whatever the agent's command tool turns out to be. A shell
/// declaration changes what may be promised about *syntax* and nothing else.
#[test]
fn every_windows_block_states_the_facts_no_shell_changes() {
    for shell in ALL_SHELLS {
        let block = platform_context_section(Platform::Windows, shell, BASH);
        assert!(
            block.contains("## Platform — Windows"),
            "{shell:?} must name the OS: {block}"
        );
        assert!(
            block.contains("`\\` is this machine's path separator"),
            "{shell:?} must state the separator, not imply it: {block}"
        );
    }
}

/// A harness nobody has run on Windows must not be handed the bash block's
/// promise that `&&` and pipes work — that promise is the declaration, and
/// `Unknown` is the absence of one.
#[test]
fn an_undeclared_shell_promises_nothing_about_syntax() {
    let block = platform_context_section(Platform::Windows, WindowsAgentShell::Unknown, BASH);
    assert!(
        !block.contains("IS runnable"),
        "an unverified harness must not be told a POSIX command runs here: {block}"
    );
    assert!(
        !block.contains("own command tool") || !block.contains("run in PowerShell"),
        "an unverified harness must not be told its shell either: {block}"
    );
}

/// The prohibition and the wrapper are one instruction. Five shipped workflow
/// templates tell the agent to run `{{test_command}}` itself, so an agent whose
/// shell cannot parse a POSIX body has been told what not to do and — without
/// this — given no way to comply. It then skips the check or translates anyway,
/// which is the failure the block exists to prevent.
#[test]
fn a_non_bash_agent_can_run_a_quoted_command_without_rewriting_it() {
    for shell in [WindowsAgentShell::PowerShell, WindowsAgentShell::Unknown] {
        let block = platform_context_section(Platform::Windows, shell, BASH);
        assert!(
            block.contains("-lc '<the command exactly as quoted above>'"),
            "{shell:?} must be given an invocation that leaves the body alone: {block}"
        );
        assert!(
            block.contains(BASH.unwrap()),
            "{shell:?} must get the resolved interpreter, not a name to search for: {block}"
        );
        assert!(
            block.contains("not translating"),
            "{shell:?} must be told the wrapper is not the rewrite it was forbidden: {block}"
        );
    }
}

/// A Windows box whose Git install could not be located still gets a
/// followable instruction. What it must never get is a plausible literal: the
/// resolver exists because the install root is not guessable, so a hardcoded
/// `C:\Program Files\…` would be wrong on every machine that needed resolving.
#[test]
fn an_unresolved_interpreter_is_described_rather_than_invented() {
    let block = platform_context_section(Platform::Windows, WindowsAgentShell::PowerShell, None);
    assert!(
        block.contains("-lc"),
        "the wrapper shape survives an unresolved path: {block}"
    );
    assert!(
        !block.contains("C:\\"),
        "an unresolved path must not be filled in with a guess: {block}"
    );
}

/// The wrapper is dead weight for an agent whose own tool is already that
/// interpreter, and the block is paid for on every turn of every step.
#[test]
fn a_git_bash_agent_gets_no_wrapper() {
    let block = platform_context_section(Platform::Windows, WindowsAgentShell::GitBash, BASH);
    assert!(
        !block.contains("-lc"),
        "an agent already running bash needs no wrapper: {block}"
    );
}

#[test]
fn a_git_bash_agent_is_told_a_posix_body_runs_as_written() {
    let block = platform_context_section(Platform::Windows, WindowsAgentShell::GitBash, BASH);
    assert!(
        block.contains("Git for") && block.contains("bash"),
        "the agent must be told which interpreter runs a POSIX body: {block}"
    );
    assert!(
        block.contains("IS runnable"),
        "the agent must be told a POSIX command runs here at all: {block}"
    );
}

/// The block corrects a default; on Linux and macOS the default is already
/// right, so a render on either is byte-for-byte what it was before this
/// module existed.
#[test]
fn a_posix_render_is_unchanged() {
    for platform in [Platform::Linux, Platform::MacOS] {
        for shell in ALL_SHELLS {
            assert_eq!(
                platform_context_section(platform, shell, BASH),
                "",
                "{platform} needs no correction"
            );
            let placed = place_platform_context(Some(platform), shell, BASH, "do the work");
            assert_eq!(placed.bound, "", "{platform} needs no correction");
            assert_eq!(
                render(&placed, "do the work"),
                "do the work",
                "{platform} must render exactly the prompt it renders today"
            );
        }
    }
}

/// A machine whose OS the port could not name is not a machine assumed POSIX —
/// it is one the block says nothing about, whatever the template asked for.
#[test]
fn an_unresolved_platform_places_nothing_either_way() {
    for template in ["do the work", "intro\n\n{{platform_context}}\n\noutro"] {
        let placed = place_platform_context(None, WindowsAgentShell::PowerShell, BASH, template);
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

    let placed = place_platform_context(
        Some(Platform::Windows),
        WindowsAgentShell::PowerShell,
        BASH,
        template,
    );
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

    let placed = place_platform_context(
        Some(Platform::Windows),
        WindowsAgentShell::PowerShell,
        BASH,
        template,
    );
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
        let placed = place_platform_context(
            Some(Platform::Windows),
            WindowsAgentShell::PowerShell,
            BASH,
            template,
        );
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
