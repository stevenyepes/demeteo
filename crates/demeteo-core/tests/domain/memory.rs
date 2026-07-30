// Tests extracted from `crates/demeteo-core/src/domain/memory.rs` (mirrored-tests convention). `super` = that module.

use super::*;

#[test]
fn embedding_blob_roundtrips() {
    let v = vec![0.0f32, 1.0, -2.5, 123.456, 1e-9];
    assert_eq!(blob_to_embedding(&embedding_to_blob(&v)), v);
}

#[test]
fn cosine_identical_is_one_orthogonal_is_zero() {
    let a = vec![1.0f32, 2.0, 3.0];
    assert!((cosine_similarity(&a, &a) - 1.0).abs() < 1e-6);
    assert_eq!(cosine_similarity(&[1.0, 0.0], &[0.0, 1.0]), 0.0);
    // length mismatch / empty are guarded to 0.0
    assert_eq!(cosine_similarity(&[1.0, 0.0], &[1.0]), 0.0);
    assert_eq!(cosine_similarity(&[], &[]), 0.0);
}

#[test]
fn memory_type_str_roundtrips() {
    for t in [
        MemoryType::Convention,
        MemoryType::Lesson,
        MemoryType::Decision,
        MemoryType::Preference,
        MemoryType::Fact,
    ] {
        assert_eq!(MemoryType::from_str(t.as_str()), Some(t));
    }
    assert_eq!(MemoryType::from_str("bogus"), None);
}

// ── Which memories reach the prompt, and in what order ───────────────────────

fn entry(key: &str, confidence: f64, embedding: Option<Vec<f32>>) -> ProjectMemoryEntry {
    ProjectMemoryEntry {
        id: format!("m-{key}"),
        project_id: ProjectId::from("p-1".to_string()),
        key: key.to_string(),
        value: format!("{key} value"),
        source: MemorySource::Agent,
        confidence,
        memory_type: None,
        statement: None,
        embedding,
        embedding_model: None,
        last_used_at: None,
        use_count: 0,
        created_at: 0,
        updated_at: 0,
    }
}

fn keys(selected: &[&ProjectMemoryEntry]) -> Vec<String> {
    selected.iter().map(|m| m.key.clone()).collect()
}

/// The floor is a floor: below it a memory is not offered at any similarity,
/// including a perfect one.
#[test]
fn a_memory_under_the_confidence_floor_is_never_offered() {
    let memories = vec![
        entry("unsure", 0.2, Some(vec![1.0, 0.0])),
        entry("sure", 0.9, Some(vec![1.0, 0.0])),
    ];

    assert_eq!(
        keys(&rank_memories(&memories, &[1.0, 0.0], 0.5, 10)),
        vec!["sure".to_string()]
    );

    let exactly_at = vec![entry("borderline", 0.5, Some(vec![1.0, 0.0]))];
    assert_eq!(
        keys(&rank_memories(&exactly_at, &[1.0, 0.0], 0.5, 10)),
        vec!["borderline".to_string()],
        "the floor is inclusive: a memory *at* min_confidence is offered"
    );
}

/// An un-embedded row is dropped, not scored at zero — otherwise rows that
/// have simply not been embedded yet crowd out ranked ones at the tail of
/// `top_k`.
#[test]
fn a_memory_with_no_embedding_is_dropped_rather_than_scored_at_zero() {
    let memories = vec![
        entry("pending", 1.0, None),
        entry("embedded", 1.0, Some(vec![0.0, 1.0])),
    ];

    // Orthogonal to the query, so its similarity is 0.0 — it still survives,
    // where the un-embedded row does not.
    assert_eq!(
        keys(&rank_memories(&memories, &[1.0, 0.0], 0.0, 10)),
        vec!["embedded".to_string()]
    );
}

/// The score is similarity **× confidence**. `hedged` is a perfect match the
/// distiller was unsure of; `confident` is a weaker match it stood behind. On
/// similarity alone `hedged` would lead, and a prompt would open with the line
/// the system trusts least.
#[test]
fn confidence_weights_the_similarity_rather_than_only_gating_it() {
    let memories = vec![
        entry("hedged", 0.3, Some(vec![1.0, 0.0])),
        entry("confident", 1.0, Some(vec![1.0, 1.0])),
    ];

    assert_eq!(
        keys(&rank_memories(&memories, &[1.0, 0.0], 0.0, 10)),
        vec!["confident".to_string(), "hedged".to_string()],
        "0.707 x 1.0 must outrank 1.0 x 0.3"
    );
}

/// `top_k` truncates *after* ranking, so the prompt gets the best k rather
/// than the first k the repository happened to return.
#[test]
fn top_k_keeps_the_best_not_the_first() {
    let memories = vec![
        entry("weak", 1.0, Some(vec![0.0, 1.0])),
        entry("strong", 1.0, Some(vec![1.0, 0.0])),
        entry("middling", 1.0, Some(vec![1.0, 1.0])),
    ];

    assert_eq!(
        keys(&rank_memories(&memories, &[1.0, 0.0], 0.0, 2)),
        vec!["strong".to_string(), "middling".to_string()]
    );
}

/// The sort is stable and the tie order is observable in the prompt: equal
/// scores keep the repository's order. An unstable sort would make the prompt
/// vary between runs over identical data, which is not a difference anyone
/// could debug.
///
/// Two score groups, large and interleaved, so the tie order has to survive a
/// real partition rather than a slice the sort recognises as already ordered.
#[test]
fn ties_keep_the_order_the_repository_returned() {
    // Within a group these are scaled multiples of each other, and cosine
    // similarity is scale-invariant — so every member of a group ties exactly.
    let memories: Vec<ProjectMemoryEntry> = (0..40)
        .map(|i| {
            let v = 1.0 + i as f32;
            let embedding = if i % 2 == 0 {
                vec![v, 0.0] // similarity 1.0
            } else {
                vec![v, v] // similarity ~0.707
            };
            entry(&format!("m{i:02}"), 0.8, Some(embedding))
        })
        .collect();

    let mut expected: Vec<String> = (0..40).filter(|i| i % 2 == 0).map(fmt_key).collect();
    expected.extend((0..40).filter(|i| i % 2 == 1).map(fmt_key));

    assert_eq!(
        keys(&rank_memories(&memories, &[1.0, 0.0], 0.0, 40)),
        expected
    );
}

fn fmt_key(i: i32) -> String {
    format!("m{i:02}")
}

#[test]
fn an_empty_selection_renders_nothing() {
    assert_eq!(rank_memories(&[], &[1.0], 0.0, 10), Vec::<&_>::new());
    assert_eq!(render_memory_md(&[]), "");
}

// ── The two rendered shapes ──────────────────────────────────────────────────

/// A typed memory leads with its category; an untyped (legacy or manually
/// entered) one leads with its key. Both name the source, because "the agent
/// decided this" and "a human told us this" are different instructions.
#[test]
fn a_typed_memory_leads_with_its_category_and_an_untyped_one_with_its_key() {
    let mut typed = entry("ignored-key", 1.0, None);
    typed.memory_type = Some(MemoryType::Convention);
    typed.statement = Some("always run npm run checks".to_string());
    typed.source = MemorySource::Human;

    let untyped = entry("build-cmd", 1.0, None);

    assert_eq!(
        render_memory_md(&[&typed, &untyped]),
        "- [convention] always run npm run checks (Source: Human)\n\
         - **build-cmd**: build-cmd value (Source: Agent)\n"
    );
}

/// `statement` is the canonical prose and wins when present; `value` is the
/// legacy row's only body.
#[test]
fn the_statement_is_the_body_when_a_row_has_one() {
    let mut with_statement = entry("k", 1.0, None);
    with_statement.statement = Some("the prose form".to_string());

    assert_eq!(
        render_memory_md(&[&with_statement]),
        "- **k**: the prose form (Source: Agent)\n"
    );
    assert_eq!(
        render_memory_md(&[&entry("k", 1.0, None)]),
        "- **k**: k value (Source: Agent)\n"
    );
}
