//! Tauri surface for Ask storage/lifecycle (`docs/PRD_DISCOVERY.md`-adjacent,
//! but not itself a Discovery — see [`demeteo_core::application::ask`]).
//!
//! Thin by construction: every decision below is made in
//! [`demeteo_core::application::ask`]. No turn execution, worktree
//! allocation, or stream events belong here — those are `ask-turn-loop`'s.

use crate::domain::ids::AskThreadId;
use crate::domain::models::AskThread;
use crate::state::AppContext;
use demeteo_core::application::ask::{self, AskThreadDetail, NewAskThread};
use tauri::State;

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
