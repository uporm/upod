use crate::handler::sandbox_management::{
    create_sandbox, delete_sandbox, get_sandbox_endpoint,
    pause_sandbox, resume_sandbox, get_sandbox, list_sandboxes
};
use crate::service::sandbox_lifecycle::{
    start_expiration_cleanup_task, start_sandbox_store_sync_task,
};
use axum::{
    Router,
    routing::{get, post},
};
use axum::routing::delete;

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
        .route(
            "/v1/sandboxes/{sandbox_id}/endpoints/{port}",
            get(get_sandbox_endpoint),
        )
}
