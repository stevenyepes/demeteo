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
