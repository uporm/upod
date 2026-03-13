use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 创建沙箱的请求体
#[derive(Debug, Deserialize, Serialize)]
pub struct CreateSandboxReq {
    /// 镜像信息
    pub image: Image,
    /// 入口点命令
    pub entrypoint: Option<Vec<String>>,
    /// 超时时间（秒）
    pub timeout: Option<u64>,
    /// 资源限制配置
    #[serde(rename = "resourceLimits")]
    pub resource_limits: Option<ResourceLimits>,
    /// 环境变量
    pub env: Option<HashMap<String, String>>,
    /// 元数据标签
    pub metadata: Option<HashMap<String, String>>,
}

/// 镜像定义
#[derive(Debug, Deserialize, Serialize)]
pub struct Image {
    /// 镜像 URI
    pub uri: String,
}

/// 资源限制定义
#[derive(Debug, Deserialize, Serialize)]
pub struct ResourceLimits {
    /// CPU 限制（例如 "500m" 或 "0.5"）
    pub cpu: Option<String>,
    /// 内存限制（例如 "512Mi", "1Gi"）
    pub memory: Option<String>,
}

/// 创建沙箱的响应体
#[derive(Debug, Deserialize, Serialize)]
pub struct CreateSandboxResp {
    /// 创建的容器 ID
    pub id: String,
    /// 创建过程中的警告信息
    pub warnings: Vec<String>,
}

/// 沙箱列表请求参数
#[derive(Debug, Deserialize)]
pub struct ListSandboxesReq {
    /// 状态过滤，支持多个状态
    pub state: Option<Vec<String>>,
    /// 元数据过滤，格式为 metadata[key]=value
    pub metadata: Option<HashMap<String, String>>,
}

/// 沙箱信息
#[derive(Debug, Serialize)]
pub struct Sandbox {
    /// 沙箱 ID
    pub id: String,
    /// 沙箱名称
    pub name: String,
    /// 沙箱状态
    pub status: String,
    /// 镜像 URI
    pub image: String,
    /// 创建时间
    #[serde(rename = "createdAt")]
    pub created_at: String,
    /// 元数据
    pub metadata: HashMap<String, String>,
}

/// 沙箱列表响应
#[derive(Debug, Serialize)]
pub struct ListSandboxesResp {
    /// 沙箱列表
    pub items: Vec<Sandbox>,
}

/// 沙箱详情响应
#[derive(Debug, Serialize)]
pub struct GetSandboxResp {
    /// 沙箱 ID
    pub id: String,
    /// 沙箱名称
    pub name: String,
    /// 沙箱状态
    pub status: String,
    /// 镜像 URI
    pub image: String,
    /// 创建时间
    #[serde(rename = "createdAt")]
    pub created_at: String,
    /// 启动时间
    #[serde(rename = "startedAt")]
    pub started_at: Option<String>,
    /// 入口命令
    pub entrypoint: Vec<String>,
    /// 环境变量
    pub env: Vec<String>,
    /// 元数据
    pub metadata: HashMap<String, String>,
}

/// 续期请求参数
#[derive(Debug, Deserialize)]
pub struct RenewSandboxExpirationReq {
    /// 续期时长（秒）
    #[serde(rename = "ttlSeconds")]
    pub ttl_seconds: u64,
}

/// 续期响应
#[derive(Debug, Serialize)]
pub struct RenewSandboxExpirationResp {
    /// 新的过期时间
    #[serde(rename = "expiresAt")]
    pub expires_at: String,
}

/// 服务端点响应
#[derive(Debug, Serialize)]
pub struct SandboxEndpointResp {
    /// 沙箱 ID
    #[serde(rename = "sandboxId")]
    pub sandbox_id: String,
    /// 服务端口
    pub port: u16,
    /// 公开访问地址
    pub endpoint: String,
}
