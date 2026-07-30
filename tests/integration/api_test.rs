use axum_test::TestServer;
use otter::api::routes::build_router;
use otter::config::Settings;
use otter::api::models::response::SubmissionResponse;
use otter::api::models::request::SubmissionRequest;
use std::time::Duration;
use std::sync::Arc;

fn get_test_settings() -> Settings {
    Settings {
        max_concurrent: 4,
        allow_loopback_webhooks: true,
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
        webhook_url: None,
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
            panic!("Unexpected failed status: {:?}, stdout: {:?}, stderr: {:?}, exit_code: {:?}", poll_res.status, poll_res.stdout, poll_res.stderr, poll_res.exit_code);
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
        webhook_url: None,
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
    let response = server.get("/admin/metrics").await;
    response.assert_status_ok();
    let body = response.json::<serde_json::Value>();
    assert_eq!(body["submissions"]["count"].as_u64().unwrap(), 0);
    assert_eq!(body["submissions"]["error_rate"].as_f64().unwrap(), 0.0);
    assert_eq!(body["submissions"]["avg_latency_ms"].as_f64().unwrap(), 0.0);
    assert_eq!(body["status_breakdown"]["accepted"].as_u64().unwrap(), 0);
    assert_eq!(body["languages"]["python"].as_u64().unwrap(), 0);

    // 2. Submit a successful Python job
    let request_payload = SubmissionRequest {
        language: "python".to_string(),
        source_code: "print('metrics test')".to_string(),
        stdin: "".to_string(),
        cpu_time_limit_ms: None,
        memory_limit_mb: None,
        wall_time_limit_ms: None,
        webhook_url: None,
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
        } else if poll_res.status.id > 3 {
            panic!("Metrics test job failed with status: {:?}, stdout: {:?}, stderr: {:?}, exit_code: {:?}", poll_res.status, poll_res.stdout, poll_res.stderr, poll_res.exit_code);
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(finished, "Job did not finish");

    // 3. Verify metrics updated (count=1, error_rate=0.0)
    let response = server.get("/admin/metrics").await;
    response.assert_status_ok();
    let body = response.json::<serde_json::Value>();
    assert_eq!(body["submissions"]["count"].as_u64().unwrap(), 1);
    assert_eq!(body["submissions"]["error_rate"].as_f64().unwrap(), 0.0);
    assert!(body["submissions"]["avg_latency_ms"].as_f64().unwrap() >= 0.0);
    assert_eq!(body["status_breakdown"]["accepted"].as_u64().unwrap(), 1);
    assert_eq!(body["languages"]["python"].as_u64().unwrap(), 1);
    assert_eq!(body["queue"]["depth"].as_u64().unwrap(), 0);

    // 4. Submit a compile error job
    let request_payload_err = SubmissionRequest {
        language: "c".to_string(),
        source_code: "invalid compile code here".to_string(),
        stdin: "".to_string(),
        cpu_time_limit_ms: None,
        memory_limit_mb: None,
        wall_time_limit_ms: None,
        webhook_url: None,
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
    let response = server.get("/admin/metrics").await;
    response.assert_status_ok();
    let body = response.json::<serde_json::Value>();
    assert_eq!(body["submissions"]["count"].as_u64().unwrap(), 2);
    assert_eq!(body["submissions"]["error_rate"].as_f64().unwrap(), 0.5);
    assert_eq!(body["status_breakdown"]["compilation_error"].as_u64().unwrap(), 1);
    assert_eq!(body["languages"]["c"].as_u64().unwrap(), 1);

    // 6. Test authorization for /admin/metrics when API key is set
    let mut authed_settings = get_test_settings();
    authed_settings.otter_api_key = Some("admin-secret-key".to_string());
    let authed_app = build_router(authed_settings);
    let authed_server = TestServer::new(authed_app).unwrap();

    // 6a. Request without auth -> 401 Unauthorized
    let response = authed_server.get("/admin/metrics").await;
    response.assert_status(axum::http::StatusCode::UNAUTHORIZED);

    // 6b. Request with bad auth -> 401 Unauthorized
    let response = authed_server.get("/admin/metrics")
        .add_header(
            axum::http::HeaderName::from_static("authorization"),
            axum::http::HeaderValue::from_static("Bearer bad-key")
        )
        .await;
    response.assert_status(axum::http::StatusCode::UNAUTHORIZED);

    // 6c. Request with correct auth -> 200 OK
    let response = authed_server.get("/admin/metrics")
        .add_header(
            axum::http::HeaderName::from_static("authorization"),
            axum::http::HeaderValue::from_static("Bearer admin-secret-key")
        )
        .await;
    response.assert_status_ok();
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
                webhook_url: None,
            },
            SubmissionRequest {
                language: "javascript".to_string(),
                source_code: "console.log('batch 2');".to_string(),
                stdin: "".to_string(),
                cpu_time_limit_ms: None,
                memory_limit_mb: None,
                wall_time_limit_ms: None,
                webhook_url: None,
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
        } else if poll.status.id > 3 {
            panic!("Batch 2 failed with status: {:?}, stdout: {:?}, stderr: {:?}, exit_code: {:?}", poll.status, poll.stdout, poll.stderr, poll.exit_code);
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(finished2, "Batch 2 did not finish");
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
        webhook_url: None,
};
    
    let response = server.post("/submissions").json(&request_payload).await;
    response.assert_status_bad_request();
    let body = response.json::<serde_json::Value>();
    assert!(body["error"].as_str().unwrap().contains("cannot exceed server limit"));
}

#[tokio::test]
async fn test_queue_capacity_limit() {
    let mut settings = get_test_settings();
    settings.max_queue_depth = 2; // Very tight queue limit

    let app = build_router(settings);
    let server = TestServer::new(app).unwrap();

    let request_payload = SubmissionRequest {
        language: "python".to_string(),
        source_code: "import time\ntime.sleep(1.0)".to_string(),
        stdin: "".to_string(),
        cpu_time_limit_ms: None,
        memory_limit_mb: None,
        wall_time_limit_ms: None,
        webhook_url: None,
};

    // Submit 1st -> ok
    let response1 = server.post("/submissions").json(&request_payload).await;
    response1.assert_status(axum::http::StatusCode::CREATED);

    // Submit 2nd -> ok
    let response2 = server.post("/submissions").json(&request_payload).await;
    response2.assert_status(axum::http::StatusCode::CREATED);

    // Submit 3rd -> 429 Too Many Requests (since capacity is 2)
    let response3 = server.post("/submissions").json(&request_payload).await;
    response3.assert_status(axum::http::StatusCode::TOO_MANY_REQUESTS);
    let body = response3.json::<serde_json::Value>();
    assert!(body["error"].as_str().unwrap().contains("server is at capacity"));
}

#[tokio::test]
async fn test_disable_sandbox_mode() {
    let mut settings = get_test_settings();
    settings.disable_sandbox = true; // explicitly disable sandbox

    let app = build_router(settings);
    let server = TestServer::new(app).unwrap();

    let request_payload = SubmissionRequest {
        language: "python".to_string(),
        source_code: "print('running un-jailed raw mode!')".to_string(),
        stdin: "".to_string(),
        cpu_time_limit_ms: None,
        memory_limit_mb: None,
        wall_time_limit_ms: None,
        webhook_url: None,
};

    let response = server.post("/submissions").json(&request_payload).await;
    response.assert_status(axum::http::StatusCode::CREATED);
    let token = response.json::<SubmissionResponse>().token;

    // Poll until Accepted
    let mut finished = false;
    for _ in 0..100 {
        let get_response = server.get(&format!("/submissions/{}", token)).await;
        get_response.assert_status_ok();
        let poll_res = get_response.json::<SubmissionResponse>();
        if poll_res.status.id == 3 { // Accepted
            assert_eq!(poll_res.stdout.unwrap(), "running un-jailed raw mode!\n");
            finished = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(finished, "Un-jailed execution did not complete");
}

#[tokio::test]
async fn test_api_key_authentication() {
    let mut settings = get_test_settings();
    settings.otter_api_key = Some("test-secret-api-key".to_string());

    let app = build_router(settings);
    let server = TestServer::new(app).unwrap();

    let request_payload = SubmissionRequest {
        language: "python".to_string(),
        source_code: "print('authenticated!')".to_string(),
        stdin: "".to_string(),
        cpu_time_limit_ms: None,
        memory_limit_mb: None,
        wall_time_limit_ms: None,
        webhook_url: None,
};

    // 1. Request without auth header -> 401 Unauthorized
    let response_no_auth = server.post("/submissions").json(&request_payload).await;
    response_no_auth.assert_status(axum::http::StatusCode::UNAUTHORIZED);

    // 2. Request with malformed header -> 401 Unauthorized
    let response_bad_auth = server.post("/submissions")
        .add_header(
            axum::http::HeaderName::from_static("authorization"),
            axum::http::HeaderValue::from_static("Bearer wrong-key")
        )
        .json(&request_payload)
        .await;
    response_bad_auth.assert_status(axum::http::StatusCode::UNAUTHORIZED);

    // 3. Request with valid header -> 201 Created
    let response_valid_auth = server.post("/submissions")
        .add_header(
            axum::http::HeaderName::from_static("authorization"),
            axum::http::HeaderValue::from_static("Bearer test-secret-api-key")
        )
        .json(&request_payload)
        .await;
    response_valid_auth.assert_status(axum::http::StatusCode::CREATED);

    // 4. Request to public /health without auth -> 200 OK
    let response_health = server.get("/health").await;
    response_health.assert_status_ok();
}

#[tokio::test]
async fn test_api_key_rate_limiting() {
    let mut settings = get_test_settings();
    settings.rate_limit_requests = Some(2);
    settings.rate_limit_window_seconds = Some(60);
    settings.otter_api_key = Some("key-a,key-b".to_string());

    let app = build_router(settings);
    let server = TestServer::new(app).unwrap();

    let request_payload = SubmissionRequest {
        language: "python".to_string(),
        source_code: "print('auth rate limit test')".to_string(),
        stdin: "".to_string(),
        cpu_time_limit_ms: None,
        memory_limit_mb: None,
        wall_time_limit_ms: None,
        webhook_url: None,
};

    // 1. First request for key-a -> 201 Created
    let response = server.post("/submissions")
        .add_header(
            axum::http::HeaderName::from_static("authorization"),
            axum::http::HeaderValue::from_static("Bearer key-a")
        )
        .json(&request_payload)
        .await;
    response.assert_status(axum::http::StatusCode::CREATED);

    // 2. Second request for key-a -> 201 Created
    let response = server.post("/submissions")
        .add_header(
            axum::http::HeaderName::from_static("authorization"),
            axum::http::HeaderValue::from_static("Bearer key-a")
        )
        .json(&request_payload)
        .await;
    response.assert_status(axum::http::StatusCode::CREATED);

    // 3. Third request for key-a -> 429 Too Many Requests
    let response = server.post("/submissions")
        .add_header(
            axum::http::HeaderName::from_static("authorization"),
            axum::http::HeaderValue::from_static("Bearer key-a")
        )
        .json(&request_payload)
        .await;
    response.assert_status(axum::http::StatusCode::TOO_MANY_REQUESTS);

    // 4. Request for key-b (from same test client/IP) -> 201 Created
    let response = server.post("/submissions")
        .add_header(
            axum::http::HeaderName::from_static("authorization"),
            axum::http::HeaderValue::from_static("Bearer key-b")
        )
        .json(&request_payload)
        .await;
    response.assert_status(axum::http::StatusCode::CREATED);
}

#[tokio::test]
async fn test_list_submissions() {
    let app = build_router(get_test_settings());
    let server = TestServer::new(app).unwrap();

    // 1. Submit two jobs
    let payload1 = SubmissionRequest {
        language: "python".to_string(),
        source_code: "print('job 1')".to_string(),
        stdin: "".to_string(),
        cpu_time_limit_ms: None,
        memory_limit_mb: None,
        wall_time_limit_ms: None,
        webhook_url: None,
};
    let response1 = server.post("/submissions").json(&payload1).await;
    response1.assert_status(axum::http::StatusCode::CREATED);
    let token1 = response1.json::<SubmissionResponse>().token;

    let payload2 = SubmissionRequest {
        language: "python".to_string(),
        source_code: "print('job 2')".to_string(),
        stdin: "".to_string(),
        cpu_time_limit_ms: None,
        memory_limit_mb: None,
        wall_time_limit_ms: None,
        webhook_url: None,
};
    let response2 = server.post("/submissions").json(&payload2).await;
    response2.assert_status(axum::http::StatusCode::CREATED);
    let token2 = response2.json::<SubmissionResponse>().token;

    // 2. Query list submissions
    let response = server.get("/submissions").await;
    response.assert_status_ok();
    let list = response.json::<Vec<SubmissionResponse>>();

    // Verify both submissions exist in the list
    let tokens: Vec<String> = list.iter().map(|s| s.token.clone()).collect();
    assert!(tokens.contains(&token1));
    assert!(tokens.contains(&token2));
}

#[tokio::test]
async fn test_webhook_happy_path() {
    let received_body = Arc::new(tokio::sync::Mutex::new(None));
    let received_body_clone = received_body.clone();
    
    let mock_app = axum::Router::new().route("/callback", axum::routing::post(move |axum::Json(payload): axum::Json<serde_json::Value>| {
        let received_body = received_body_clone.clone();
        async move {
            let mut guard = received_body.lock().await;
            *guard = Some(payload);
            axum::http::StatusCode::OK
        }
    }));
    
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    
    tokio::spawn(async move {
        axum::serve(listener, mock_app).await.unwrap();
    });
    
    // 2. Submit a job to Otter specifying this webhook
    let app = build_router(get_test_settings());
    let server = TestServer::new(app).unwrap();
    
    let webhook_url = format!("http://127.0.0.1:{}/callback", port);
    let request_payload = SubmissionRequest {
        language: "python".to_string(),
        source_code: "print('webhook works!')".to_string(),
        stdin: "".to_string(),
        cpu_time_limit_ms: None,
        memory_limit_mb: None,
        wall_time_limit_ms: None,
        webhook_url: Some(webhook_url),
    };
    
    let response = server.post("/submissions").json(&request_payload).await;
    response.assert_status(axum::http::StatusCode::CREATED);
    let token = response.json::<SubmissionResponse>().token;
    
    // 3. Wait for the webhook to be received
    let mut received = None;
    for _ in 0..100 {
        tokio::time::sleep(Duration::from_millis(50)).await;
        let guard = received_body.lock().await;
        if guard.is_some() {
            received = guard.clone();
            break;
        }
    }
    
    let received = received.expect("Webhook was not received");
    assert_eq!(received["token"].as_str().unwrap(), token);
    assert_eq!(received["status"]["id"].as_u64().unwrap(), 3); // Accepted
    assert_eq!(received["stdout"].as_str().unwrap(), "webhook works!\n");
}

#[tokio::test]
async fn test_webhook_ssrf_prevention() {
    use tokio::net::TcpListener;
    use tokio::io::AsyncWriteExt;

    // Try to submit with a loopback webhook URL
    let mut settings = get_test_settings();
    settings.allow_loopback_webhooks = false;
    let app = build_router(settings);
    let server = TestServer::new(app).unwrap();
    
    // 127.0.0.1 is in blocklist. The endpoint resolves DNS to 127.0.0.1, detects blocklist, and aborts.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    
    let received_flag = Arc::new(tokio::sync::Mutex::new(false));
    let received_flag_clone = received_flag.clone();
    
    tokio::spawn(async move {
        if let Ok((mut stream, _)) = listener.accept().await {
            let mut guard = received_flag_clone.lock().await;
            *guard = true;
            let _ = stream.write_all(b"HTTP/1.1 200 OK\r\n\r\n").await;
        }
    });
    
    let webhook_url = format!("http://127.0.0.1:{}/callback", port);
    let request_payload = SubmissionRequest {
        language: "python".to_string(),
        source_code: "print('ssrf check')".to_string(),
        stdin: "".to_string(),
        cpu_time_limit_ms: None,
        memory_limit_mb: None,
        wall_time_limit_ms: None,
        webhook_url: Some(webhook_url),
    };
    
    let response = server.post("/submissions").json(&request_payload).await;
    response.assert_status(axum::http::StatusCode::CREATED);
    let token = response.json::<SubmissionResponse>().token;
    
    // Wait for job to finish processing
    let mut finished = false;
    for _ in 0..100 {
        let res = server.get(&format!("/submissions/{}", token)).await;
        let poll = res.json::<SubmissionResponse>();
        if poll.status.id >= 3 {
            finished = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(finished);
    
    // Wait another 200ms to be absolutely sure no webhook was sent
    tokio::time::sleep(Duration::from_millis(200)).await;
    
    // Assert that the listener NEVER accepted a connection (flag remains false)
    let flag = *received_flag.lock().await;
    assert!(!flag, "Webhook request was sent to blocklisted IP!");
}
