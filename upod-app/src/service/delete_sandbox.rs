use crate::core::code::Code;
use upod_base::web::error::WebError;

/// 删除指定的沙箱
///
/// 包含以下步骤：
/// 1. 调用生命周期模块的强制删除逻辑
/// 2. 如果容器不存在（404）则返回 SandboxNotFound 错误
///
/// # 参数
/// - `sandbox_id`: 沙箱唯一标识符
pub async fn delete_sandbox(sandbox_id: &str) -> Result<(), WebError> {
    // 复用生命周期模块的强制删除逻辑
    let found = crate::service::sandbox_lifecycle::force_remove_sandbox(sandbox_id).await?;

    // 如果 Docker 中容器已不存在，同样返回 404 错误
    if !found {
        return Err(WebError::Biz(Code::SandboxNotFound.into()));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::sandbox_lifecycle::SANDBOX_ID_LABEL;
    use super::*;
    use bollard::Docker;
    use bollard::service::ContainerCreateBody;
    use futures_util::stream::StreamExt;
    use std::collections::HashMap;

    #[tokio::test]
    async fn test_delete_sandbox() {
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

        let sandbox_id = "test-sandbox-delete-success";

        let config = ContainerCreateBody {
            image: Some(image.to_string()),
            cmd: Some(vec!["sleep".to_string(), "100".to_string()]),
            labels: Some(HashMap::from([(
                SANDBOX_ID_LABEL.to_string(),
                sandbox_id.to_string(),
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

        docker
            .start_container(
                &id,
                None::<bollard::query_parameters::StartContainerOptions>,
            )
            .await
            .unwrap();

        // 注册到存储，让 delete_sandbox 能够找到
        crate::service::sandbox_store::register_container(
            sandbox_id,
            crate::service::sandbox_store::SandboxContainer {
                container_id: id.clone(),
                ports: HashMap::new(),
                expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
            }
        );

        let result = delete_sandbox(sandbox_id).await;
        assert!(result.is_ok());

        let inspect = docker.inspect_container(&id, None).await;
        assert!(inspect.is_err());
    }

    #[tokio::test]
    async fn test_delete_non_existent_sandbox() {
        let result = delete_sandbox("non-existent-id-12345").await;
        match result {
            Err(WebError::Biz(code)) => assert_eq!(code, Code::SandboxNotFound as i32),
            _ => panic!("expected SandboxNotFound"),
        }
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

        // 虽然容器存在，但是没有注册到 cache 里
        let result = delete_sandbox("non-existent-sandbox").await;
        match result {
            Err(WebError::Biz(code)) => assert_eq!(code, Code::SandboxNotFound as i32),
            _ => panic!("expected SandboxNotFound"),
        }

        use bollard::query_parameters::RemoveContainerOptions;
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