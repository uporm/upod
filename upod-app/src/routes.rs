use crate::handler::sandbox_management::{
    create_sandbox, delete_sandbox, get_sandbox, list_sandboxes, pause_sandbox, resume_sandbox,
};
use crate::service::sandbox_lifecycle::{
    start_expiration_cleanup_task, start_sandbox_store_sync_task,
};
use axum::routing::delete;
use axum::{
    Router,
    routing::{get, post},
};

pub fn router() -> Router {
    start_sandbox_store_sync_task();
    start_expiration_cleanup_task();
    Router::new()
        .route("/v1/sandboxes", get(list_sandboxes))
        .route("/v1/sandboxes/{sandbox_id}", get(get_sandbox))
        .route("/v1/sandboxes", post(create_sandbox))
        .route("/v1/sandboxes/{sandbox_id}", delete(delete_sandbox))
        .route("/v1/sandboxes/{sandbox_id}/pause", post(pause_sandbox))
        .route("/v1/sandboxes/{sandbox_id}/resume", post(resume_sandbox))
}
