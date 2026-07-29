use axum_test::TestServer;
use otter::api::routes::build_router;
use otter::config::Settings;
use otter::api::models::response::SubmissionResponse;
use otter::api::models::request::SubmissionRequest;
use std::time::Duration;

fn get_test_settings() -> Settings {
    Settings {
        max_concurrent: 4,
        cpu_limit_ms: 5000,
        wall_limit_ms: 10000,
        memory_limit_mb: 128,
        max_output_bytes: 10240, // 10KB output cap for testing
        ..Settings::default()
    }
}

async fn run_attack(
    server: &TestServer,
    language: &str,
    source_path: &str,
    cpu_limit: Option<u64>,
    memory_limit: Option<u64>,
    wall_limit: Option<u64>,
) -> SubmissionResponse {
    let source_code = std::fs::read_to_string(source_path)
        .unwrap_or_else(|e| panic!("Failed to read attack program at {}: {}", source_path, e));
    
    let request_payload = SubmissionRequest {
        language: language.to_string(),
        source_code,
        stdin: "".to_string(),
        cpu_time_limit_ms: cpu_limit,
        memory_limit_mb: memory_limit,
        wall_time_limit_ms: wall_limit,
    };
    
    let post_response = server.post("/submissions").json(&request_payload).await;
    post_response.assert_status(axum::http::StatusCode::CREATED);
    
    let token = post_response.json::<SubmissionResponse>().token;
    
    // Poll until finished (status id > 2 means final state, 1=Queued, 2=Processing, 3=Accepted, >3 are limits/errors)
    for _ in 0..200 {
        let get_response = server.get(&format!("/submissions/{}", token)).await;
        get_response.assert_status_ok();
        let poll_res = get_response.json::<SubmissionResponse>();
        if poll_res.status.id > 2 {
            return poll_res;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("Attack program did not terminate within polling duration");
}

#[tokio::test]
async fn test_cpu_bombs() {
    let app = build_router(get_test_settings());
    let server = TestServer::new(app).unwrap();
    
    // 1. Python CPU bomb -> TimeLimitExceeded
    let res_py = run_attack(&server, "python", "tests/sandbox_attacks/programs/cpu_bomb.py", Some(500), None, Some(1000)).await;
    println!("DEBUG CPU BOMB PY: status={:?}, stdout={:?}, stderr={:?}, exit_code={:?}", res_py.status, res_py.stdout, res_py.stderr, res_py.exit_code);
    assert_eq!(res_py.status.description, "Time Limit Exceeded");
    
    // 2. JavaScript CPU bomb -> TimeLimitExceeded
    let res_js = run_attack(&server, "javascript", "tests/sandbox_attacks/programs/cpu_bomb.js", Some(500), None, Some(1000)).await;
    println!("DEBUG CPU BOMB JS: status={:?}, stdout={:?}, stderr={:?}, exit_code={:?}", res_js.status, res_js.stdout, res_js.stderr, res_js.exit_code);
    assert_eq!(res_js.status.description, "Time Limit Exceeded");
    
    // 3. C CPU bomb -> TimeLimitExceeded
    let res_c = run_attack(&server, "c", "tests/sandbox_attacks/programs/cpu_bomb.c", Some(500), None, Some(1000)).await;
    println!("DEBUG CPU BOMB C: status={:?}, stdout={:?}, stderr={:?}, exit_code={:?}", res_c.status, res_c.stdout, res_c.stderr, res_c.exit_code);
    assert_eq!(res_c.status.description, "Time Limit Exceeded");
}

#[tokio::test]
async fn test_memory_bombs() {
    let app = build_router(get_test_settings());
    let server = TestServer::new(app).unwrap();
    
    // 1. Python Memory bomb -> MemoryLimitExceeded
    // Set a tight limit of 32MB to trigger OOM quickly
    let res_py = run_attack(&server, "python", "tests/sandbox_attacks/programs/memory_bomb.py", None, Some(32), Some(5000)).await;
    println!("DEBUG MEM BOMB PY: status={:?}, stdout={:?}, stderr={:?}, exit_code={:?}, memory_kb={:?}", res_py.status, res_py.stdout, res_py.stderr, res_py.exit_code, res_py.memory_kb);
    assert_eq!(res_py.status.description, "Memory Limit Exceeded");
    
    // 2. JavaScript Memory bomb -> MemoryLimitExceeded
    let res_js = run_attack(&server, "javascript", "tests/sandbox_attacks/programs/memory_bomb.js", None, Some(64), Some(5000)).await;
    println!("DEBUG MEM BOMB JS: status={:?}, stdout={:?}, stderr={:?}, exit_code={:?}, memory_kb={:?}", res_js.status, res_js.stdout, res_js.stderr, res_js.exit_code, res_js.memory_kb);
    assert_eq!(res_js.status.description, "Memory Limit Exceeded");
}

#[tokio::test]
async fn test_fork_bomb() {
    let app = build_router(get_test_settings());
    let server = TestServer::new(app).unwrap();
    
    // C Fork bomb -> contained by RLIMIT_NPROC, finishes as RuntimeError/exit
    let res = run_attack(&server, "c", "tests/sandbox_attacks/programs/fork_bomb.c", None, None, None).await;
    println!("DEBUG FORK BOMB: status={:?}, stdout={:?}, stderr={:?}, exit_code={:?}", res.status, res.stdout, res.stderr, res.exit_code);
    // Should be terminated/blocked and server remains completely responsive
    assert!(res.status.description == "Runtime Error" || res.status.description == "Time Limit Exceeded");
}

#[tokio::test]
async fn test_network_attempts() {
    let app = build_router(get_test_settings());
    let server = TestServer::new(app).unwrap();
    
    // 1. Python network connect -> blocked and prints FAILED
    let res_py = run_attack(&server, "python", "tests/sandbox_attacks/programs/network_attempt.py", None, None, None).await;
    println!("DEBUG NET PY: status={:?}, stdout={:?}, stderr={:?}, exit_code={:?}", res_py.status, res_py.stdout, res_py.stderr, res_py.exit_code);
    assert_eq!(res_py.status.description, "Accepted");
    assert!(res_py.stdout.unwrap().contains("FAILED"));
    
    // 2. JavaScript network connect -> blocked and prints FAILED
    let res_js = run_attack(&server, "javascript", "tests/sandbox_attacks/programs/network_attempt.js", None, None, None).await;
    println!("DEBUG NET JS: status={:?}, stdout={:?}, stderr={:?}, exit_code={:?}", res_js.status, res_js.stdout, res_js.stderr, res_js.exit_code);
    assert_eq!(res_js.status.description, "Accepted");
    assert!(res_js.stdout.unwrap().contains("FAILED"));
}

#[tokio::test]
async fn test_file_escapes() {
    let app = build_router(get_test_settings());
    let server = TestServer::new(app).unwrap();
    
    // 1. Python file escape -> fails to read /etc/passwd
    let res_py = run_attack(&server, "python", "tests/sandbox_attacks/programs/file_escape.py", None, None, None).await;
    assert_eq!(res_py.status.description, "Accepted");
    assert!(res_py.stdout.unwrap().contains("ESCAPE_FAILED"));
    
    // 2. C file read -> fails to read /etc/passwd
    let res_c = run_attack(&server, "c", "tests/sandbox_attacks/programs/file_read.c", None, None, None).await;
    assert_eq!(res_c.status.description, "Accepted");
    assert!(res_c.stdout.unwrap().contains("READ_FAILED"));
}

#[tokio::test]
async fn test_output_flood() {
    let app = build_router(get_test_settings());
    let server = TestServer::new(app).unwrap();
    
    // Python output flood -> capped at 10KB (10240 bytes)
    let res = run_attack(&server, "python", "tests/sandbox_attacks/programs/output_flood.py", None, None, Some(1000)).await;
    assert!(res.status.description == "Accepted" || res.status.description == "Time Limit Exceeded");
    let stdout = res.stdout.unwrap();
    assert!(stdout.len() <= 10240);
}

#[tokio::test]
async fn test_disk_fill() {
    let app = build_router(get_test_settings());
    let server = TestServer::new(app).unwrap();
    
    // C disk fill -> blocked by RLIMIT_FSIZE and returns RuntimeError / terminated by SIGXFSZ
    let res = run_attack(&server, "c", "tests/sandbox_attacks/programs/disk_fill.c", None, None, None).await;
    println!("DEBUG DISK FILL: status={:?}, stdout={:?}, stderr={:?}, exit_code={:?}", res.status, res.stdout, res.stderr, res.exit_code);
    assert_eq!(res.status.description, "Runtime Error");
}
