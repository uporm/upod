use super::docker::{resolve_container_id, resolve_expiration, track_sandbox_expiration};
use axum::Json;
use axum::extract::Path;
use bollard::Docker;
use bollard::errors::Error as DockerError;
use chrono::{Duration, Utc};
use crate::core::code::Code;
use upod_base::web::error::WebError;
use upod_base::web::r::R;

use crate::models::sandbox::{RenewSandboxExpirationReq, RenewSandboxExpirationResp};

/// 暂停沙箱
///
/// 暂停运行中的沙箱，保留其运行状态数据。
pub async fn pause_sandbox(Path(sandbox_id): Path<String>) -> R<()> {
    let docker = match Docker::connect_with_local_defaults() {
        Ok(docker) => docker,
        Err(error) => return R::err(docker_connect_error(error)),
    };

    let container_id =
        match resolve_container_id(&docker, &sandbox_id, sandbox_lifecycle_error).await {
            Ok(id) => id,
            Err(error) => return R::err(error),
        };

    let detail = match docker.inspect_container(&container_id, None).await {
        Ok(detail) => detail,
        Err(error) => return R::err(map_lifecycle_or_not_found_error(error)),
    };

    // 仅允许从 running 状态执行暂停，避免对非运行态容器重复操作。
    let running = detail
        .state
        .as_ref()
        .and_then(|state| state.running)
        .unwrap_or(false);
    if !running {
        return R::err(WebError::BizWithArgs(
            Code::SandboxLifecycleError.into(),
            vec![(
                "error".to_string(),
                "sandbox is not in running state".to_string(),
            )],
        ));
    }

    match docker.pause_container(&container_id).await {
        Ok(_) => R::ok(()),
        Err(error) => map_lifecycle_error(error),
    }
}

/// 恢复沙箱
///
/// 恢复已暂停的沙箱运行。
pub async fn resume_sandbox(Path(sandbox_id): Path<String>) -> R<()> {
    let docker = match Docker::connect_with_local_defaults() {
        Ok(docker) => docker,
        Err(error) => return R::err(docker_connect_error(error)),
    };

    let container_id =
        match resolve_container_id(&docker, &sandbox_id, sandbox_lifecycle_error).await {
            Ok(id) => id,
            Err(error) => return R::err(error),
        };

    let detail = match docker.inspect_container(&container_id, None).await {
        Ok(detail) => detail,
        Err(error) => return R::err(map_lifecycle_or_not_found_error(error)),
    };

    // 仅允许从 paused 状态恢复，确保生命周期操作语义明确。
    let paused = detail
        .state
        .as_ref()
        .and_then(|state| state.paused)
        .unwrap_or(false);
    if !paused {
        return R::err(WebError::BizWithArgs(
            Code::SandboxLifecycleError.into(),
            vec![(
                "error".to_string(),
                "sandbox is not in paused state".to_string(),
            )],
        ));
    }

    match docker.unpause_container(&container_id).await {
        Ok(_) => R::ok(()),
        Err(error) => map_lifecycle_error(error),
    }
}

/// 续期沙箱
///
/// 延长沙箱生存时间，并返回新的过期时间。
pub async fn renew_sandbox_expiration(
    Path(sandbox_id): Path<String>,
    Json(req): Json<RenewSandboxExpirationReq>,
) -> R<RenewSandboxExpirationResp> {
    // ttl_seconds 必须大于 0，避免产生无效续期请求。
    if req.ttl_seconds == 0 {
        return R::err(WebError::Biz(Code::InvalidRenewExpiration.into()));
    }

    let docker = match Docker::connect_with_local_defaults() {
        Ok(docker) => docker,
        Err(error) => return R::err(docker_connect_error(error)),
    };

    let container_id =
        match resolve_container_id(&docker, &sandbox_id, sandbox_lifecycle_error).await {
            Ok(id) => id,
            Err(error) => return R::err(error),
        };

    let detail = match docker.inspect_container(&container_id, None).await {
        Ok(detail) => detail,
        Err(error) => return R::err(map_lifecycle_or_not_found_error(error)),
    };

    let now = Utc::now();
    let expires_at = resolve_next_expiration(now, req.ttl_seconds, &container_id, &detail);
    track_sandbox_expiration(&container_id, &expires_at);

    R::ok(RenewSandboxExpirationResp { expires_at })
}

fn resolve_next_expiration(
    now: chrono::DateTime<Utc>,
    ttl_seconds: u64,
    container_id: &str,
    detail: &bollard::models::ContainerInspectResponse,
) -> String {
    // 续期基准取“已有过期时间”和“当前时间”较大值，避免把过期时间续回过去。
    let from_labels = detail
        .config
        .as_ref()
        .and_then(|config| config.labels.as_ref());
    let base = resolve_expiration(container_id, from_labels)
        .unwrap_or(now)
        .max(now);

    (base + Duration::seconds(ttl_seconds as i64)).to_rfc3339()
}

fn map_lifecycle_error(error: DockerError) -> R<()> {
    R::err(map_lifecycle_or_not_found_error(error))
}

fn map_lifecycle_or_not_found_error(error: DockerError) -> WebError {
    // 404 在业务上应表示沙箱不存在，其余错误统一归类为生命周期错误。
    if let DockerError::DockerResponseServerError {
        status_code: 404, ..
    } = error
    {
        return WebError::Biz(Code::SandboxNotFound.into());
    }
    sandbox_lifecycle_error(error)
}

fn docker_connect_error(error: DockerError) -> WebError {
    WebError::BizWithArgs(
        Code::DockerConnectError.into(),
        vec![("error".to_string(), error.to_string())],
    )
}

fn sandbox_lifecycle_error(error: DockerError) -> WebError {
    WebError::BizWithArgs(
        Code::SandboxLifecycleError.into(),
        vec![("error".to_string(), error.to_string())],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_renew_sandbox_expiration_invalid_ttl() {
        let resp = renew_sandbox_expiration(
            Path("sandbox-id".to_string()),
            Json(RenewSandboxExpirationReq { ttl_seconds: 0 }),
        )
        .await;
        assert_eq!(resp.code, Code::InvalidRenewExpiration as i32);
    }
}
