//! 沙箱容器存储模块
//!
//! 本模块负责管理沙箱容器信息在内存中的存储和查询。

use std::collections::HashMap;
use std::sync::OnceLock;

use chrono::{DateTime, Utc};
use dashmap::DashMap;

/// 沙箱容器信息结构体
///
/// 存储单个沙箱容器的关键信息，包括容器 ID、端口映射和过期时间。
#[derive(Clone)]
pub struct SandboxContainer {
    /// Docker 容器 ID
    pub container_id: String,
    /// 端口映射表：键为容器内部端口，值为宿主机端口
    pub ports: HashMap<u16, u16>,
    /// 沙箱过期时间（UTC 时间）
    pub expires_at: DateTime<Utc>,
}

/// 全局沙箱容器存储
///
/// 使用 DashMap 实现并发安全的键值存储，键为沙箱 ID，值为容器信息。
/// OnceLock 确保存储只被初始化一次。
static SANDBOX_CONTAINER_STORE: OnceLock<DashMap<String, SandboxContainer>> = OnceLock::new();

/// 注册沙箱容器到存储
///
/// 将沙箱容器信息添加到全局存储中，用于后续查询和管理。
///
/// # 参数
/// - `sandbox_id`: 沙箱唯一标识符
/// - `container`: 容器信息
pub fn register_container(sandbox_id: &str, container: SandboxContainer) {
    let store = SANDBOX_CONTAINER_STORE.get_or_init(DashMap::new);
    store.insert(sandbox_id.to_string(), container);
}

/// 获取指定沙箱的容器信息
///
/// 从全局存储中查询沙箱容器信息，返回克隆后的容器数据。
///
/// # 参数
/// - `sandbox_id`: 沙箱唯一标识符
///
/// # 返回
/// 如果沙箱存在，返回 `Some(SandboxContainer)`，否则返回 `None`
pub fn get_container(sandbox_id: &str) -> Option<SandboxContainer> {
    SANDBOX_CONTAINER_STORE
        .get()
        .and_then(|store| store.get(sandbox_id).map(|c| c.clone()))
}

/// 获取所有沙箱容器信息
///
/// 返回全局存储中所有沙箱容器的克隆副本。
/// 如果存储未初始化，返回空的 DashMap。
pub fn get_containers() -> DashMap<String, SandboxContainer> {
    SANDBOX_CONTAINER_STORE
        .get()
        .cloned()
        .unwrap_or_else(DashMap::new)
}

/// 从存储中移除沙箱容器
///
/// 删除指定沙箱 ID 的容器信息记录。
///
/// # 参数
/// - `sandbox_id`: 沙箱唯一标识符
pub fn clear_container(sandbox_id: &str) {
    if let Some(store) = SANDBOX_CONTAINER_STORE.get() {
        store.remove(sandbox_id);
    }
}

/// 替换所有沙箱容器记录
///
/// 清空当前存储并插入新的容器记录，用于全量同步状态。
///
/// # 参数
/// - `containers`: 新的沙箱容器记录迭代器
pub fn replace_all_containers(containers: impl IntoIterator<Item = (String, SandboxContainer)>) {
    let store = SANDBOX_CONTAINER_STORE.get_or_init(DashMap::new);
    store.clear();
    for (sandbox_id, container) in containers {
        store.insert(sandbox_id, container);
    }
}
