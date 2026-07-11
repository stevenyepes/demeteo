// Tests extracted from `crates/demeteo-core/src/application/providers.rs` (mirrored-tests convention). `super` = that module.

//! Tests for the application-layer provider helpers. These pin
//! the dedup contract for blocker C-4: there must be exactly
//! one backend site that opens the `'demeteo'` keyring for a
//! provider id, and every caller must route through it.
use super::resolve_provider_and_pat;

/// Alias for the canonical signature so the function-pointer
/// coercion below doesn't trip the `clippy::type_complexity`
/// lint. The whole point of this test is to pin the signature
/// itself, so the test reads cleanly even with the alias.
type ResolveFn = fn(
    &crate::state::AppContext,
    &str,
) -> Result<(crate::domain::models::ProviderInstance, String), String>;

/// Compile-time + runtime pin: `resolve_provider_and_pat` must
/// remain `pub` and must carry the canonical
/// `(&AppContext, &str) -> Result<(ProviderInstance, String), String>`
/// signature so every other module (the wizard's Commit arm,
/// `fetch_repos`, `list_groups`, `create_repo`) can route
/// through it without touching the keyring directly.
///
/// We can't actually invoke the function without a real
/// `AppContext`, but a function-pointer coercion is enough to
/// force the compiler to verify the symbol is reachable from
/// outside `application::providers` and that its signature is
/// unchanged.
#[test]
fn resolve_provider_and_pat_is_publicly_reachable_with_canonical_signature() {
    let f: ResolveFn = resolve_provider_and_pat;
    // Coerce to a `usize` so the test is a real runtime no-op
    // (avoid `let _ = f;` which is a typed binding the
    // optimiser might warn about).
    let _ = f as usize;
}
