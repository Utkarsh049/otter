use axum_test::TestServer;
use otter::api::routes::build_router;
use otter::config::Settings;
use otter::api::models::response::SubmissionResponse;
use otter::api::models::request::SubmissionRequest;
use std::time::Duration;

fn get_test_settings() -> Settings {
    Settings {
        max_concurrent: 4,
        ..Settings::default()
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
    post_response.assert_status(axum::http::StatusCode::CREATED);
    
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
    post_response.assert_status(axum::http::StatusCode::CREATED);
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
    post_response.assert_status(axum::http::StatusCode::CREATED);
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

#[tokio::test]
async fn test_batch_submissions() {
    let app = build_router(get_test_settings());
    let server = TestServer::new(app).unwrap();

    let request_payload = otter::api::models::request::BatchSubmissionRequest {
        submissions: vec![
            SubmissionRequest {
                language: "python".to_string(),
                source_code: "print('batch 1')".to_string(),
                stdin: "".to_string(),
                cpu_time_limit_ms: None,
                memory_limit_mb: None,
                wall_time_limit_ms: None,
            },
            SubmissionRequest {
                language: "javascript".to_string(),
                source_code: "console.log('batch 2');".to_string(),
                stdin: "".to_string(),
                cpu_time_limit_ms: None,
                memory_limit_mb: None,
                wall_time_limit_ms: None,
            },
        ],
    };

    let post_response = server.post("/submissions/batch").json(&request_payload).await;
    post_response.assert_status(axum::http::StatusCode::CREATED);

    let batch_res = post_response.json::<otter::api::models::response::BatchSubmissionResponse>();
    assert_eq!(batch_res.submissions.len(), 2);

    let token1 = &batch_res.submissions[0].token;
    let token2 = &batch_res.submissions[1].token;

    // Poll first
    let mut finished1 = false;
    for _ in 0..100 {
        let get_res = server.get(&format!("/submissions/{}", token1)).await;
        get_res.assert_status_ok();
        let poll = get_res.json::<SubmissionResponse>();
        if poll.status.id == 3 {
            assert_eq!(poll.stdout.unwrap(), "batch 1\n");
            finished1 = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(finished1);

    // Poll second
    let mut finished2 = false;
    for _ in 0..100 {
        let get_res = server.get(&format!("/submissions/{}", token2)).await;
        get_res.assert_status_ok();
        let poll = get_res.json::<SubmissionResponse>();
        if poll.status.id == 3 {
            assert_eq!(poll.stdout.unwrap(), "batch 2\n");
            finished2 = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(finished2);
}

#[tokio::test]
async fn test_rate_limiting() {
    // Configure settings with rate limit of 2 requests per 60 seconds
    let mut settings = get_test_settings();
    settings.rate_limit_requests = Some(2);
    settings.rate_limit_window_seconds = Some(60);

    let app = build_router(settings);
    let server = TestServer::new(app).unwrap();

    // 1st request -> ok
    let response = server.get("/health").await;
    response.assert_status_ok();

    // 2nd request -> ok
    let response = server.get("/health").await;
    response.assert_status_ok();

    // 3rd request -> 429 Too Many Requests
    let response = server.get("/health").await;
    response.assert_status(axum::http::StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn test_exceeded_limit_validation() {
    let app = build_router(get_test_settings());
    let server = TestServer::new(app).unwrap();
    
    // CPU limit is 5000ms. Try to submit with 6000ms.
    let request_payload = SubmissionRequest {
        language: "python".to_string(),
        source_code: "print('limit test')".to_string(),
        stdin: "".to_string(),
        cpu_time_limit_ms: Some(6000),
        memory_limit_mb: None,
        wall_time_limit_ms: None,
    };
    
    let response = server.post("/submissions").json(&request_payload).await;
    response.assert_status_bad_request();
    let body = response.json::<serde_json::Value>();
    assert!(body["error"].as_str().unwrap().contains("cannot exceed server limit"));
}
