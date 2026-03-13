use axum::Json;
use bollard::Docker;
use bollard::service::{ContainerCreateBody, HostConfig};
use bollard::query_parameters::{CreateContainerOptions, StartContainerOptions};
use bollard::query_parameters::CreateImageOptions;
use chrono::{Duration, Utc};
use futures_util::stream::StreamExt;
use upod_base::web::r::R;
use upod_base::web::error::WebError;
use upod_base::core::code::Code;
use super::docker::{SANDBOX_EXPIRES_AT_LABEL, SANDBOX_ID_LABEL, generate_sandbox_id};
use crate::models::sandbox::{CreateSandboxReq, CreateSandboxResp};

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
    let ttl_seconds = req.timeout.unwrap_or(3600).max(1);
    let expires_at = (Utc::now() + Duration::seconds(ttl_seconds as i64)).to_rfc3339();

    let config = build_container_config(&req, &sandbox_id, &expires_at);
    let options = build_container_options(&sandbox_id);

    // 4. 调用 Docker API 创建容器
    let res = match docker.create_container(options, config).await {
        Ok(res) => res,
        Err(error) => return R::err(sandbox_create_error(error)),
    };

    // 5. 启动容器
    if let Err(error) = docker.start_container(&res.id, None::<StartContainerOptions>).await {
        return R::err(sandbox_create_error(error));
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
    labels.insert(
        SANDBOX_EXPIRES_AT_LABEL.to_string(),
        expires_at.to_string(),
    );

    ContainerCreateBody {
        image: Some(req.image.uri.clone()),
        entrypoint: req.entrypoint.clone(),
        env: Some(env_vars),
        host_config: Some(build_host_config(req)),
        labels: Some(labels),
        ..Default::default()
    }
}

fn build_host_config(req: &CreateSandboxReq) -> HostConfig {
    // 仅在用户提供 resource_limits 时写入对应限制
    let mut host_config = HostConfig::default();
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
             return Err(WebError::BizWithArgs(Code::ImagePullError.into(), vec![("error".to_string(), e.to_string())]));
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
        cpu.trim_end_matches('m').parse::<f64>().ok().map(|v| (v * 1_000_000.0) as i64) 
    } else {
        // 1 => 1 CPU => 1_000_000_000 nano_cpus
        cpu.parse::<f64>().ok().map(|v| (v * 1_000_000_000.0) as i64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Json;
    use std::collections::HashMap;
    use tokio;
    use crate::models::sandbox::{Image, ResourceLimits};

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
            env: Some(HashMap::from([("TEST_ENV".to_string(), "1".to_string())])),
            metadata: Some(HashMap::from([("project".to_string(), "test".to_string())])),
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

}
