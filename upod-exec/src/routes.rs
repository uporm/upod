use axum::Router;
use axum::routing::{delete, get, post};

use crate::api::command::command_exec::{interrupt_command, run_command};
use crate::api::command::command_log::get_command_output;
use crate::api::command::command_status::get_command_status;

pub fn router() -> Router {
    Router::new()
        .route("/command", post(run_command))
        .route("/command", delete(interrupt_command))
        .route("/command/status/{id}", get(get_command_status))
        .route("/command/output/{id}", get(get_command_output))
}
