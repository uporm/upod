use axum::Router;
use axum::routing::{delete, get, post};

use crate::api::command::command_exec::{interrupt_command, run_command};
use crate::api::command::command_log::get_command_output;
use crate::api::command::command_status::get_command_status;
use crate::api::filesystem::directory_transfer::{make_directories, remove_directories};
use crate::api::filesystem::file_operations::{
    chmod_files, get_files_info, remove_files, rename_files, replace_content, search_files,
};
use crate::api::filesystem::file_transfer::{download_file, upload_file};
use crate::api::metrics::{get_metrics, watch_metrics};

pub fn router() -> Router {
    Router::new()
        .route("/command", post(run_command))
        .route("/command", delete(interrupt_command))
        .route("/command/status/{id}", get(get_command_status))
        .route("/command/output/{id}", get(get_command_output))
        .route("/files", delete(remove_files))
        .route("/files/info", get(get_files_info))
        .route("/files/mv", post(rename_files))
        .route("/files/permissions", post(chmod_files))
        .route("/files/search", get(search_files))
        .route("/files/replace", post(replace_content))
        .route("/files/upload", post(upload_file))
        .route("/files/download", get(download_file))
        .route("/directories", post(make_directories))
        .route("/directories", delete(remove_directories))
        .route("/metrics", get(get_metrics))
        .route("/metrics/watch", get(watch_metrics))
}
