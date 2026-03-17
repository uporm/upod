use super::docker::{
    SANDBOX_EXPIRES_AT_LABEL, SANDBOX_ID_LABEL, sync_sandbox_port_mappings_from_detail,
};
use crate::core::code::Code;
use crate::models::sandbox::{CreateSandboxReq, CreateSandboxResp};
use crate::utils::id::generate_sandbox_id;
use axum::Json;
use bollard::body_full;
use bollard::Docker;
use bollard::query_parameters::CreateImageOptions;
use bollard::query_parameters::{
    CreateContainerOptions, StartContainerOptions, UploadToContainerOptionsBuilder,
};
use bollard::service::{ContainerCreateBody, HostConfig};
use chrono::{Duration, Utc};
use futures_util::stream::StreamExt;
use std::io::Read;
use std::path::{Path, PathBuf};
use tar::{Builder, Header};
use upod_base::web::error::WebError;
use upod_base::web::r::R;

const BRIDGE_BINARY_CONTAINER_PATH: &str = "/opt/upod/bin/upod-bridge";

pub fn ensure_bridge_binary_ready() -> Result<(), WebError> {
    resolve_bridge_binary_path().map(|_| ())
}

/// 创建沙箱环境
///
/// 该接口接收创建请求，配置并启动一个 Docker 容器作为沙箱。
pub async fn create_sandbox(Json(req): Json<CreateSandboxReq>) -> R<CreateSandboxResp> {
    // 1. 连接到本地 Docker 守护进程
    let docker = match Docker::connect_with_local_defaults() {
        Ok(docker) => docker,
        Err(error) => return R::err(docker_connect_error(error)),
    };

    // 2. 确保镜像存在，如果不存在则拉取
    if let Err(error) = ensure_image_exists(&docker, &req.image.uri).await {
        return R::err(error);
    }

    let sandbox_id = generate_sandbox_id();
    // timeout 最小值限制为 1 秒，避免传入 0 导致“刚创建即过期”。
    let ttl_seconds = req.timeout.unwrap_or(3600).max(1);
    let expires_at = (Utc::now() + Duration::seconds(ttl_seconds as i64)).to_rfc3339();
    // 在创建容器前就确认桥接程序位置，失败时尽早返回，避免创建无效容器。
    let bridge_binary_host_path = match resolve_bridge_binary_path() {
        Ok(path) => path,
        Err(error) => return R::err(error),
    };

    let config = build_container_config(&req, &sandbox_id, &expires_at);
    let options = build_container_options(&sandbox_id);

    // 4. 调用 Docker API 创建容器
    let res = match docker.create_container(options, config).await {
        Ok(res) => res,
        Err(error) => return R::err(sandbox_create_error(error)),
    };

    if let Err(error) =
        copy_bridge_binary_to_container(&docker, &res.id, &bridge_binary_host_path).await
    {
        return R::err(error);
    }

    // 5. 启动容器
    if let Err(error) = docker
        .start_container(&res.id, None::<StartContainerOptions>)
        .await
    {
        return R::err(sandbox_create_error(error));
    }
    // 创建后立刻预热端口映射缓存，减少容器首次 endpoint 查询时的回源开销。
    // inspect 失败不影响创建成功语义，仅跳过预热，后续查询会走慢路径回填。
    if let Ok(detail) = docker.inspect_container(&res.id, None).await {
        sync_sandbox_port_mappings_from_detail(&res.id, &detail);
    }

    // 6. 返回创建结果
    // warnings 来自 Docker API，通常用于提示镜像或容器配置中的非阻断问题
    let resp = CreateSandboxResp {
        id: sandbox_id,
        warnings: res.warnings,
    };
    R::ok(resp)
}

fn build_container_config(
    req: &CreateSandboxReq,
    sandbox_id: &str,
    expires_at: &str,
) -> ContainerCreateBody {
    // 将 map 形式的环境变量转换成 Docker 需要的 "KEY=VALUE" 形式
    let env_vars = req
        .env
        .as_ref()
        .map(|env| {
            env.iter()
                .map(|(k, v)| format!("{}={}", k, v))
                .collect::<Vec<String>>()
        })
        .unwrap_or_default();

    // 保留用户传入 metadata，同时写入系统保留标签用于追踪与回收
    let mut labels = req.metadata.clone().unwrap_or_default();
    labels.insert(SANDBOX_ID_LABEL.to_string(), sandbox_id.to_string());
    labels.insert(SANDBOX_EXPIRES_AT_LABEL.to_string(), expires_at.to_string());

    // 本项目的运行时入口设计为：
    // 1) 先启动 upod-bridge（后台）；
    // 2) 再通过 exec 切到业务命令，保证 PID1 为业务进程，信号处理更符合预期。
    ContainerCreateBody {
        image: Some(req.image.uri.clone()),
        entrypoint: Some(build_runtime_entrypoint()),
        cmd: Some(build_runtime_command(req)),
        env: Some(env_vars),
        exposed_ports: Some(build_exposed_ports()),
        host_config: Some(build_host_config(req)),
        labels: Some(labels),
        ..Default::default()
    }
}

fn build_exposed_ports() -> Vec<String> {
    vec!["8080/tcp".to_string(), "45000/tcp".to_string()]
}

fn build_runtime_entrypoint() -> Vec<String> {
    // /bin/sh -lc "<script>" -- <cmd...> 语义说明：
    // - exec "$@" 用传入命令替换 shell 进程，避免多一层壳影响信号转发；
    // - "--" 用于分隔 shell 参数与后续业务命令，避免命令被 shell 误解析。
    vec![
        "/bin/sh".to_string(),
        "-lc".to_string(),
        format!(
            "bridge_bin=\"{BRIDGE_BINARY_CONTAINER_PATH}\"; bridge_dir=\"$(dirname \"$bridge_bin\")\"; echo \"starting upod-bridge: $bridge_bin\" >&2; if [ ! -x \"$bridge_bin\" ]; then echo \"upod-bridge missing or not executable: $bridge_bin\" >&2; ls -la \"$bridge_dir\" >&2 || true; exit 127; fi; \"$bridge_bin\" & bridge_pid=$!; sleep 1; if ! kill -0 \"$bridge_pid\" 2>/dev/null; then echo \"upod-bridge exited during startup\" >&2; wait \"$bridge_pid\"; exit 1; fi; echo \"upod-bridge started, pid=$bridge_pid\" >&2; if [ \"$#\" -gt 0 ]; then \"$@\"; cmd_exit=$?; echo \"sandbox entrypoint exited with code $cmd_exit\" >&2; fi; wait \"$bridge_pid\""
        ),
        "--".to_string(),
    ]
}

fn build_runtime_command(req: &CreateSandboxReq) -> Vec<String> {
    // 请求提供 entrypoint 时，直接作为业务命令透传。
    // 未提供时使用 tail -f /dev/null 作为兜底，保持容器常驻，便于后续调试与连接。
    req.entrypoint.clone().unwrap_or_else(|| {
        vec![
            "tail".to_string(),
            "-f".to_string(),
            "/dev/null".to_string(),
        ]
    })
}

fn build_host_config(req: &CreateSandboxReq) -> HostConfig {
    let mut host_config = HostConfig {
        publish_all_ports: Some(true),
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

async fn copy_bridge_binary_to_container(
    docker: &Docker,
    container_id: &str,
    bridge_binary_host_path: &Path,
) -> Result<(), WebError> {
    let archive = build_bridge_archive(bridge_binary_host_path)?;
    let options = UploadToContainerOptionsBuilder::default()
        .path("/")
        .build();
    docker
        .upload_to_container(container_id, Some(options), body_full(archive.into()))
        .await
        .map_err(sandbox_create_error)
}

fn build_bridge_archive(bridge_binary_host_path: &Path) -> Result<Vec<u8>, WebError> {
    let bridge_binary = std::fs::read(bridge_binary_host_path).map_err(|error| {
        bridge_runtime_error(format!(
            "read upod-bridge binary failed ({}): {error}",
            bridge_binary_host_path.display()
        ))
    })?;
    let mut archive = Builder::new(Vec::<u8>::new());
    let bridge_binary_rel_path = BRIDGE_BINARY_CONTAINER_PATH.trim_start_matches('/');
    append_parent_directories(&mut archive, bridge_binary_rel_path)?;

    let mut bridge_header = Header::new_gnu();
    bridge_header.set_size(bridge_binary.len() as u64);
    bridge_header.set_mode(0o755);
    bridge_header.set_cksum();
    archive
        .append_data(
            &mut bridge_header,
            bridge_binary_rel_path,
            bridge_binary.as_slice(),
        )
        .map_err(|error| bridge_runtime_error(format!("pack upod-bridge archive failed: {error}")))?;

    archive
        .into_inner()
        .map_err(|error| bridge_runtime_error(format!("finalize upod-bridge archive failed: {error}")))
}

fn append_parent_directories(
    archive: &mut Builder<Vec<u8>>,
    file_rel_path: &str,
) -> Result<(), WebError> {
    let Some(parent) = Path::new(file_rel_path).parent() else {
        return Ok(());
    };
    let mut current = PathBuf::new();
    for component in parent.components() {
        current.push(component.as_os_str());
        let current = current.to_string_lossy();
        if !current.is_empty() {
            append_directory(archive, current.as_ref())?;
        }
    }
    Ok(())
}

fn append_directory(archive: &mut Builder<Vec<u8>>, path: &str) -> Result<(), WebError> {
    let mut header = Header::new_gnu();
    header.set_entry_type(tar::EntryType::Directory);
    header.set_mode(0o755);
    header.set_size(0);
    header.set_cksum();
    archive
        .append_data(&mut header, path, std::io::empty())
        .map_err(|error| bridge_runtime_error(format!("pack directory {path} failed: {error}")))
}

fn build_container_options(sandbox_id: &str) -> Option<CreateContainerOptions> {
    // 以固定前缀 + sandbox_id 命名，便于在 Docker 侧快速检索
    Some(CreateContainerOptions {
        name: Some(format!("upod-{sandbox_id}")),
        platform: "".to_string(),
    })
}

fn docker_connect_error(error: bollard::errors::Error) -> WebError {
    WebError::BizWithArgs(
        Code::DockerConnectError.into(),
        vec![("error".to_string(), error.to_string())],
    )
}

fn sandbox_create_error(error: bollard::errors::Error) -> WebError {
    WebError::BizWithArgs(
        Code::SandboxCreateError.into(),
        vec![("error".to_string(), error.to_string())],
    )
}

fn bridge_runtime_error(message: impl Into<String>) -> WebError {
    WebError::BizWithArgs(
        Code::SandboxCreateError.into(),
        vec![("error".to_string(), message.into())],
    )
}

fn resolve_bridge_binary_path() -> Result<PathBuf, WebError> {
    const BRIDGE_BINARY_ENV: &str = "UPOD_BRIDGE_BIN";
    if let Ok(path) = std::env::var(BRIDGE_BINARY_ENV) {
        let value = PathBuf::from(path);
        if value.is_file() {
            if !is_elf_binary(&value) {
                return Err(bridge_runtime_error(format!(
                    "{BRIDGE_BINARY_ENV} is not a Linux ELF binary: {}",
                    value.display()
                )));
            }
            return Ok(value);
        }
        return Err(bridge_runtime_error(format!(
            "{BRIDGE_BINARY_ENV} points to a non-existent file: {}",
            value.display()
        )));
    }

    let current_exe = std::env::current_exe().map_err(|error| {
        bridge_runtime_error(format!("resolve current executable path failed: {error}"))
    })?;
    let bin_dir = current_exe.parent().ok_or_else(|| {
        bridge_runtime_error("current executable has no parent directory".to_string())
    })?;

    let mut candidates = Vec::new();
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    if let Some(repo_root) = manifest_dir.parent() {
        candidates.push(repo_root.join("target/release/upod-bridge"));
        candidates.push(repo_root.join("target/x86_64-unknown-linux-gnu/release/upod-bridge"));
        candidates.push(repo_root.join("target/aarch64-unknown-linux-gnu/release/upod-bridge"));
    }
    candidates.push(bin_dir.join("upod-bridge"));

    if let Some(candidate) = candidates
        .iter()
        .find(|path| path.is_file() && is_elf_binary(path))
    {
        return Ok(candidate.clone());
    }

    let non_elf_files = candidates
        .iter()
        .filter(|path| path.is_file())
        .map(|path| path.display().to_string())
        .collect::<Vec<String>>()
        .join(", ");
    if !non_elf_files.is_empty() {
        return Err(bridge_runtime_error(format!(
            "upod-bridge binary found but not Linux ELF format: {non_elf_files}"
        )));
    }

    let checked_paths = candidates
        .into_iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<String>>()
        .join(", ");
    Err(bridge_runtime_error(format!(
        "upod-bridge binary not found, checked paths: {checked_paths}"
    )))
}

fn is_elf_binary(path: &Path) -> bool {
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    let mut magic = [0u8; 4];
    file.read_exact(&mut magic).is_ok() && magic == [0x7f, b'E', b'L', b'F']
}

/// 辅助函数：确保镜像存在
async fn ensure_image_exists(docker: &Docker, image: &str) -> Result<(), WebError> {
    // 检查镜像是否存在
    if docker.inspect_image(image).await.is_ok() {
        return Ok(());
    }

    // 镜像不存在，尝试拉取
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

/// 辅助函数：解析内存字符串
/// 支持格式：512Mi, 1Gi, 100M, 1G 等
fn parse_memory(mem: &str) -> Option<i64> {
    let mem = mem.trim();
    if mem.is_empty() {
        None
    } else {
        // 优先解析带单位值，其次回退纯数字（默认按字节处理）
        parse_memory_with_suffix(mem, 2)
            .or_else(|| parse_memory_with_suffix(mem, 1))
            .or_else(|| mem.parse::<i64>().ok())
    }
}

fn parse_memory_with_suffix(mem: &str, suffix_len: usize) -> Option<i64> {
    if mem.len() <= suffix_len {
        None
    } else {
        let (num, unit) = mem.split_at(mem.len() - suffix_len);
        let value = num.parse::<i64>().ok()?;
        // 同时兼容 IEC（Mi/Gi）与 SI（M/G）单位
        let multiplier = match unit {
            "Mi" => 1024 * 1024,
            "Gi" => 1024 * 1024 * 1024,
            "M" => 1000 * 1000,
            "G" => 1000 * 1000 * 1000,
            _ => return None,
        };

        Some(value * multiplier)
    }
}

/// 辅助函数：解析 CPU 字符串
/// 支持格式：500m (0.5 CPU), 1 (1 CPU) 等
fn parse_cpu(cpu: &str) -> Option<i64> {
    let cpu = cpu.trim();
    if cpu.ends_with('m') {
        // 500m => 0.5 CPU => 500_000_000 nano_cpus
        cpu.trim_end_matches('m')
            .parse::<f64>()
            .ok()
            .map(|v| (v * 1_000_000.0) as i64)
    } else {
        // 1 => 1 CPU => 1_000_000_000 nano_cpus
        cpu.parse::<f64>()
            .ok()
            .map(|v| (v * 1_000_000_000.0) as i64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::sandbox::{Image, ResourceLimits};
    use axum::Json;
    use tokio;

    #[tokio::test]
    async fn test_create_sandbox() {
        let req = CreateSandboxReq {
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

        // 注意：此测试需要本地 Docker 环境运行
        let res = create_sandbox(Json(req)).await;
        println!("Result: {:?}", serde_json::to_string(&res));
    }

    #[test]
    fn test_parse_memory() {
        assert_eq!(parse_memory("512Mi"), Some(512 * 1024 * 1024));
        assert_eq!(parse_memory("1Gi"), Some(1024 * 1024 * 1024));
        assert_eq!(parse_memory("100M"), Some(100 * 1000 * 1000));
        assert_eq!(parse_memory(""), None);
    }

    #[test]
    fn test_parse_cpu() {
        assert_eq!(parse_cpu("500m"), Some(500_000_000));
        assert_eq!(parse_cpu("1"), Some(1_000_000_000));
        assert_eq!(parse_cpu("0.5"), Some(500_000_000));
    }

    #[test]
    fn test_build_container_config_includes_random_port_mapping() {
        let req = CreateSandboxReq {
            image: Image {
                uri: "python:3.11-slim".to_string(),
            },
            entrypoint: None,
            timeout: None,
            resource_limits: None,
            env: None,
            metadata: None,
        };

        let config = build_container_config(&req, "sandbox-test", "2026-03-17T00:00:00Z");

        let exposed_ports = config.exposed_ports.expect("exposed ports should exist");
        assert!(exposed_ports.contains(&"8080/tcp".to_string()));
        assert!(exposed_ports.contains(&"45000/tcp".to_string()));

        let host_config = config.host_config.expect("host config should exist");
        assert_eq!(host_config.publish_all_ports, Some(true));
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
    fn test_build_runtime_entrypoint_uses_bridge_stdout() {
        let entrypoint = build_runtime_entrypoint();
        assert_eq!(entrypoint[0], "/bin/sh");
        assert_eq!(entrypoint[1], "-lc");
        assert_eq!(entrypoint[3], "--");
        assert!(entrypoint[2].contains("/opt/upod/bin/upod-bridge"));
        assert!(entrypoint[2].contains("starting upod-bridge"));
        assert!(entrypoint[2].contains("if [ ! -x \"$bridge_bin\" ]"));
        assert!(entrypoint[2].contains("kill -0 \"$bridge_pid\""));
        assert!(entrypoint[2].contains("upod-bridge started, pid="));
        assert!(entrypoint[2].contains("sandbox entrypoint exited with code"));
        assert!(entrypoint[2].contains("wait \"$bridge_pid\""));
        assert!(!entrypoint[2].contains("\"$bridge_bin\" >"));
        assert!(!entrypoint[2].contains("2>&1"));
    }

    #[test]
    fn test_build_bridge_archive_contains_bridge_binary() {
        let bridge_path = std::env::temp_dir().join(format!(
            "upod-bridge-test-{}-{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::write(&bridge_path, b"bridge-binary").expect("write temp bridge binary");

        let archive = build_bridge_archive(&bridge_path).expect("build bridge archive");
        assert!(!archive.is_empty());
        let mut entries = tar::Archive::new(std::io::Cursor::new(archive));
        let paths = entries
            .entries()
            .expect("read archive entries")
            .map(|entry| {
                entry
                    .expect("read archive entry")
                    .path()
                    .expect("read archive path")
                    .to_string_lossy()
                    .to_string()
            })
            .collect::<Vec<_>>();
        assert!(paths.contains(&"opt".to_string()));
        assert!(paths.contains(&"opt/upod".to_string()));
        assert!(paths.contains(&"opt/upod/bin".to_string()));
        assert!(paths.contains(&"opt/upod/bin/upod-bridge".to_string()));

        let _ = std::fs::remove_file(bridge_path);
    }

    #[test]
    fn test_is_elf_binary() {
        let elf_path = std::env::temp_dir().join(format!(
            "upod-bridge-elf-test-{}-{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::write(&elf_path, [0x7f, b'E', b'L', b'F', 1, 2, 3, 4]).expect("write elf file");
        assert!(is_elf_binary(&elf_path));
        let _ = std::fs::remove_file(elf_path);

        let non_elf_path = std::env::temp_dir().join(format!(
            "upod-bridge-non-elf-test-{}-{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::write(&non_elf_path, b"MACH").expect("write non-elf file");
        assert!(!is_elf_binary(&non_elf_path));
        let _ = std::fs::remove_file(non_elf_path);
    }
}
