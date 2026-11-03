//! A desktop managing a `demeteo-runner`'s Hub pairing over SSH: each call is
//! one of the runner's `hub_*` control methods (`crates/demeteo-runner/src/rpc/hub.rs`),
//! answered in the spelling of [`crate::domain::hub_pairing::runner_view`].
//!
//! The runner decides everything — whether it can pair, what its policy is,
//! what it logged — so a runner's own refusal is returned unchanged: it is
//! the runner's user who has to act on it, and the seal hint it carries is
//! written for them. The one answer translated is a runner predating a method
//! ([`runner_lacks_method`]), which comes back as `None` so the caller can
//! name the version gap instead of showing a dispatcher error.

use serde::de::DeserializeOwned;
use serde_json::{json, Value};

use super::rpc::remote_rpc_via;
use crate::domain::hub_pairing::device::DeviceTicket;
use crate::domain::hub_pairing::runner_view::{runner_lacks_method, HubStatusView, PairStateView};
use crate::domain::hub_pairing::HubStatus;
use crate::domain::hub_request::log::MAX_LOG_PAGE;
use crate::domain::hub_request::{HubLocalPolicy, HubRequestLogRow};
use crate::ports::db::AppSettingsRepository;
use crate::ports::execution::ExecutionPort;
use crate::state::AppContext;

/// The two ports a call reads, so a test drives it without an [`AppContext`].
pub struct RunnerHubRpc<'a> {
    pub exec: &'a dyn ExecutionPort,
    pub settings: &'a dyn AppSettingsRepository,
}

impl<'a> RunnerHubRpc<'a> {
    pub fn of(ctx: &'a AppContext) -> Self {
        Self {
            exec: &*ctx.exec,
            settings: &*ctx.app_settings,
        }
    }

    async fn answer(&self, machine_id: &str, method: &str, params: Value) -> Result<Value, String> {
        remote_rpc_via(self.exec, self.settings, machine_id, method, params).await
    }

    async fn call<T: DeserializeOwned>(
        &self,
        machine_id: &str,
        method: &str,
        params: Value,
    ) -> Result<T, String> {
        parse(method, self.answer(machine_id, method, params).await?)
    }

    /// `Ok(None)` when the runner predates `method`.
    async fn call_if_known<T: DeserializeOwned>(
        &self,
        machine_id: &str,
        method: &str,
        params: Value,
    ) -> Result<Option<T>, String> {
        match self.answer(machine_id, method, params).await {
            Ok(value) => parse(method, value).map(Some),
            Err(error) if runner_lacks_method(&error) => Ok(None),
            Err(error) => Err(error),
        }
    }

    pub async fn pair_begin(
        &self,
        machine_id: &str,
        hub_url: &str,
    ) -> Result<DeviceTicket, String> {
        self.call(machine_id, "hub_pair_begin", json!({ "hub_url": hub_url }))
            .await
    }

    pub async fn pair_state(&self, machine_id: &str) -> Result<PairStateView, String> {
        self.call(machine_id, "hub_pair_state", json!({})).await
    }

    pub async fn pair_cancel(&self, machine_id: &str) -> Result<PairStateView, String> {
        self.call(machine_id, "hub_pair_cancel", json!({})).await
    }

    /// `Ok(None)` for a runner from before Hub pairing.
    pub async fn status(&self, machine_id: &str) -> Result<Option<HubStatusView>, String> {
        self.call_if_known(machine_id, "hub_status", json!({}))
            .await
    }

    pub async fn disconnect(&self, machine_id: &str) -> Result<HubStatus, String> {
        self.call(machine_id, "hub_disconnect", json!({})).await
    }

    pub async fn policy_get(&self, machine_id: &str) -> Result<HubLocalPolicy, String> {
        self.call(machine_id, "hub_policy_get", json!({})).await
    }

    /// Answers the policy the runner stored, which is what is in force there.
    pub async fn policy_set(
        &self,
        machine_id: &str,
        policy: &HubLocalPolicy,
    ) -> Result<HubLocalPolicy, String> {
        self.call(machine_id, "hub_policy_set", json!({ "policy": policy }))
            .await
    }

    /// Newest first; `Ok(None)` for a runner from before its activity log.
    pub async fn request_log(
        &self,
        machine_id: &str,
        limit: usize,
    ) -> Result<Option<Vec<HubRequestLogRow>>, String> {
        self.call_if_known(
            machine_id,
            "hub_request_log",
            json!({ "limit": limit.min(MAX_LOG_PAGE) }),
        )
        .await
    }
}

fn parse<T: DeserializeOwned>(method: &str, value: Value) -> Result<T, String> {
    serde_json::from_value(value)
        .map_err(|e| format!("the runner's answer to {method} did not parse: {e}"))
}

#[cfg(test)]
#[path = "../../../tests/application/remote_runs/runner_hub.rs"]
mod tests;
