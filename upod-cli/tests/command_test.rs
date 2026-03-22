use std::sync::Arc;
use upod_cli::{
    models::{CreateSandboxReq, RunCommandReq},
    UpodClient, SandboxHandle,
};

/// 辅助函数：创建测试沙箱
async fn setup_test_sandbox() -> (Arc<UpodClient>, SandboxHandle) {
    let client = UpodClient::new("http://localhost:8080").expect("Failed to create client");

    let req = CreateSandboxReq {
        sandbox_id: Some("test-sandbox".to_string()),
        image: upod_cli::models::Image { uri: "alpine:latest".to_string() },
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

/// 测试使用回调处理器执行命令
#[tokio::test]
#[ignore = "requires running upod service"]
async fn test_run_command_handlers() {
    let (_client, sandbox) = setup_test_sandbox().await;
    let sandbox_id = sandbox.id().to_string();
    println!("=== [Test] Running command with handlers in sandbox {} ===", sandbox_id);

    let handlers = upod_cli::command::ExecutionHandlers {
        on_stdout: Some(Box::new(|text| {
            println!("STDOUT: {}", text);
        })),
        on_stderr: Some(Box::new(|text| {
            println!("STDERR: {}", text);
        })),
        on_execution_complete: Some(Box::new(|time_ms| {
            println!("Execution completed in {} ms", time_ms);
        })),
        ..Default::default()
    };

    match sandbox.run_command(
        RunCommandReq {
            command: "echo 'Hello Handlers!' && sleep 1 && echo 'Handlers finished.'".to_string(),
            cwd: None,
            background: Some(false),
            timeout: None,
        },
        handlers,
    ).await {
        Ok(_) => println!("Command executed successfully with handlers."),
        Err(e) => panic!("Failed to run command with handlers: {}", e),
    }

    teardown_test_sandbox(sandbox).await;
}

/// 测试后台执行命令并主动中断
#[tokio::test]
#[ignore = "requires running upod service"]
async fn test_run_command_background_and_interrupt() {
    let (_client, sandbox) = setup_test_sandbox().await;
    let sandbox_id = sandbox.id().to_string();
    println!("=== [Test] Running background command and interrupting it in sandbox {} ===", sandbox_id);

    let bg_cmd_req = RunCommandReq {
        command: "echo 'Start long task' && sleep 10 && echo 'Should not reach here'".to_string(),
        cwd: None,
        background: Some(true),
        timeout: None,
    };

    let bg_command_id = std::sync::Arc::new(tokio::sync::Mutex::new(String::new()));
    let bg_command_id_clone = bg_command_id.clone();

    let handlers = upod_cli::command::ExecutionHandlers {
        on_session_init: Some(Box::new(move |id| {
            println!("Background command initiated. Extracted command ID: {}", id);
            if let Ok(mut guard) = bg_command_id_clone.try_lock() {
                *guard = id;
            }
        })),
        ..Default::default()
    };

    match sandbox.run_command(bg_cmd_req, handlers).await {
        Ok(_) => {}
        Err(e) => panic!("Failed to start background command: {}", e),
    }

    let extracted_id = bg_command_id.lock().await.clone();
    assert!(!extracted_id.is_empty(), "Should extract a command ID");

    // 短暂等待让后台任务运行一会
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    println!("Interrupting command ID: {}", extracted_id);
    match sandbox.interrupt_command(&extracted_id).await {
        Ok(_) => println!("Successfully sent interrupt signal to command."),
        Err(e) => panic!("Failed to interrupt command: {}", e),
    }

    teardown_test_sandbox(sandbox).await;
}

/// 测试查询命令状态和获取增量日志
#[tokio::test]
#[ignore = "requires running upod service"]
async fn test_run_command_status_and_logs() {
    let (_client, sandbox) = setup_test_sandbox().await;
    let sandbox_id = sandbox.id().to_string();
    println!("=== [Test] Checking command status and getting logs in sandbox {} ===", sandbox_id);

    let bg_cmd_req = RunCommandReq {
        command: "echo 'Start task for logs' && sleep 2 && echo 'Task completed'".to_string(),
        cwd: None,
        background: Some(true),
        timeout: None,
    };

    let bg_command_id = std::sync::Arc::new(tokio::sync::Mutex::new(String::new()));
    let bg_command_id_clone = bg_command_id.clone();

    let handlers = upod_cli::command::ExecutionHandlers {
        on_session_init: Some(Box::new(move |id| {
            println!("Background command initiated. Extracted command ID: {}", id);
            if let Ok(mut guard) = bg_command_id_clone.try_lock() {
                *guard = id;
            }
        })),
        ..Default::default()
    };

    match sandbox.run_command(bg_cmd_req, handlers).await {
        Ok(_) => {}
        Err(e) => panic!("Failed to start background command: {}", e),
    }

    let extracted_id = bg_command_id.lock().await.clone();
    assert!(!extracted_id.is_empty(), "Should extract a command ID");

    // 等待命令执行一部分
    tokio::time::sleep(std::time::Duration::from_millis(2500)).await;
    
    println!("\n=== Checking command status ===");
    match sandbox.get_command_status(&extracted_id).await {
        Ok(status) => {
            println!("Command Status: {:?}", status);
            println!("Is running: {}", status.running);
            println!("Exit code: {:?}", status.exit_code);
        }
        Err(e) => panic!("Failed to get command status: {}", e),
    }

    println!("\n=== Getting command logs ===");
    match sandbox.get_command_output(&extracted_id, Some(0)).await {
        Ok((bytes, next_cursor)) => {
            let log_str = String::from_utf8_lossy(&bytes);
            println!("Command Log Output:\n{}", log_str);
            println!("Next Cursor for logs: {}", next_cursor);
            assert!(log_str.contains("Start task for logs"), "Logs should contain initial output");
        }
        Err(e) => panic!("Failed to get command logs: {}", e),
    }

    teardown_test_sandbox(sandbox).await;
}
