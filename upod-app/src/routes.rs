use axum::{
    Router,
    routing::{get, post},
};
use upod_sandbox::handler::create_sandbox::create_sandbox;
use upod_sandbox::handler::delete_sandbox::delete_sandbox;
use upod_sandbox::handler::docker::start_expiration_cleanup_task;
use upod_sandbox::handler::get_sandbox::get_sandbox;
use upod_sandbox::handler::get_sandbox_endpoint::get_sandbox_endpoint;
use upod_sandbox::handler::list_sandbox::list_sandboxes;
use upod_sandbox::handler::sandbox_lifecycle::{
    pause_sandbox, renew_sandbox_expiration, resume_sandbox,
};

pub fn router() -> Router {
    start_expiration_cleanup_task();
    Router::new()
        .route("/v1/sandboxes", post(create_sandbox).get(list_sandboxes))
        .route(
            "/v1/sandboxes/{sandbox_id}",
            get(get_sandbox).delete(delete_sandbox),
        )
        .route("/v1/sandboxes/{sandbox_id}/pause", post(pause_sandbox))
        .route("/v1/sandboxes/{sandbox_id}/resume", post(resume_sandbox))
        .route(
            "/v1/sandboxes/{sandbox_id}/renew-expiration",
            post(renew_sandbox_expiration),
        )
        .route(
            "/v1/sandboxes/{sandbox_id}/endpoints/{port}",
            get(get_sandbox_endpoint),
        )
}
