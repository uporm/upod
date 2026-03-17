use rivus_logger::LoggerConfig;
use tracing::info;
use crate::config::AppConfig;
use crate::handler::create_sandbox::ensure_bridge_binary_ready;
use crate::proxy::run_proxy_server;
use crate::web::server::WebServer;

mod config;
mod core;
mod handler;
mod models;
mod routes;
mod web;
mod proxy;
mod utils;

// 初始化翻译文件
rust_i18n::i18n!("resources/locales", fallback = "zh");

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let conf = AppConfig::global();
    let gateway_addr = conf.gateway.addr.clone();

    // 1. 初始化日志
    let _guard = LoggerConfig::new()
        .enable_console(conf.logger.console)
        .level(conf.logger.level.clone())
        .filter_directives(conf.logger.filter_directives.clone())
        .init();

    ensure_bridge_binary_ready()?;

    std::thread::spawn(move || {
        info!("🚀 Starting proxy server at {}", &gateway_addr);
        if let Err(error) = run_proxy_server(&gateway_addr) {
            eprintln!("proxy server stopped with error: {}", error);
        }
    });

    // 3. 启动服务
    WebServer::new(conf.server.addr.clone())
        .mount(routes::router())
        .layer_i18n()
        .start()
        .await?;

    Ok(())
}
