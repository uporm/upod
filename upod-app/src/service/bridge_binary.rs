use crate::core::code::Code;
use std::io::Read;
use std::path::{Path, PathBuf};
use tar::{Builder, Header};
use upod_base::web::error::WebError;

pub(crate) const BRIDGE_BINARY_CONTAINER_PATH: &str = "/opt/upod/bin/upod-bridge";

/// 确保宿主机可解析出可用的 upod-bridge 二进制。
pub fn ensure_bridge_binary_ready() -> Result<(), WebError> {
    resolve_bridge_binary_path().map(|_| ())
}

/// 解析 upod-bridge 在宿主机上的路径，并校验为 Linux ELF 文件。
pub(crate) fn resolve_bridge_binary_path() -> Result<PathBuf, WebError> {
    let current_exe = std::env::current_exe()
        .map_err(|error| bridge_runtime_error(format!("解析当前可执行文件路径失败: {error}")))?;
    let bin_dir = current_exe
        .parent()
        .ok_or_else(|| bridge_runtime_error("当前可执行文件没有父目录".to_string()))?;

    let mut candidates = Vec::new();
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    if let Some(repo_root) = manifest_dir.parent() {
        candidates.push(repo_root.join("target/release/upod-bridge"));
        candidates.push(repo_root.join("target/x86_64-unknown-linux-musl/release/upod-bridge"));
        candidates.push(repo_root.join("target/aarch64-unknown-linux-musl/release/upod-bridge"));
        candidates.push(repo_root.join("target/x86_64-unknown-linux-gnu/release/upod-bridge"));
        candidates.push(repo_root.join("target/aarch64-unknown-linux-gnu/release/upod-bridge"));
    }
    candidates.push(bin_dir.join("upod-bridge"));

    // 仅接受 ELF，可避免把当前平台上的 Mach-O 等格式打进 Linux 容器。
    if let Some(candidate) = candidates
        .iter()
        .find(|path| path.is_file() && is_elf_binary(path))
    {
        return Ok(candidate.clone());
    }

    let non_elf_files = candidates
        .iter()
        .filter(|path| path.is_file())
        .map(|path| path.display().to_string())
        .collect::<Vec<String>>()
        .join(", ");
    if !non_elf_files.is_empty() {
        return Err(bridge_runtime_error(format!(
            "找到 upod-bridge 文件，但不是 Linux ELF 格式: {non_elf_files}"
        )));
    }

    let checked_paths = candidates
        .into_iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<String>>()
        .join(", ");
    Err(bridge_runtime_error(format!(
        "未找到 upod-bridge 二进制，已检查路径: {checked_paths}"
    )))
}

/// 打包 upod-bridge 为 tar 字节流，供容器创建阶段直接解压使用。
pub(crate) fn build_bridge_archive(bridge_binary_host_path: &Path) -> Result<Vec<u8>, WebError> {
    let bridge_binary = std::fs::read(bridge_binary_host_path).map_err(|error| {
        bridge_runtime_error(format!(
            "读取 upod-bridge 二进制失败（{}）: {error}",
            bridge_binary_host_path.display()
        ))
    })?;
    let mut archive = Builder::new(Vec::<u8>::new());
    let bridge_binary_rel_path = BRIDGE_BINARY_CONTAINER_PATH.trim_start_matches('/');
    // tar 默认不会自动补齐父目录，这里手动补齐以保证解压路径稳定。
    append_parent_directories(&mut archive, bridge_binary_rel_path)?;

    let mut bridge_header = Header::new_gnu();
    bridge_header.set_size(bridge_binary.len() as u64);
    bridge_header.set_mode(0o755);
    bridge_header.set_cksum();
    archive
        .append_data(
            &mut bridge_header,
            bridge_binary_rel_path,
            bridge_binary.as_slice(),
        )
        .map_err(|error| bridge_runtime_error(format!("打包 upod-bridge 归档失败: {error}")))?;

    archive
        .into_inner()
        .map_err(|error| bridge_runtime_error(format!("完成 upod-bridge 归档失败: {error}")))
}

/// 将目标文件的所有父目录写入 tar，避免解压时目录缺失。
fn append_parent_directories(
    archive: &mut Builder<Vec<u8>>,
    file_rel_path: &str,
) -> Result<(), WebError> {
    let Some(parent) = Path::new(file_rel_path).parent() else {
        return Ok(());
    };
    let mut current = PathBuf::new();
    for component in parent.components() {
        current.push(component.as_os_str());
        let current = current.to_string_lossy();
        if !current.is_empty() {
            append_directory(archive, current.as_ref())?;
        }
    }
    Ok(())
}

/// 向 tar 中追加一个目录项，并设置可执行目录权限。
fn append_directory(archive: &mut Builder<Vec<u8>>, path: &str) -> Result<(), WebError> {
    let mut header = Header::new_gnu();
    header.set_entry_type(tar::EntryType::Directory);
    header.set_mode(0o755);
    header.set_size(0);
    header.set_cksum();
    archive
        .append_data(&mut header, path, std::io::empty())
        .map_err(|error| bridge_runtime_error(format!("打包目录 {path} 失败: {error}")))
}

/// 构造统一的业务错误，便于上层输出一致的错误结构。
fn bridge_runtime_error(message: impl Into<String>) -> WebError {
    WebError::BizWithArgs(
        Code::SandboxCreateError.into(),
        vec![("error".to_string(), message.into())],
    )
}

/// 通过 ELF 魔数校验文件格式是否为 Linux 可执行文件。
fn is_elf_binary(path: &Path) -> bool {
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    let mut magic = [0u8; 4];
    file.read_exact(&mut magic).is_ok() && magic == [0x7f, b'E', b'L', b'F']
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    /// 验证归档包含 bridge 文件及其父目录层级。
    #[test]
    fn test_build_bridge_archive_contains_bridge_binary() {
        let bridge_path = std::env::temp_dir().join(format!(
            "upod-bridge-test-{}-{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::write(&bridge_path, b"bridge-binary").expect("write temp bridge binary");

        let archive = build_bridge_archive(&bridge_path).expect("build bridge archive");
        assert!(!archive.is_empty());
        let mut entries = tar::Archive::new(std::io::Cursor::new(archive));
        let paths = entries
            .entries()
            .expect("read archive entries")
            .map(|entry| {
                entry
                    .expect("read archive entry")
                    .path()
                    .expect("read archive path")
                    .to_string_lossy()
                    .to_string()
            })
            .collect::<Vec<_>>();
        assert!(paths.contains(&"opt".to_string()));
        assert!(paths.contains(&"opt/upod".to_string()));
        assert!(paths.contains(&"opt/upod/bin".to_string()));
        assert!(paths.contains(&"opt/upod/bin/upod-bridge".to_string()));

        let _ = std::fs::remove_file(bridge_path);
    }

    /// 验证 ELF 魔数检测在正反用例下都能正确工作。
    #[test]
    fn test_is_elf_binary() {
        let elf_path = std::env::temp_dir().join(format!(
            "upod-bridge-elf-test-{}-{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::write(&elf_path, [0x7f, b'E', b'L', b'F', 1, 2, 3, 4]).expect("write elf file");
        assert!(is_elf_binary(&elf_path));
        let _ = std::fs::remove_file(elf_path);

        let non_elf_path = std::env::temp_dir().join(format!(
            "upod-bridge-non-elf-test-{}-{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::write(&non_elf_path, b"MACH").expect("write non-elf file");
        assert!(!is_elf_binary(&non_elf_path));
        let _ = std::fs::remove_file(non_elf_path);
    }
}
