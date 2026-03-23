use async_trait::async_trait;
use pingora::http::RequestHeader;
use pingora::prelude::*;
use pingora::proxy::{ProxyHttp, Session, http_proxy_service};
use tracing::{info, warn};

use crate::service::sandbox_store;

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
    ///
    /// 1. 从请求路径解析出 `sandbox_id` 和容器内端口 `container_port`。
    /// 2. 从 `sandbox_store` 查询该沙箱的端口映射关系。
    /// 3. 将请求转发到宿主机的 `127.0.0.1:{host_port}`。
    async fn upstream_peer(
        &self,
        session: &mut Session,
        ctx: &mut Self::CTX,
    ) -> Result<Box<HttpPeer>> {
        let request_header = session.req_header();
        info!("Incoming proxy request: uri={}", request_header.uri);

        // 从原始请求路径中提取 sandbox_id / container_port / 重写后的 path+query。
        let (sandbox_id, container_port, rewritten_path) = parse_proxy_path(request_header)
            .ok_or_else(|| {
                warn!("parse_proxy_path failed for uri: {}", request_header.uri);
                Error::explain(
                    ErrorType::HTTPStatus(404),
                    "path should match /sandboxes/{sandbox_id}/port/{port}[/{full_path}]",
                )
            })?;

        // 从内存存储中获取沙箱信息
        let container = sandbox_store::get_container(&sandbox_id).ok_or_else(|| {
            warn!("sandbox not found in store: {}", sandbox_id);
            Error::explain(
                ErrorType::HTTPStatus(404),
                format!("sandbox not found in store: {sandbox_id}"),
            )
        })?;

        // 查找容器内端口对应的宿主机映射端口
        let host_port = container
            .ports
            .get(&container_port)
            .copied()
            .ok_or_else(|| {
                warn!(
                    "port {} is not mapped for sandbox {}, available ports: {:?}",
                    container_port, sandbox_id, container.ports
                );
                Error::explain(
                    ErrorType::HTTPStatus(404),
                    format!("port {container_port} is not mapped for sandbox {sandbox_id}"),
                )
            })?;

        info!("Proxying {} to 127.0.0.1:{}", sandbox_id, host_port);

        // 由于使用了端口映射，直接请求宿主机本地地址即可
        let host = "127.0.0.1";
        ctx.rewritten_path_with_query = rewritten_path;
        ctx.upstream_authority = format!("{host}:{host_port}");

        // 这里使用明文 HTTP（第二个参数为 false），转发到宿主机映射端口
        Ok(Box::new(HttpPeer::new(
            (host, host_port),
            false,
            String::new(),
        )))
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

/// 解析路径 `/sandboxes/{sandbox_id}/port/{port}[/{full_path}]`。
fn parse_proxy_path(header: &RequestHeader) -> Option<(String, u16, String)> {
    // 仅处理约定的网关前缀，非目标路径直接返回 None。
    let path = header.uri.path();
    let prefix = "/sandboxes/";
    let rest = path.strip_prefix(prefix)?;

    // 拆分出 sandbox_id 与 port 之后的部分。
    let (sandbox_id, rest) = rest.split_once("/port/")?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_proxy_path_user_example() {
        // 模拟用户给出的真实场景：
        // http://localhost:9000/sandboxes/ff222/port/44321/command -> http://127.0.0.1:52323/command
        let req =
            RequestHeader::build("GET", b"/sandboxes/ff222/port/44321/command", None).unwrap();
        let parsed = parse_proxy_path(&req).unwrap();

        assert_eq!(parsed.0, "ff222"); // sandbox_id
        assert_eq!(parsed.1, 44321); // container_port
        assert_eq!(parsed.2, "/command"); // rewritten_path
    }

    #[test]
    fn parse_proxy_path_with_suffix_and_query() {
        let req = RequestHeader::build(
            "GET",
            b"/sandboxes/sandbox-1/port/8080/api/v1/ping?hello=world",
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
        let req = RequestHeader::build("GET", b"/sandboxes/abc/port/3000/", None).unwrap();
        let parsed = parse_proxy_path(&req).unwrap();

        assert_eq!(parsed.0, "abc");
        assert_eq!(parsed.1, 3000);
        assert_eq!(parsed.2, "/");
    }

    #[test]
    fn parse_proxy_path_reject_invalid_port() {
        let req = RequestHeader::build("GET", b"/sandboxes/abc/port/not-port/index", None).unwrap();
        assert!(parse_proxy_path(&req).is_none());
    }

    #[test]
    fn parse_proxy_path_allow_empty_suffix_path() {
        let req = RequestHeader::build("GET", b"/sandboxes/abc/port/3000", None).unwrap();
        let parsed = parse_proxy_path(&req).unwrap();

        assert_eq!(parsed.0, "abc");
        assert_eq!(parsed.1, 3000);
        assert_eq!(parsed.2, "/");
    }

    #[test]
    fn parse_proxy_path_allow_empty_suffix_path_with_query() {
        let req =
            RequestHeader::build("GET", b"/sandboxes/abc/port/3000?token=hello", None).unwrap();
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
