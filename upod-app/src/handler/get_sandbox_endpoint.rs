use super::docker::{
    resolve_container_id, resolve_sandbox_port_mapping, sync_sandbox_port_mappings_from_detail,
    untrack_sandbox_port_mappings,
};
use crate::core::code::Code;
use axum::extract::Path;
use bollard::Docker;
use bollard::errors::Error as DockerError;
use upod_base::web::error::WebError;
use upod_base::web::r::R;

use crate::models::sandbox::SandboxEndpointResp;

/// 获取沙箱服务端点
///
/// 获取指定沙箱端口的公开访问地址。
pub async fn get_sandbox_endpoint(
    Path((sandbox_id, port)): Path<(String, u16)>,
) -> R<SandboxEndpointResp> {
    let docker = match Docker::connect_with_local_defaults() {
        Ok(docker) => docker,
        Err(error) => return R::err(docker_connect_error(error)),
    };

    let container_id =
        match resolve_container_id(&docker, &sandbox_id, sandbox_endpoint_error).await {
            Ok(id) => id,
            Err(error) => return R::err(error),
        };

    // 快路径：优先走内存缓存，避免每次请求都触发 Docker inspect。
    // 命中后可直接返回，显著降低 endpoint 高频查询的延迟和守护进程压力。
    if let Some(endpoint_port) = resolve_sandbox_port_mapping(&container_id, port) {
        return R::ok(SandboxEndpointResp {
            sandbox_id,
            port,
            endpoint: endpoint_port.to_string(),
        });
    }

    let detail = match docker.inspect_container(&container_id, None).await {
        Ok(detail) => detail,
        Err(error) => {
            if let DockerError::DockerResponseServerError {
                status_code: 404, ..
            } = error
            {
                // 容器已不存在时同步删除端口缓存，保证后续请求不会命中脏数据。
                untrack_sandbox_port_mappings(&container_id);
                return R::err(WebError::Biz(Code::SandboxNotFound.into()));
            }
            return R::err(sandbox_endpoint_error(error));
        }
    };
    // 慢路径回源后立即回填缓存，使同容器后续端口查询转为内存命中。
    sync_sandbox_port_mappings_from_detail(&container_id, &detail);

    let endpoint = match resolve_endpoint(&detail, port) {
        Some(endpoint) => endpoint,
        None => return R::err(WebError::Biz(Code::SandboxEndpointNotFound.into())),
    };

    R::ok(SandboxEndpointResp {
        sandbox_id,
        port,
        endpoint,
    })
}

/// 根据容器检查结果解析映射到宿主机的端口。
fn resolve_endpoint(
    detail: &bollard::models::ContainerInspectResponse,
    port: u16,
) -> Option<String> {
    let key = format!("{port}/tcp");

    // 优先使用端口映射，返回宿主机端口。
    if let Some(network_settings) = &detail.network_settings
        && let Some(ports) = &network_settings.ports
        && let Some(Some(bindings)) = ports.get(&key)
        && let Some(binding) = bindings.first()
        && let Some(host_port) = &binding.host_port
    {
        return Some(host_port.to_string());
    }

    None
}

fn docker_connect_error(error: DockerError) -> WebError {
    WebError::BizWithArgs(
        Code::DockerConnectError.into(),
        vec![("error".to_string(), error.to_string())],
    )
}

fn sandbox_endpoint_error(error: DockerError) -> WebError {
    WebError::BizWithArgs(
        Code::SandboxEndpointError.into(),
        vec![("error".to_string(), error.to_string())],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use bollard::models::{ContainerInspectResponse, NetworkSettings, PortBinding};

    #[test]
    fn test_resolve_endpoint_returns_host_port() {
        let mut ports = std::collections::HashMap::new();
        ports.insert(
            "8080/tcp".to_string(),
            Some(vec![PortBinding {
                host_port: Some("32768".to_string()),
                ..Default::default()
            }]),
        );
        let detail = ContainerInspectResponse {
            network_settings: Some(NetworkSettings {
                ports: Some(ports),
                ..Default::default()
            }),
            ..Default::default()
        };

        assert_eq!(resolve_endpoint(&detail, 8080).as_deref(), Some("32768"));
    }
}
