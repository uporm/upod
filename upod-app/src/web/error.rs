use serde::Serialize;
use thiserror::Error;
use validator::ValidationErrors;

#[derive(Debug, Serialize)]
pub struct BizError {
    pub code: i32,
    pub args: Vec<(String, String)>,
}

#[derive(Error, Debug)]
pub enum WebError {
    #[error("{0}")]
    Val(#[from] ValidationErrors),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("System error: {0}")]
    Sys(String),
    #[error("code={0}, args={1:?}")]
    BizWithArgs(i32, Vec<(String, String)>),
    #[error("{0}")]
    Biz(i32),
    #[error("{0}")]
    Anyhow(#[from] anyhow::Error),
}

pub type Result<T> = std::result::Result<T, WebError>;
