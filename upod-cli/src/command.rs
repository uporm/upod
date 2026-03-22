use futures_util::StreamExt;
use reqwest_eventsource::EventSource;

use crate::{
    error::{Result, UpodError},
    models::{CommandStatus, RunCommandReq, StreamEvent, StreamEventType},
    sandbox::SandboxHandle,
};

/// 命令执行的回调处理器
///
/// 允许用户在命令执行的不同生命周期（例如收到 stdout、stderr，或执行完成时）
/// 注册自定义的回调函数，以便进行日志记录或状态更新。
#[derive(Default)]
pub struct ExecutionHandlers {
    /// 当会话初始化时触发，参数为 command ID
    pub on_session_init: Option<Box<dyn FnMut(String) + Send + Sync>>,
    /// 当收到标准输出时触发的回调
    pub on_stdout: Option<Box<dyn FnMut(String) + Send + Sync>>,
    /// 当收到标准错误时触发的回调
    pub on_stderr: Option<Box<dyn FnMut(String) + Send + Sync>>,
    /// 当命令执行完成时触发的回调，参数为耗时（毫秒）
    pub on_execution_complete: Option<Box<dyn FnMut(u64) + Send + Sync>>,
}

impl SandboxHandle {
    /// 在沙箱上执行命令并通过回调处理输出
    ///
    /// POST /command
    pub async fn run_command(
        &self,
        req: RunCommandReq,
        mut handlers: ExecutionHandlers,
    ) -> Result<()> {
        let bridge_url = self.get_bridge_url(44321);
        let url = format!("{}/command", bridge_url);
        
        let req_builder = self.client.inner.post(&url).json(&req);
        let mut event_source = EventSource::new(req_builder)
            .map_err(|e| UpodError::Client(format!("Failed to create EventSource: {}", e)))?;

        // 循环监听 SSE 流事件
        while let Some(event_result) = event_source.next().await {
            match event_result {
                Ok(reqwest_eventsource::Event::Open) => {
                    // SSE 连接成功建立，无需特殊处理
                }
                Ok(reqwest_eventsource::Event::Message(message)) => {
                    // 解析流事件 JSON 数据
                    if let Ok(stream_event) = serde_json::from_str::<StreamEvent>(&message.data) {
                        match stream_event.event_type {
                            StreamEventType::SessionInit => {
                                if let Some(ref mut cb) = handlers.on_session_init {
                                    if let Some(text) = stream_event.text {
                                        cb(text);
                                    }
                                }
                            }
                            StreamEventType::StdoutLine => {
                                if let Some(ref mut cb) = handlers.on_stdout {
                                    if let Some(text) = stream_event.text {
                                        cb(text);
                                    }
                                }
                            }
                            StreamEventType::StderrLine => {
                                if let Some(ref mut cb) = handlers.on_stderr {
                                    if let Some(text) = stream_event.text {
                                        cb(text);
                                    }
                                }
                            }
                            StreamEventType::ExecutionComplete => {
                                if let Some(ref mut cb) = handlers.on_execution_complete {
                                    // 若无执行时间则默认传 0，避免丢失完成事件
                                    let time = stream_event.execution_time.unwrap_or(0);
                                    cb(time);
                                }
                                // 执行完成，退出循环
                                break;
                            }
                            StreamEventType::ExecutionError => {
                                // 遇到执行异常时返回错误信息并显式抛出
                                let err_msg = stream_event
                                    .error
                                    .map(|e| e.evalue)
                                    .unwrap_or_else(|| "Unknown execution error".to_string());
                                return Err(UpodError::Client(format!("Command execution error: {}", err_msg)));
                            }
                            // 忽略其他无关事件
                            _ => {}
                        }
                    }
                }
                Err(reqwest_eventsource::Error::StreamEnded) => {
                    // SSE stream ended normally
                    break;
                }
                Err(e) => {
                    // 网络中断或流异常时返回错误，避免静默失败
                    return Err(UpodError::Client(format!("SSE stream error: {}", e)));
                }
            }
        }

        Ok(())
    }

    /// 中断正在执行的命令
    ///
    /// DELETE /command?id={id}
    pub async fn interrupt_command(&self, command_id: &str) -> Result<()> {
        let bridge_url = self.get_bridge_url(44321);
        let url = format!("{}/command?id={}", bridge_url, command_id);
        
        let response = self.client.inner.delete(&url).send().await?;
        let status = response.status();
        if status.is_success() {
            Ok(())
        } else {
            let text = response.text().await.unwrap_or_default();
            Err(UpodError::Api { status, message: text })
        }
    }

    /// 获取命令的执行状态
    ///
    /// GET /command/status/{id}
    pub async fn get_command_status(&self, command_id: &str) -> Result<CommandStatus> {
        let bridge_url = self.get_bridge_url(44321);
        let url = format!("{}/command/status/{}", bridge_url, command_id);
        
        let response = self.client.inner.get(&url).send().await?;
        let status = response.status();
        if status.is_success() {
            let body = response.text().await?;
            match serde_json::from_str::<CommandStatus>(&body) {
                Ok(data) => Ok(data),
                Err(e) => {
                    Err(UpodError::Client(format!("Failed to parse CommandStatus: {} from body: {}", e, body)))
                }
            }
        } else {
            let text = response.text().await.unwrap_or_default();
            Err(UpodError::Api { status, message: text })
        }
    }

    /// 获取后台运行命令的输出日志（原始字节）及下一次拉取的 cursor
    ///
    /// GET /command/output/{id}?cursor={cursor}
    /// 返回值：(日志字节内容, 下一次拉取的 cursor)
    pub async fn get_command_output(&self, command_id: &str, cursor: Option<usize>) -> Result<(Vec<u8>, usize)> {
        let bridge_url = self.get_bridge_url(44321);
        let mut url = format!("{}/command/output/{}", bridge_url, command_id);
        if let Some(c) = cursor {
            url.push_str(&format!("?cursor={}", c));
        }
        
        let response = self.client.inner.get(&url).send().await?;
        let status = response.status();
        if status.is_success() {
            // 从响应头中提取新的 cursor
            let next_cursor = response
                .headers()
                .get("EXECD-COMMANDS-TAIL-CURSOR")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(cursor.unwrap_or(0));

            let bytes = response.bytes().await?;
            Ok((bytes.to_vec(), next_cursor))
        } else {
            let text = response.text().await.unwrap_or_default();
            Err(UpodError::Api { status, message: text })
        }
    }
}
