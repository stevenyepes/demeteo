use globset::{Glob, GlobMatcher};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::policy::{ActionKind, AgentAction, PolicyDecision, PolicyRule};

#[derive(Debug)]
pub enum CompiledError {
    InvalidGlob(String),
}

impl std::fmt::Display for CompiledError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CompiledError::InvalidGlob(p) => write!(f, "Invalid glob pattern: {}", p),
        }
    }
}

impl std::error::Error for CompiledError {}

pub struct CompiledRule {
    pub raw: PolicyRule,
    pub path_matcher: Option<Arc<GlobMatcher>>,
    pub cmd_prefix: Option<String>,
}

impl CompiledRule {
    pub fn compile(rule: PolicyRule) -> Result<Self, CompiledError> {
        let (path_matcher, cmd_prefix) = match rule.action {
            ActionKind::Read | ActionKind::Edit | ActionKind::Write => {
                let glob = Glob::new(&rule.target_pattern)
                    .map_err(|_| CompiledError::InvalidGlob(rule.target_pattern.clone()))?;
                let matcher = glob.compile_matcher();
                (Some(Arc::new(matcher)), None)
            }
            ActionKind::RunBash => {
                let trimmed = rule.target_pattern.trim().to_string();
                let prefix = trimmed.trim_end_matches('*').trim_end().to_string();
                (None, Some(prefix))
            }
        };
        Ok(Self { raw: rule, path_matcher, cmd_prefix })
    }

    pub fn matches(&self, action: &AgentAction) -> bool {
        if self.raw.action != action.kind() || !self.raw.enabled {
            return false;
        }
        match action {
            AgentAction::Read { path } | AgentAction::Edit { path, .. } | AgentAction::Write { path, .. } => {
                self.path_matcher.as_ref().is_some_and(|g| g.is_match(path))
            }
            AgentAction::RunBash { cmd } => {
                let trimmed = cmd.trim_start();
                match &self.cmd_prefix {
                    Some(prefix) if prefix.is_empty() => true,
                    Some(prefix) => trimmed.starts_with(prefix.as_str()),
                    None => false,
                }
            }
        }
    }

    /// True if this rule is tagged as a scope-override escape hatch.
    pub fn is_scope_override(&self) -> bool {
        self.raw.source == "scope_override"
    }
}

pub struct PolicyEngine;

impl PolicyEngine {
    /// Evaluate a single action against the full rule set, with an optional
    /// scope fence pre-rule. The fence is consulted first; if it returns a
    /// decision, that decision wins (unless an override rule is present).
    ///
    /// If `fence` is `None`, behavior is identical to the prior single-rule-loop
    /// version (preserved for the policy_decorator tests that don't need scope
    /// fencing).
    pub fn evaluate(
        action: &AgentAction,
        rules: &[CompiledRule],
        fence: Option<&ScopeFence>,
    ) -> PolicyDecision {
        // 1. Scope fence pre-rule, if present and this is a path action.
        if let Some(f) = fence {
            if let Some(decision) = f.check(action) {
                // Even with a fence rejection, a rule with source=scope_override
                // can yield. The fence yields only when an enabled override rule
                // is also a match for the same action+path shape.
                if matches!(decision, PolicyDecision::Reject { .. }) {
                    let has_override = rules.iter().any(|r| {
                        r.is_scope_override() && r.matches(action)
                    });
                    if !has_override {
                        return decision;
                    }
                }
            }
        }

        // 2. User rules, first match wins.
        for rule in rules {
            if rule.matches(action) {
                return rule.raw.decision.clone();
            }
        }
        PolicyDecision::EscalateToUser
    }
}

/// A path-only pre-rule that confines file actions to a thread's sandbox.
/// For `Read`, `Edit`, and `Write` actions, the target is canonicalized and
/// rejected if it falls outside the sandbox. `RunBash` is intentionally
/// outside the fence's jurisdiction — it defers to the existing bash-prefix
/// policy.
#[derive(Debug, Clone)]
pub struct ScopeFence {
    pub sandbox_path: PathBuf,
}

impl ScopeFence {
    pub fn new(sandbox_path: PathBuf) -> Self {
        Self { sandbox_path }
    }

    /// Returns `Some(decision)` if the fence has an opinion, `None` if the
    /// action should defer to user rules (bash actions).
    pub fn check(&self, action: &AgentAction) -> Option<PolicyDecision> {
        let target_path = match action {
            AgentAction::Read { path }
            | AgentAction::Edit { path, .. }
            | AgentAction::Write { path, .. } => path,
            AgentAction::RunBash { .. } => return None,
        };

        let resolved = match resolve_path(&self.sandbox_path, target_path) {
            Ok(p) => p,
            Err(_) => {
                return Some(PolicyDecision::Reject {
                    reason: "path resolution failed".into(),
                });
            }
        };

        if !is_within(&self.sandbox_path, &resolved) {
            return Some(PolicyDecision::Reject {
                reason: format!("path '{}' is outside thread scope", target_path),
            });
        }

        None
    }
}

/// Canonicalize a target path against the sandbox. For absolute paths, we
/// canonicalize the target as given. For relative paths, we join with the
/// sandbox first. We fail closed: any canonicalization error (path doesn't
/// exist, invalid component, IO error) returns `Err`.
pub fn resolve_path(sandbox: &Path, target: &str) -> Result<PathBuf, String> {
    let candidate = Path::new(target);
    let joined = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        sandbox.join(candidate)
    };
    // dunce::canonicalize strips Windows `\\?\` prefixes, but on Linux it's a
    // pass-through to std::fs::canonicalize. Use std::fs directly to avoid
    // adding a new dependency.
    std::fs::canonicalize(&joined).map_err(|e| format!("canonicalize({}): {}", joined.display(), e))
}

/// True if `child` is the sandbox itself or a descendant. Compares
/// canonicalized components so trailing separators and `..` don't fool us.
pub fn is_within(sandbox: &Path, child: &Path) -> bool {
    let s = match std::fs::canonicalize(sandbox) {
        Ok(p) => p,
        Err(_) => return false,
    };
    let c = match std::fs::canonicalize(child) {
        Ok(p) => p,
        Err(_) => return false,
    };
    if s == c {
        return true;
    }
    c.starts_with(&s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::policy::{ActionKind, AgentAction, PolicyDecision, PolicyRule};

    fn rule(action: ActionKind, pattern: &str, decision: PolicyDecision) -> CompiledRule {
        let r = PolicyRule {
            id: "r".into(),
            machine_id: "m".into(),
            action,
            target_pattern: pattern.into(),
            decision,
            enabled: true,
            source: "user".into(),
            sort_order: 0,
        };
        CompiledRule::compile(r).expect("compile")
    }

    fn rule_with_source(
        action: ActionKind,
        pattern: &str,
        decision: PolicyDecision,
        source: &str,
    ) -> CompiledRule {
        let r = PolicyRule {
            id: "r".into(),
            machine_id: "m".into(),
            action,
            target_pattern: pattern.into(),
            decision,
            enabled: true,
            source: source.into(),
            sort_order: 0,
        };
        CompiledRule::compile(r).expect("compile")
    }

    fn read(path: &str) -> AgentAction {
        AgentAction::Read { path: path.into() }
    }
    fn edit(path: &str, content: &str) -> AgentAction {
        AgentAction::Edit { path: path.into(), content: content.into() }
    }
    fn write(path: &str, content: &str) -> AgentAction {
        AgentAction::Write { path: path.into(), content: content.into() }
    }
    fn bash(cmd: &str) -> AgentAction {
        AgentAction::RunBash { cmd: cmd.into() }
    }

    #[test]
    fn read_matches_glob() {
        let r = rule(ActionKind::Read, "src/**/*.rs", PolicyDecision::Approve);
        assert!(r.matches(&read("src/main.rs")));
        assert!(r.matches(&read("src/foo/bar.rs")));
        assert!(!r.matches(&read("src/main.ts")));
    }

    #[test]
    fn edit_matches_specific_path() {
        let r = rule(ActionKind::Edit, "/abs/path/file.rs", PolicyDecision::EscalateToUser);
        assert!(r.matches(&edit("/abs/path/file.rs", "x")));
        assert!(!r.matches(&edit("/abs/path/other.rs", "x")));
    }

    #[test]
    fn bash_prefix_with_trailing_star() {
        let r = rule(ActionKind::RunBash, "git status*", PolicyDecision::Approve);
        assert!(r.matches(&bash("git status")));
        assert!(r.matches(&bash("git status --short")));
        assert!(r.matches(&bash("  git status -sb")));
        assert!(!r.matches(&bash("git commit -m foo")));
    }

    #[test]
    fn disabled_rule_never_matches() {
        let mut r = rule(ActionKind::Read, "**/*", PolicyDecision::Approve);
        r.raw.enabled = false;
        assert!(!r.matches(&read("anything.txt")));
    }

    #[test]
    fn first_match_wins() {
        let r1 = rule(ActionKind::Read, "secrets/**", PolicyDecision::Reject { reason: "nope".into() });
        let r2 = rule(ActionKind::Read, "**/*.txt", PolicyDecision::Approve);
        let decision = PolicyEngine::evaluate(&read("secrets/a.txt"), &[r1, r2], None);
        assert!(matches!(decision, PolicyDecision::Reject { .. }));
    }

    #[test]
    fn no_match_escalates() {
        let r = rule(ActionKind::Read, "**/*.rs", PolicyDecision::Approve);
        let d = PolicyEngine::evaluate(&write("foo.txt", "x"), &[r], None);
        assert_eq!(d, PolicyDecision::EscalateToUser);
    }

    #[test]
    fn empty_bash_prefix_matches_anything() {
        let r = rule(ActionKind::RunBash, "*", PolicyDecision::EscalateToUser);
        assert!(r.matches(&bash("anything goes")));
    }

    #[test]
    fn write_matches_glob() {
        let r = rule(ActionKind::Write, "**/.env", PolicyDecision::Reject { reason: "secret".into() });
        assert!(r.matches(&write("/home/u/proj/.env", "X")));
        assert!(!r.matches(&write("/home/u/proj/main.rs", "X")));
    }

    // -----------------------------------------------------------------
    // ScopeFence
    // -----------------------------------------------------------------

    /// Create a unique temp dir for the fence test and register cleanup.
    fn make_tmp(label: &str) -> PathBuf {
        let base = std::env::temp_dir().join(format!(
            "demeteo_fence_{}_{}",
            label,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&base).unwrap();
        std::fs::create_dir_all(base.join("inside")).unwrap();
        std::fs::write(base.join("inside").join("file.txt"), "ok").unwrap();
        base
    }

    #[test]
    fn fence_allows_path_inside_sandbox() {
        let sandbox = make_tmp("allow");
        let target = sandbox.join("inside").join("file.txt");
        let fence = ScopeFence::new(sandbox.clone());
        assert!(fence.check(&read(target.to_str().unwrap())).is_none());
        let _ = std::fs::remove_dir_all(&sandbox);
    }

    #[test]
    fn fence_rejects_path_outside_sandbox() {
        let sandbox = make_tmp("reject");
        let target = std::env::temp_dir().join("definitely_outside_target.txt");
        // Make sure the target exists so canonicalize succeeds; we just need
        // the path-resolution branch to pass before the within-check rejects.
        std::fs::write(&target, "x").unwrap();
        let fence = ScopeFence::new(sandbox.clone());
        let decision = fence.check(&read(target.to_str().unwrap()));
        assert!(matches!(decision, Some(PolicyDecision::Reject { .. })));
        let _ = std::fs::remove_dir_all(&sandbox);
        let _ = std::fs::remove_file(&target);
    }

    #[test]
    fn fence_rejects_relative_traversal() {
        let sandbox = make_tmp("traverse");
        // Place a marker file one level *above* the sandbox. The relative
        // path "../../marker" joined to the sandbox (which is a single-level
        // dir) canonicalizes to the marker — clearly outside the sandbox.
        let parent = sandbox.parent().unwrap().to_path_buf();
        let marker = parent.join("demeteo_fence_traverse_marker.txt");
        std::fs::write(&marker, "x").unwrap();
        // Inside `sandbox`, the relative path `../demeteo_fence_traverse_marker.txt`
        // resolves to `marker` (outside the sandbox).
        let target_relative = "../demeteo_fence_traverse_marker.txt";
        let fence = ScopeFence::new(sandbox.clone());
        let decision = fence.check(&read(target_relative));
        assert!(
            matches!(decision, Some(PolicyDecision::Reject { .. })),
            "expected Reject for traversal, got {:?}",
            decision
        );
        let _ = std::fs::remove_dir_all(&sandbox);
        let _ = std::fs::remove_file(&marker);
    }

    #[test]
    fn fence_defers_on_bash() {
        let sandbox = make_tmp("bash");
        let fence = ScopeFence::new(sandbox.clone());
        // Bash actions return None from fence.check — the fence is invisible.
        assert!(fence.check(&bash("cat /etc/passwd")).is_none());
        let _ = std::fs::remove_dir_all(&sandbox);
    }

    #[test]
    fn scope_override_yields_to_fence() {
        let sandbox = make_tmp("override");
        // Pick a real existing file outside the sandbox to target.
        let outside = std::env::temp_dir().join("demeteo_fence_override_target.txt");
        std::fs::write(&outside, "x").unwrap();
        let fence = ScopeFence::new(sandbox.clone());
        let outside_str = outside.to_str().unwrap().to_string();

        let rules = vec![rule_with_source(
            ActionKind::Read,
            outside_str.as_str(),
            PolicyDecision::Approve,
            "scope_override",
        )];

        let d = PolicyEngine::evaluate(&read(&outside_str), &rules, Some(&fence));
        assert_eq!(d, PolicyDecision::Approve);

        // Without the override, fence rejects.
        let rules_user = vec![rule_with_source(
            ActionKind::Read,
            outside_str.as_str(),
            PolicyDecision::Approve,
            "user",
        )];
        let d = PolicyEngine::evaluate(&read(&outside_str), &rules_user, Some(&fence));
        assert!(matches!(d, PolicyDecision::Reject { .. }));

        let _ = std::fs::remove_dir_all(&sandbox);
        let _ = std::fs::remove_file(&outside);
    }
}
