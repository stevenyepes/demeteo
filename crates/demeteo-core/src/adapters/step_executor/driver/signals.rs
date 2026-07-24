//! Memory-signal emission from `ExecutionDriver`.

use super::ExecutionDriver;

impl ExecutionDriver {
    /// Capture a raw run observation for the memory agent's queue. Best-effort:
    /// an empty body, a missing feature row, or an enqueue failure is silently
    /// swallowed so signal capture never perturbs the run itself.
    pub(crate) fn capture_signal(
        &self,
        step_execution_id: Option<String>,
        kind: crate::domain::memory::SignalKind,
        content: impl Into<String>,
    ) {
        let content = content.into();
        if content.trim().is_empty() {
            return;
        }
        let project_id = match self.features.get(&self.f_id) {
            Ok(Some(f)) => f.project_id,
            _ => return,
        };
        let now = crate::paths::now_ms();
        let signal = crate::domain::memory::MemorySignal {
            id: format!("ms-{}", crate::paths::new_id()),
            project_id,
            feature_id: self.f_id_str.clone(),
            step_execution_id,
            kind,
            content,
            created_at: now,
            processed_at: None,
            attempts: 0,
        };
        let _ = self.signals.enqueue(signal);
    }
}
