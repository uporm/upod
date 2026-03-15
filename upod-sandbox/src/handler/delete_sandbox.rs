use super::docker::{resolve_container_id, untrack_sandbox_expiration};
use axum::extract::Path;
use bollard::Docker;
use bollard::errors::Error as DockerError;
use bollard::query_parameters::RemoveContainerOptions;
use crate::core::code::Code;
use upod_base::web::error::WebError;
use upod_base::web::r::R;

/// 删除沙箱
///
/// 终止沙箱并清理资源
pub async fn delete_sandbox(Path(sandbox_id): Path<String>) -> R<()> {
    // 1. 连接到本地 Docker 守护进程
    let docker = match Docker::connect_with_local_defaults() {
        Ok(docker) => docker,
        Err(error) => return R::err(docker_connect_error(error)),
    };

    // 2. 解析真实容器 ID：
    // 支持传入容器 ID 或业务 sandbox_id，两者都可定位到目标容器
    let container_id = match resolve_container_id(&docker, &sandbox_id, sandbox_delete_error).await
    {
        Ok(id) => id,
        Err(error) => return R::err(error),
    };

    // 3. 强制删除容器，覆盖运行中或异常状态容器，保证删除接口幂等可用
    let options = RemoveContainerOptions {
        force: true,
        ..Default::default()
    };

    // 4. 删除成功后同步清理内存中的过期跟踪记录，避免残留状态
    match docker.remove_container(&container_id, Some(options)).await {
        Ok(_) => {
            untrack_sandbox_expiration(&container_id);
            R::ok(())
        }
        Err(error) => {
            // 容器不存在按业务语义返回 SandboxNotFound
            if let DockerError::DockerResponseServerError {
                status_code: 404, ..
            } = error
            {
                return R::err(WebError::Biz(Code::SandboxNotFound.into()));
            }
            // 其余错误统一映射为删除失败，保留底层错误信息
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
}
