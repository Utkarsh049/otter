use axum_test::TestServer;
use otter::api::routes::build_router;
use otter::config::Settings;
use otter::api::models::response::SubmissionResponse;
use otter::api::models::request::SubmissionRequest;
use std::time::Duration;
use futures::future::join_all;

fn get_test_settings(max_concurrent: usize) -> Settings {
    Settings {
        host: "0.0.0.0".to_string(),
        port: 8080,
        max_concurrent,
        cpu_limit_ms: 5000,
        wall_limit_ms: 10000,
        memory_limit_mb: 128,
        max_output_bytes: 1048576,
        redis_url: None,
    }
}

#[tokio::test]
async fn test_twenty_simultaneous_submissions() {
    // 1. Build server with max_concurrent = 4
    let app = build_router(get_test_settings(4));
    let server = TestServer::new(app).unwrap();
    
    // 2. Build 20 futures in parallel
    let mut futures = Vec::new();
    for i in 0..20 {
        let server_ref = &server;
        futures.push(async move {
            let request_payload = SubmissionRequest {
                language: "python".to_string(),
                source_code: format!("print('concurrent task {}')", i),
                stdin: "".to_string(),
                cpu_time_limit_ms: None,
                memory_limit_mb: None,
                wall_time_limit_ms: None,
            };
            
            let response = server_ref.post("/submissions").json(&request_payload).await;
            response.assert_status_ok();
            
            let initial_res = response.json::<SubmissionResponse>();
            let token = initial_res.token;
            
            // Poll until Accepted
            let mut result_stdout = String::new();
            for _ in 0..50 {
                let get_response = server_ref.get(&format!("/submissions/{}", token)).await;
                get_response.assert_status_ok();
                let poll_res = get_response.json::<SubmissionResponse>();
                if poll_res.status.id == 3 { // Accepted
                    result_stdout = poll_res.stdout.unwrap();
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            assert_eq!(result_stdout, format!("concurrent task {}\n", i));
        });
    }
    
    // Await all futures concurrently
    join_all(futures).await;
}

#[tokio::test]
async fn test_bounded_concurrency_cap() {
    // 1. Build server with max_concurrent = 2
    let app = build_router(get_test_settings(2));
    let server = TestServer::new(app).unwrap();
    
    // 2. Submit 6 long-running jobs (sleep 100ms each)
    let mut tokens = Vec::new();
    for _ in 0..6 {
        let request_payload = SubmissionRequest {
            language: "python".to_string(),
            // Sleep for 100ms to allow polling of concurrent states
            source_code: "import time; time.sleep(0.1)".to_string(),
            stdin: "".to_string(),
            cpu_time_limit_ms: None,
            memory_limit_mb: None,
            wall_time_limit_ms: None,
        };
        let response = server.post("/submissions").json(&request_payload).await;
        response.assert_status_ok();
        tokens.push(response.json::<SubmissionResponse>().token);
    }
    
    // 3. Monitor statuses repeatedly and verify the cap
    let mut peak_processing_count = 0;
    let start_time = std::time::Instant::now();
    
    while start_time.elapsed() < Duration::from_millis(500) {
        let mut processing_count = 0;
        for token in &tokens {
            let res = server.get(&format!("/submissions/{}", token)).await;
            res.assert_status_ok();
            let poll_res = res.json::<SubmissionResponse>();
            if poll_res.status.id == 2 { // Processing
                processing_count += 1;
            }
        }
        peak_processing_count = peak_processing_count.max(processing_count);
        // Sleep a short duration to poll again
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    
    // The number of processing tasks must NEVER exceed the max_concurrent limit (2)
    assert!(peak_processing_count <= 2, "Peak processing count was {} which exceeded limit of 2", peak_processing_count);
    assert!(peak_processing_count > 0, "No processing tasks were observed");
}
