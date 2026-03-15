use std::collections::HashMap;

use axum::extract::Path;
use bollard::Docker;
use bollard::errors::Error as DockerError;
use crate::core::code::Code;
use upod_base::web::error::WebError;
use upod_base::web::r::R;
use super::docker::resolve_container_id;

use crate::models::sandbox::SandboxEndpointResp;

/// 获取沙箱服务端点
///
/// 获取指定沙箱端口的公开访问地址。
pub async fn get_sandbox_endpoint(Path((sandbox_id, port)): Path<(String, u16)>) -> R<SandboxEndpointResp> {
    let docker = match Docker::connect_with_local_defaults() {
        Ok(docker) => docker,
        Err(error) => return R::err(docker_connect_error(error)),
    };

    let container_id = match resolve_container_id(&docker, &sandbox_id, sandbox_endpoint_error).await {
        Ok(id) => id,
        Err(error) => return R::err(error),
    };

    let detail = match docker.inspect_container(&container_id, None).await {
        Ok(detail) => detail,
        Err(error) => {
            if let DockerError::DockerResponseServerError { status_code: 404, .. } = error {
                return R::err(WebError::Biz(Code::SandboxNotFound.into()));
            }
            return R::err(sandbox_endpoint_error(error));
        }
    };

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

/// 根据容器检查结果解析可访问的 HTTP 端点。
///
/// 优先返回 Docker 端口映射到宿主机后的地址；如果未配置映射，则回退为容器内网地址。
fn resolve_endpoint(detail: &bollard::models::ContainerInspectResponse, port: u16) -> Option<String> {
    let key = format!("{port}/tcp");

    // 优先使用端口映射，返回宿主机地址
    if let Some(network_settings) = &detail.network_settings
        && let Some(ports) = &network_settings.ports
        && let Some(Some(bindings)) = ports.get(&key)
        && let Some(binding) = bindings.first()
        && let Some(host_port) = &binding.host_port
    {
        let host_ip = binding.host_ip.as_deref().unwrap_or("127.0.0.1");
        let normalized_host_ip = if host_ip.is_empty() || host_ip == "0.0.0.0" {
            "127.0.0.1"
        } else {
            host_ip
        };
        return Some(format!("http://{normalized_host_ip}:{host_port}"));
    }

    // 兜底返回容器内网地址
    resolve_container_ip(detail).map(|ip| format!("http://{ip}:{port}"))
}

/// 从容器网络配置中提取第一个可用的容器内网 IP。
fn resolve_container_ip(detail: &bollard::models::ContainerInspectResponse) -> Option<String> {
    if let Some(network_settings) = &detail.network_settings
        && let Some(networks) = &network_settings.networks
    {
        return first_non_empty_network_ip(networks);
    }
    None
}

fn first_non_empty_network_ip(
    networks: &HashMap<String, bollard::models::EndpointSettings>,
) -> Option<String> {
    // Docker 容器可能挂载多个网络，这里按遍历顺序返回第一个非空 IP。
    for endpoint in networks.values() {
        if let Some(ip) = endpoint.ip_address.as_deref()
            && !ip.is_empty()
        {
            return Some(ip.to_string());
        }
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
    use bollard::models::{ContainerInspectResponse, EndpointSettings, NetworkSettings};

    #[test]
    fn test_resolve_container_ip() {
        let mut networks = HashMap::new();
        networks.insert(
            "bridge".to_string(),
            EndpointSettings {
                ip_address: Some("172.17.0.10".to_string()),
                ..Default::default()
            },
        );

        let detail = ContainerInspectResponse {
            network_settings: Some(NetworkSettings {
                networks: Some(networks),
                ..Default::default()
            }),
            ..Default::default()
        };

        let ip = resolve_container_ip(&detail);
        assert_eq!(ip.as_deref(), Some("172.17.0.10"));
    }
}
