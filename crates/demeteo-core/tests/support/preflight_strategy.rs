//! A `WorktreeStrategy` builder shared by every preflight test.
//!
//! Lives here rather than beside one of its callers because three test modules
//! need it and the mirrored-tests convention puts each of them under a
//! different parent (`domain::harness_preflight::{commands, report}` and
//! `adapters::step_executor::preflight`). It was copied into all three, so a
//! new field on `WorktreeStrategy` meant three identical edits and the copies
//! were free to drift apart between them.
//!
//! `#[path]`-included by each caller rather than declared once, because the
//! three callers live under different parent modules.

use crate::domain::models::{ScriptVariants, WorktreeStrategy};

fn posix(command: &str) -> ScriptVariants {
    ScriptVariants {
        posix: Some(command.to_string()),
        powershell: None,
    }
}

/// A `WorktreeStrategy` carrying only what the preflight reads.
///
/// Every field is spelled out rather than mutated from a default, so each test
/// states its whole input: which of the three sources a binary comes from is
/// the entire subject of the HB4 tests.
pub(crate) fn strategy(
    prepare: Option<&str>,
    test: Option<&str>,
    harnesses: &[(&str, &str)],
) -> WorktreeStrategy {
    WorktreeStrategy {
        default_branch: "main".to_string(),
        branch_prefix: "demeteo/features/".to_string(),
        test_command: test.map(posix),
        build_command: None,
        coverage_command: None,
        conventions_file: None,
        pr_template: None,
        harnesses: (!harnesses.is_empty()).then(|| {
            harnesses
                .iter()
                .map(|(name, cmd)| (name.to_string(), posix(cmd)))
                .collect()
        }),
        validation_gates: None,
        prepare_command: prepare.map(posix),
        extra_writable_paths: Vec::new(),
    }
}
