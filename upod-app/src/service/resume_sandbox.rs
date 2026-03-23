use super::sandbox_lifecycle::{docker_connect_error, map_lifecycle_or_not_found_error};
use super::sandbox_store::get_container;
use crate::core::code::Code;
use bollard::Docker;
use upod_base::web::error::WebError;

pub async fn resume_sandbox(sandbox_id: &str) -> Result<(), WebError> {
    let docker = Docker::connect_with_local_defaults().map_err(docker_connect_error)?;
    let container_id = get_container(sandbox_id)
        .map(|container| container.container_id)
        .ok_or_else(|| WebError::Biz(Code::SandboxNotFound.into()))?;

    let detail = docker
        .inspect_container(&container_id, None)
        .await
        .map_err(map_lifecycle_or_not_found_error)?;

    let paused = detail
        .state
        .as_ref()
        .and_then(|state| state.paused)
        .unwrap_or(false);
    if !paused {
        return Err(WebError::BizWithArgs(
            Code::SandboxLifecycleError.into(),
            vec![(
                "error".to_string(),
                "sandbox is not in paused state".to_string(),
            )],
        ));
    }

    docker
        .unpause_container(&container_id)
        .await
        .map_err(map_lifecycle_or_not_found_error)
}
