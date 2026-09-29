use async_trait::async_trait;

/// Free the disk that leaked feature caches hold, so a cache seed about to be
/// refused for want of room can look again.
///
/// A port because the adapter that seeds caches cannot judge which ones leaked:
/// that verdict needs every feature's state, which only the application layer
/// reads. The adapter holds this optionally and, without one, refuses on the
/// first short reading.
#[async_trait]
pub trait CacheReclaimPort: Send + Sync {
    async fn reclaim(&self) -> Result<(), String>;
}
