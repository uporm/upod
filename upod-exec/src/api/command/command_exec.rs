use std::convert::Infallible;
use std::env;
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::Json;
use axum::extract::Query;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::response::sse::{Event, KeepAlive, Sse};
use nix::sys::signal::{self, Signal};
use nix::unistd::Pid;
use tokio::process::Command;
use tokio::sync::{mpsc, watch};
use tokio_stream::StreamExt;
use tokio_stream::wrappers::UnboundedReceiverStream;

use super::command_log::{create_output_file, tail_file};
use super::command_session::{CommandSession, get_session};
use crate::models::command::{
    CommandError, InterruptReq, RunCommandReq, StreamEvent, StreamEventType,
};

/// 创建命令会话并以 SSE 方式返回执行生命周期事件。
/// 参数：req 为命令、命令执行目录、后台模式和超时配置。
/// 返回：SSE 响应，首条事件包含会话 ID。
/// 异常：请求命令为空时返回 400。
pub(crate) async fn run_command(Json(req): Json<RunCommandReq>) -> impl IntoResponse {
    if req.command.trim().is_empty() {
        return (StatusCode::BAD_REQUEST, "命令不能为空").into_response();
    }

    let (tx, rx) = mpsc::unbounded_channel::<StreamEvent>();
    let (cancel_tx, cancel_rx) = watch::channel(false);
    let session = CommandSession::new(
        req.command.clone(),
        req.background.unwrap_or(false),
        cancel_tx,
    )
    .await;

    let _ = tx.send(StreamEvent {
        event_type: StreamEventType::SessionInit,
        text: Some(session.id.clone()),
        error: None,
        execution_time: None,
    });

    tokio::spawn(async move {
        execute_command(session, req, cancel_rx, tx).await;
    });

    let stream = UnboundedReceiverStream::new(rx).map(|evt| {
        let payload = serde_json::to_string(&evt).unwrap_or_else(|_| "{}".to_string());
        Ok::<Event, Infallible>(Event::default().data(payload))
    });
    Sse::new(stream)
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(10)))
        .into_response()
}

/// 按会话 ID 中断正在执行的命令。
/// 参数：req.id 为目标会话 ID。
/// 返回：中断成功返回 204，不存在会话返回 404。
/// 异常：发送信号失败会被忽略，由执行循环兜底收敛状态。
pub(crate) async fn interrupt_command(Query(req): Query<InterruptReq>) -> impl IntoResponse {
    let Some(session) = get_session(&req.id).await else {
        return (StatusCode::NOT_FOUND, "命令会话不存在或已结束").into_response();
    };
    let _ = session.cancel_tx.send(true);

    let pid = *session.pid.lock().await;
    if let Some(p) = pid {
        let _ = signal::killpg(Pid::from_raw(p as i32), Signal::SIGINT);
    }

    StatusCode::NO_CONTENT.into_response()
}

/// 命令执行阶段的统一结果。
/// - Success: 子进程自然结束并返回退出码。
/// - Failure: 统一封装启动失败或运行失败，用于后续会话落库与 SSE 回传。
enum ExecutionResult {
    Success(i32),
    Failure(CommandFailure),
}

/// 命令失败的语义化分类。
/// Start 表示子进程尚未进入运行循环前即失败；
/// Runtime 表示进程启动成功后，在运行、超时、中断或状态查询中失败。
enum CommandFailure {
    Start { message: String },
    Runtime { message: String, code: i32 },
}

impl CommandFailure {
    /// 获取对外展示的错误消息文本。
    /// 用于会话状态记录与 SSE error 事件内容拼装。
    fn message(&self) -> &str {
        match self {
            Self::Start { message } => message,
            Self::Runtime { message, .. } => message,
        }
    }

    /// 获取 error 事件中可选的退出码。
    /// 启动失败通常没有可靠退出码，因此返回 None。
    fn code_for_event(&self) -> Option<i32> {
        match self {
            Self::Start { .. } => None,
            Self::Runtime { code, .. } => Some(*code),
        }
    }

    /// 获取会话最终写入的退出码。
    /// 启动失败统一映射为 1，运行失败使用真实退出码。
    fn session_exit_code(&self) -> i32 {
        match self {
            Self::Start { .. } => 1,
            Self::Runtime { code, .. } => *code,
        }
    }
}

/// 执行命令并维护会话完成态、取消、超时与事件发送。
/// 参数：session 为命令会话，req 为执行参数，cancel_rx 为取消信号，tx 为 SSE 发送器。
/// 异常：内部错误会转成 error/complete 事件，不向上传播。
async fn execute_command(
    session: Arc<CommandSession>,
    req: RunCommandReq,
    cancel_rx: watch::Receiver<bool>,
    tx: mpsc::UnboundedSender<StreamEvent>,
) {
    // started_at 仅用于最终 complete 事件中的 execution_time 计算。
    let started_at = now_millis();
    // background 在会话创建时已固化，执行阶段只读取一次，避免后续分支重复取值。
    let background = session.background;
    // timeout 统一转换为绝对 deadline，避免循环里重复做时长计算。
    let timeout_at = build_timeout_deadline(req.timeout);

    // 先完成“可运行命令”的准备工作（日志文件、shell 参数、spawn、记录 PID）。
    // 该阶段失败视为 Start 类错误，直接统一收敛并发送 error/complete。
    let child = match start_command_process(&session, &req).await {
        Ok(child) => child,
        Err(failure) => {
            finalize_execution_failure(&session, &tx, started_at, failure).await;
            return;
        }
    };

    // 后台模式：先回 accepted，再异步等待结束并更新会话，不阻塞当前请求链路。
    if background {
        handle_background_command(session, tx, started_at, child, cancel_rx, timeout_at);
        return;
    }

    // 前台模式：持续 tail 输出并等待进程结束，随后统一发送 complete（必要时先发 error）。
    let result = run_foreground_command(&session, &tx, child, cancel_rx, timeout_at).await;
    finalize_foreground_result(&session, &tx, started_at, result).await;
}

/// 启动并配置子进程。
/// 参数：session 提供输出文件路径和 pid 写回位置，req 提供命令文本与可选 cwd。
/// 返回：成功时返回已启动 child，失败时返回 Start 类错误。
/// 约束：stdout/stderr 写入同一输出文件，且进程加入独立进程组，便于超时或中断时回收子进程树。
async fn start_command_process(
    session: &Arc<CommandSession>,
    req: &RunCommandReq,
) -> Result<tokio::process::Child, CommandFailure> {
    let (output_file, stderr_file) =
        create_output_file(&session.output_file_path).map_err(|message| CommandFailure::Start {
            message,
        })?;

    let mut command = build_shell_command(&req.command);
    if let Some(cwd) = &req.cwd
        && !cwd.trim().is_empty()
    {
        command.current_dir(cwd);
    }

    command.stdin(Stdio::null());
    command.stdout(Stdio::from(output_file));
    command.stderr(Stdio::from(stderr_file));
    command.process_group(0);

    let child = command
        .spawn()
        .map_err(|error| CommandFailure::Start {
            message: format!("命令启动失败：{error}"),
        })?;

    if let Some(id) = child.id() {
        *session.pid.lock().await = Some(id);
    }

    Ok(child)
}

/// 执行前台命令并等待完成。
/// 过程：先启动 tail 任务持续推送 stdout 行事件，再等待命令完成或失败，
/// 最后通知 tail 任务退出并等待其收尾，确保尾部输出尽可能被消费。
/// 返回值为语义化执行结果，交由上层统一决定会话落库和 SSE 收尾策略。
async fn run_foreground_command(
    session: &Arc<CommandSession>,
    tx: &mpsc::UnboundedSender<StreamEvent>,
    child: tokio::process::Child,
    cancel_rx: watch::Receiver<bool>,
    timeout_at: Option<Instant>,
) -> ExecutionResult {
    let (done_tx, done_rx) = watch::channel(false);
    let output_file_path = session.output_file_path.clone();
    let tx_clone = tx.clone();
    let tail_task = tokio::spawn(async move {
        tail_file(
            output_file_path,
            false,
            tx_clone,
            done_rx.clone(),
        )
        .await;
    });

    let result = wait_for_command_completion(child, cancel_rx, timeout_at).await;

    let _ = done_tx.send(true);
    let _ = tail_task.await;

    result
}

/// 处理前台模式的最终结果。
/// 成功：仅落库成功状态并发送 complete。
/// 失败：走统一失败收敛（落库失败 + error + complete）。
async fn finalize_foreground_result(
    session: &Arc<CommandSession>,
    tx: &mpsc::UnboundedSender<StreamEvent>,
    started_at: u64,
    result: ExecutionResult,
) {
    match result {
        ExecutionResult::Success(code) => {
            session.finish(code, None).await;
            emit_execution_complete(tx, started_at);
        }
        ExecutionResult::Failure(failure) => {
            finalize_execution_failure(session, tx, started_at, failure).await;
        }
    }
}

/// 处理后台模式。
/// 该函数会先发送 accepted，告知请求侧“已受理并转后台”，
/// 然后在独立任务中等待进程结束并写回会话状态。
/// 后台模式不通过当前 SSE 流发送 complete/error，消费方通过状态查询和输出轮询感知结果。
fn handle_background_command(
    session: Arc<CommandSession>,
    tx: mpsc::UnboundedSender<StreamEvent>,
    started_at: u64,
    child: tokio::process::Child,
    cancel_rx: watch::Receiver<bool>,
    timeout_at: Option<Instant>,
) {
    let _ = tx.send(StreamEvent {
        event_type: StreamEventType::ExecutionAccepted,
        text: None,
        error: None,
        execution_time: Some(now_millis() - started_at),
    });

    tokio::spawn(async move {
        let result = wait_for_command_completion(child, cancel_rx, timeout_at).await;
        match result {
            ExecutionResult::Success(code) => {
                session.finish(code, None).await;
            }
            ExecutionResult::Failure(failure) => {
                finish_session_with_failure(&session, &failure).await;
            }
        }
    });
}

/// 等待子进程完成，并统一转为语义化执行结果。
/// 判定优先级依次为：外部取消、超时、自然退出、状态查询异常。
/// 实现上，取消和超时都会先 kill 进程组再 wait，以降低僵尸进程风险；
/// 同时通过短间隔 sleep 与 watch.changed 组合来平衡响应性和 CPU 占用。
async fn wait_for_command_completion(
    mut child: tokio::process::Child,
    mut cancel_rx: watch::Receiver<bool>,
    timeout_at: Option<Instant>,
) -> ExecutionResult {
    loop {
        if *cancel_rx.borrow() {
            kill_process_group(child.id()).await;
            let _ = child.wait().await;
            break ExecutionResult::Failure(CommandFailure::Runtime {
                message: "命令已被中断".to_string(),
                code: 130,
            });
        }
        if let Some(deadline) = timeout_at
            && Instant::now() >= deadline
        {
            kill_process_group(child.id()).await;
            let _ = child.wait().await;
            break ExecutionResult::Failure(CommandFailure::Runtime {
                message: "命令执行超时".to_string(),
                code: 124,
            });
        }

        match child.try_wait() {
            Ok(Some(status)) => break ExecutionResult::Success(status.code().unwrap_or(0)),
            Ok(None) => {}
            Err(error) => {
                break ExecutionResult::Failure(CommandFailure::Runtime {
                    message: format!("查询进程状态失败：{error}"),
                    code: 1,
                });
            }
        }

        tokio::select! {
            _ = cancel_rx.changed() => {}
            _ = tokio::time::sleep(Duration::from_millis(50)) => {}
        }
    }
}

/// 统一处理失败路径的收敛动作。
/// 顺序固定为“写会话失败状态 -> 发送 error -> 发送 complete（含 execution_time）”。
/// 这样可确保客户端在收到 complete 前，已拿到完整失败上下文。
async fn finalize_execution_failure(
    session: &Arc<CommandSession>,
    tx: &mpsc::UnboundedSender<StreamEvent>,
    started_at: u64,
    failure: CommandFailure,
) {
    finish_session_with_failure(session, &failure).await;
    emit_execution_error(tx, &failure);
    emit_execution_complete(tx, started_at);
}

/// 将失败结果写入会话终态。
/// exit_code 使用语义化映射，error 字段写入标准化 message，便于状态接口直接展示。
async fn finish_session_with_failure(session: &Arc<CommandSession>, failure: &CommandFailure) {
    session
        .finish(
            failure.session_exit_code(),
            Some(failure.message().to_string()),
        )
        .await;
}

/// 发送统一格式的 error 事件。
/// Start 失败时 evalue 直接展示错误信息；
/// Runtime 失败时 evalue 优先拼接退出码和错误信息，便于快速定位。
fn emit_execution_error(tx: &mpsc::UnboundedSender<StreamEvent>, failure: &CommandFailure) {
    let message = failure.message();
    let evalue = match failure.code_for_event() {
        Some(exit_code) if exit_code >= 0 => format!("退出码 {exit_code}：{message}"),
        _ => message.to_string(),
    };
    let _ = tx.send(StreamEvent {
        event_type: StreamEventType::ExecutionError,
        text: None,
        error: Some(CommandError {
            ename: "命令执行错误".to_string(),
            evalue,
            traceback: vec![message.to_string()],
        }),
        execution_time: None,
    });
}

/// 发送 complete 事件并附带执行耗时（毫秒）。
/// 注意：耗时基于 execute_command 开始时间计算，覆盖启动、等待与收尾阶段。
fn emit_execution_complete(tx: &mpsc::UnboundedSender<StreamEvent>, started_at: u64) {
    let _ = tx.send(StreamEvent {
        event_type: StreamEventType::ExecutionComplete,
        text: None,
        error: None,
        execution_time: Some(now_millis() - started_at),
    });
}

/// 向目标进程组发送 SIGKILL，确保子进程树终止。
/// 参数：pid 为子进程 PID。
async fn kill_process_group(pid: Option<u32>) {
    if let Some(p) = pid {
        let _ = signal::killpg(Pid::from_raw(p as i32), Signal::SIGKILL);
    }
}

/// 生成毫秒级 Unix 时间戳。
/// 返回：当前毫秒时间戳；系统时钟异常时返回 0。
fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|v| v.as_millis() as u64)
        .unwrap_or(0)
}

/// 构建超时截止时间。
/// 参数：timeout_ms 为超时毫秒数。
/// 返回：无超时时为 None，合法超时时返回 deadline。
fn build_timeout_deadline(timeout_ms: Option<u64>) -> Option<Instant> {
    timeout_ms.and_then(|ms| Instant::now().checked_add(Duration::from_millis(ms)))
}

/// 选择 Unix 平台的 shell 程序路径。
/// 参数：shell_from_env 为环境变量 SHELL。
/// 返回：优先使用非空 SHELL，否则回退 /bin/sh。
fn select_unix_shell(shell_from_env: Option<String>) -> String {
    shell_from_env
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "/bin/sh".to_string())
}

/// 按平台构造 shell 执行命令。
/// 参数：command 为待执行的 shell 文本。
/// 返回：配置好参数的 tokio::process::Command。
/// 异常：无；真正失败在 spawn 阶段处理。
fn build_shell_command(command: &str) -> Command {
    #[cfg(target_os = "windows")]
    {
        let mut cmd = Command::new("cmd");
        cmd.arg("/C").arg(command);
        cmd
    }
    #[cfg(not(target_os = "windows"))]
    {
        let shell = select_unix_shell(env::var("SHELL").ok());
        let mut cmd = Command::new(shell);
        cmd.arg("-c").arg(command);
        cmd
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 验证 Unix shell 选择优先使用环境变量值。
    #[cfg(not(target_os = "windows"))]
    #[test]
    fn select_unix_shell_prefers_env_value() {
        assert_eq!(
            select_unix_shell(Some("/bin/zsh".to_string())),
            "/bin/zsh".to_string()
        );
    }

    /// 验证 Unix shell 在空值场景下回退到 /bin/sh。
    #[cfg(not(target_os = "windows"))]
    #[test]
    fn select_unix_shell_falls_back_to_sh() {
        assert_eq!(select_unix_shell(None), "/bin/sh".to_string());
        assert_eq!(
            select_unix_shell(Some("   ".to_string())),
            "/bin/sh".to_string()
        );
    }

    /// 验证 Windows 平台使用 cmd /C 组装命令。
    #[cfg(target_os = "windows")]
    #[test]
    fn build_shell_command_uses_cmd() {
        let command = build_shell_command("echo hello");
        let program = command.as_std().get_program().to_string_lossy().to_string();
        let args = command
            .as_std()
            .get_args()
            .map(|value| value.to_string_lossy().to_string())
            .collect::<Vec<_>>();

        assert_eq!(program.to_ascii_lowercase(), "cmd".to_string());
        assert_eq!(args, vec!["/C".to_string(), "echo hello".to_string()]);
    }

    /// 验证 Unix 平台使用 -c 传递 shell 命令文本。
    #[cfg(not(target_os = "windows"))]
    #[test]
    fn build_shell_command_uses_c_flag() {
        let command = build_shell_command("echo hello");
        let args = command
            .as_std()
            .get_args()
            .map(|value| value.to_string_lossy().to_string())
            .collect::<Vec<_>>();

        assert_eq!(args, vec!["-c".to_string(), "echo hello".to_string()]);
    }

    /// 验证超时为空时不设置截止时间。
    #[test]
    fn build_timeout_deadline_none() {
        let timeout = build_timeout_deadline(None);
        assert!(timeout.is_none());
    }
}
