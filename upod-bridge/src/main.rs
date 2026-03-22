mod api;
mod core;
mod models;
mod routes;
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let listen_addr = "0.0.0.0:44321";
    println!("[upod-bridge] 正在初始化命令输出目录...");
    api::command::command_session::init_command_output_dir()?;
    println!("[upod-bridge] 命令输出目录初始化完成");
    println!("[upod-bridge] 正在绑定监听地址: {listen_addr}");
    let listener = tokio::net::TcpListener::bind(listen_addr).await?;
    println!("[upod-bridge] 服务启动成功，开始监听: {listen_addr}");
    axum::serve(listener, routes::router()).await?;
    Ok(())
}
