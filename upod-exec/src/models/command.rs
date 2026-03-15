use serde::{Deserialize, Serialize};

// 执行命令接口的请求体。
// - command: 必填，待执行的 shell 命令文本。
// - cwd: 可选，命令执行时的工作目录；为空时使用进程默认目录。
// - background: 可选，是否后台执行；为 true 时可通过输出轮询接口拉取增量日志。
// - timeout: 可选，超时时间（毫秒）；超过后服务端会主动终止子进程。
#[derive(Deserialize)]
pub(crate) struct RunCommandReq {
    // 待执行命令，例如 `echo hello` 或 `python -V`。
    pub(crate) command: String,
    // 命令执行目录，可用于切换到项目目录或临时目录。
    pub(crate) cwd: Option<String>,
    // 是否后台运行；后台模式会提前返回 completion 事件并继续执行进程。
    pub(crate) background: Option<bool>,
    // 执行超时（毫秒）。未设置时表示不启用超时控制。
    pub(crate) timeout: Option<u64>,
}

// 中断命令接口的查询参数。
// id 由执行命令接口返回的 init 事件提供，用于定位具体会话。
#[derive(Deserialize)]
pub(crate) struct InterruptReq {
    // 需要中断的命令会话 ID。
    pub(crate) id: String,
}

// 拉取后台输出接口的查询参数。
// cursor 表示已消费字节偏移，服务端将从该位置返回后续新增输出。
#[derive(Deserialize)]
pub(crate) struct OutputReq {
    // 输出读取偏移量。为空时默认从 0 开始读取。
    pub(crate) cursor: Option<usize>,
}

// SSE error 事件中的错误对象。
// 该结构用于统一描述命令执行失败、超时、中断等场景。
#[derive(Serialize)]
pub(crate) struct CommandError {
    // 错误类别名称，例如“命令执行错误”。
    pub(crate) ename: String,
    // 错误值，可能是具体错误描述或包含退出码的文本。
    pub(crate) evalue: String,
    // 错误追踪信息列表，通常包含底层错误消息。
    pub(crate) traceback: Vec<String>,
}

// 命令执行 SSE 事件模型。
// 常见 type：
// - init: 初始化事件，text 携带会话 ID。
// - stdout/stderr: 标准输出或错误输出行。
// - error: 错误事件，error 字段携带结构化错误详情。
// - complete: 执行结束事件，execution_time 为耗时。
#[derive(Serialize, Clone, Copy)]
pub(crate) enum StreamEventType {
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

#[derive(Serialize)]
pub(crate) struct StreamEvent {
    // 事件类型。序列化后字段名固定为 `type`，与前端事件协议对齐。
    #[serde(rename = "type")]
    pub(crate) event_type: StreamEventType,
    // 事件文本内容，按事件类型选择性返回。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) text: Option<String>,
    // 结构化错误信息，仅在 error 事件中返回。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) error: Option<CommandError>,
    // 执行耗时（毫秒），通常仅在 completion 事件中返回。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) execution_time: Option<u64>,
}

// 命令状态查询接口的响应模型。
// 用于表达命令是否仍在运行、是否有错误及时间轴信息。
#[derive(Serialize)]
pub(crate) struct CommandStatus {
    // 命令会话 ID。
    pub(crate) id: String,
    // 原始命令文本。
    pub(crate) content: String,
    // 当前运行状态。
    pub(crate) running: bool,
    // 退出码；运行中为 None。
    pub(crate) exit_code: Option<i32>,
    // 错误信息；成功或运行中可能为空。
    pub(crate) error: Option<String>,
    // 开始时间（Unix 毫秒时间戳）。
    pub(crate) started_at: u64,
    // 结束时间（Unix 毫秒时间戳）；运行中为 None。
    pub(crate) finished_at: Option<u64>,
}
