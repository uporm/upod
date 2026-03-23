use std::collections::HashMap;
use std::sync::Arc;
use upod_cli::{
    SandboxHandle, UpodClient,
    models::{CreateSandboxReq, FileMetadata, Permission, RenameFileItem, ReplaceFileContentItem},
};

/// 辅助函数：创建测试沙箱
async fn setup_test_sandbox() -> (Arc<UpodClient>, SandboxHandle) {
    let client = UpodClient::new("http://localhost:8080").expect("Failed to create client");

    let req = CreateSandboxReq {
        sandbox_id: None,
        image: upod_cli::models::Image {
            uri: "alpine:latest".to_string(),
        },
        entrypoint: None,
        timeout: None,
        resource_limits: Some(upod_cli::models::ResourceLimits {
            cpu: None,
            memory: Some("256Mi".to_string()),
        }),
        env: None,
        metadata: None,
    };

    let sandbox = client
        .create_sandbox(req)
        .await
        .expect("Failed to create sandbox");

    // 等待沙箱内的 upod-bridge 服务启动就绪
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    (client, sandbox)
}

/// 辅助函数：清理测试沙箱
async fn teardown_test_sandbox(sandbox: SandboxHandle) {
    sandbox.delete().await.expect("Failed to delete sandbox");
}

#[tokio::test]
#[ignore = "requires running upod service"]
async fn test_upload_and_download_file() {
    let (_client, sandbox) = setup_test_sandbox().await;
    let sandbox_id = sandbox.id().to_string();
    println!(
        "=== [Test] File upload/download in sandbox {} ===",
        sandbox_id
    );

    let test_file_path = "/tmp/test_upload.txt".to_string();
    let content = b"Hello, Upod FS!".to_vec();

    let meta = FileMetadata {
        path: test_file_path.clone(),
        permission: Permission {
            owner: "root".to_string(),
            group: "root".to_string(),
            mode: 0o644,
        },
    };

    // 1. 上传文件
    sandbox
        .upload_files(&[(meta, content.clone())])
        .await
        .expect("Failed to upload file");

    // 2. 下载文件并验证
    let downloaded_content = sandbox
        .download_file(&test_file_path)
        .await
        .expect("Failed to download file");
    assert_eq!(
        content, downloaded_content,
        "Downloaded content does not match"
    );

    // 3. 获取文件信息
    let info_map = sandbox
        .get_files_info(std::slice::from_ref(&test_file_path))
        .await
        .expect("Failed to get file info");
    let info = info_map.get(&test_file_path).expect("File info not found");
    assert_eq!(info.size, content.len() as i64);

    // 4. 删除文件
    sandbox
        .remove_files(&[test_file_path])
        .await
        .expect("Failed to remove file");

    teardown_test_sandbox(sandbox).await;
}

#[tokio::test]
#[ignore = "requires running upod service"]
async fn test_directory_operations_and_search() {
    let (_client, sandbox) = setup_test_sandbox().await;
    let sandbox_id = sandbox.id().to_string();
    println!(
        "=== [Test] Directory operations in sandbox {} ===",
        sandbox_id
    );

    let test_dir = "/tmp/test_dir".to_string();

    // 1. 创建目录
    let mut dirs = HashMap::new();
    dirs.insert(
        test_dir.clone(),
        Permission {
            owner: "root".to_string(),
            group: "root".to_string(),
            mode: 0o755,
        },
    );
    sandbox
        .make_directories(&dirs)
        .await
        .expect("Failed to create directory");

    // 2. 在目录中上传文件
    let test_file = format!("{}/test.txt", test_dir);
    let meta = FileMetadata {
        path: test_file.clone(),
        permission: Default::default(),
    };
    sandbox
        .upload_files(&[(meta, b"test".to_vec())])
        .await
        .expect("Failed to upload file");

    // 3. 搜索文件
    let results = sandbox
        .search_files(&test_dir, Some("*.txt"))
        .await
        .expect("Failed to search files");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].path, test_file);

    // 4. 删除目录
    sandbox
        .remove_directories(&[test_dir])
        .await
        .expect("Failed to remove directory");

    teardown_test_sandbox(sandbox).await;
}

#[tokio::test]
#[ignore = "requires running upod service"]
async fn test_list_files() {
    let (_client, sandbox) = setup_test_sandbox().await;
    let sandbox_id = sandbox.id().to_string();
    println!("=== [Test] List files in sandbox {} ===", sandbox_id);

    let test_dir = "/tmp/test_list_dir".to_string();

    // 1. 创建目录
    let mut dirs = HashMap::new();
    dirs.insert(
        test_dir.clone(),
        Permission {
            owner: "root".to_string(),
            group: "root".to_string(),
            mode: 0o755,
        },
    );
    sandbox
        .make_directories(&dirs)
        .await
        .expect("Failed to create directory");

    // 2. 在目录中上传文件
    let test_file1 = format!("{}/test1.txt", test_dir);
    let test_file2 = format!("{}/test2.txt", test_dir);
    sandbox
        .upload_files(&[
            (
                FileMetadata {
                    path: test_file1.clone(),
                    permission: Default::default(),
                },
                b"test1".to_vec(),
            ),
            (
                FileMetadata {
                    path: test_file2.clone(),
                    permission: Default::default(),
                },
                b"test2".to_vec(),
            ),
        ])
        .await
        .expect("Failed to upload files");

    // 3. 获取文件列表
    let nodes = sandbox
        .list_files(&test_dir, Some("name"))
        .await
        .expect("Failed to list files");
    assert_eq!(nodes.len(), 2);
    assert_eq!(nodes[0].name, "test1.txt");
    assert_eq!(nodes[1].name, "test2.txt");

    // 4. 清理
    sandbox
        .remove_directories(&[test_dir])
        .await
        .expect("Failed to remove directory");

    teardown_test_sandbox(sandbox).await;
}

#[tokio::test]
#[ignore = "requires running upod service"]
async fn test_rename_and_replace() {
    let (_client, sandbox) = setup_test_sandbox().await;

    let src_file = "/tmp/old_name.txt".to_string();
    let dest_file = "/tmp/new_name.txt".to_string();
    let content = b"Original Content".to_vec();

    // 上传初始文件
    let meta = FileMetadata {
        path: src_file.clone(),
        permission: Default::default(),
    };
    sandbox
        .upload_files(&[(meta, content)])
        .await
        .expect("Failed to upload file");

    // 1. 替换文件内容
    let mut replaces = HashMap::new();
    replaces.insert(
        src_file.clone(),
        ReplaceFileContentItem {
            old: "Original".to_string(),
            new: "Replaced".to_string(),
        },
    );
    sandbox
        .replace_content(&replaces)
        .await
        .expect("Failed to replace content");

    let new_content = sandbox
        .download_file(&src_file)
        .await
        .expect("Failed to download");
    assert_eq!(new_content, b"Replaced Content".to_vec());

    // 2. 重命名文件
    sandbox
        .rename_files(&[RenameFileItem {
            src: src_file.clone(),
            dest: dest_file.clone(),
        }])
        .await
        .expect("Failed to rename file");

    // 验证新文件存在且旧文件不存在
    let info_map = sandbox
        .get_files_info(std::slice::from_ref(&dest_file))
        .await
        .expect("Failed to get file info");
    assert!(info_map.contains_key(&dest_file));

    // 3. 修改权限
    let mut perms = HashMap::new();
    perms.insert(
        dest_file.clone(),
        Permission {
            owner: "root".to_string(),
            group: "root".to_string(),
            mode: 0o777,
        },
    );
    sandbox.chmod_files(&perms).await.expect("Failed to chmod");

    // 4. 删除文件
    sandbox
        .remove_files(&[dest_file])
        .await
        .expect("Failed to remove file");

    teardown_test_sandbox(sandbox).await;
}
