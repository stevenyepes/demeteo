//! The [`CacheReclaimPort`] every seed guard reaches: the same sweep startup
//! runs, deleting only what [`crate::domain::cache_sweep`] judges leaked.

use std::sync::OnceLock;

use async_trait::async_trait;

use crate::ports::cache_reclaim::CacheReclaimPort;
use crate::state::AppContext;

pub struct SweepReclaim(AppContext);

impl SweepReclaim {
    pub fn new(ctx: AppContext) -> Self {
        Self(ctx)
    }
}

#[async_trait]
impl CacheReclaimPort for SweepReclaim {
    async fn reclaim(&self) -> Result<(), String> {
        let report = super::cache_sweep::sweep_feature_caches(&self.0, false).await?;
        report.log();
        Ok(())
    }
}

/// [`SweepReclaim`] for a port that must exist before the [`AppContext`] it
/// sweeps with: the step executor is built first, because the context holds
/// it. Until [`bind`](Self::bind) runs, a reclaim fails, and the seed guard
/// treats that as nothing freed.
#[derive(Default)]
pub struct DeferredReclaim(OnceLock<SweepReclaim>);

impl DeferredReclaim {
    pub fn bind(&self, ctx: AppContext) {
        let _ = self.0.set(SweepReclaim::new(ctx));
    }
}

#[async_trait]
impl CacheReclaimPort for DeferredReclaim {
    async fn reclaim(&self) -> Result<(), String> {
        match self.0.get() {
            Some(sweep) => sweep.reclaim().await,
            None => Err("the cache sweep is not bound yet".to_string()),
        }
    }
}
