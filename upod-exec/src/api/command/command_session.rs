use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use tokio::sync::{Mutex, RwLock, watch};
use uuid::Uuid;

use crate::models::command::CommandStatus;

type CommandSessions = RwLock<HashMap<String, Arc<CommandSession>>>;
static COMMAND_SESSIONS: OnceLock<CommandSessions> = OnceLock::new();
const SESSION_RETENTION_MS: u64 = 10 * 60 * 1000;

fn command_output_dir() -> PathBuf {
    std::env::temp_dir().join("upod")
}

pub fn command_output_path_by_id(id: &str) -> PathBuf {
    command_output_dir().join(format!("{id}.output"))
}

pub(crate) fn init_command_output_dir() -> Result<(), std::io::Error> {
    std::fs::create_dir_all(command_output_dir())
}

/// 返回全局命令会话表的惰性单例。
/// 参数：无。
/// 返回：全局读写锁保护的会话表引用。
/// 异常：无；初始化过程不向外抛错。
fn command_sessions() -> &'static CommandSessions {
    COMMAND_SESSIONS.get_or_init(|| RwLock::new(HashMap::new()))
}

/// 按会话 ID 查询会话快照源对象。
/// 参数：id 为会话 ID。
/// 返回：存在时返回会话 Arc，不存在返回 None。
/// 异常：无；锁竞争会等待。
pub(crate) async fn get_session(id: &str) -> Option<Arc<CommandSession>> {
    command_sessions().read().await.get(id).cloned()
}

/// 清理超过保留时间且已结束的会话。
/// 参数：无。
/// 返回：无。
/// 异常：无；锁竞争会等待。
async fn prune_expired_sessions() {
    let now = now_millis();
    let sessions = {
        let sessions = command_sessions().read().await;
        sessions
            .iter()
            .map(|(id, session)| (id.clone(), session.clone()))
            .collect::<Vec<_>>()
    };

    let mut expired_ids = Vec::new();
    for (id, session) in sessions {
        if session.is_expired(now).await {
            expired_ids.push(id);
        }
    }

    if expired_ids.is_empty() {
        return;
    }

    let mut sessions = command_sessions().write().await;
    for id in expired_ids {
        sessions.remove(&id);
    }
}

struct CompletionState {
    exit_code: Option<i32>,
    error: Option<String>,
    finished_at: Option<u64>,
}

pub(crate) struct CommandSession {
    pub(crate) id: String,
    pub(crate) content: String,
    pub(crate) started_at: u64,
    pub(crate) running: AtomicBool,
    completion: Mutex<CompletionState>,
    pub(crate) output_file_path: PathBuf,
    pub(crate) cancel_tx: watch::Sender<bool>,
    pub(crate) background: bool,
    pub(crate) pid: Mutex<Option<u32>>,
}

impl CommandSession {
    /// 构造会话并立即注册到全局会话表。
    /// 参数：id 为会话 ID，content 为命令文本，background 为后台标记，cancel_tx 为取消信号发送器。
    /// 返回：已注册的会话 Arc。
    /// 异常：无；锁竞争会等待，不会静默丢弃写入。
    pub(crate) async fn new(
        content: String,
        background: bool,
        cancel_tx: watch::Sender<bool>,
    ) -> Arc<Self> {
        prune_expired_sessions().await;
        let id = Uuid::new_v4().simple().to_string();
        let output_file_path = command_output_path_by_id(&id);

        let session = Arc::new(Self {
            id: id.clone(),
            content,
            started_at: now_millis(),
            running: AtomicBool::new(true),
            completion: Mutex::new(CompletionState {
                exit_code: None,
                error: None,
                finished_at: None,
            }),
            output_file_path,
            cancel_tx,
            background,
            pid: Mutex::new(None),
        });
        command_sessions().write().await.insert(id, session.clone());
        session
    }

    /// 读取当前会话状态并生成对外响应模型。
    /// 参数：self 为会话实例引用。
    /// 返回：包含运行态与完成态的 CommandStatus。
    /// 异常：无；读取时会等待互斥锁。
    pub(crate) async fn snapshot(&self) -> CommandStatus {
        let completion = self.completion.lock().await;
        CommandStatus {
            id: self.id.clone(),
            content: self.content.clone(),
            running: self.running.load(Ordering::Acquire),
            exit_code: completion.exit_code,
            error: completion.error.clone(),
            started_at: self.started_at,
            finished_at: completion.finished_at,
        }
    }

    /// 将会话标记为完成并写入退出信息。
    /// 参数：exit_code 为进程退出码，err 为可选错误信息。
    /// 返回：无。
    /// 异常：无；写入时会等待互斥锁。
    pub(crate) async fn finish(&self, exit_code: i32, err: Option<String>) {
        let mut completion = self.completion.lock().await;
        completion.exit_code = Some(exit_code);
        completion.error = err;
        completion.finished_at = Some(now_millis());
        self.running.store(false, Ordering::Release);
        drop(completion);
        prune_expired_sessions().await;
    }

    /// 判断会话是否超过保留窗口并可回收。
    /// 参数：now 为当前毫秒时间戳。
    /// 返回：true 表示可回收，false 表示应保留。
    /// 异常：无；读取时会等待互斥锁。
    async fn is_expired(&self, now: u64) -> bool {
        if self.running.load(Ordering::Acquire) {
            return false;
        }
        let completion = self.completion.lock().await;
        let Some(finished_at) = completion.finished_at else {
            return false;
        };
        now.saturating_sub(finished_at) > SESSION_RETENTION_MS
    }
}

/// 获取当前 Unix 毫秒时间戳。
/// 参数：无。
/// 返回：当前时间毫秒值；异常场景返回 0。
/// 异常：无；系统时间异常不会向外抛出。
fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|v| u64::try_from(v.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 清空全局会话表。
    /// 参数：无。
    /// 返回：无。
    /// 异常：无；锁竞争会等待。
    async fn clear_sessions() {
        command_sessions().write().await.clear();
    }

    #[tokio::test]
    async fn finished_session_is_pruned_after_retention() {
        clear_sessions().await;
        let (cancel_tx, _) = watch::channel(false);
        let session = CommandSession::new("echo hello".to_string(), false, cancel_tx).await;
        session.finish(0, None).await;
        {
            let mut completion = session.completion.lock().await;
            completion.finished_at = Some(now_millis().saturating_sub(SESSION_RETENTION_MS + 1));
        }

        prune_expired_sessions().await;

        let result = get_session(&session.id).await;
        assert!(result.is_none());
        clear_sessions().await;
    }

    #[tokio::test]
    async fn running_session_is_not_pruned() {
        clear_sessions().await;
        let (cancel_tx, _) = watch::channel(false);
        let session = CommandSession::new("echo hello".to_string(), false, cancel_tx).await;

        prune_expired_sessions().await;

        let result = get_session(&session.id).await;
        assert!(result.is_some());
        clear_sessions().await;
    }
}
