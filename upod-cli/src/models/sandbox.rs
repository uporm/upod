use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 创建沙箱的请求体
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSandboxReq {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sandbox_id: Option<String>,
    /// 镜像信息
    pub image: Image,
    /// 入口点命令
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entrypoint: Option<Vec<String>>,
    /// 超时时间（秒）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u64>,
    /// 资源限制配置
    #[serde(rename = "resourceLimits", skip_serializing_if = "Option::is_none")]
    pub resource_limits: Option<ResourceLimits>,
    /// 环境变量
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env: Option<HashMap<String, String>>,
    /// 元数据标签
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<HashMap<String, String>>,
}

/// 镜像定义
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Image {
    /// 镜像 URI
    pub uri: String,
}

/// 资源限制定义
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceLimits {
    /// CPU 限制（例如 "500m" 或 "0.5"）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu: Option<String>,
    /// 内存限制（例如 "512Mi", "1Gi"）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory: Option<String>,
}

/// 创建沙箱的响应体
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSandboxResp {
    /// 创建的容器 ID
    pub id: String,
    /// 创建过程中的警告信息
    pub warnings: Vec<String>,
}

/// 沙箱简要信息，用于列表展示
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SandboxInfo {
    /// 沙箱唯一标识符
    pub id: String,
    /// 名称
    pub name: String,
    /// 状态（如：running, paused, stopped）
    pub status: String,
    /// 镜像名
    pub image: String,
    /// 创建时间 (UNIX timestamp 或者 ISO string)
    pub created_at: String,
}

/// 列表查询的响应数据结构
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListSandboxResp {
    pub items: Vec<SandboxInfo>,
}
