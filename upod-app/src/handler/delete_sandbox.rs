use super::docker::{
    resolve_container_id, untrack_sandbox_expiration, untrack_sandbox_port_mappings,
};
use crate::core::code::Code;
use axum::extract::Path;
use bollard::Docker;
use bollard::errors::Error as DockerError;
use bollard::query_parameters::{RemoveContainerOptions, StopContainerOptions};
use upod_base::web::error::WebError;
use upod_base::web::r::R;

/// 删除沙箱
///
/// 终止沙箱并清理资源
pub async fn delete_sandbox(Path(sandbox_id): Path<String>) -> R<()> {
    let docker = match Docker::connect_with_local_defaults() {
        Ok(docker) => docker,
        Err(error) => return R::err(docker_connect_error(error)),
    };

    let container_id = match resolve_container_id(&docker, &sandbox_id, sandbox_delete_error).await
    {
        Ok(id) => id,
        Err(error) => return R::err(error),
    };

    let detail = match docker.inspect_container(&container_id, None).await {
        Ok(detail) => detail,
        Err(error) => return R::err(map_delete_or_not_found_error(error)),
    };
    let paused = detail
        .state
        .as_ref()
        .and_then(|state| state.paused)
        .unwrap_or(false);
    let running = detail
        .state
        .as_ref()
        .and_then(|state| state.running)
        .unwrap_or(false);

    if paused {
        match docker.unpause_container(&container_id).await {
            Ok(_) => {}
            Err(DockerError::DockerResponseServerError {
                status_code: 304, ..
            }) => {}
            Err(error) => return R::err(map_delete_or_not_found_error(error)),
        }
    }

    if running || paused {
        match docker
            .stop_container(&container_id, None::<StopContainerOptions>)
            .await
        {
            Ok(_) => {}
            Err(DockerError::DockerResponseServerError {
                status_code: 304, ..
            }) => {}
            Err(error) => return R::err(map_delete_or_not_found_error(error)),
        }
    }

    let options = RemoveContainerOptions {
        force: true,
        ..Default::default()
    };

    match docker.remove_container(&container_id, Some(options)).await {
        Ok(_) => {
            untrack_sandbox_expiration(&container_id);
            untrack_sandbox_port_mappings(&container_id);
            R::ok(())
        }
        Err(error) => {
            if let DockerError::DockerResponseServerError {
                status_code: 404, ..
            } = error
            {
                untrack_sandbox_expiration(&container_id);
                untrack_sandbox_port_mappings(&container_id);
                return R::err(WebError::Biz(Code::SandboxNotFound.into()));
            }
            R::err(sandbox_delete_error(error))
        }
    }
}

fn docker_connect_error(error: DockerError) -> WebError {
    WebError::BizWithArgs(
        Code::DockerConnectError.into(),
        vec![("error".to_string(), error.to_string())],
    )
}

fn sandbox_delete_error(error: DockerError) -> WebError {
    WebError::BizWithArgs(
        Code::SandboxDeleteError.into(),
        vec![("error".to_string(), error.to_string())],
    )
}

fn map_delete_or_not_found_error(error: DockerError) -> WebError {
    if let DockerError::DockerResponseServerError {
        status_code: 404, ..
    } = error
    {
        return WebError::Biz(Code::SandboxNotFound.into());
    }
    sandbox_delete_error(error)
}

#[cfg(test)]
mod tests {
    use super::super::docker::SANDBOX_ID_LABEL;
    use super::*;
    use bollard::service::ContainerCreateBody;
    use futures_util::stream::StreamExt;
    use std::collections::HashMap;

    #[tokio::test]
    async fn test_delete_sandbox() {
        // 1. Create a dummy container to delete
        let docker = Docker::connect_with_local_defaults().unwrap();
        let image = "alpine:latest";

        // Ensure image exists
        if docker.inspect_image(image).await.is_err() {
            use bollard::query_parameters::CreateImageOptions;
            let mut stream = docker.create_image(
                Some(CreateImageOptions {
                    from_image: Some(image.to_string()),
                    ..Default::default()
                }),
                None,
                None,
            );
            while stream.next().await.is_some() {}
        }

        let config = ContainerCreateBody {
            image: Some(image.to_string()),
            cmd: Some(vec!["sleep".to_string(), "100".to_string()]),
            labels: Some(HashMap::from([(
                SANDBOX_ID_LABEL.to_string(),
                "test-sandbox".to_string(),
            )])),
            ..Default::default()
        };

        let res = docker
            .create_container(
                None::<bollard::query_parameters::CreateContainerOptions>,
                config,
            )
            .await
            .unwrap();
        let id = res.id;

        // 2. Start it (optional, but good to test force delete)
        docker
            .start_container(
                &id,
                None::<bollard::query_parameters::StartContainerOptions>,
            )
            .await
            .unwrap();

        // 3. Call delete_sandbox
        let result = delete_sandbox(Path(id.clone())).await;
        assert_eq!(result.code, Code::Ok as i32);

        // 4. Verify it's gone
        let inspect = docker.inspect_container(&id, None).await;
        assert!(inspect.is_err()); // Should be Not Found
    }

    #[tokio::test]
    async fn test_delete_non_existent_sandbox() {
        let result = delete_sandbox(Path("non-existent-id-12345".to_string())).await;
        assert_eq!(result.code, Code::SandboxNotFound as i32);
    }

    #[tokio::test]
    async fn test_delete_non_sandbox_container() {
        let docker = Docker::connect_with_local_defaults().unwrap();
        let image = "alpine:latest";

        if docker.inspect_image(image).await.is_err() {
            use bollard::query_parameters::CreateImageOptions;
            let mut stream = docker.create_image(
                Some(CreateImageOptions {
                    from_image: Some(image.to_string()),
                    ..Default::default()
                }),
                None,
                None,
            );
            while stream.next().await.is_some() {}
        }

        let config = ContainerCreateBody {
            image: Some(image.to_string()),
            cmd: Some(vec!["sleep".to_string(), "100".to_string()]),
            ..Default::default()
        };

        let res = docker
            .create_container(
                None::<bollard::query_parameters::CreateContainerOptions>,
                config,
            )
            .await
            .unwrap();
        let id = res.id.clone();

        docker
            .start_container(
                &id,
                None::<bollard::query_parameters::StartContainerOptions>,
            )
            .await
            .unwrap();

        let result = delete_sandbox(Path(id.clone())).await;
        assert_eq!(result.code, Code::SandboxNotFound as i32);

        docker
            .remove_container(
                &id,
                Some(RemoveContainerOptions {
                    force: true,
                    ..Default::default()
                }),
            )
            .await
            .unwrap();
    }

    #[test]
    fn test_map_delete_or_not_found_error_404() {
        let error = DockerError::DockerResponseServerError {
            status_code: 404,
            message: "not found".to_string(),
        };
        let mapped = map_delete_or_not_found_error(error);
        match mapped {
            WebError::Biz(code) => assert_eq!(code, Code::SandboxNotFound as i32),
            _ => panic!("expected SandboxNotFound"),
        }
    }

    #[test]
    fn test_map_delete_or_not_found_error_non_404() {
        let error = DockerError::DockerResponseServerError {
            status_code: 500,
            message: "internal error".to_string(),
        };
        let mapped = map_delete_or_not_found_error(error);
        match mapped {
            WebError::BizWithArgs(code, _) => assert_eq!(code, Code::SandboxDeleteError as i32),
            _ => panic!("expected SandboxDeleteError"),
        }
    }
}
