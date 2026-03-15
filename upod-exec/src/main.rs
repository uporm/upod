mod api;
mod core;
mod models;
mod routes;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    api::command::command_session::init_command_output_dir()?;
    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080").await?;
    axum::serve(listener, routes::router()).await?;
    Ok(())
}
