use axum::{Extension, Json};
use std::sync::Arc;
use crate::api::models::response::LanguageInfo;
use crate::execution::languages::registry::LanguageRegistry;

pub async fn list_languages(
    Extension(registry): Extension<Arc<LanguageRegistry>>,
) -> Json<Vec<LanguageInfo>> {
    Json(registry.list())
}