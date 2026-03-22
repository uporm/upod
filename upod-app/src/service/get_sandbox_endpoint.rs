use super::sandbox_store::get_container;
use crate::config::AppConfig;
use crate::core::code::Code;
use crate::models::sandbox::SandboxEndpointResp;
use std::net::UdpSocket;
use upod_base::web::error::WebError;

/// 从本地缓存中获取沙箱端点映射。
///
/// 参数：
/// - sandbox_id：沙箱唯一标识。
/// - port：容器内端口。
///
/// 返回：
/// - 命中缓存时返回端点信息。
///
/// 异常：
/// - 未找到沙箱时返回 `SandboxNotFound`。
/// - 沙箱存在但端口映射缺失时返回 `SandboxEndpointNotFound`。
pub async fn get_sandbox_endpoint(
    sandbox_id: &str,
    port: u16,
) -> Result<SandboxEndpointResp, WebError> {
    let container = get_container(sandbox_id).ok_or_else(|| WebError::Biz(Code::SandboxNotFound.into()))?;

    container.ports.get(&port).ok_or_else(|| WebError::Biz(Code::SandboxEndpointNotFound.into()))?;

    Ok(SandboxEndpointResp {
        sandbox_id: sandbox_id.to_string(),
        port,
        endpoint: build_sandbox_endpoint_url(sandbox_id, port),
    })
}

/// 生成对外可访问的沙箱端点 URL。
///
/// 参数：
/// - sandbox_id：沙箱唯一标识。
/// - port：容器内端口。
///
/// 返回：
/// - 形如 `{base}/sandboxes/{sandbox_id}/port/{port}` 的完整地址。
///
/// 异常：
/// - 无。
fn build_sandbox_endpoint_url(sandbox_id: &str, port: u16) -> String {
    let base_url = resolve_endpoint_base_url();
    format!("{base_url}/sandboxes/{sandbox_id}/port/{port}")
}

/// 解析端点基础地址。
/// - 优先返回配置中的 `server.endpoint_base_url`。
/// - 配置缺失时回退为 `http://{local_ip}:{gateway_port}`。
fn resolve_endpoint_base_url() -> String {
    let config = AppConfig::global();
    if let Some(base_url) = config.server.endpoint_base_url.as_deref() {
        let base_url = normalize_base_url(base_url);
        if !base_url.is_empty() {
            return base_url;
        }
    }
    let port = parse_server_port(&config.gateway.addr).unwrap_or(9000);
    let ip = resolve_local_ip().unwrap_or_else(|| "127.0.0.1".to_string());
    format!("http://{ip}:{port}")
}

/// 规范化基础 URL 字符串。
///
/// 参数：
/// - value：原始配置值。
///
/// 返回：
/// - 去除前后空白和末尾 `/` 后的 URL。
/// - 未携带协议时默认补 `http://`。
fn normalize_base_url(value: &str) -> String {
    let value = value.trim();
    if value.is_empty() {
        return String::new();
    }
    let value = value.trim_end_matches('/');
    if value.starts_with("http://") || value.starts_with("https://") {
        return value.to_string();
    }
    format!("http://{value}")
}

/// 从服务监听地址中提取端口。
///
/// 参数：
/// - addr：服务监听地址，如 `0.0.0.0:8080`。
///
/// 返回：
/// - 解析成功时返回端口号。
fn parse_server_port(addr: &str) -> Option<u16> {
    addr.parse::<std::net::SocketAddr>()
        .ok()
        .map(|socket_addr| socket_addr.port())
        .or_else(|| addr.rsplit(':').next()?.parse::<u16>().ok())
}

/// 解析本机可出站访问的 IP 地址。
///
/// 参数：
/// - 无。
///
/// 返回：
/// - 成功时返回本机 IP 字符串。
fn resolve_local_ip() -> Option<String> {
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    let ip = socket.local_addr().ok()?.ip();
    if ip.is_unspecified() {
        return None;
    }
    Some(ip.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use crate::service::sandbox_store::{SandboxContainer, clear_container, register_container};

    /// 校验命中缓存时返回端点。
    #[tokio::test]
    async fn test_get_sandbox_endpoint_from_cache() {
        let sandbox_id = "sandbox-test-cache-hit";
        clear_container(sandbox_id);

        register_container(
            sandbox_id,
            SandboxContainer {
                container_id: "container-id".to_string(),
                ports: std::collections::HashMap::from([(8080, 32768)]),
                expires_at: Utc::now(),
            },
        );

        let resp = get_sandbox_endpoint(sandbox_id, 8080).await.unwrap();
        assert_eq!(resp.sandbox_id, sandbox_id);
        assert_eq!(resp.port, 8080);
        assert_eq!(
            resp.endpoint,
            format!("http://localhost:9000/sandboxes/{sandbox_id}/port/8080")
        );

        clear_container(sandbox_id);
    }

    /// 校验沙箱存在但端口缺失时返回端点不存在错误。
    #[tokio::test]
    async fn test_get_sandbox_endpoint_port_not_found() {
        let sandbox_id = "sandbox-test-port-miss";
        clear_container(sandbox_id);

        register_container(
            sandbox_id,
            SandboxContainer {
                container_id: "container-id".to_string(),
                ports: std::collections::HashMap::new(),
                expires_at: Utc::now(),
            },
        );

        let err = get_sandbox_endpoint(sandbox_id, 8080).await.unwrap_err();
        match err {
            WebError::Biz(code) => assert_eq!(code, Code::SandboxEndpointNotFound as i32),
            _ => panic!("unexpected error type"),
        }

        clear_container(sandbox_id);
    }

    /// 校验沙箱缺失时返回沙箱不存在错误。
    #[tokio::test]
    async fn test_get_sandbox_endpoint_sandbox_not_found() {
        let sandbox_id = "sandbox-test-missing";
        clear_container(sandbox_id);

        let err = get_sandbox_endpoint(sandbox_id, 8080).await.unwrap_err();
        match err {
            WebError::Biz(code) => assert_eq!(code, Code::SandboxNotFound as i32),
            _ => panic!("unexpected error type"),
        }
    }

    /// 校验基础 URL 规范化逻辑。
    #[test]
    fn test_normalize_base_url() {
        assert_eq!(
            normalize_base_url("http://localhost:8080/"),
            "http://localhost:8080"
        );
        assert_eq!(
            normalize_base_url("localhost:8080"),
            "http://localhost:8080"
        );
    }

    /// 校验服务地址端口解析逻辑。
    #[test]
    fn test_parse_server_port() {
        assert_eq!(parse_server_port("0.0.0.0:8080"), Some(8080));
        assert_eq!(parse_server_port("127.0.0.1:9000"), Some(9000));
    }
}
