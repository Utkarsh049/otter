use axum_test::TestServer;
use otter::api::routes::build_router;
use otter::config::Settings;
use otter::api::models::response::SubmissionResponse;
use otter::api::models::request::SubmissionRequest;
use std::time::Duration;

fn get_test_settings() -> Settings {
    Settings {
        host: "0.0.0.0".to_string(),
        port: 8080,
        max_concurrent: 4,
        cpu_limit_ms: 5000,
        wall_limit_ms: 10000,
        memory_limit_mb: 128,
        max_output_bytes: 1048576,
        redis_url: None,
    }
}

#[tokio::test]
async fn test_health_endpoint() {
    let app = build_router(get_test_settings());
    let server = TestServer::new(app).unwrap();
    
    let response = server.get("/health").await;
    response.assert_status_ok();
    
    let body = response.json::<serde_json::Value>();
    assert_eq!(body["status"], "ok");
    assert_eq!(body["version"], "0.1.0");
}

#[tokio::test]
async fn test_languages_endpoint() {
    let app = build_router(get_test_settings());
    let server = TestServer::new(app).unwrap();
    
    let response = server.get("/languages").await;
    response.assert_status_ok();
    
    let body = response.json::<Vec<serde_json::Value>>();
    assert!(!body.is_empty());
    
    // Ensure all 4 languages are returned
    let ids: Vec<&str> = body.iter().map(|l| l["id"].as_str().unwrap()).collect();
    assert!(ids.contains(&"c"));
    assert!(ids.contains(&"cpp"));
    assert!(ids.contains(&"python"));
    assert!(ids.contains(&"javascript"));
}

#[tokio::test]
async fn test_submit_and_poll_happy_path() {
    let app = build_router(get_test_settings());
    let server = TestServer::new(app).unwrap();
    
    let request_payload = SubmissionRequest {
        language: "python".to_string(),
        source_code: "print('hello from integration test')".to_string(),
        stdin: "".to_string(),
        cpu_time_limit_ms: None,
        memory_limit_mb: None,
        wall_time_limit_ms: None,
    };
    
    let post_response = server.post("/submissions").json(&request_payload).await;
    post_response.assert_status_ok();
    
    let initial_res = post_response.json::<SubmissionResponse>();
    let token = initial_res.token;
    assert_eq!(initial_res.status.id, 1); // Queued
    
    // Poll the status
    let mut finished = false;
    for _ in 0..10 {
        let get_response = server.get(&format!("/submissions/{}", token)).await;
        get_response.assert_status_ok();
        
        let poll_res = get_response.json::<SubmissionResponse>();
        if poll_res.status.id == 3 { // Accepted
            assert_eq!(poll_res.stdout.unwrap(), "hello from integration test\n");
            assert_eq!(poll_res.exit_code.unwrap(), 0);
            finished = true;
            break;
        } else if poll_res.status.id > 3 {
            panic!("Unexpected failed status: {:?}", poll_res.status);
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    
    assert!(finished, "Submission did not finish in time");
}

#[tokio::test]
async fn test_unsupported_language() {
    let app = build_router(get_test_settings());
    let server = TestServer::new(app).unwrap();
    
    let request_payload = SubmissionRequest {
        language: "rust".to_string(),
        source_code: "fn main() {}".to_string(),
        stdin: "".to_string(),
        cpu_time_limit_ms: None,
        memory_limit_mb: None,
        wall_time_limit_ms: None,
    };
    
    let post_response = server.post("/submissions").json(&request_payload).await;
    post_response.assert_status_bad_request();
}

#[tokio::test]
async fn test_malformed_json() {
    let app = build_router(get_test_settings());
    let server = TestServer::new(app).unwrap();
    
    // Send schema-invalid JSON (language as integer) to trigger JSON parsing failure (400)
    let payload = serde_json::json!({
        "language": 12345,
        "source_code": "print('hello')"
    });
    
    let response = server.post("/submissions").json(&payload).await;
    response.assert_status_bad_request();
}

#[tokio::test]
async fn test_missing_token() {
    let app = build_router(get_test_settings());
    let server = TestServer::new(app).unwrap();
    
    let response = server.get("/submissions/non_existent_token_123").await;
    response.assert_status_not_found();
}

#[tokio::test]
async fn test_metrics_endpoint() {
    let app = build_router(get_test_settings());
    let server = TestServer::new(app).unwrap();

    // 1. Initial metrics should be 0 count, 0.0 error rate, 0.0 latency
    let response = server.get("/metrics").await;
    response.assert_status_ok();
    let body = response.json::<serde_json::Value>();
    assert_eq!(body["count"].as_u64().unwrap(), 0);
    assert_eq!(body["error_rate"].as_f64().unwrap(), 0.0);
    assert_eq!(body["avg_latency"].as_f64().unwrap(), 0.0);

    // 2. Submit a successful Python job
    let request_payload = SubmissionRequest {
        language: "python".to_string(),
        source_code: "print('metrics test')".to_string(),
        stdin: "".to_string(),
        cpu_time_limit_ms: None,
        memory_limit_mb: None,
        wall_time_limit_ms: None,
    };
    let post_response = server.post("/submissions").json(&request_payload).await;
    post_response.assert_status_ok();
    let token = post_response.json::<SubmissionResponse>().token;

    // Poll until Accepted
    let mut finished = false;
    for _ in 0..100 {
        let get_response = server.get(&format!("/submissions/{}", token)).await;
        get_response.assert_status_ok();
        let poll_res = get_response.json::<SubmissionResponse>();
        if poll_res.status.id == 3 {
            finished = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(finished, "Job did not finish");

    // 3. Verify metrics updated (count=1, error_rate=0.0)
    let response = server.get("/metrics").await;
    response.assert_status_ok();
    let body = response.json::<serde_json::Value>();
    assert_eq!(body["count"].as_u64().unwrap(), 1);
    assert_eq!(body["error_rate"].as_f64().unwrap(), 0.0);
    assert!(body["avg_latency"].as_f64().unwrap() >= 0.0);

    // 4. Submit a compile error job
    let request_payload_err = SubmissionRequest {
        language: "c".to_string(),
        source_code: "invalid compile code here".to_string(),
        stdin: "".to_string(),
        cpu_time_limit_ms: None,
        memory_limit_mb: None,
        wall_time_limit_ms: None,
    };
    let post_response = server.post("/submissions").json(&request_payload_err).await;
    post_response.assert_status_ok();
    let token_err = post_response.json::<SubmissionResponse>().token;

    // Poll until CompilationError (status.id > 3)
    let mut finished_err = false;
    for _ in 0..100 {
        let get_response = server.get(&format!("/submissions/{}", token_err)).await;
        get_response.assert_status_ok();
        let poll_res = get_response.json::<SubmissionResponse>();
        if poll_res.status.id > 3 {
            finished_err = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(finished_err, "Err job did not finish");

    // 5. Verify metrics count is 2, error rate is 0.5
    let response = server.get("/metrics").await;
    response.assert_status_ok();
    let body = response.json::<serde_json::Value>();
    assert_eq!(body["count"].as_u64().unwrap(), 2);
    assert_eq!(body["error_rate"].as_f64().unwrap(), 0.5);
}
