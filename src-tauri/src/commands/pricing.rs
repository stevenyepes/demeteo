//! Tauri commands that surface the model pricing table to the UI.

use serde::Serialize;
use tauri::State;

use crate::state::AppContext;

#[derive(Serialize)]
pub struct ModelPricingDto {
    pub model: String,
    pub input_per_million: f64,
    pub output_per_million: f64,
}

#[tauri::command]
pub fn pricing_list(ctx: State<'_, AppContext>) -> Vec<ModelPricingDto> {
    ctx.pricing
        .known_models()
        .into_iter()
        .filter_map(|m| {
            ctx.pricing.price_for(&m).map(|p| ModelPricingDto {
                model: m,
                input_per_million: p.input_per_million,
                output_per_million: p.output_per_million,
            })
        })
        .collect()
}

#[tauri::command]
pub fn pricing_for(model: String, ctx: State<'_, AppContext>) -> Option<ModelPricingDto> {
    ctx.pricing.price_for(&model).map(|p| ModelPricingDto {
        model,
        input_per_million: p.input_per_million,
        output_per_million: p.output_per_million,
    })
}
