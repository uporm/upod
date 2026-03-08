use rivus_core::include_yaml;
use serde::Deserialize;

#[derive(Deserialize)]
pub struct PoolOptions {
    pub max_open_conns: u64, // Set the maximum number of connections in the pool
    pub max_idle_conns: u64, // Set the maximum number of idle connections in the pool
    pub max_lifetime: u64,   // Set the maximum lifetime of a connection
    pub timeout: u64,        // Set the timeout for getting a connection from the pool
}

#[derive(Deserialize)]
pub struct DatabaseConfig {
    pub url: String,
    pub option: Option<PoolOptions>,
}

#[derive(Deserialize)]
pub struct LoggerConfig {
    /// 日志级别
    pub level: String,
    /// 是否启用控制台输出
    #[serde(rename = "console_enabled")]
    pub console: bool,
}

#[derive(Deserialize)]
pub struct AppConfig {
    pub server: String,
    pub logger: LoggerConfig,
    pub database: DatabaseConfig,
}

impl AppConfig {
    pub fn load() -> Self {
        let config = include_yaml!("../resources/application.yaml", AppConfig).unwrap();
        config
    }
}
