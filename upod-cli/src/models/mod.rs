use serde::{Deserialize, Serialize};

pub mod sandbox;
pub mod command;
pub mod filesystem;

pub use sandbox::*;
pub use command::*;
pub use filesystem::*;

/// 响应体通用结构，适配 upod-base::web::r::R
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiResponse<T> {
    pub code: u32,
    pub message: String,
    pub data: Option<T>,
}
