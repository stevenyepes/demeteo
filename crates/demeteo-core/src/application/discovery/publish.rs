//! Opening the integration MR for a Discovery's decomposition
//! (`docs/PRD_DISCOVERY.md` §9, V55).
//!
//! The only caller is the Tauri command this backs — never
//! `discovery::close`, and never at branch-creation time.

use crate::domain::ids::DiscoveryId;
use crate::domain::models::{
    integration_pr_body, missing_base_branch_refusal, no_merged_ticket_refusal, MrInfo,
    PublishOptions,
};
use crate::ports::discovery::DiscoveryPatch;
use crate::state::AppContext;

/// Open (or replay) the integration MR from a Discovery's `base_branch` into
/// the project's default branch.
///
/// Both refusals and the idempotency short-circuit are decided in `domain/`
/// and applied here in order, before [`AppContext::mr_publisher`] is ever
/// touched — the ordering `tests/application/discovery/publish.rs`'s
/// panicking fake `MrPublisher` exists to prove.
pub async fn publish_integration_mr(ctx: &AppContext, id: &DiscoveryId) -> Result<MrInfo, String> {
    let discovery = super::load(ctx, id)?;
    if let Some(url) = discovery.integration_mr_url.clone() {
        return Ok(MrInfo {
            url,
            state: discovery.integration_mr_state.clone().unwrap_or_default(),
            number: 0,
            provider_kind: String::new(),
            provider_host: String::new(),
        });
    }
    let Some(base_branch) = discovery.base_branch.clone() else {
        return Err(missing_base_branch_refusal(&discovery).unwrap_or_default());
    };

    let tickets = ctx.tickets.list_for_discovery(id)?;
    let (nodes, _) = crate::application::tickets::nodes_for(&tickets, &*ctx.features)?;
    if let Some(reason) = no_merged_ticket_refusal(&nodes) {
        return Err(reason);
    }

    let default_branch = ctx
        .projects
        .get_settings(&discovery.project_id)?
        .unwrap_or_else(crate::adapters::step_executor::setup::fetch_default_settings)
        .worktree_strategy
        .default_branch;
    let body = integration_pr_body(&tickets, &nodes);

    let info = ctx
        .mr_publisher
        .publish_branch_mr(
            discovery.project_id.as_str(),
            &base_branch,
            &default_branch,
            PublishOptions {
                title: None,
                body: Some(body),
                draft: false,
                target_branch: None,
            },
        )
        .await?;

    ctx.discoveries.update(
        id,
        &DiscoveryPatch {
            integration_mr_url: Some(Some(info.url.clone())),
            integration_mr_state: Some(Some(info.state.clone())),
            ..Default::default()
        },
        crate::paths::now_ms(),
    )?;

    Ok(info)
}
