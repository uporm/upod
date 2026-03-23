use serde::{Deserialize, Serialize};

/// 文件权限信息，用于描述文件或目录的属主、属组和模式位。
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub(crate) struct Permission {
    /// 文件属主标识，通常为用户 ID 的字符串形式。
    #[serde(default)]
    pub(crate) owner: String,
    /// 文件属组标识，通常为组 ID 的字符串形式。
    #[serde(default)]
    pub(crate) group: String,
    /// 八进制权限数字（如 644、755），以十进制 u32 传输。
    #[serde(default)]
    pub(crate) mode: u32,
}

/// 上传文件的元数据，包含目标路径和可选权限设置。
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub(crate) struct FileMetadata {
    /// 目标文件路径。
    pub(crate) path: String,
    /// 扁平化权限字段，序列化后直接展开为 owner/group/mode。
    #[serde(flatten)]
    pub(crate) permission: Permission,
}

/// 文件详情信息，用于文件查询与搜索结果返回。
#[derive(Debug, Clone, Deserialize, Serialize)]
pub(crate) struct FileInfo {
    /// 文件绝对路径。
    pub(crate) path: String,
    /// 文件大小，单位为字节。
    pub(crate) size: i64,
    /// 最后修改时间（Unix 毫秒时间戳）。
    pub(crate) modified_at: u64,
    /// 创建时间（Unix 毫秒时间戳）。
    pub(crate) created_at: u64,
    /// 扁平化权限字段，序列化后直接展开为 owner/group/mode。
    #[serde(flatten)]
    pub(crate) permission: Permission,
}

/// 批量路径查询参数。
#[derive(Debug, Deserialize, Default)]
pub(crate) struct PathsReq {
    /// 需要处理的路径列表。
    #[serde(default)]
    pub(crate) path: Vec<String>,
}

/// 文件搜索查询参数。
#[derive(Debug, Deserialize)]
pub(crate) struct SearchReq {
    /// 搜索根目录路径。
    pub(crate) path: String,
    /// 文件名匹配模式，支持 `*` 与 `?`。
    pub(crate) pattern: Option<String>,
}

/// 批量重命名项，定义单次重命名的源路径与目标路径。
#[derive(Debug, Clone, Deserialize, Serialize)]
pub(crate) struct RenameFileItem {
    /// 源文件路径。
    pub(crate) src: String,
    /// 目标文件路径。
    pub(crate) dest: String,
}

/// 文件内容替换项，定义旧文本与新文本。
#[derive(Debug, Clone, Deserialize, Serialize)]
pub(crate) struct ReplaceFileContentItem {
    /// 要被替换的旧内容。
    pub(crate) old: String,
    /// 替换后的新内容。
    pub(crate) new: String,
}

/// 获取文件列表请求参数
#[derive(Debug, Deserialize)]
pub(crate) struct ListFilesReq {
    pub(crate) path: String,
    #[serde(rename = "sortBy")]
    pub(crate) sort_by: Option<String>,
}

/// 文件类型
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub(crate) enum FileType {
    #[serde(rename = "file")]
    File,
    #[serde(rename = "directory")]
    Directory,
    #[serde(rename = "symlink")]
    Symlink,
}

/// 文件树节点
#[derive(Debug, Clone, Deserialize, Serialize)]
pub(crate) struct FileNode {
    pub(crate) name: String,
    pub(crate) path: String,
    pub(crate) size: u64,
    pub(crate) mtime: u64,
    pub(crate) ctime: u64,
    #[serde(rename = "type")]
    pub(crate) file_type: FileType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) children: Option<Vec<FileNode>>,
}
