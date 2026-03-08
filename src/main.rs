use crate::config::AppConfig;
use crate::web::server::WebServer;
use rivus_logger::LoggerConfig;

mod config;
mod models;
mod routes;
mod utils;
mod web;
mod business;
mod core;
mod runtime;

// 初始化翻译文件
rust_i18n::i18n!("resources/locales", fallback = "zh");

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let conf = AppConfig::load();

    // 1. 初始化日志
    let _guard = LoggerConfig::new()
        .enable_console(conf.logger.console)
        .level(conf.logger.level)
        .init();

    // 3. 启动服务
    WebServer::new(&conf.server)
        // .mount(routes::router())
        .layer_i18n()
        .start()
        .await?;

    Ok(())
}
