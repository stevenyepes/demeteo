//! The production [`HarnessModelProbe`]: the agent registry's availability
//! probe and [`listed_models`], both against [`LOCAL_MACHINE`]. On a desktop
//! the router sends that to the local subprocess adapter, and a runner has no
//! other, so both instance kinds probe the machine they run on through this
//! one path.

use std::sync::Arc;

use async_trait::async_trait;

use super::registry::AgentRegistry;
use crate::application::agent_probe::listed_models;
use crate::domain::ids::LOCAL_MACHINE;
use crate::domain::models::{Availability, ConfigOptionValue, ModelSource};
use crate::ports::execution::ExecutionPort;
use crate::ports::hub::HarnessModelProbe;

pub struct LocalModelProbe {
    registry: Arc<AgentRegistry>,
    exec: Arc<dyn ExecutionPort>,
}

impl LocalModelProbe {
    pub fn new(registry: Arc<AgentRegistry>, exec: Arc<dyn ExecutionPort>) -> Self {
        Self { registry, exec }
    }
}

#[async_trait]
impl HarnessModelProbe for LocalModelProbe {
    /// Forced past the registry's session cache: a harness installed after
    /// the process started must reach the next probe, and a runner has no
    /// Settings button to re-check with.
    async fn availability(&self, kinds: &[String]) -> Vec<(String, Availability)> {
        let kinds: Vec<&str> = kinds.iter().map(String::as_str).collect();
        self.registry
            .availability_of(&kinds, self.exec.as_ref(), LOCAL_MACHINE, true)
            .await
            .into_iter()
            .map(|(kind, availability)| (kind.to_owned(), availability))
            .collect()
    }

    async fn models(&self, kind: &str) -> Option<(Vec<ConfigOptionValue>, ModelSource)> {
        Some(listed_models(self.exec.as_ref(), &self.registry, LOCAL_MACHINE, kind).await)
    }
}
