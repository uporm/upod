use axum::Json;
use axum::extract::Path;
use axum::http::StatusCode;
use axum::response::IntoResponse;

use super::command_session::get_session;

pub(crate) async fn get_command_status(Path(id): Path<String>) -> impl IntoResponse {
    let Some(session) = get_session(&id).await else {
        return (StatusCode::NOT_FOUND, "未找到命令会话").into_response();
    };
    Json(session.snapshot().await).into_response()
}
