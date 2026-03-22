use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Clone, Default)]
pub struct RunCommandReq {
    pub command: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u64>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub enum StreamEventType {
    #[serde(rename = "init")]
    SessionInit,
    #[serde(rename = "accepted")]
    ExecutionAccepted,
    #[serde(rename = "stdout")]
    StdoutLine,
    #[serde(rename = "stderr")]
    StderrLine,
    #[serde(rename = "error")]
    ExecutionError,
    #[serde(rename = "complete")]
    ExecutionComplete,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct CommandError {
    pub ename: String,
    pub evalue: String,
    pub traceback: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct StreamEvent {
    #[serde(rename = "type")]
    pub event_type: StreamEventType,
    pub text: Option<String>,
    pub error: Option<CommandError>,
    pub execution_time: Option<u64>,
}

/// 命令执行状态
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandStatus {
    /// 命令 ID
    pub id: String,
    /// 命令内容
    pub content: String,
    /// 是否正在运行
    pub running: bool,
    /// 退出码（执行完毕后才会有）
    pub exit_code: Option<i32>,
    /// 错误信息
    pub error: Option<String>,
    /// 启动时间（Unix 毫秒时间戳）
    #[serde(alias = "start_time")]
    pub started_at: Option<u64>,
    /// 结束时间（Unix 毫秒时间戳）
    #[serde(alias = "end_time")]
    pub finished_at: Option<u64>,
}
