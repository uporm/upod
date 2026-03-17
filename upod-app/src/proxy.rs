use std::collections::HashMap;

use async_trait::async_trait;
use bollard::errors::Error as DockerError;
use bollard::query_parameters::ListContainersOptions;
use bollard::Docker;
use pingora::http::RequestHeader;
use pingora::prelude::*;
use pingora::proxy::{ProxyHttp, Session, http_proxy_service};
use tracing::warn;

use crate::handler::docker::SANDBOX_ID_LABEL;

/// 启动沙盒代理服务，监听指定地址。
///
/// 该服务基于 Pingora 的 `http_proxy_service`，职责是：
/// 1) 解析外部访问路径中的 `sandbox_id` 与目标端口；
/// 2) 通过 Docker API 找到对应容器 IP；
/// 3) 将请求重写后转发到容器内服务。
///
/// 该函数会阻塞当前线程并持续运行，直到进程退出。
pub fn run_proxy_server(addr: &str) -> Result<()> {
    let mut server = Server::new(None)?;
    server.bootstrap();

    let mut proxy = http_proxy_service(&server.configuration, SandboxProxy);
    proxy.add_tcp(addr);
    server.add_service(proxy);
    server.run_forever();
}

/// 沙盒代理入口。
///
/// 通过实现 `ProxyHttp`，将一个请求拆分为两个关键阶段：
/// - `upstream_peer`：确定上游目标（IP + 端口）；
/// - `upstream_request_filter`：在发往上游前改写请求头与 URI。
struct SandboxProxy;

/// 单次请求在各阶段共享的上下文。
///
/// Pingora 会先调用 `upstream_peer`，再调用 `upstream_request_filter`。
/// 这里缓存“改写后的路径”和“上游 Host”，避免重复计算并保证阶段间传参清晰。
struct ProxyContext {
    /// 转发到容器服务时使用的路径与查询串，例如 `/api/ping?x=1`。
    rewritten_path_with_query: String,
    /// 上游 `Host` 请求头值，格式 `{container_ip}:{port}`。
    upstream_authority: String,
}

#[async_trait]
impl ProxyHttp for SandboxProxy {
    type CTX = ProxyContext;

    fn new_ctx(&self) -> Self::CTX {
        ProxyContext {
            rewritten_path_with_query: "/".to_string(),
            upstream_authority: String::new(),
        }
    }

    /// 解析目标容器地址并构建上游连接。
    async fn upstream_peer(&self, session: &mut Session, ctx: &mut Self::CTX) -> Result<Box<HttpPeer>> {
        // 从原始请求路径中提取 sandbox_id / port / 重写后的 path+query。
        let request_header = session.req_header();
        let (sandbox_id, port, rewritten_path) = parse_proxy_path(request_header).ok_or_else(|| {
            Error::explain(
                ErrorType::HTTPStatus(404),
                "path should match /sandboxes/{sandbox_id}/endpoints/{port}[/{full_path}]",
            )
        })?;

        // 通过 sandbox_id 查询容器 IP，并将后续阶段所需信息写入上下文。
        let host = resolve_container_ip(&sandbox_id).await?;
        ctx.rewritten_path_with_query = rewritten_path;
        ctx.upstream_authority = format!("{host}:{port}");

        // 这里使用明文 HTTP（第二个参数为 false），由内网容器通信场景决定。
        Ok(Box::new(HttpPeer::new((host.as_str(), port), false, String::new())))
    }

    /// 转发前重写 URI 和 Host。
    async fn upstream_request_filter(
        &self,
        _session: &mut Session,
        upstream_request: &mut RequestHeader,
        ctx: &mut Self::CTX,
    ) -> Result<()> {
        rewrite_upstream_request(
            upstream_request,
            &ctx.rewritten_path_with_query,
            &ctx.upstream_authority,
        )
    }
}

fn rewrite_upstream_request(
    upstream_request: &mut RequestHeader,
    rewritten_path_with_query: &str,
    upstream_authority: &str,
) -> Result<()> {
    // 将下游路径替换为容器服务期望的路径，避免上游收到网关前缀。
    let uri = rewritten_path_with_query.parse().map_err(|error| {
        Error::explain(
            ErrorType::new("invalid_rewrite_uri"),
            format!("invalid rewrite uri: {error}"),
        )
    })?;
    upstream_request.set_uri(uri);

    // 同步改写 Host，确保上游服务按正确 authority 处理路由/虚拟主机逻辑。
    upstream_request
        .insert_header("host", upstream_authority)
        .map_err(|error| {
            Error::explain(
                ErrorType::new("invalid_upstream_host"),
                format!("invalid upstream host: {error}"),
            )
        })?;
    Ok(())
}

/// 解析路径 `/sandboxes/{sandbox_id}/endpoints/{port}[/{full_path}]`。
fn parse_proxy_path(header: &RequestHeader) -> Option<(String, u16, String)> {
    // 仅处理约定的网关前缀，非目标路径直接返回 None。
    let path = header.uri.path();
    let prefix = "/sandboxes/";
    let rest = path.strip_prefix(prefix)?;

    // 拆分出 sandbox_id 与 endpoints 之后的部分。
    let (sandbox_id, rest) = rest.split_once("/endpoints/")?;
    if sandbox_id.is_empty() {
        return None;
    }

    // port 后如果带了路径，拼回带前导斜杠的 suffix_path；
    // 如果没有路径，则统一视为根路径 `/`。
    let (port_text, suffix_path) = match rest.split_once('/') {
        Some((port, tail)) => (port, format!("/{tail}")),
        None => (rest, "/".to_string()),
    };
    let port = port_text.parse::<u16>().ok()?;

    // 保留原始 query，避免 token / 过滤参数在转发时丢失。
    let rewritten = match header.uri.query() {
        Some(query) => format!("{suffix_path}?{query}"),
        None => suffix_path,
    };
    Some((sandbox_id.to_string(), port, rewritten))
}

/// 根据 sandbox_id 查找容器 IP。
async fn resolve_container_ip(sandbox_id: &str) -> Result<String> {
    // 每次查询时创建 Docker 客户端，避免跨请求共享状态带来的复杂生命周期问题。
    let docker = Docker::connect_with_local_defaults().map_err(docker_error)?;
    let container_id = resolve_container_id(&docker, sandbox_id).await?;
    let detail = docker
        .inspect_container(&container_id, None)
        .await
        .map_err(docker_error)?;

    // 从容器网络配置中选取第一个可用 IP。
    // 在多网络场景下，当前策略是“按遍历顺序取第一个非空地址”。
    if let Some(network_settings) = &detail.network_settings
        && let Some(networks) = &network_settings.networks
    {
        for endpoint in networks.values() {
            if let Some(ip) = endpoint.ip_address.as_deref()
                && !ip.is_empty()
            {
                return Ok(ip.to_string());
            }
        }
    }

    Err(Error::explain(
        ErrorType::new("sandbox_endpoint_not_found"),
        format!("sandbox endpoint not found for sandbox_id={sandbox_id}"),
    ))
}

/// 根据 sandbox_id 查找容器 ID，支持直接 ID 与 label 匹配。
async fn resolve_container_id(docker: &Docker, sandbox_id: &str) -> Result<String> {
    // 快路径：把 sandbox_id 当作容器 ID 直接 inspect，且要求存在约定 label，防止误命中无关容器。
    if let Ok(detail) = docker.inspect_container(sandbox_id, None).await
        && detail
            .config
            .as_ref()
            .and_then(|config| config.labels.as_ref())
            .and_then(|labels| labels.get(SANDBOX_ID_LABEL))
            .is_some()
    {
        return Ok(detail.id.unwrap_or_else(|| sandbox_id.to_string()));
    }

    // 慢路径：按 `SANDBOX_ID_LABEL=sandbox_id` 过滤容器列表。
    let mut filters = HashMap::new();
    filters.insert(
        "label".to_string(),
        vec![format!("{SANDBOX_ID_LABEL}={sandbox_id}")],
    );

    let containers = docker
        .list_containers(Some(ListContainersOptions {
            all: true,
            filters: Some(filters),
            ..Default::default()
        }))
        .await
        .map_err(docker_error)?;

    // 命中第一个容器即返回；当前语义默认一个 sandbox_id 对应一个容器。
    if let Some(id) = containers.into_iter().find_map(|item| item.id) {
        return Ok(id);
    }

    Err(Error::explain(
        ErrorType::new("sandbox_not_found"),
        format!("sandbox not found: {sandbox_id}"),
    ))
}

/// 将 Docker 错误转换为统一代理错误。
fn docker_error(error: DockerError) -> Box<Error> {
    warn!("docker operation failed: {}", error);
    Error::explain(ErrorType::new("docker_error"), error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_proxy_path_with_suffix_and_query() {
        let req = RequestHeader::build(
            "GET",
            b"/sandboxes/sandbox-1/endpoints/8080/api/v1/ping?hello=world",
            None,
        )
        .unwrap();
        let parsed = parse_proxy_path(&req).unwrap();

        assert_eq!(parsed.0, "sandbox-1");
        assert_eq!(parsed.1, 8080);
        assert_eq!(parsed.2, "/api/v1/ping?hello=world");
    }

    #[test]
    fn parse_proxy_path_with_root_suffix() {
        let req = RequestHeader::build("GET", b"/sandboxes/abc/endpoints/3000/", None).unwrap();
        let parsed = parse_proxy_path(&req).unwrap();

        assert_eq!(parsed.0, "abc");
        assert_eq!(parsed.1, 3000);
        assert_eq!(parsed.2, "/");
    }

    #[test]
    fn parse_proxy_path_reject_invalid_port() {
        let req = RequestHeader::build(
            "GET",
            b"/sandboxes/abc/endpoints/not-port/index",
            None,
        )
        .unwrap();
        assert!(parse_proxy_path(&req).is_none());
    }

    #[test]
    fn parse_proxy_path_allow_empty_suffix_path() {
        let req = RequestHeader::build("GET", b"/sandboxes/abc/endpoints/3000", None).unwrap();
        let parsed = parse_proxy_path(&req).unwrap();

        assert_eq!(parsed.0, "abc");
        assert_eq!(parsed.1, 3000);
        assert_eq!(parsed.2, "/");
    }

    #[test]
    fn parse_proxy_path_allow_empty_suffix_path_with_query() {
        let req =
            RequestHeader::build("GET", b"/sandboxes/abc/endpoints/3000?token=hello", None)
                .unwrap();
        let parsed = parse_proxy_path(&req).unwrap();

        assert_eq!(parsed.0, "abc");
        assert_eq!(parsed.1, 3000);
        assert_eq!(parsed.2, "/?token=hello");
    }

    #[test]
    fn rewrite_upstream_request_preserve_websocket_upgrade_headers() {
        let mut req = RequestHeader::build("GET", b"/origin?x=1", None).unwrap();
        req.insert_header("host", "example.com").unwrap();
        req.insert_header("connection", "Upgrade").unwrap();
        req.insert_header("upgrade", "websocket").unwrap();
        req.insert_header("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==")
            .unwrap();

        rewrite_upstream_request(&mut req, "/ws?token=hello", "172.18.0.2:3000").unwrap();

        assert_eq!(req.uri.path(), "/ws");
        assert_eq!(req.uri.query(), Some("token=hello"));
        assert_eq!(
            req.headers.get("host").unwrap().to_str().unwrap(),
            "172.18.0.2:3000"
        );
        assert_eq!(
            req.headers.get("connection").unwrap().to_str().unwrap(),
            "Upgrade"
        );
        assert_eq!(
            req.headers.get("upgrade").unwrap().to_str().unwrap(),
            "websocket"
        );
        assert_eq!(
            req.headers
                .get("sec-websocket-key")
                .unwrap()
                .to_str()
                .unwrap(),
            "dGhlIHNhbXBsZSBub25jZQ=="
        );
    }
}
