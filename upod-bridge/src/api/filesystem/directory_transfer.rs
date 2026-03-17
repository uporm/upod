use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use axum::Json;
use axum::extract::Query;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde::Deserialize;

use crate::models::filesystem::Permission;

use super::file_operations::{apply_permission, map_file_error};

/// 目录批量删除查询参数。
#[derive(Debug, Deserialize, Default)]
pub(crate) struct PathsReq {
    /// 需要删除的目录路径列表。
    #[serde(default)]
    path: Vec<String>,
}

/// 批量创建目录并按请求应用权限。
/// 参数：`request` 为目录路径到权限对象的映射。
/// 返回：全部创建完成时返回 `200`。
/// 错误：创建目录或设置权限失败时返回映射后的 HTTP 错误。
pub(crate) async fn make_directories(
    Json(request): Json<HashMap<String, Permission>>,
) -> impl IntoResponse {
    for (path, permission) in request {
        let target = PathBuf::from(path);
        if let Err(error) = fs::create_dir_all(&target) {
            return map_file_error(error);
        }
        if let Err(error) = apply_permission(target.to_string_lossy().as_ref(), &permission) {
            return map_file_error(error);
        }
    }
    StatusCode::OK.into_response()
}

/// 批量删除目录；目录不存在时按成功处理。
/// 参数：`query.path` 为待删除目录列表。
/// 返回：全部处理完成时返回 `200`。
/// 错误：删除目录失败时返回映射后的 HTTP 错误。
pub(crate) async fn remove_directories(Query(query): Query<PathsReq>) -> impl IntoResponse {
    for path in &query.path {
        let target = PathBuf::from(path);
        match fs::remove_dir_all(&target) {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return map_file_error(error),
        }
    }
    StatusCode::OK.into_response()
}
