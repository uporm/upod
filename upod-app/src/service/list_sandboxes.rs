use std::collections::HashMap;

use bollard::Docker;
use bollard::models::ContainerSummary;
use bollard::query_parameters::ListContainersOptions;
use upod_base::web::error::WebError;

use crate::models::sandbox::{ListSandboxesReq, ListSandboxesResp, Sandbox};

use super::sandbox_lifecycle::{SANDBOX_ID_LABEL, docker_connect_error};

/// 获取沙箱列表
///
/// 通过 Docker API 查询所有带有沙箱标签的容器，
/// 并根据请求参数进行状态和元数据过滤。
///
/// # 参数
/// - `req`: 包含过滤条件的请求参数
///
/// # 返回
/// 成功返回包含沙箱列表的 `ListSandboxesResp`，失败返回 `WebError`
pub async fn list_sandboxes(req: ListSandboxesReq) -> Result<ListSandboxesResp, WebError> {
    let docker = Docker::connect_with_local_defaults().map_err(docker_connect_error)?;

    let mut filters = HashMap::new();

    // 添加基础标签过滤，只查询沙箱容器
    let mut label_filters = vec![SANDBOX_ID_LABEL.to_string()];

    // 处理元数据过滤
    if let Some(metadata) = req.metadata {
        for (k, v) in metadata {
            label_filters.push(format!("{}={}", k, v));
        }
    }
    filters.insert("label".to_string(), label_filters);

    // 处理状态过滤
    if let Some(states) = req.state.filter(|s| !s.is_empty()) {
        filters.insert("status".to_string(), states);
    }

    let containers = docker
        .list_containers(Some(ListContainersOptions {
            all: true,
            filters: Some(filters),
            ..Default::default()
        }))
        .await
        .map_err(|e| {
            WebError::BizWithArgs(
                crate::core::code::Code::SandboxLifecycleError.into(),
                vec![("error".to_string(), e.to_string())],
            )
        })?;

    let items = containers.into_iter().map(build_sandbox_item).collect();

    Ok(ListSandboxesResp { items })
}

/// 将 Docker 容器摘要转换为 Sandbox 结构体
///
/// # 参数
/// - `summary`: Docker 容器摘要信息
///
/// # 返回
/// 转换后的 `Sandbox` 对象
fn build_sandbox_item(summary: ContainerSummary) -> Sandbox {
    let id = summary
        .labels
        .as_ref()
        .and_then(|l| l.get(SANDBOX_ID_LABEL))
        .cloned()
        .unwrap_or_else(|| summary.id.clone().unwrap_or_default());

    let name = summary
        .names
        .as_ref()
        .and_then(|names| names.first())
        .map(|n| n.trim_start_matches('/').to_string())
        .unwrap_or_default();

    let status = summary
        .state
        .map(|s| s.to_string())
        .unwrap_or_else(|| "unknown".to_string());

    let image = summary.image.clone().unwrap_or_default();

    let created_at = summary.created.map(|c| c.to_string()).unwrap_or_default();

    let metadata = summary.labels.unwrap_or_default();

    Sandbox {
        id,
        name,
        status,
        image,
        created_at,
        metadata,
    }
}
