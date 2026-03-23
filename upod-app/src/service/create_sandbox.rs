use super::bridge_binary::{
    BRIDGE_BINARY_CONTAINER_PATH, build_bridge_archive, resolve_bridge_binary_path,
};
use super::sandbox_lifecycle::{SANDBOX_EXPIRES_AT_LABEL, SANDBOX_ID_LABEL};
use super::sandbox_store::{SandboxContainer, get_container, register_container};
use crate::core::code::Code;
use crate::models::sandbox::{CreateSandboxReq, CreateSandboxResp};
use crate::utils::id::generate_sandbox_id;
use crate::utils::resource_limits::{parse_cpu, parse_memory};
use bollard::Docker;
use bollard::body_full;
use bollard::query_parameters::CreateImageOptions;
use bollard::query_parameters::{
    CreateContainerOptions, StartContainerOptions, UploadToContainerOptionsBuilder,
};
use bollard::service::{ContainerCreateBody, HostConfig, PortBinding};
use chrono::{DateTime, Duration, Utc};
use futures_util::stream::StreamExt;
use std::collections::HashMap;
use std::path::Path;
use tracing::debug;
use upod_base::web::error::WebError;

/// 创建沙箱容器并注册生命周期信息。
/// 参数 req 为创建请求，返回沙箱 ID 与 Docker 警告信息。
/// 可能返回 Docker 连接、镜像拉取、容器创建等业务错误。
pub async fn create_sandbox(req: CreateSandboxReq) -> Result<CreateSandboxResp, WebError> {
    // 命中同 sandbox_id 的有效缓存时直接复用，避免重复创建容器。
    if let Some(resp) = build_cached_sandbox_response(req.sandbox_id.as_deref()) {
        return Ok(resp);
    }

    let docker = Docker::connect_with_local_defaults().map_err(docker_connect_error)?;

    ensure_image_exists(&docker, &req.image.uri).await?;

    let sandbox_id = resolve_sandbox_id(req.sandbox_id.as_deref());
    // 超时时间至少为 6 秒，防止出现立即过期的异常行为。
    let ttl_seconds = req.timeout.unwrap_or(6).max(1);
    let expires_at = (Utc::now() + Duration::seconds(ttl_seconds as i64)).to_rfc3339();
    let bridge_binary_host_path = resolve_bridge_binary_path()?;

    let container_config = build_container_config(&req, &sandbox_id, &expires_at)?;
    let container_options = build_container_options(&sandbox_id);

    let res = docker
        .create_container(container_options, container_config.config)
        .await
        .map_err(sandbox_create_error)?;

    copy_bridge_binary_to_container(&docker, &res.id, &bridge_binary_host_path).await?;

    docker
        .start_container(&res.id, None::<StartContainerOptions>)
        .await
        .map_err(sandbox_create_error)?;

    debug!(
        "sandbox {} created, ports: {:?}",
        sandbox_id, container_config.ports
    );
    register_container(
        &sandbox_id,
        SandboxContainer {
            container_id: res.id.clone(),
            ports: container_config.ports,
            expires_at: parse_rfc3339_utc(&expires_at).unwrap_or_else(Utc::now),
        },
    );

    Ok(CreateSandboxResp {
        id: sandbox_id,
        warnings: res.warnings,
    })
}

/// 在请求带 sandbox_id 且容器仍在缓存中时返回复用响应。
/// 参数 requested_sandbox_id 为用户传入的可选沙箱 ID。
/// 返回命中缓存时的响应；未命中或 ID 非法时返回 None。
fn build_cached_sandbox_response(requested_sandbox_id: Option<&str>) -> Option<CreateSandboxResp> {
    let sandbox_id = requested_sandbox_id
        .map(str::trim)
        .filter(|id| !id.is_empty())?;

    get_container(sandbox_id).map(|_| CreateSandboxResp {
        id: sandbox_id.to_string(),
        warnings: Vec::new(),
    })
}

/// 解析最终使用的沙箱 ID。
/// 参数 requested_sandbox_id 为用户传入的可选沙箱 ID。
/// 返回去空白后的原 ID；若为空则生成新 ID。
fn resolve_sandbox_id(requested_sandbox_id: Option<&str>) -> String {
    requested_sandbox_id
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(generate_sandbox_id)
}

/// 构造 Docker 容器配置和端口映射信息。
/// 参数分别为请求体、沙箱 ID、过期时间（RFC3339 字符串）。
/// 返回封装后的容器创建配置，不直接抛出异常。
fn build_container_config(
    req: &CreateSandboxReq,
    sandbox_id: &str,
    expires_at: &str,
) -> Result<BuiltContainerConfig, WebError> {
    let env_vars = req
        .env
        .as_ref()
        .map(|env| {
            env.iter()
                .map(|(k, v)| format!("{}={}", k, v))
                .collect::<Vec<String>>()
        })
        .unwrap_or_default();

    let mut labels = req.metadata.clone().unwrap_or_default();
    // 注入系统标签，供后续生命周期回收和检索使用。
    labels.insert(SANDBOX_ID_LABEL.to_string(), sandbox_id.to_string());
    labels.insert(SANDBOX_EXPIRES_AT_LABEL.to_string(), expires_at.to_string());

    let exposed_ports = vec!["8080/tcp".to_string(), "44321/tcp".to_string()];
    let mut port_bindings = HashMap::new();
    let mut ports = HashMap::new();

    for port_str in &exposed_ports {
        let (private_port_text, _) = port_str.split_once('/').unwrap_or((port_str, ""));
        let private_port = private_port_text.parse::<u16>().unwrap_or(0);

        let host_port = crate::utils::port::find_random_available_port().map_err(|e| {
            WebError::BizWithArgs(
                Code::SandboxCreateError.into(),
                vec![(
                    "error".to_string(),
                    format!("failed to allocate port: {}", e),
                )],
            )
        })?;

        port_bindings.insert(
            port_str.clone(),
            Some(vec![PortBinding {
                host_ip: Some("0.0.0.0".to_string()),
                host_port: Some(host_port.to_string()),
            }]),
        );
        ports.insert(private_port, host_port);
    }

    Ok(BuiltContainerConfig {
        config: ContainerCreateBody {
            image: Some(req.image.uri.clone()),
            entrypoint: Some(build_runtime_entrypoint()),
            cmd: Some(build_runtime_command(req)),
            env: Some(env_vars),
            exposed_ports: Some(exposed_ports),
            host_config: Some(build_host_config(req, port_bindings)),
            labels: Some(labels),
            ..Default::default()
        },
        ports,
    })
}

struct BuiltContainerConfig {
    config: ContainerCreateBody,
    ports: HashMap<u16, u16>,
}

/// 构建运行时入口命令，确保仅启动 bridge 进程。
/// 无参数，返回 Docker entrypoint 数组。
fn build_runtime_entrypoint() -> Vec<String> {
    vec![
        "/bin/sh".to_string(),
        "-lc".to_string(),
        format!("exec \"{BRIDGE_BINARY_CONTAINER_PATH}\""),
        "--".to_string(),
    ]
}

/// 构建容器启动命令。
/// 参数 req 为创建请求；若未指定 entrypoint 则返回保活命令。
/// 返回传递给 bridge 的原始目标命令参数。
fn build_runtime_command(req: &CreateSandboxReq) -> Vec<String> {
    req.entrypoint.clone().unwrap_or_else(|| {
        // 默认保活容器，等待后续通过 bridge 执行具体指令。
        vec![
            "tail".to_string(),
            "-f".to_string(),
            "/dev/null".to_string(),
        ]
    })
}

/// 构建容器 HostConfig 并应用资源限制。
/// 参数 req 为创建请求；返回可直接用于 Docker 的 HostConfig。
/// 资源格式非法时会被忽略，不在此函数抛错。
fn build_host_config(
    req: &CreateSandboxReq,
    port_bindings: HashMap<String, Option<Vec<PortBinding>>>,
) -> HostConfig {
    let mut host_config = HostConfig {
        port_bindings: Some(port_bindings),
        ..Default::default()
    };
    let Some(limits) = &req.resource_limits else {
        return host_config;
    };

    if let Some(memory) = limits.memory.as_deref().and_then(parse_memory) {
        host_config.memory = Some(memory);
    }

    if let Some(nano_cpus) = limits.cpu.as_deref().and_then(parse_cpu) {
        host_config.nano_cpus = Some(nano_cpus);
    }

    host_config
}

/// 将宿主机 bridge 二进制打包并上传到容器根目录。
/// 参数为 Docker 客户端、容器 ID 与 bridge 文件路径。
/// 可能返回压缩归档或上传过程中的业务错误。
async fn copy_bridge_binary_to_container(
    docker: &Docker,
    container_id: &str,
    bridge_binary_host_path: &Path,
) -> Result<(), WebError> {
    let archive = build_bridge_archive(bridge_binary_host_path)?;
    let options = UploadToContainerOptionsBuilder::default().path("/").build();
    docker
        .upload_to_container(container_id, Some(options), body_full(archive.into()))
        .await
        .map_err(sandbox_create_error)
}

/// 构建容器创建选项。
/// 参数 sandbox_id 用于生成稳定容器名，返回 CreateContainerOptions。
fn build_container_options(sandbox_id: &str) -> Option<CreateContainerOptions> {
    Some(CreateContainerOptions {
        name: Some(format!("upod-{sandbox_id}")),
        platform: "".to_string(),
    })
}

/// 将 RFC3339 时间字符串解析为 UTC 时间。
/// 参数 value 为时间字符串，解析失败时返回 None。
fn parse_rfc3339_utc(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

/// 将 Docker 连接错误映射为统一业务错误。
/// 参数 error 为底层 Docker SDK 错误，返回 WebError。
fn docker_connect_error(error: bollard::errors::Error) -> WebError {
    WebError::BizWithArgs(
        Code::DockerConnectError.into(),
        vec![("error".to_string(), error.to_string())],
    )
}

/// 将容器创建相关错误映射为统一业务错误。
/// 参数 error 为底层 Docker SDK 错误，返回 WebError。
fn sandbox_create_error(error: bollard::errors::Error) -> WebError {
    WebError::BizWithArgs(
        Code::SandboxCreateError.into(),
        vec![("error".to_string(), error.to_string())],
    )
}

/// 确保目标镜像可用，不存在时主动拉取。
/// 参数分别为 Docker 客户端与镜像名。
/// 可能返回镜像拉取失败等业务错误。
async fn ensure_image_exists(docker: &Docker, image: &str) -> Result<(), WebError> {
    if docker.inspect_image(image).await.is_ok() {
        return Ok(());
    }

    let options = Some(CreateImageOptions {
        from_image: Some(image.to_string()),
        ..Default::default()
    });

    let mut stream = docker.create_image(options, None, None);
    while let Some(result) = stream.next().await {
        if let Err(e) = result {
            return Err(WebError::BizWithArgs(
                Code::ImagePullError.into(),
                vec![("error".to_string(), e.to_string())],
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::sandbox::{Image, ResourceLimits};
    use crate::service::sandbox_store::clear_container;
    use tokio;

    #[tokio::test]
    async fn test_create_sandbox() {
        let req = CreateSandboxReq {
            sandbox_id: None,
            image: Image {
                uri: "python:3.11-slim".to_string(),
            },
            entrypoint: Some(vec!["python".to_string(), "--version".to_string()]),
            timeout: Some(3600),
            resource_limits: Some(ResourceLimits {
                cpu: Some("500m".to_string()),
                memory: Some("512Mi".to_string()),
            }),
            env: Some(std::collections::HashMap::from([(
                "TEST_ENV".to_string(),
                "1".to_string(),
            )])),
            metadata: Some(std::collections::HashMap::from([(
                "project".to_string(),
                "test".to_string(),
            )])),
        };

        let res = create_sandbox(req).await;
        println!("Result: {:?}", res);
    }

    #[test]
    fn test_build_container_config_includes_random_port_mapping() {
        let req = CreateSandboxReq {
            sandbox_id: None,
            image: Image {
                uri: "python:3.11-slim".to_string(),
            },
            entrypoint: None,
            timeout: None,
            resource_limits: None,
            env: None,
            metadata: None,
        };

        let container_config =
            build_container_config(&req, "sandbox-test", "2026-03-17T00:00:00Z").unwrap();
        let config = container_config.config;

        let exposed_ports = config.exposed_ports.expect("exposed ports should exist");
        assert!(exposed_ports.contains(&"8080/tcp".to_string()));
        assert!(exposed_ports.contains(&"44321/tcp".to_string()));

        let host_port_8080 = container_config.ports.get(&8080).expect("should map 8080");
        let host_port_44321 = container_config
            .ports
            .get(&44321)
            .expect("should map 44321");
        assert!(*host_port_8080 >= 40_000 && *host_port_8080 <= 60_000);
        assert!(*host_port_44321 >= 40_000 && *host_port_44321 <= 60_000);

        let host_config = config.host_config.expect("host config should exist");
        assert!(host_config.port_bindings.is_some());
        let bindings = host_config.port_bindings.unwrap();
        assert!(bindings.contains_key("8080/tcp"));
        assert!(bindings.contains_key("44321/tcp"));
        assert!(host_config.binds.is_none());
        assert_eq!(config.entrypoint, Some(build_runtime_entrypoint()));
        assert_eq!(
            config.cmd,
            Some(vec![
                "tail".to_string(),
                "-f".to_string(),
                "/dev/null".to_string()
            ])
        );
    }

    #[test]
    fn test_build_runtime_entrypoint_only_starts_bridge() {
        let entrypoint = build_runtime_entrypoint();
        assert_eq!(entrypoint[0], "/bin/sh");
        assert_eq!(entrypoint[1], "-lc");
        assert_eq!(entrypoint[3], "--");
        assert!(entrypoint[2].contains("/opt/upod/bin/upod-bridge"));
        assert_eq!(entrypoint[2], "exec \"/opt/upod/bin/upod-bridge\"");
    }

    #[test]
    fn test_resolve_sandbox_id_uses_requested_id() {
        let sandbox_id = resolve_sandbox_id(Some("sandbox-fixed-id"));
        assert_eq!(sandbox_id, "sandbox-fixed-id");
    }

    #[test]
    fn test_resolve_sandbox_id_generates_new_when_empty() {
        let sandbox_id = resolve_sandbox_id(Some("   "));
        assert!(!sandbox_id.trim().is_empty());
    }

    #[test]
    fn test_build_cached_sandbox_response_returns_existing_sandbox() {
        let sandbox_id = "sandbox-cache-hit";
        clear_container(sandbox_id);
        register_container(
            sandbox_id,
            SandboxContainer {
                container_id: "container-123".to_string(),
                ports: HashMap::new(),
                expires_at: Utc::now(),
            },
        );

        let resp = build_cached_sandbox_response(Some(sandbox_id));
        assert!(resp.is_some());
        let resp = resp.expect("cached sandbox response should exist");
        assert_eq!(resp.id, sandbox_id);
        assert!(resp.warnings.is_empty());
        clear_container(sandbox_id);
    }
}
