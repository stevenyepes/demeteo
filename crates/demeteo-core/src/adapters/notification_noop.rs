use crate::ports::notification::{DomainEvent, NotificationPort};

/// A `NotificationPort` that drops every event. Used by the headless
/// runner (docs/REMOTE_EXECUTION.md M1) until a real webhook/email
/// adapter lands (M6.3); also useful in tests that don't care about the
/// UI-event side channel.
pub struct NoopNotificationAdapter;

impl NotificationPort for NoopNotificationAdapter {
    fn emit(&self, _event: &DomainEvent) -> Result<(), String> {
        Ok(())
    }
}
