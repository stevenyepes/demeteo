//! Tauri surface for Ask storage/lifecycle (`docs/PRD_DISCOVERY.md`-adjacent,
//! but not itself a Discovery — see [`demeteo_core::application::ask`]).
//!
//! Thin by construction: every decision below is made in
//! [`demeteo_core::application::ask`].

use crate::domain::ids::AskThreadId;
use crate::domain::models::{AskMessage, AskThread, EffortLevel};
use crate::state::AppContext;
use demeteo_core::application::ask::{self, turn, AskThreadDetail, NewAskThread, NodeResolution};
use demeteo_core::ports::ask::AskThreadPatch;
use serde::Deserialize;
use tauri::{Emitter, State};

#[tauri::command]
pub fn ask_create(ctx: State<'_, AppContext>, input: NewAskThread) -> Result<AskThread, String> {
    ask::create(&ctx, input)
}

#[tauri::command]
pub fn ask_list(ctx: State<'_, AppContext>, project_id: String) -> Result<Vec<AskThread>, String> {
    ask::list_for_project(&ctx, &project_id)
}

#[tauri::command]
pub fn ask_load(ctx: State<'_, AppContext>, thread_id: String) -> Result<AskThreadDetail, String> {
    ask::load(&ctx, &AskThreadId::from(thread_id))
}

/// Whether this thread has a turn running right now — what a surface that
/// mounted mid-turn asks once, on select, since `ask_turn_status` reports
/// transitions and never repeats one already made.
#[tauri::command]
pub fn ask_running(ctx: State<'_, AppContext>, thread_id: String) -> Result<bool, String> {
    Ok(ask::turn_running(&ctx, &AskThreadId::from(thread_id)))
}

#[tauri::command]
pub fn ask_rename(
    ctx: State<'_, AppContext>,
    thread_id: String,
    title: String,
) -> Result<AskThread, String> {
    ask::rename(&ctx, &AskThreadId::from(thread_id), &title)
}

#[tauri::command]
pub fn ask_delete(ctx: State<'_, AppContext>, thread_id: String) -> Result<(), String> {
    ask::delete(&ctx, &AskThreadId::from(thread_id))
}

/// Distinguishes a key that was present-and-`null` from one that was absent,
/// which serde's derive on `Option<Option<T>>` cannot: `Option::deserialize`
/// maps JSON `null` to `None`, so both spellings collapse to the same value
/// and every `is_some()` consumer downstream reads "leave alone".
///
/// Paired with `#[serde(default)]` — absence never reaches this function, so
/// the `Some` it always returns means exactly "the key was there".
fn present<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

/// What a settings change may set. `agent_kind` is absent on purpose — a
/// thread's harness is fixed at creation, matching `AskThreadPatch`.
///
/// `model` and `effort` carry [`AskThreadPatch`]'s double `Option` all the way
/// out to JSON, which is the only place in `commands/` that happens — its
/// other nullable fields are built in Rust. Hence [`present`]; `network` is
/// non-nullable and takes the plain derive.
#[derive(Debug, Deserialize)]
pub struct AskSettingsPatch {
    #[serde(default, deserialize_with = "present")]
    pub model: Option<Option<String>>,
    #[serde(default, deserialize_with = "present")]
    pub effort: Option<Option<EffortLevel>>,
    #[serde(default)]
    pub network: Option<bool>,
}

#[tauri::command]
pub fn ask_update_settings(
    ctx: State<'_, AppContext>,
    thread_id: String,
    patch: AskSettingsPatch,
) -> Result<AskThread, String> {
    ask::update_settings(
        &ctx,
        &AskThreadId::from(thread_id),
        AskThreadPatch {
            model: patch.model,
            effort: patch.effort,
            network: patch.network,
            ..Default::default()
        },
    )
}

/// Send the user's turn and start the agent's, mirroring
/// `discovery_send_turn` (`commands::discovery`) exactly. The returned
/// message is the user's own, already persisted — the assistant's answer
/// arrives later over [`turn`]'s three events.
#[tauri::command]
pub async fn ask_send_turn(
    ctx: State<'_, AppContext>,
    app: tauri::AppHandle,
    thread_id: String,
    text: String,
) -> Result<AskMessage, String> {
    turn::send(
        &ctx,
        &AskThreadId::from(thread_id),
        &text,
        move |event, payload| {
            let _ = app.emit(event, payload);
        },
    )
    .await
}

#[tauri::command]
pub async fn ask_resolve_node(
    ctx: State<'_, AppContext>,
    thread_id: String,
    message_id: String,
    node_id: String,
) -> Result<NodeResolution, String> {
    ask::resolve_node(&ctx, &AskThreadId::from(thread_id), &message_id, &node_id).await
}

#[cfg(test)]
#[path = "../../tests/infrastructure/ask.rs"]
mod tests;
