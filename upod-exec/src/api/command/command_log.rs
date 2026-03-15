use std::fs::File;
use std::io::SeekFrom;
use std::path::Path;
use std::path::PathBuf;

use axum::extract::Query;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::IntoResponse;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio::sync::{mpsc, watch};

use super::command_session::{command_output_path_by_id, get_session};
use crate::models::command::{OutputReq, StreamEvent, StreamEventType};

const OPEN_RETRY_MS: u64 = 50;
const TAIL_RETRY_MS: u64 = 100;

/// 创建用于合并 stdout/stderr 的同一日志文件句柄。
/// 参数 path 为输出文件路径。
/// 返回 (stdout 文件句柄, stderr 克隆句柄) 或错误信息。
pub fn create_output_file(path: &Path) -> Result<(File, File), String> {
    let output_file = File::create(path).map_err(|e| format!("无法创建日志文件: {}", e))?;
    let stderr_file = output_file
        .try_clone()
        .map_err(|e| format!("无法克隆日志文件句柄: {}", e))?;
    Ok((output_file, stderr_file))
}

/// 读取命令输出的增量内容。
/// 参数：id 为会话 ID，cursor 为上次读取偏移。
/// 返回：文本输出与下一次读取 cursor 头。
/// 异常：文件读取失败时返回 500。
pub async fn get_command_output(
    axum::extract::Path(id): axum::extract::Path<String>,
    Query(query): Query<OutputReq>,
) -> impl IntoResponse {
    let combined_path = if let Some(session) = get_session(&id).await {
        session.output_file_path.clone()
    } else {
        command_output_path_by_id(&id)
    };

    let cursor = query.cursor.unwrap_or(0) as u64;
    let mut file = match tokio::fs::File::open(&combined_path).await {
        Ok(f) => f,
        Err(_) => {
            let headers = build_output_headers(cursor);
            return (StatusCode::OK, headers, "").into_response();
        }
    };

    let file_len = file.metadata().await.map(|m| m.len()).unwrap_or(0);
    let cursor = cursor.min(file_len);

    if file.seek(SeekFrom::Start(cursor)).await.is_err() {
        return (StatusCode::INTERNAL_SERVER_ERROR, "Seek failed").into_response();
    }

    let mut buffer = Vec::new();
    if file.read_to_end(&mut buffer).await.is_err() {
        return (StatusCode::INTERNAL_SERVER_ERROR, "Read failed").into_response();
    }

    let new_cursor = cursor + buffer.len() as u64;
    let output = String::from_utf8_lossy(&buffer).to_string();

    let headers = build_output_headers(new_cursor);
    (StatusCode::OK, headers, output).into_response()
}

/// 持续跟踪日志文件新增内容并按行推送 SSE 事件。
/// 参数 path 为日志文件路径，is_stderr 标识流类型，tx 为发送器，done 为结束信号。
/// 返回值为空；当收到 done 或文件结束后退出。
pub async fn tail_file(
    path: PathBuf,
    is_stderr: bool,
    tx: mpsc::UnboundedSender<StreamEvent>,
    done: watch::Receiver<bool>,
) {
    let event_type = event_type(is_stderr);
    let file = loop {
        match tokio::fs::File::open(&path).await {
            Ok(f) => break f,
            Err(_) => {
                if *done.borrow() {
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(OPEN_RETRY_MS)).await;
            }
        }
    };

    let mut reader = BufReader::new(file);
    let mut line = String::new();

    loop {
        match reader.read_line(&mut line).await {
            Ok(0) => {
                if *done.borrow() {
                    emit_line(&tx, event_type, &line);
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(TAIL_RETRY_MS)).await;
            }
            Ok(_) => {
                if line.ends_with('\n') {
                    emit_line(&tx, event_type, &line);
                    line.clear();
                }
            }
            Err(_) => {
                if *done.borrow() {
                    emit_line(&tx, event_type, &line);
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(TAIL_RETRY_MS)).await;
            }
        }
    }
}

/// 根据请求的输出偏移生成响应头。
/// 参数：cursor 为下一次轮询起点偏移。
/// 返回：包含 cursor 与 content-type 的 HeaderMap。
/// 异常：无；header 转换失败时回退为 0。
fn build_output_headers(cursor: u64) -> HeaderMap {
    let mut headers = HeaderMap::new();
    let header_value = HeaderValue::from_str(&cursor.to_string())
        .unwrap_or_else(|_| HeaderValue::from_static("0"));
    headers.insert("EXECD-COMMANDS-TAIL-CURSOR", header_value);
    headers.insert(
        axum::http::header::CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    headers
}

/// 根据流来源选择 SSE 事件类型。
/// 参数 is_stderr 为 true 时返回 stderr 事件，否则返回 stdout 事件。
fn event_type(is_stderr: bool) -> StreamEventType {
    if is_stderr {
        StreamEventType::StderrLine
    } else {
        StreamEventType::StdoutLine
    }
}

/// 发送单行输出事件，自动去除行尾换行并跳过空行。
/// 参数 tx 为事件发送器，event_type 为事件类型，line 为原始行文本。
fn emit_line(tx: &mpsc::UnboundedSender<StreamEvent>, event_type: StreamEventType, line: &str) {
    let text = line.trim_end_matches(['\n', '\r']);
    if text.is_empty() {
        return;
    }

    let _ = tx.send(StreamEvent {
        event_type,
        text: Some(text.to_string()),
        error: None,
        execution_time: None,
    });
}
