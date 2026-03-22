use std::sync::Arc;

use crate::{
    client::UpodClient,
    error::Result,
    models::EndpointInfo,
};

/// 针对单个沙箱的操作句柄。
///
/// 封装了沙箱 ID 和指向底层 `UpodClient` 的共享引用，
/// 使得用户可以直接在此对象上执行沙箱级操作。
#[derive(Clone, Debug)]
pub struct SandboxHandle {
    /// 指向底层 UpodClient 的引用，用于复用网络连接与配置
    pub(crate) client: Arc<UpodClient>,
    /// 当前沙箱的唯一 ID
    pub(crate) id: String,
}

impl SandboxHandle {
    /// 构造方法：仅限包内使用，通过 `UpodClient` 生成句柄
    pub(crate) fn new(client: Arc<UpodClient>, id: String) -> Self {
        Self { client, id }
    }

    /// 获取当前沙箱的 ID
    pub fn id(&self) -> &str {
        &self.id
    }

    /// 暂停沙箱运行
    ///
    /// POST /v1/sandboxes/{sandbox_id}/pause
    pub async fn pause(&self) -> Result<()> {
        let url = format!("{}/v1/sandboxes/{}/pause", self.client.base_url, self.id);
        self.client.post_empty(&url).await?;
        Ok(())
    }

    /// 恢复沙箱运行
    ///
    /// POST /v1/sandboxes/{sandbox_id}/resume
    pub async fn resume(&self) -> Result<()> {
        let url = format!("{}/v1/sandboxes/{}/resume", self.client.base_url, self.id);
        self.client.post_empty(&url).await?;
        Ok(())
    }

    /// 彻底删除该沙箱
    ///
    /// DELETE /v1/sandboxes/{sandbox_id}
    pub async fn delete(&self) -> Result<()> {
        let url = format!("{}/v1/sandboxes/{}", self.client.base_url, self.id);
        self.client.delete(&url).await?;
        Ok(())
    }

    /// 获取指定端口的访问入口（Endpoint）
    ///
    /// GET /v1/sandboxes/{sandbox_id}/endpoints/{port}
    pub async fn get_endpoint(&self, port: u16) -> Result<EndpointInfo> {
        let url = format!("{}/v1/sandboxes/{}/endpoints/{}", self.client.base_url, self.id, port);
        self.client.get_json::<EndpointInfo>(&url).await
    }
}
