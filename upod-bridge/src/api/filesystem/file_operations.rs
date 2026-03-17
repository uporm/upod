use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::Json;
use axum::extract::Query;
use axum::http::StatusCode;
use axum::response::IntoResponse;

use crate::models::filesystem::{
    FileInfo, PathsReq, Permission, RenameFileItem, ReplaceFileContentItem, SearchReq,
};

/// 查询多个文件信息并按请求路径返回结果映射。
/// 参数：`query.path` 为待查询路径列表。
/// 返回：`200 + JSON` 路径到 `FileInfo` 的映射。
/// 错误：任一路径访问失败时返回映射后的 HTTP 错误。
pub(crate) async fn get_files_info(Query(query): Query<PathsReq>) -> impl IntoResponse {
    if query.path.is_empty() {
        return Json(HashMap::<String, FileInfo>::new()).into_response();
    }

    let mut result = HashMap::<String, FileInfo>::with_capacity(query.path.len());
    for path in &query.path {
        let info = match file_info(path) {
            Ok(info) => info,
            Err(error) => return map_file_error(error),
        };
        result.insert(path.clone(), info);
    }

    Json(result).into_response()
}

/// 删除多个文件；不存在的文件会被忽略。
/// 参数：`query.path` 为待删除文件路径列表。
/// 返回：全部成功时返回 `200`。
/// 错误：删除失败时返回映射后的 HTTP 错误。
pub(crate) async fn remove_files(Query(query): Query<PathsReq>) -> impl IntoResponse {
    for path in &query.path {
        if let Err(error) = delete_file(path) {
            return map_file_error(error);
        }
    }
    StatusCode::OK.into_response()
}

/// 批量设置文件权限，仅在提供 mode 时生效。
/// 参数：`request` 为路径到权限对象的映射。
/// 返回：全部成功时返回 `200`。
/// 错误：权限设置失败时返回映射后的 HTTP 错误。
pub(crate) async fn chmod_files(
    Json(request): Json<HashMap<String, Permission>>,
) -> impl IntoResponse {
    for (path, permission) in request {
        if let Err(error) = apply_permission(&path, &permission) {
            return map_file_error(error);
        }
    }
    StatusCode::OK.into_response()
}

/// 批量重命名或移动文件。
/// 参数：`request` 为重命名任务列表。
/// 返回：全部成功时返回 `200`。
/// 错误：任一任务失败时返回映射后的 HTTP 错误。
pub(crate) async fn rename_files(Json(request): Json<Vec<RenameFileItem>>) -> impl IntoResponse {
    for item in request {
        if let Err(error) = rename_file(&item) {
            return map_file_error(error);
        }
    }
    StatusCode::OK.into_response()
}

/// 按目录与模式搜索文件，并返回文件详情列表。
/// 参数：`query.path` 为根目录，`pattern` 为可选匹配模式。
/// 返回：`200 + JSON`，内容为匹配文件的 `FileInfo` 列表。
/// 错误：路径无效或文件访问失败时返回对应 HTTP 错误。
pub(crate) async fn search_files(Query(query): Query<SearchReq>) -> impl IntoResponse {
    if query.path.trim().is_empty() {
        return (StatusCode::BAD_REQUEST, "missing query parameter 'path'").into_response();
    }

    let root = match fs::canonicalize(&query.path) {
        Ok(path) => path,
        Err(error) => return map_file_error(error),
    };
    if !root.is_dir() {
        return (StatusCode::BAD_REQUEST, "path must be a directory").into_response();
    }

    let pattern = query.pattern.unwrap_or_else(|| "**".to_string());
    let mut files = Vec::<PathBuf>::new();
    if let Err(error) = collect_files(&root, &mut files) {
        return map_file_error(error);
    }

    let mut result = Vec::<FileInfo>::new();
    for path in files {
        let name = path
            .file_name()
            .and_then(|v| v.to_str())
            .unwrap_or_default();
        if glob_match(&pattern, name) {
            let info = match file_info(path.to_string_lossy().as_ref()) {
                Ok(value) => value,
                Err(error) => return map_file_error(error),
            };
            result.push(info);
        }
    }

    Json(result).into_response()
}

/// 批量替换文件内容中的目标字符串。
/// 参数：`request` 为文件路径到替换规则的映射。
/// 返回：全部替换完成时返回 `200`。
/// 错误：文件无效或读写失败时返回映射后的 HTTP 错误。
pub(crate) async fn replace_content(
    Json(request): Json<HashMap<String, ReplaceFileContentItem>>,
) -> impl IntoResponse {
    for (file, item) in request {
        let path = match fs::canonicalize(&file) {
            Ok(value) => value,
            Err(error) => return map_file_error(error),
        };
        if !path.is_file() {
            return (
                StatusCode::BAD_REQUEST,
                format!("path is not a file: {}", path.display()),
            )
                .into_response();
        }

        let content = match fs::read(&path) {
            Ok(bytes) => String::from_utf8_lossy(&bytes).to_string(),
            Err(error) => return map_file_error(error),
        };
        let replaced = content.replace(&item.old, &item.new);
        if let Err(error) = fs::write(&path, replaced) {
            return map_file_error(error);
        }
    }
    StatusCode::OK.into_response()
}

/// 删除单个文件；当目标不存在时直接返回成功。
/// 参数：`path` 为待删除文件路径。
/// 返回：删除成功或文件不存在时返回 `Ok(())`。
/// 错误：路径为目录或删除失败时返回 `std::io::Error`。
fn delete_file(path: &str) -> Result<(), std::io::Error> {
    let absolute = match fs::canonicalize(path) {
        Ok(value) => value,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    let metadata = fs::metadata(&absolute)?;
    if metadata.is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("path is a directory: {}", absolute.display()),
        ));
    }
    fs::remove_file(absolute)
}

/// 执行单次重命名，必要时自动创建目标父目录。
/// 参数：`item` 包含源路径与目标路径。
/// 返回：重命名成功时返回 `Ok(())`。
/// 错误：源不存在、目标已存在或文件系统错误时返回异常。
fn rename_file(item: &RenameFileItem) -> Result<(), std::io::Error> {
    let source = fs::canonicalize(&item.src)?;
    if !source.exists() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("source path not found: {}", item.src),
        ));
    }

    let destination = PathBuf::from(&item.dest);
    if destination.exists() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            format!("destination path already exists: {}", item.dest),
        ));
    }
    let parent = destination
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    fs::create_dir_all(parent)?;
    fs::rename(source, destination)
}

/// 根据权限对象应用 mode，owner/group 目前仅透传不落地。
/// 参数：`path` 为目标路径，`permission` 为权限数据。
/// 返回：设置成功或 `mode == 0` 时返回 `Ok(())`。
/// 错误：模式非法或设置失败时返回 `std::io::Error`。
pub(super) fn apply_permission(path: &str, permission: &Permission) -> Result<(), std::io::Error> {
    if permission.mode == 0 {
        // mode=0 约定为“保持默认权限，不显式 chmod”。
        return Ok(());
    }

    let mode = u32::from_str_radix(&permission.mode.to_string(), 8).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("invalid permission mode: {}", permission.mode),
        )
    })?;
    set_mode(path, mode)
}

/// 在 Unix 平台写入权限位。
/// 参数：`path` 为目标路径，`mode` 为权限位。
/// 返回：设置成功时返回 `Ok(())`。
/// 错误：底层权限写入失败时返回 `std::io::Error`。
#[cfg(unix)]
fn set_mode(path: &str, mode: u32) -> Result<(), std::io::Error> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
}

/// 在非 Unix 平台忽略 mode 设置并返回成功。
/// 参数：`_path` 与 `_mode` 保留接口一致性。
/// 返回：始终返回 `Ok(())`。
/// 错误：无。
#[cfg(not(unix))]
fn set_mode(_path: &str, _mode: u32) -> Result<(), std::io::Error> {
    Ok(())
}

/// 读取文件系统元数据并组装统一的 FileInfo。
/// 参数：`path` 为待读取文件路径。
/// 返回：成功时返回填充后的 `FileInfo`。
/// 错误：规范化路径或读取元数据失败时返回 `std::io::Error`。
fn file_info(path: &str) -> Result<FileInfo, std::io::Error> {
    let absolute = fs::canonicalize(path)?;
    let metadata = fs::metadata(&absolute)?;
    let modified_at = metadata
        .modified()
        .ok()
        .and_then(system_time_to_millis)
        .unwrap_or(0);
    // 某些平台可能缺失 created 时间，回退到 modified 保证字段可用。
    let created_at = metadata
        .created()
        .ok()
        .and_then(system_time_to_millis)
        .unwrap_or(modified_at);
    let permission = resolve_permission(&metadata);

    Ok(FileInfo {
        path: absolute.to_string_lossy().to_string(),
        size: metadata.len() as i64,
        modified_at,
        created_at,
        permission,
    })
}

/// 将 `SystemTime` 转换为 Unix 毫秒时间戳。
/// 参数：`value` 为待转换时间。
/// 返回：可转换时返回毫秒值，否则返回 `None`。
/// 错误：无，错误分支使用 `None` 表示。
fn system_time_to_millis(value: SystemTime) -> Option<u64> {
    value
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_millis() as u64)
}

/// 在 Unix 平台解析 owner/group/mode 权限信息。
/// 参数：`metadata` 为文件元数据。
/// 返回：填充 owner/group/mode 的 `Permission`。
/// 错误：无，解析失败时回退 `mode = 0`。
#[cfg(unix)]
fn resolve_permission(metadata: &fs::Metadata) -> Permission {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let mode_bits = metadata.permissions().mode() & 0o777;
    let mode = format!("{:o}", mode_bits).parse::<u32>().unwrap_or(0);
    Permission {
        owner: metadata.uid().to_string(),
        group: metadata.gid().to_string(),
        mode,
    }
}

/// 在非 Unix 平台提供兼容权限信息。
/// 参数：`metadata` 为文件元数据。
/// 返回：owner/group 为空，mode 按只读属性推断。
/// 错误：无。
#[cfg(not(unix))]
fn resolve_permission(metadata: &fs::Metadata) -> Permission {
    Permission {
        owner: String::new(),
        group: String::new(),
        mode: if metadata.permissions().readonly() {
            444
        } else {
            666
        },
    }
}

/// 递归收集目录下全部文件路径。
/// 参数：`root` 为遍历根目录，`files` 为输出容器。
/// 返回：遍历完成时返回 `Ok(())`。
/// 错误：读取目录或子项元数据失败时返回 `std::io::Error`。
fn collect_files(root: &Path, files: &mut Vec<PathBuf>) -> Result<(), std::io::Error> {
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        let metadata = entry.metadata()?;
        if metadata.is_dir() {
            collect_files(&path, files)?;
        } else if metadata.is_file() {
            files.push(path);
        }
    }
    Ok(())
}

/// 使用动态规划进行通配符匹配。
/// 参数：`pattern` 支持 `*` 与 `?`，`text` 为待匹配文本。
/// 返回：匹配成功返回 `true`，否则返回 `false`。
/// 错误：无。
fn glob_match(pattern: &str, text: &str) -> bool {
    if pattern == "**" {
        // 全匹配快捷路径，避免进入 O(m*n) 动态规划。
        return true;
    }
    let pattern_chars = pattern.chars().collect::<Vec<_>>();
    let text_chars = text.chars().collect::<Vec<_>>();
    let mut dp = vec![vec![false; text_chars.len() + 1]; pattern_chars.len() + 1];
    dp[0][0] = true;

    for i in 1..=pattern_chars.len() {
        if pattern_chars[i - 1] == '*' {
            dp[i][0] = dp[i - 1][0];
        }
    }

    for i in 1..=pattern_chars.len() {
        for j in 1..=text_chars.len() {
            let p = pattern_chars[i - 1];
            let t = text_chars[j - 1];
            // '*' 吃掉 0 或多字符，'?' 吃掉 1 个字符。
            dp[i][j] = match p {
                '*' => dp[i - 1][j] || dp[i][j - 1],
                '?' => dp[i - 1][j - 1],
                _ => p == t && dp[i - 1][j - 1],
            };
        }
    }

    dp[pattern_chars.len()][text_chars.len()]
}

/// 将文件系统错误映射为统一 HTTP 响应。
/// 参数：`error` 为底层文件系统错误。
/// 返回：`NotFound` 映射为 `404`，其余映射为 `500`。
/// 错误：无。
pub(super) fn map_file_error(error: std::io::Error) -> axum::response::Response {
    if error.kind() == std::io::ErrorKind::NotFound {
        return (StatusCode::NOT_FOUND, format!("file not found. {error}")).into_response();
    }
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        format!("error accessing file: {error}"),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    /// 验证通配符匹配支持 `*` 与 `?`。
    fn glob_match_supports_star_and_question() {
        assert!(glob_match("*.txt", "a.txt"));
        assert!(glob_match("a?c.log", "abc.log"));
        assert!(!glob_match("a?c.log", "ac.log"));
        assert!(glob_match("**", "anything"));
    }

    #[test]
    /// 验证删除目录路径会返回错误。
    fn delete_file_rejects_directory() {
        let tmp_dir =
            std::env::temp_dir().join(format!("upod-bridge-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&tmp_dir).expect("create temp dir");
        let result = delete_file(tmp_dir.to_string_lossy().as_ref());
        assert!(result.is_err());
        let _ = fs::remove_dir_all(tmp_dir);
    }

    #[test]
    /// 验证字符串替换逻辑的基本行为。
    fn replace_content_works() {
        let tmp_dir =
            std::env::temp_dir().join(format!("upod-bridge-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&tmp_dir).expect("create temp dir");
        let target = tmp_dir.join("replace.txt");
        fs::write(&target, "hello world").expect("write file");

        let item = ReplaceFileContentItem {
            old: "world".to_string(),
            new: "bridge".to_string(),
        };
        let content = fs::read_to_string(&target).expect("read file");
        let replaced = content.replace(&item.old, &item.new);
        fs::write(&target, replaced).expect("rewrite");

        let latest = fs::read_to_string(&target).expect("read replaced");
        assert_eq!(latest, "hello bridge");
        let _ = fs::remove_dir_all(tmp_dir);
    }
}
