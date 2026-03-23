use std::collections::HashMap;

use bollard::Docker;
use bollard::models::ContainerInspectResponse;
use bollard::query_parameters::ListContainersOptions;
use upod_base::web::error::WebError;

use crate::core::code::Code;
use crate::models::sandbox::GetSandboxResp;

use super::sandbox_lifecycle::{
    SANDBOX_ID_LABEL, docker_connect_error, map_lifecycle_or_not_found_error,
};
use super::sandbox_store::get_container;

/// 获取指定沙箱的详细信息
///
/// 包含以下步骤：
/// 1. 从存储或 Docker 获取容器 ID
/// 2. 检查容器详细信息
/// 3. 构建并返回响应
///
/// # 参数
/// - `sandbox_id`: 沙箱唯一标识符
///
/// # 返回
/// 成功返回 `GetSandboxResp`，失败返回相应的 `WebError` 异常（如容器未找到、Docker 异常等）
pub async fn get_sandbox(sandbox_id: &str) -> Result<GetSandboxResp, WebError> {
    let docker = Docker::connect_with_local_defaults().map_err(docker_connect_error)?;

    // 优先从缓存获取容器 ID，若无则通过 Docker Label 查找以处理未同步或缓存丢失情况
    let container_id = if let Some(container) = get_container(sandbox_id) {
        container.container_id
    } else {
        let mut filters = HashMap::new();
        filters.insert(
            "label".to_string(),
            vec![format!("{}={}", SANDBOX_ID_LABEL, sandbox_id)],
        );
        let containers = docker
            .list_containers(Some(ListContainersOptions {
                all: true,
                filters: Some(filters),
                ..Default::default()
            }))
            .await
            .map_err(|e| {
                WebError::BizWithArgs(
                    Code::SandboxLifecycleError.into(),
                    vec![("error".to_string(), e.to_string())],
                )
            })?;

        if let Some(c) = containers.into_iter().next() {
            c.id.unwrap_or_default()
        } else {
            return Err(WebError::Biz(Code::SandboxNotFound.into()));
        }
    };

    let detail = docker
        .inspect_container(&container_id, None)
        .await
        .map_err(map_lifecycle_or_not_found_error)?;

    Ok(build_get_sandbox_resp(sandbox_id, detail))
}

/// 构造沙箱详情响应对象
///
/// 将 Docker 返回的 `ContainerInspectResponse` 转换为业务层的 `GetSandboxResp` 结构体，
/// 处理并提取状态、时间、环境变量等关键信息。
///
/// # 参数
/// - `sandbox_id`: 沙箱唯一标识符
/// - `detail`: Docker 容器详情响应
///
/// # 返回
/// 组装完成的 `GetSandboxResp` 结构体
fn build_get_sandbox_resp(sandbox_id: &str, detail: ContainerInspectResponse) -> GetSandboxResp {
    let name = detail
        .name
        .unwrap_or_default()
        .trim_start_matches('/')
        .to_string();

    let status = detail
        .state
        .as_ref()
        .and_then(|s| s.status.as_ref())
        .map(|s| s.to_string())
        .unwrap_or_else(|| "unknown".to_string());

    let image = detail
        .config
        .as_ref()
        .and_then(|c| c.image.as_ref())
        .cloned()
        .unwrap_or_default();

    let created_at = detail.created.unwrap_or_default();

    let started_at = detail
        .state
        .as_ref()
        .and_then(|s| s.started_at.as_ref())
        .filter(|s| !s.is_empty())
        .cloned();

    let entrypoint = detail
        .config
        .as_ref()
        .and_then(|c| c.entrypoint.as_ref())
        .cloned()
        .unwrap_or_default();

    let env = detail
        .config
        .as_ref()
        .and_then(|c| c.env.as_ref())
        .cloned()
        .unwrap_or_default();

    let metadata = detail
        .config
        .as_ref()
        .and_then(|c| c.labels.as_ref())
        .cloned()
        .unwrap_or_default();

    GetSandboxResp {
        id: sandbox_id.to_string(),
        name,
        status,
        image,
        created_at,
        started_at,
        entrypoint,
        env,
        metadata,
    }
}
