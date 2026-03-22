use upod_cli::{models::CreateSandboxReq, models::RunCommandReq, UpodClient};

#[tokio::test]
async fn test_client_creation() {
    let client = UpodClient::new("http://localhost:8080").unwrap();
    assert_eq!(client.base_url(), "http://localhost:8080");
}

#[tokio::test]
#[ignore = "requires running upod service"]
async fn test_sandbox_lifecycle() {
    let client = UpodClient::new("http://localhost:8080").expect("Failed to create client");
    
    // 1. 创建沙箱
    println!("=== 1. Creating sandbox ===");
    let req = CreateSandboxReq {
        sandbox_id: None,
        image: upod_cli::models::Image { uri: "alpine:latest".to_string() },
        entrypoint: None,
        timeout: None,
        resource_limits: Some(upod_cli::models::ResourceLimits {
            cpu: None,
            memory: Some("512Mi".to_string()),
        }),
        env: None,
        metadata: None,
    };
    
    let sandbox = client.create_sandbox(req).await.expect("Failed to create sandbox");
    let sandbox_id = sandbox.id().to_string();
    println!("Successfully created sandbox with ID: {}", sandbox_id);
    
    // 2. 获取沙箱列表并验证
    println!("=== 2. Listing sandboxes ===");
    let sandboxes = client.list_sandboxes().await.expect("Failed to list sandboxes");
    println!("Current active sandboxes count: {}", sandboxes.len());
    assert!(sandboxes.iter().any(|s| s.id == sandbox_id));
    
    // 3. 暂停和恢复
    println!("=== 3. Pausing and resuming sandbox ===");
    sandbox.pause().await.expect("Failed to pause sandbox");
    println!("Sandbox paused.");
    
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    
    sandbox.resume().await.expect("Failed to resume sandbox");
    println!("Sandbox resumed.");
    
    // 5. 执行命令测试
    println!("=== 5. Running command ===");
    let cmd_req = RunCommandReq {
        command: "echo 'Hello from upod sandbox!' && sleep 1 && echo 'Done.'".to_string(),
        cwd: None,
        background: Some(false),
        timeout: None,
    };
    
    let handlers = upod_cli::command::ExecutionHandlers {
        on_stdout: Some(Box::new(|text| {
            println!("Received event: {}", text);
        })),
        on_execution_complete: Some(Box::new(|_| {
            println!("Command execution completed.");
        })),
        ..Default::default()
    };
    
    match sandbox.run_command(cmd_req, handlers).await {
        Ok(_) => {
            println!("Command execution started, waiting for events...");
        },
        Err(e) => println!("Failed to start command: {}", e),
    }

    // 6. 删除沙箱
    println!("=== 6. Deleting sandbox ===");
    sandbox.delete().await.expect("Failed to delete sandbox");
    println!("Sandbox deleted successfully.");
}
