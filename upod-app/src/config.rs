use config::{Config, File};
use dotenvy::dotenv;
use serde::Deserialize;
use std::env;
use std::path::PathBuf;
use std::sync::OnceLock;

#[derive(Debug, Deserialize)]
pub struct AppConfig {
    pub server_port: u16,
    pub proxy: ProxyConfig,
}

#[derive(Debug, Deserialize)]
pub struct ProxyConfig {
    pub server_port: u16,
}

static CONFIG: OnceLock<AppConfig> = OnceLock::new();

fn resolve_config_path() -> PathBuf {
    let mut candidates = Vec::new();

    if let Ok(explicit_path) = env::var("UPOD_APP_CONFIG")
        && !explicit_path.trim().is_empty()
    {
        candidates.push(PathBuf::from(explicit_path));
    }

    candidates.push(PathBuf::from("./resources/application.toml"));
    candidates.push(PathBuf::from("./upod-app/resources/application.toml"));
    candidates.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/application.toml"));

    if let Ok(executable_path) = env::current_exe()
        && let Some(executable_dir) = executable_path.parent()
    {
        candidates.push(executable_dir.join("resources/application.toml"));
    }

    if let Some(path) = candidates.iter().find(|path| path.is_file()) {
        return path.clone();
    }

    let searched_paths = candidates
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join(", ");

    panic!(
        "Failed to build configuration: configuration file \"application.toml\" not found. searched: {}",
        searched_paths
    );
}

impl AppConfig {
    pub fn global() -> &'static AppConfig {
        CONFIG.get_or_init(Self::load)
    }

    fn load() -> Self {
        dotenv().ok();
        let config_path = resolve_config_path();

        let builder = Config::builder()
            .add_source(File::from(config_path))
            .add_source(config::Environment::default().separator("__"));

        match builder.build() {
            Ok(config) => match config.try_deserialize() {
                Ok(app_config) => app_config,
                Err(e) => panic!("Failed to deserialize configuration: {}", e),
            },
            Err(e) => panic!("Failed to build configuration: {}", e),
        }
    }
}
