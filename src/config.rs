use crate::core::constants::{
    STORAGE_CONTAINERS_DIR, STORAGE_IMAGES_DIR, STORAGE_LINUX_DIR, STORAGE_MACOS_DIR,
    STORAGE_STATE_DIR,
};
use crate::utils::system_env::SystemOS;
use rivus_core::include_yaml;
use serde::Deserialize;
use std::sync::OnceLock;

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
    pub storage_dir: String,
    pub logger: LoggerConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        let storage_dir = match SystemOS::current() {
            SystemOS::MacOS => STORAGE_MACOS_DIR,
            SystemOS::Linux => STORAGE_LINUX_DIR,
            _ => panic!("unsupported operating system"),
        };

        Self {
            server: "0.0.0.0:8080".to_string(),
            storage_dir: storage_dir.to_string(),
            logger: LoggerConfig {
                level: "debug".to_string(),
                console: true,
            },
        }
    }
}

/// 全局配置单例
static CONFIG: OnceLock<AppConfig> = OnceLock::new();

impl AppConfig {
    pub fn get() -> &'static AppConfig {
        CONFIG.get_or_init(|| Self::load())
    }
    pub fn load() -> Self {
        let config = include_yaml!("../resources/application.yaml", AppConfig).unwrap();
        config
    }

    pub fn images_dir(&self) -> String {
        format!("{}/{}", self.storage_dir, STORAGE_IMAGES_DIR)
    }

    pub fn container_dir(&self) -> String {
        format!("{}/{}", self.storage_dir, STORAGE_CONTAINERS_DIR)
    }

    pub fn state_dir(&self) -> String {
        format!("{}/{}", self.storage_dir, STORAGE_STATE_DIR)
    }
}
