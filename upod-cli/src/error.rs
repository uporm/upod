/// upod 客户端统一错误类型
#[derive(thiserror::Error, Debug)]
pub enum UpodError {
    /// 包装 reqwest 抛出的底层网络/HTTP 错误
    #[error("Network or HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    /// 包装 serde 序列化/反序列化错误
    #[error("Serialization/Deserialization error: {0}")]
    Serde(#[from] serde_json::Error),

    /// 包装 url 解析错误
    #[error("URL parsing error: {0}")]
    Url(#[from] url::ParseError),

    /// 后端 API 明确返回的业务错误（如 400, 404）
    #[error("API error (status: {status}): {message}")]
    Api {
        status: reqwest::StatusCode,
        message: String,
    },

    /// 客户端内部或参数错误
    #[error("Client error: {0}")]
    Client(String),
}

/// 客户端全局别名
pub type Result<T> = std::result::Result<T, UpodError>;
