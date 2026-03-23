use reqwest::{Client as ReqwestClient, Response};
use serde::Deserialize;
use std::sync::Arc;

use crate::{
    error::{Result, UpodError},
    models::{ApiResponse, CreateSandboxReq, CreateSandboxResp, ListSandboxResp, SandboxInfo},
    sandbox::SandboxHandle,
};

/// Upod 服务的主客户端。
///
/// 封装了全局级的 `reqwest` Client，内部基于 Arc，具有内部可变性和并发安全的网络连接池能力。
#[derive(Clone, Debug)]
pub struct UpodClient {
    /// Reqwest 内部已经是基于 Arc 的，此处 Arc 主要为了使得 SandboxHandle 也能方便地共享整个 UpodClient（含 base_url 配置）
    pub(crate) inner: ReqwestClient,
    /// 绑定的后端服务的 Base URL
    pub(crate) base_url: String,
}

impl UpodClient {
    /// 初始化新的客户端实例。
    ///
    /// 建议传入带协议头的完整 base_url（例如: http://localhost:8080）
    pub fn new(base_url: impl Into<String>) -> Result<Arc<Self>> {
        let base_url = base_url.into();
        let inner = ReqwestClient::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()?;

        Ok(Arc::new(Self { inner, base_url }))
    }

    /// 获取绑定的基础 URL
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// 创建一个新沙箱，并返回对该沙箱的操作句柄
    ///
    /// POST /v1/sandboxes
    pub async fn create_sandbox(self: &Arc<Self>, req: CreateSandboxReq) -> Result<SandboxHandle> {
        let url = format!("{}/v1/sandboxes", self.base_url);

        let response = self.inner.post(&url).json(&req).send().await?;

        let sandbox_resp = Self::handle_response::<CreateSandboxResp>(response).await?;

        Ok(SandboxHandle::new(Arc::clone(self), sandbox_resp.id))
    }

    /// 获取特定沙箱句柄
    ///
    /// 此处并没有直接发出请求（除非想获取完整详细状态）。
    /// 按照面向对象设计，通常先拿到句柄，在句柄上直接操作即可，降低重复传参和不必要的 API 校验开销。
    ///
    /// GET /v1/sandboxes/{sandbox_id} -> 在实际业务中可能用于校验 id 是否存在
    pub async fn get_sandbox(self: &Arc<Self>, id: &str) -> Result<SandboxHandle> {
        let url = format!("{}/v1/sandboxes/{}", self.base_url, id);
        // 如果想严格检查沙箱存活，可以在此处发起网络请求，若成功则返回句柄。
        let response = self.inner.get(&url).send().await?;
        Self::handle_response::<SandboxInfo>(response).await?;

        Ok(SandboxHandle::new(Arc::clone(self), id.to_string()))
    }

    /// 获取当前可用的所有沙箱列表
    ///
    /// GET /v1/sandboxes
    pub async fn list_sandboxes(self: &Arc<Self>) -> Result<Vec<SandboxInfo>> {
        let url = format!("{}/v1/sandboxes", self.base_url);
        let response = self.inner.get(&url).send().await?;
        let resp_data = Self::handle_response::<ListSandboxResp>(response).await?;
        Ok(resp_data.items)
    }

    // --- 内部公共网络请求帮助方法 ---

    /// 帮助方法：处理 HTTP 响应，并反序列化为给定的 JSON 模型 `T`
    pub(crate) async fn handle_response<T: for<'de> Deserialize<'de>>(resp: Response) -> Result<T> {
        let status = resp.status();
        if !status.is_success() {
            let message = resp.text().await.unwrap_or_else(|_| "Unknown Error".into());
            return Err(UpodError::Api { status, message });
        }

        let api_resp = resp.json::<ApiResponse<T>>().await?;
        if api_resp.code != 0 && api_resp.code != 200 {
            return Err(UpodError::Api {
                status: reqwest::StatusCode::BAD_REQUEST, // Or map code properly
                message: api_resp.message,
            });
        }

        api_resp
            .data
            .ok_or_else(|| UpodError::Client("Response data is missing".into()))
    }

    /// 内部帮助方法：发起无参 POST 请求（如 pause/resume 等动作）
    pub(crate) async fn post_empty(&self, url: &str) -> Result<()> {
        let response = self.inner.post(url).send().await?;
        Self::handle_empty_response(response).await
    }

    /// 内部帮助方法：发起 DELETE 请求
    pub(crate) async fn delete(&self, url: &str) -> Result<()> {
        let response = self.inner.delete(url).send().await?;
        Self::handle_empty_response(response).await
    }

    /// 帮助方法：处理 HTTP 响应，但不要求反序列化具体数据
    pub(crate) async fn handle_empty_response(resp: Response) -> Result<()> {
        let status = resp.status();
        if !status.is_success() {
            let message = resp.text().await.unwrap_or_else(|_| "Unknown Error".into());
            return Err(UpodError::Api { status, message });
        }

        let api_resp = resp.json::<ApiResponse<serde_json::Value>>().await?;
        if api_resp.code != 0 && api_resp.code != 200 {
            return Err(UpodError::Api {
                status: reqwest::StatusCode::BAD_REQUEST,
                message: api_resp.message,
            });
        }

        Ok(())
    }
}
