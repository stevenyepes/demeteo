use crate::domain::memory::MemorySignal;

/// Append-only queue of raw run observations. Producers (gate/failure/agent step
/// handlers) enqueue synchronously and cheaply; the background memory worker is
/// the sole consumer.
pub trait MemorySignalsPort: Send + Sync {
    fn enqueue(&self, signal: MemorySignal) -> Result<(), String>;
    /// Oldest-first unprocessed signals whose `attempts < max_attempts`.
    fn take_unprocessed(
        &self,
        limit: usize,
        max_attempts: i64,
    ) -> Result<Vec<MemorySignal>, String>;
    fn mark_processed(&self, ids: &[String], now: i64) -> Result<(), String>;
    /// Increment the retry counter for signals whose processing failed.
    fn bump_attempts(&self, ids: &[String]) -> Result<(), String>;
}
