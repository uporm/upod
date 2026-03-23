//! `upod-cli` 提供了用于和 upod 后端服务交互的 Rust 客户端库。
//!
//! # 核心架构
//!
//! - [`UpodClient`]: 客户端入口，负责管理 HTTP 连接和全局级操作。
//! - [`SandboxHandle`]: 沙箱操作句柄，封装了沙箱 ID 并复用 `UpodClient`，
//!   实现面向对象风格的调用（直接对该沙箱实例进行 `pause`、`resume` 等操作）。
//!
//! # 示例
//!
//! ```no_run
//! use upod_cli::{UpodClient, models::CreateSandboxReq};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // 1. 初始化客户端
//!     let client = UpodClient::new("http://localhost:8080")?;
//!
//!     // 2. 创建一个沙箱，获取 SandboxHandle
//!     let req = CreateSandboxReq {
//!         sandbox_id: None,
//!         image: upod_cli::models::Image { uri: "python:3.11-slim".to_string() },
//!         entrypoint: None,
//!         timeout: None,
//!         resource_limits: Some(upod_cli::models::ResourceLimits {
//!             cpu: None,
//!             memory: Some("512Mi".to_string()),
//!         }),
//!         env: None,
//!         metadata: None,
//!     };
//!     let sandbox = client.create_sandbox(req).await?;
//!     println!("Created sandbox: {}", sandbox.id());
//!
//!     // 3. 对沙箱直接进行操作
//!     sandbox.pause().await?;
//!     println!("Sandbox paused.");
//!
//!     sandbox.resume().await?;
//!     println!("Sandbox resumed.");
//!
//!     // 4. 清理沙箱
//!     sandbox.delete().await?;
//!     println!("Sandbox deleted.");
//!
//!     Ok(())
//! }
//! ```

pub mod client;
pub mod command;
pub mod error;
mod filesystem;
pub mod models;
pub mod sandbox;

pub use client::UpodClient;
pub use error::{Result, UpodError};
pub use sandbox::SandboxHandle;

// Re-export reqwest_eventsource for convenience
pub use reqwest_eventsource;
