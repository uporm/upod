use super::docker::{resolve_container_id, resolve_sandbox_id};
use crate::core::code::Code;
use axum::extract::Path;
use bollard::Docker;
use bollard::errors::Error as DockerError;
use upod_base::web::error::WebError;
use upod_base::web::r::R;

use crate::models::sandbox::GetSandboxResp;

/// 获取沙箱详情
///
/// 获取指定沙箱的完整信息。
pub async fn get_sandbox(Path(sandbox_id): Path<String>) -> R<GetSandboxResp> {
    // 连接到本地 Docker 守护进程，后续所有查询都依赖该连接。
    let docker = match Docker::connect_with_local_defaults() {
        Ok(docker) => docker,
        Err(error) => return R::err(docker_connect_error(error)),
    };

    let container_id = match resolve_container_id(&docker, &sandbox_id, sandbox_get_error).await {
        Ok(id) => id,
        Err(error) => return R::err(error),
    };

    // 读取容器详情；如果容器已不存在，统一映射为业务错误 SandboxNotFound。
    let detail = match docker.inspect_container(&container_id, None).await {
        Ok(detail) => detail,
        Err(error) => {
            if let DockerError::DockerResponseServerError {
                status_code: 404, ..
            } = error
            {
                return R::err(WebError::Biz(Code::SandboxNotFound.into()));
            }
            return R::err(sandbox_get_error(error));
        }
    };

    // 将 Docker inspect 的结构转换为对外 API 返回结构，并兜底缺失字段。
    let resp = GetSandboxResp {
        id: resolve_sandbox_id(
            detail
                .config
                .as_ref()
                .and_then(|config| config.labels.as_ref()),
            &sandbox_id,
        ),
        name: detail
            .name
            .unwrap_or_default()
            .trim_start_matches('/')
            .to_string(),
        status: detail
            .state
            .as_ref()
            .and_then(|state| state.status.as_ref())
            .map(ToString::to_string)
            .unwrap_or_default(),
        image: detail
            .config
            .as_ref()
            .and_then(|config| config.image.clone())
            .unwrap_or_default(),
        created_at: detail.created.unwrap_or_default(),
        started_at: detail
            .state
            .as_ref()
            .and_then(|state| state.started_at.clone()),
        entrypoint: detail
            .config
            .as_ref()
            .and_then(|config| config.entrypoint.clone())
            .unwrap_or_default(),
        env: detail
            .config
            .as_ref()
            .and_then(|config| config.env.clone())
            .unwrap_or_default(),
        metadata: detail
            .config
            .as_ref()
            .and_then(|config| config.labels.clone())
            .unwrap_or_default(),
    };

    R::ok(resp)
}

fn docker_connect_error(error: DockerError) -> WebError {
    WebError::BizWithArgs(
        Code::DockerConnectError.into(),
        vec![("error".to_string(), error.to_string())],
    )
}

fn sandbox_get_error(error: DockerError) -> WebError {
    WebError::BizWithArgs(
        Code::SandboxGetError.into(),
        vec![("error".to_string(), error.to_string())],
    )
}
