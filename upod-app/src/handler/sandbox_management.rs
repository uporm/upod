use axum::Json;
use axum::extract::Path;
use serde_qs::axum::QsQuery;
use upod_base::web::r::R;

use crate::models::sandbox::{
    CreateSandboxReq, CreateSandboxResp, GetSandboxResp, ListSandboxesReq, ListSandboxesResp,
};

pub async fn create_sandbox(Json(req): Json<CreateSandboxReq>) -> R<CreateSandboxResp> {
    match crate::service::create_sandbox::create_sandbox(req).await {
        Ok(resp) => R::ok(resp),
        Err(error) => R::err(error),
    }
}

pub async fn delete_sandbox(Path(sandbox_id): Path<String>) -> R<()> {
    match crate::service::delete_sandbox::delete_sandbox(&sandbox_id).await {
        Ok(()) => R::ok(()),
        Err(error) => R::err(error),
    }
}

pub async fn pause_sandbox(Path(sandbox_id): Path<String>) -> R<()> {
    match crate::service::pause_sandbox::pause_sandbox(&sandbox_id).await {
        Ok(()) => R::ok(()),
        Err(error) => R::err(error),
    }
}

pub async fn resume_sandbox(Path(sandbox_id): Path<String>) -> R<()> {
    match crate::service::resume_sandbox::resume_sandbox(&sandbox_id).await {
        Ok(()) => R::ok(()),
        Err(error) => R::err(error),
    }
}

/// 获取沙箱详情
///
/// 接收沙箱 ID 参数，调用 service 层查询指定沙箱的详细信息并返回。
///
/// # 参数
/// - `sandbox_id`: 通过路径提取的沙箱唯一标识符
///
/// # 返回
/// 成功时返回包含沙箱详情的 R<GetSandboxResp>，失败时返回相应的错误 R 响应
pub async fn get_sandbox(Path(sandbox_id): Path<String>) -> R<GetSandboxResp> {
    match crate::service::get_sandbox::get_sandbox(&sandbox_id).await {
        Ok(resp) => R::ok(resp),
        Err(error) => R::err(error),
    }
}

/// 获取沙箱列表
///
/// 接收过滤参数，调用 service 层查询符合条件的沙箱列表并返回。
///
/// # 参数
/// - `req`: 通过查询字符串提取的过滤参数
///
/// # 返回
/// 成功时返回包含沙箱列表的 R<ListSandboxesResp>，失败时返回相应的错误 R 响应
pub async fn list_sandboxes(QsQuery(req): QsQuery<ListSandboxesReq>) -> R<ListSandboxesResp> {
    match crate::service::list_sandboxes::list_sandboxes(req).await {
        Ok(resp) => R::ok(resp),
        Err(error) => R::err(error),
    }
}
