//! 沙箱容器生命周期管理模块
//!
//! 本模块负责管理沙箱容器的注册、查询、同步和过期清理等功能。
//! 主要包含以下核心功能：
//! - 容器信息存储：使用 DashMap 实现线程安全的容器信息缓存
//! - 过期清理：定期检查并清理已过期的沙箱容器
//! - 状态同步：从 Docker 守护进程同步容器状态到内存存储

use std::collections::HashMap;
use std::sync::OnceLock;

use bollard::Docker;
use bollard::errors::Error as DockerError;
use bollard::query_parameters::{
    ListContainersOptions, RemoveContainerOptions, StopContainerOptions,
};
use chrono::{DateTime, Utc};
use tokio::time::{Duration, sleep};

use crate::core::code::Code;
use upod_base::web::error::WebError;

/// 沙箱 ID 标签键，用于在 Docker 容器上标识沙箱
pub(crate) const SANDBOX_ID_LABEL: &str = "upod.io/sandbox-id";

/// 过期时间标签键，用于记录沙箱的过期时间（RFC3339 格式）
pub(crate) const SANDBOX_EXPIRES_AT_LABEL: &str = "upod.io/expires-at";

use super::sandbox_store::{
    SandboxContainer, clear_container, get_containers, replace_all_containers,
};

/// 清理任务启动标记
///
/// 用于确保过期清理任务只启动一次，避免重复创建任务。
static CLEANUP_TASK_STARTED: OnceLock<()> = OnceLock::new();

/// 启动沙箱存储同步任务
///
/// 创建一个后台异步任务，每 30 秒从 Docker 守护进程同步沙箱容器状态到内存存储。
/// 这确保了服务重启后能够恢复之前创建的沙箱信息。
pub fn start_sandbox_store_sync_task() {
    tokio::spawn(async {
        loop {
            if let Err(error) = sync_sandbox_store_from_containers().await {
                tracing::warn!("sync sandbox store failed: {}", error);
            }
            sleep(Duration::from_secs(30)).await;
        }
    });
}

/// 启动过期清理任务
///
/// 创建一个后台异步任务，每 30 秒检查并清理已过期的沙箱容器。
/// 使用 OnceLock 确保任务只启动一次，避免重复创建。
pub fn start_expiration_cleanup_task() {
    CLEANUP_TASK_STARTED.get_or_init(|| {
        tokio::spawn(async {
            loop {
                cleanup_expired_sandboxes().await;
                sleep(Duration::from_secs(30)).await;
            }
        });
    });
}

/// 清理已过期的沙箱容器
///
/// 执行以下步骤：
/// 1. 从内存存储中获取所有沙箱容器信息
/// 2. 检查每个容器是否已过期
/// 3. 对于已过期的容器，调用 `force_remove_sandbox` 强制删除
async fn cleanup_expired_sandboxes() {
    let now = Utc::now();
    let store = get_containers();

    // 找出所有已过期的沙箱 ID
    let mut expired_sandboxes = Vec::new();
    for entry in store.iter() {
        if entry.value().expires_at <= now {
            expired_sandboxes.push(entry.key().clone());
        }
    }

    // 清理已过期的沙箱
    for sandbox_id in expired_sandboxes {
        if let Err(error) = force_remove_sandbox(&sandbox_id).await {
            tracing::warn!("remove expired sandbox {} failed: {:?}", sandbox_id, error);
        }
    }
}

/// 强制删除 Docker 容器并清理沙箱缓存
///
/// 包含以下步骤：
/// 1. 获取容器 ID（优先查缓存，缓存不命中查 Docker）
/// 2. 尝试恢复容器运行（忽略非暂停状态等潜在错误）
/// 3. 尝试优雅停止容器（忽略已停止等潜在错误）
/// 4. 强制删除容器
/// 5. 从内存存储中移除缓存记录
///
/// # 参数
/// - `sandbox_id`: 沙箱唯一标识符
///
/// # 返回
/// 成功返回 `Ok(true)`，如果容器不存在返回 `Ok(false)` 并清理缓存，失败返回 WebError
pub(crate) async fn force_remove_sandbox(sandbox_id: &str) -> Result<bool, WebError> {
    let docker = Docker::connect_with_local_defaults().map_err(docker_connect_error)?;

    // 优先从缓存获取容器 ID
    let container_id = if let Some(container) = super::sandbox_store::get_container(sandbox_id) {
        container.container_id
    } else {
        // 如果缓存没有，通过 Docker label 查找，避免漏删
        let mut filters = HashMap::new();
        filters.insert(
            "label".to_string(),
            vec![format!("{}={}", SANDBOX_ID_LABEL, sandbox_id)],
        );
        let containers = docker
            .list_containers(Some(ListContainersOptions {
                all: true,
                filters: Some(filters),
                ..Default::default()
            }))
            .await
            .map_err(|e| {
                WebError::BizWithArgs(
                    Code::SandboxDeleteError.into(),
                    vec![("error".to_string(), e.to_string())],
                )
            })?;

        if let Some(c) = containers.into_iter().next() {
            c.id.unwrap_or_default()
        } else {
            return Err(WebError::Biz(Code::SandboxNotFound.into()));
        }
    };

    // 1. 尝试恢复运行，确保处于暂停状态的容器可以被正常删除
    let _ = docker.unpause_container(&container_id).await;

    // 2. 尝试优雅停止容器，忽略可能的错误
    let _ = docker
        .stop_container(&container_id, None::<StopContainerOptions>)
        .await;

    // 3. 强制删除容器并清理内存记录
    match docker
        .remove_container(
            &container_id,
            Some(RemoveContainerOptions {
                force: true,
                ..Default::default()
            }),
        )
        .await
    {
        // 删除成功
        Ok(_) => {
            clear_container(sandbox_id);
            Ok(true)
        }
        // 或是 404: 容器已被删除，只需清理内存记录
        Err(DockerError::DockerResponseServerError {
            status_code: 404, ..
        }) => {
            clear_container(sandbox_id);
            Ok(false)
        }
        Err(error) => Err(WebError::BizWithArgs(
            Code::SandboxDeleteError.into(),
            vec![("error".to_string(), error.to_string())],
        )),
    }
}

/// 解析 RFC3339 格式的 UTC 时间字符串
///
/// # 参数
/// - `value`: RFC3339 格式的时间字符串
///
/// # 返回
/// 解析成功返回 `Some(DateTime<Utc>)`，失败返回 `None`
fn parse_rfc3339_utc(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

/// 将 Docker 连接错误转换为 WebError
///
/// 用于统一处理 Docker 守护进程连接失败的情况。
pub(crate) fn docker_connect_error(error: DockerError) -> WebError {
    WebError::BizWithArgs(
        Code::DockerConnectError.into(),
        vec![("error".to_string(), error.to_string())],
    )
}

/// 将 Docker 生命周期错误转换为 WebError
///
/// 对于 404 错误，转换为沙箱未找到错误；其他错误转换为生命周期错误。
pub(crate) fn map_lifecycle_or_not_found_error(error: DockerError) -> WebError {
    if let DockerError::DockerResponseServerError {
        status_code: 404, ..
    } = error
    {
        return WebError::Biz(Code::SandboxNotFound.into());
    }
    WebError::BizWithArgs(
        Code::SandboxLifecycleError.into(),
        vec![("error".to_string(), error.to_string())],
    )
}

/// 从 Docker 守护进程同步沙箱容器状态到内存存储
///
/// 此函数用于服务启动或运行时恢复沙箱信息：
/// 1. 列出所有带有沙箱标签的容器
/// 2. 检查每个容器获取端口映射信息
/// 3. 从标签解析过期时间
/// 4. 清空现有存储并重新填充
///
/// # 返回
/// 成功返回 `Ok(())`，失败返回 Docker 错误
async fn sync_sandbox_store_from_containers() -> Result<(), DockerError> {
    let docker = Docker::connect_with_local_defaults()?;
    let containers = list_sandbox_containers(&docker).await?;

    // 构建新的沙箱信息映射
    let mut next = HashMap::new();

    for container in containers {
        let Some(container_id) = container.id else {
            continue;
        };

        let labels = container.labels.unwrap_or_default();
        let sandbox_id = resolve_sandbox_id(Some(&labels), &container_id);

        // 获取容器的端口映射信息
        let Ok(Some(ports)) = inspect_container_ports(&docker, &container_id).await else {
            continue;
        };

        // 解析过期时间，如果没有设置则使用当前时间（立即过期）
        let expires_at = labels
            .get(SANDBOX_EXPIRES_AT_LABEL)
            .and_then(|value| parse_rfc3339_utc(value))
            .unwrap_or_else(Utc::now);

        next.insert(
            sandbox_id,
            SandboxContainer {
                container_id,
                ports,
                expires_at,
            },
        );
    }

    // 原子性地替换存储内容
    replace_all_containers(next);

    Ok(())
}

/// 列出所有沙箱容器
///
/// 使用沙箱 ID 标签过滤 Docker 容器列表。
///
/// # 参数
/// - `docker`: Docker 客户端引用
///
/// # 返回
/// 成功返回容器摘要列表，失败返回 Docker 错误
async fn list_sandbox_containers(
    docker: &Docker,
) -> Result<Vec<bollard::models::ContainerSummary>, DockerError> {
    let mut filters = HashMap::new();
    filters.insert("label".to_string(), vec![SANDBOX_ID_LABEL.to_string()]);

    docker
        .list_containers(Some(ListContainersOptions {
            all: true,
            filters: Some(filters),
            ..Default::default()
        }))
        .await
}

/// 检查容器并提取端口映射
///
/// 通过 Docker API 检查容器详情，提取端口映射信息。
///
/// # 参数
/// - `docker`: Docker 客户端引用
/// - `container_id`: 容器 ID
///
/// # 返回
/// - `Ok(Some(ports))`: 成功获取端口映射
/// - `Ok(None)`: 容器不存在（404）
/// - `Err(error)`: 其他 Docker 错误
async fn inspect_container_ports(
    docker: &Docker,
    container_id: &str,
) -> Result<Option<HashMap<u16, u16>>, DockerError> {
    let detail = match docker.inspect_container(container_id, None).await {
        Ok(detail) => detail,
        // 容器不存在时返回 None
        Err(DockerError::DockerResponseServerError {
            status_code: 404, ..
        }) => return Ok(None),
        Err(error) => return Err(error),
    };

    Ok(Some(extract_ports_from_detail(&detail)))
}

/// 从容器检查响应中提取端口映射
///
/// 解析容器的网络设置，提取端口绑定信息。
/// 端口键格式为 "端口号/协议"（如 "8080/tcp"）。
///
/// # 参数
/// - `detail`: 容器检查响应
///
/// # 返回
/// 端口映射 HashMap：键为容器内部端口，值为宿主机端口
fn extract_ports_from_detail(
    detail: &bollard::models::ContainerInspectResponse,
) -> HashMap<u16, u16> {
    detail
        .network_settings
        .as_ref()
        .and_then(|ns| ns.ports.as_ref())
        .map(|ports| {
            ports
                .iter()
                .filter_map(|(port_key, bindings)| {
                    // 解析内部端口
                    let private_port = port_key.split_once('/')?.0.parse::<u16>().ok()?;
                    // 解析绑定的宿主机端口
                    let host_port = bindings
                        .as_ref()?
                        .first()?
                        .host_port
                        .as_ref()?
                        .parse::<u16>()
                        .ok()?;
                    Some((private_port, host_port))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// 从容器标签解析沙箱 ID
///
/// 优先从标签中获取沙箱 ID，如果不存在则使用 fallback 值。
///
/// # 参数
/// - `labels`: 容器标签映射
/// - `fallback`: 备用值（通常为容器 ID）
///
/// # 返回
/// 解析出的沙箱 ID
pub(crate) fn resolve_sandbox_id(
    labels: Option<&HashMap<String, String>>,
    fallback: &str,
) -> String {
    labels
        .and_then(|values| values.get(SANDBOX_ID_LABEL))
        .cloned()
        .unwrap_or_else(|| fallback.to_string())
}
