use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use crate::core::code::Code;
use bollard::Docker;
use bollard::errors::Error as DockerError;
use bollard::query_parameters::{ListContainersOptions, RemoveContainerOptions};
use chrono::{DateTime, Utc};
use tokio::time::{Duration, sleep};
use upod_base::web::error::WebError;

pub(crate) const SANDBOX_ID_LABEL: &str = "upod.io/sandbox-id";
pub(crate) const SANDBOX_EXPIRES_AT_LABEL: &str = "upod.io/expires-at";
// 续期接口写入的内存态过期时间，优先于容器 label。
static EXPIRATION_STORE: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
// 端口映射缓存：key: 容器 ID, val: 该容器内端口 -> 宿主机映射端口
// 设计目标是让端口查询优先走内存，减少高频 inspect 带来的 Docker API 开销。
static PORT_MAPPING_STORE: OnceLock<Mutex<HashMap<String, HashMap<u16, u16>>>> = OnceLock::new();
// 全局清理任务只允许启动一次，避免重复扫描与重复删除。
static CLEANUP_TASK_STARTED: OnceLock<()> = OnceLock::new();

pub(crate) fn resolve_sandbox_id(
    labels: Option<&HashMap<String, String>>,
    fallback: &str,
) -> String {
    labels
        .and_then(|values| values.get(SANDBOX_ID_LABEL))
        .cloned()
        .unwrap_or_else(|| fallback.to_string())
}

pub fn start_expiration_cleanup_task() {
    // 路由初始化可能被重复触发，这里确保全局只启动一个后台任务。
    CLEANUP_TASK_STARTED.get_or_init(|| {
        tokio::spawn(async {
            // 服务启动后先做一次全量同步，避免重启后内存态为空导致清理判断滞后。
            if let Err(error) = sync_expiration_store_from_containers().await {
                tracing::warn!("sync sandbox expirations failed: {}", error);
            }
            // 启动即同步端口映射，避免服务重启后首次端口查询全部回源 Docker。
            if let Err(error) = sync_port_mapping_store_from_containers().await {
                tracing::warn!("sync sandbox port mappings failed: {}", error);
            }
            loop {
                // 后续按固定周期扫描并回收已过期容器。
                if let Err(error) = cleanup_expired_sandboxes().await {
                    tracing::warn!("cleanup expired sandboxes failed: {}", error);
                }
                sleep(Duration::from_secs(30)).await;
            }
        });
    });
}

pub(crate) fn track_sandbox_expiration(container_id: &str, expires_at: &str) {
    // 续期接口会写入最新过期时间，保证新 TTL 在内存态立即可见。
    let store = EXPIRATION_STORE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(mut guard) = store.lock() {
        guard.insert(container_id.to_string(), expires_at.to_string());
    }
}

pub(crate) fn untrack_sandbox_expiration(container_id: &str) {
    // 容器被删除后及时移除缓存，避免后续扫描仍命中旧数据。
    if let Some(store) = EXPIRATION_STORE.get()
        && let Ok(mut guard) = store.lock()
    {
        guard.remove(container_id);
    }
}

pub(crate) fn track_sandbox_port_mappings(
    container_id: &str,
    port_mappings: HashMap<u16, u16>,
) {
    // 直接覆盖同容器旧值：端口重新发布时，新的映射应立刻生效。
    let store = PORT_MAPPING_STORE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(mut guard) = store.lock() {
        guard.insert(container_id.to_string(), port_mappings);
    }
}

pub(crate) fn untrack_sandbox_port_mappings(container_id: &str) {
    // 容器删除/不存在时移除缓存，避免查询命中过期端口映射。
    if let Some(store) = PORT_MAPPING_STORE.get()
        && let Ok(mut guard) = store.lock()
    {
        guard.remove(container_id);
    }
}

fn replace_tracked_port_mappings(entries: HashMap<String, HashMap<u16, u16>>) {
    // 启动同步场景使用“全量替换”，确保缓存内容严格对齐当前 Docker 实际容器集合。
    let store = PORT_MAPPING_STORE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(mut guard) = store.lock() {
        *guard = entries;
    }
}

pub(crate) fn resolve_sandbox_port_mapping(container_id: &str, port: u16) -> Option<u16> {
    // 查询路径只做内存读取，不访问 Docker；若 miss 由调用方决定是否回源并回填。
    PORT_MAPPING_STORE
        .get()
        .and_then(|store| store.lock().ok())
        .and_then(|guard| {
            guard
                .get(container_id)
                .and_then(|mappings| mappings.get(&port).cloned())
        })
}

pub(crate) fn sync_sandbox_port_mappings_from_detail(
    container_id: &str,
    detail: &bollard::models::ContainerInspectResponse,
) {
    // 单容器增量同步：常用于“刚创建后预热”与“查询回源后回填”。
    let mappings = extract_port_mappings_from_detail(detail);
    // 没有任何可用映射时删除该容器缓存，防止保留历史端口数据。
    if mappings.is_empty() {
        untrack_sandbox_port_mappings(container_id);
    } else {
        track_sandbox_port_mappings(container_id, mappings);
    }
}

fn replace_tracked_expirations(entries: HashMap<String, String>) {
    // 启动同步使用整体替换，确保缓存与当前容器集合保持一致。
    let store = EXPIRATION_STORE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(mut guard) = store.lock() {
        *guard = entries;
    }
}

pub(crate) fn resolve_expiration(
    container_id: &str,
    labels: Option<&HashMap<String, String>>,
) -> Option<DateTime<Utc>> {
    // 内存态代表续期后的最新值，label 代表容器创建时值；两者取更晚时间。
    let tracked = EXPIRATION_STORE
        .get()
        .and_then(|store| store.lock().ok())
        .and_then(|guard| guard.get(container_id).cloned())
        .and_then(|value| parse_rfc3339_utc(&value));
    let labeled = labels
        .and_then(|values| values.get(SANDBOX_EXPIRES_AT_LABEL))
        .and_then(|value| parse_rfc3339_utc(value));
    tracked.into_iter().chain(labeled).max()
}

pub(crate) fn parse_rfc3339_utc(value: &str) -> Option<DateTime<Utc>> {
    // 输入允许任意时区偏移，统一转换到 UTC 便于比较。
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

async fn sync_expiration_store_from_containers() -> Result<(), DockerError> {
    // 仅同步被系统识别为 sandbox 的容器，避免误纳入业务外容器。
    let docker = Docker::connect_with_local_defaults()?;
    let mut filters = HashMap::new();
    filters.insert("label".to_string(), vec![SANDBOX_ID_LABEL.to_string()]);
    let containers = docker
        .list_containers(Some(ListContainersOptions {
            all: true,
            filters: Some(filters),
            ..Default::default()
        }))
        .await?;

    let mut entries = HashMap::new();
    for container in containers {
        let Some(id) = container.id else {
            continue;
        };
        let Some(labels) = container.labels else {
            continue;
        };
        let Some(expires_at) = labels.get(SANDBOX_EXPIRES_AT_LABEL) else {
            continue;
        };
        // 跳过非法时间，避免污染内存缓存并影响清理逻辑。
        if parse_rfc3339_utc(expires_at).is_none() {
            continue;
        }
        entries.insert(id, expires_at.to_string());
    }

    // 用当前扫描结果覆盖旧缓存，处理重启后容器变更与残留记录。
    replace_tracked_expirations(entries);
    Ok(())
}

async fn sync_port_mapping_store_from_containers() -> Result<(), DockerError> {
    // 与过期时间同步一致，仅处理标记为 sandbox 的容器，避免误缓存业务外容器。
    let docker = Docker::connect_with_local_defaults()?;
    let mut filters = HashMap::new();
    filters.insert("label".to_string(), vec![SANDBOX_ID_LABEL.to_string()]);
    let containers = docker
        .list_containers(Some(ListContainersOptions {
            all: true,
            filters: Some(filters),
            ..Default::default()
        }))
        .await?;

    let mut entries = HashMap::new();
    for container in containers {
        let Some(id) = container.id else {
            continue;
        };
        // 端口映射位于 inspect 详情中，list 结果不足以构建完整映射信息。
        let detail = match docker.inspect_container(&id, None).await {
            Ok(detail) => detail,
            Err(DockerError::DockerResponseServerError {
                status_code: 404, ..
            }) => continue,
            Err(error) => return Err(error),
        };
        let mappings = extract_port_mappings_from_detail(&detail);
        // 仅缓存存在映射的容器，减少无效条目和锁竞争。
        if mappings.is_empty() {
            continue;
        }
        entries.insert(id, mappings);
    }

    replace_tracked_port_mappings(entries);
    Ok(())
}

fn extract_port_mappings_from_detail(
    detail: &bollard::models::ContainerInspectResponse,
) -> HashMap<u16, u16> {
    // 目标产物：
    // - key: 容器内部端口（u16）
    // - value: 映射后的宿主机端口
    let mut mappings = HashMap::new();
    let Some(network_settings) = &detail.network_settings else {
        return mappings;
    };
    let Some(ports) = &network_settings.ports else {
        return mappings;
    };
    for (port_key, bindings) in ports {
        // Docker key 形如 "8080/tcp"，这里只抽取端口号并忽略协议片段。
        let Some((private_port_text, _)) = port_key.split_once('/') else {
            continue;
        };
        let Ok(private_port) = private_port_text.parse::<u16>() else {
            continue;
        };
        let Some(bindings) = bindings else {
            continue;
        };
        let Some(binding) = bindings.first() else {
            continue;
        };
        let Some(host_port) = &binding.host_port else {
            continue;
        };
        let Ok(host_port) = host_port.parse::<u16>() else {
            continue;
        };
        mappings.insert(private_port, host_port);
    }
    mappings
}

async fn cleanup_expired_sandboxes() -> Result<(), DockerError> {
    // 清理阶段同样按 sandbox 标签过滤，范围与同步逻辑保持一致。
    let docker = Docker::connect_with_local_defaults()?;
    let mut filters = HashMap::new();
    filters.insert("label".to_string(), vec![SANDBOX_ID_LABEL.to_string()]);
    let containers = docker
        .list_containers(Some(ListContainersOptions {
            all: true,
            filters: Some(filters),
            ..Default::default()
        }))
        .await?;
    let now = Utc::now();

    for container in containers {
        let Some(id) = container.id else {
            continue;
        };
        let labels = container.labels.unwrap_or_default();
        // Docker status 是展示字符串，这里按文本特征识别运行态与暂停态。
        let status = container
            .status
            .as_deref()
            .unwrap_or_default()
            .to_ascii_lowercase();
        let is_paused = status.contains("paused");
        let is_running = status.starts_with("up");
        let Some(expires_at) = resolve_expiration(&id, Some(&labels)) else {
            continue;
        };
        if expires_at > now {
            continue;
        }
        // 暂停容器需先恢复再停止，避免 stop 在 paused 状态下失败。
        if is_paused {
            match docker.unpause_container(&id).await {
                Ok(_) => {}
                Err(DockerError::DockerResponseServerError {
                    status_code: 304, ..
                }) => {}
                Err(DockerError::DockerResponseServerError {
                    status_code: 404, ..
                }) => {}
                Err(error) => tracing::warn!("unpause expired sandbox {} failed: {}", id, error),
            }
        }
        if is_running || is_paused {
            match docker.stop_container(&id, None).await {
                Ok(_) => {}
                Err(DockerError::DockerResponseServerError {
                    status_code: 304, ..
                }) => {}
                Err(DockerError::DockerResponseServerError {
                    status_code: 404, ..
                }) => {}
                Err(error) => tracing::warn!("stop expired sandbox {} failed: {}", id, error),
            }
        }
        match docker
            .remove_container(
                &id,
                Some(RemoveContainerOptions {
                    force: true,
                    ..Default::default()
                }),
            )
            .await
        {
            Ok(_) => {
                untrack_sandbox_expiration(&id);
                // 清理成功后同时删除端口缓存，避免端口查询命中幽灵容器。
                untrack_sandbox_port_mappings(&id);
            }
            // 并发删除场景下，容器可能已不存在，按幂等成功处理。
            Err(DockerError::DockerResponseServerError {
                status_code: 404, ..
            }) => {
                untrack_sandbox_expiration(&id);
                // 404 视为幂等成功，同样需要清理可能存在的历史缓存。
                untrack_sandbox_port_mappings(&id);
            }
            Err(error) => tracing::warn!("remove expired sandbox {} failed: {}", id, error),
        }
    }

    Ok(())
}

pub(crate) async fn resolve_container_id(
    docker: &Docker,
    sandbox_id: &str,
    map_error: fn(DockerError) -> WebError,
) -> Result<String, WebError> {
    // 优先把入参当作容器 ID 直接 inspect，命中则无需再 list
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

    // 否则按业务 sandbox_id 标签查询容器
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
        .map_err(map_error)?;

    if let Some(id) = containers.into_iter().find_map(|item| item.id) {
        return Ok(id);
    }

    Err(WebError::Biz(Code::SandboxNotFound.into()))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use chrono::{Duration, Utc};

    use super::{
        SANDBOX_EXPIRES_AT_LABEL, replace_tracked_expirations, replace_tracked_port_mappings,
        resolve_expiration, resolve_sandbox_id,
        resolve_sandbox_port_mapping, track_sandbox_expiration, track_sandbox_port_mappings,
        untrack_sandbox_expiration, untrack_sandbox_port_mappings,
    };

    #[test]
    fn test_resolve_sandbox_id() {
        let mut labels = HashMap::new();
        labels.insert("upod.io/sandbox-id".to_string(), "sandboxabc".to_string());
        let sandbox_id = resolve_sandbox_id(Some(&labels), "fallback");
        assert_eq!(sandbox_id, "sandboxabc");
    }

    #[test]
    fn test_resolve_sandbox_id_with_fallback() {
        let labels = HashMap::new();
        let sandbox_id = resolve_sandbox_id(Some(&labels), "fallback");
        assert_eq!(sandbox_id, "fallback");
    }

    #[test]
    fn test_resolve_expiration_uses_later_of_store_and_label() {
        let container_id = "container-1";
        let label_time = (Utc::now() + Duration::seconds(30)).to_rfc3339();
        let tracked_time = (Utc::now() + Duration::seconds(60)).to_rfc3339();
        let mut labels = HashMap::new();
        labels.insert(SANDBOX_EXPIRES_AT_LABEL.to_string(), label_time.clone());
        track_sandbox_expiration(container_id, &tracked_time);

        let resolved = resolve_expiration(container_id, Some(&labels)).unwrap();
        assert_eq!(resolved.to_rfc3339(), tracked_time);

        untrack_sandbox_expiration(container_id);
    }

    #[test]
    fn test_resolve_expiration_fallback_to_label() {
        let container_id = "container-2";
        let label_time = (Utc::now() + Duration::seconds(15)).to_rfc3339();
        let mut labels = HashMap::new();
        labels.insert(SANDBOX_EXPIRES_AT_LABEL.to_string(), label_time.clone());

        let resolved = resolve_expiration(container_id, Some(&labels)).unwrap();
        assert_eq!(resolved.to_rfc3339(), label_time);
    }

    #[test]
    fn test_replace_tracked_expirations_overwrites_old_entries() {
        let old_container = "container-old";
        let old_time = (Utc::now() + Duration::seconds(10)).to_rfc3339();
        track_sandbox_expiration(old_container, &old_time);

        let new_container = "container-new";
        let new_time = (Utc::now() + Duration::seconds(20)).to_rfc3339();
        let mut entries = HashMap::new();
        entries.insert(new_container.to_string(), new_time.clone());
        replace_tracked_expirations(entries);

        assert!(resolve_expiration(old_container, None).is_none());
        let resolved = resolve_expiration(new_container, None).unwrap();
        assert_eq!(resolved.to_rfc3339(), new_time);

        untrack_sandbox_expiration(new_container);
    }

    #[test]
    fn test_resolve_sandbox_port_mapping_from_memory_store() {
        let container_id = "container-port-1";
        let mut mappings = HashMap::new();
        mappings.insert(8080, 32768);
        track_sandbox_port_mappings(container_id, mappings);

        let endpoint_port = resolve_sandbox_port_mapping(container_id, 8080);
        assert_eq!(endpoint_port, Some(32768));

        untrack_sandbox_port_mappings(container_id);
    }

    #[test]
    fn test_replace_tracked_port_mappings_overwrites_old_entries() {
        let old_container = "container-port-old";
        let mut old_mapping = HashMap::new();
        old_mapping.insert(8080, 30001);
        track_sandbox_port_mappings(old_container, old_mapping);

        let new_container = "container-port-new";
        let mut new_mapping = HashMap::new();
        new_mapping.insert(8080, 30002);
        let mut entries = HashMap::new();
        entries.insert(new_container.to_string(), new_mapping);
        replace_tracked_port_mappings(entries);

        assert!(resolve_sandbox_port_mapping(old_container, 8080).is_none());
        assert_eq!(resolve_sandbox_port_mapping(new_container, 8080), Some(30002));

        untrack_sandbox_port_mappings(new_container);
    }
}
