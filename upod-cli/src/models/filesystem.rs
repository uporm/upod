use serde::{Deserialize, Serialize};

/// 文件权限信息，用于描述文件或目录的属主、属组和模式位。
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct Permission {
    /// 文件属主标识，通常为用户 ID 的字符串形式。
    #[serde(default)]
    pub owner: String,
    /// 文件属组标识，通常为组 ID 的字符串形式。
    #[serde(default)]
    pub group: String,
    /// 八进制权限数字（如 644、755），以十进制 u32 传输。
    #[serde(default)]
    pub mode: u32,
}

/// 上传文件的元数据，包含目标路径和可选权限设置。
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct FileMetadata {
    /// 目标文件路径。
    pub path: String,
    /// 扁平化权限字段，序列化后直接展开为 owner/group/mode。
    #[serde(flatten)]
    pub permission: Permission,
}

/// 文件详情信息，用于文件查询与搜索结果返回。
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FileInfo {
    /// 文件绝对路径。
    pub path: String,
    /// 文件大小，单位为字节。
    pub size: i64,
    /// 最后修改时间（Unix 毫秒时间戳）。
    pub modified_at: u64,
    /// 创建时间（Unix 毫秒时间戳）。
    pub created_at: u64,
    /// 扁平化权限字段，序列化后直接展开为 owner/group/mode。
    #[serde(flatten)]
    pub permission: Permission,
}

/// 批量重命名项，定义单次重命名的源路径与目标路径。
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RenameFileItem {
    /// 源文件路径。
    pub src: String,
    /// 目标文件路径。
    pub dest: String,
}

/// 文件内容替换项，定义旧文本与新文本。
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ReplaceFileContentItem {
    /// 要被替换的旧内容。
    pub old: String,
    /// 替换后的新内容。
    pub new: String,
}

/// 文件类型
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub enum FileType {
    #[serde(rename = "file")]
    File,
    #[serde(rename = "directory")]
    Directory,
    #[serde(rename = "symlink")]
    Symlink,
}

/// 文件树节点
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FileNode {
    pub name: String,
    pub path: String,
    pub size: u64,
    pub mtime: u64,
    pub ctime: u64,
    #[serde(rename = "type")]
    pub file_type: FileType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<FileNode>>,
}
