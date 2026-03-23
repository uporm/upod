use std::sync::Arc;

use crate::{client::UpodClient, error::Result};

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

    /// 内部获取桥接服务的代理地址
    pub(crate) fn get_bridge_url(&self, port: u16) -> String {
        let base = &self.client.base_url;
        let gateway_url = if base.contains(":") {
            let parts: Vec<&str> = base.rsplitn(2, ':').collect();
            if parts.len() == 2 && parts[0].parse::<u16>().is_ok() {
                format!("{}:9000", parts[1])
            } else {
                format!("{}:9000", base)
            }
        } else {
            format!("{}:9000", base)
        };
        format!("{}/sandboxes/{}/port/{}", gateway_url, self.id, port)
    }
}
