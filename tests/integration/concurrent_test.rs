use axum_test::TestServer;
use futures::future::join_all;
use otter::api::models::request::SubmissionRequest;
use otter::api::models::response::SubmissionResponse;
use otter::api::routes::build_router;
use otter::config::Settings;
use std::time::Duration;

fn get_test_settings(max_concurrent: usize) -> Settings {
    Settings {
        max_concurrent,
        ..Settings::default()
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
                webhook_url: None,
            };

            let response = server_ref.post("/submissions").json(&request_payload).await;
            response.assert_status(axum::http::StatusCode::CREATED);

            let initial_res = response.json::<SubmissionResponse>();
            let token = initial_res.token;

            // Poll until Accepted
            let mut result_stdout = String::new();
            for _ in 0..50 {
                let get_response = server_ref.get(&format!("/submissions/{}", token)).await;
                get_response.assert_status_ok();
                let poll_res = get_response.json::<SubmissionResponse>();
                if poll_res.status.id == 3 {
                    // Accepted
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
            webhook_url: None,
        };
        let response = server.post("/submissions").json(&request_payload).await;
        response.assert_status(axum::http::StatusCode::CREATED);
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
            if poll_res.status.id == 2 {
                // Processing
                processing_count += 1;
            }
        }
        peak_processing_count = peak_processing_count.max(processing_count);
        // Sleep a short duration to poll again
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    // The number of processing tasks must NEVER exceed the max_concurrent limit (2)
    assert!(
        peak_processing_count <= 2,
        "Peak processing count was {} which exceeded limit of 2",
        peak_processing_count
    );
    assert!(
        peak_processing_count > 0,
        "No processing tasks were observed"
    );
}

#[tokio::test]
async fn test_memory_leak_and_consistency() {
    let app = build_router(get_test_settings(4));
    let server = TestServer::new(app).unwrap();

    // Execute 50 sequential executions to verify memory stability and correct cleaning of resources
    for i in 0..50 {
        let request_payload = SubmissionRequest {
            language: "python".to_string(),
            source_code: format!("print('loop {}')", i),
            stdin: "".to_string(),
            cpu_time_limit_ms: None,
            memory_limit_mb: None,
            wall_time_limit_ms: None,
            webhook_url: None,
        };

        let response = server.post("/submissions").json(&request_payload).await;
        response.assert_status(axum::http::StatusCode::CREATED);

        let token = response.json::<SubmissionResponse>().token;

        // Poll until finished
        let mut finished = false;
        for _ in 0..100 {
            let get_response = server.get(&format!("/submissions/{}", token)).await;
            get_response.assert_status_ok();
            let poll_res = get_response.json::<SubmissionResponse>();
            if poll_res.status.id == 3 {
                // Accepted
                assert_eq!(poll_res.stdout.unwrap(), format!("loop {}\n", i));
                finished = true;
                break;
            } else if poll_res.status.id > 3 {
                panic!(
                    "Task failed at loop {} with status: {:?}",
                    i, poll_res.status
                );
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(finished, "Submission {} did not finish", i);
    }
}

#[tokio::test]
async fn test_per_ip_concurrency_capping() {
    // 1. Build server with max_concurrent = 4, max_concurrent_per_ip = 1
    let mut settings = get_test_settings(4);
    settings.max_concurrent_per_ip = 1; // 1 job per IP concurrently
    let app = build_router(settings);
    let server = TestServer::new(app).unwrap();

    // 2. Submit 2 long-running jobs (sleep 200ms each) from IP "1.1.1.1"
    let mut tokens_ip1 = Vec::new();
    for _ in 0..2 {
        let request_payload = SubmissionRequest {
            language: "python".to_string(),
            source_code: "import time; time.sleep(0.2)".to_string(),
            stdin: "".to_string(),
            cpu_time_limit_ms: None,
            memory_limit_mb: None,
            wall_time_limit_ms: None,
            webhook_url: None,
        };
        let response = server
            .post("/submissions")
            .add_header(
                axum::http::HeaderName::from_static("x-forwarded-for"),
                axum::http::HeaderValue::from_static("1.1.1.1"),
            )
            .json(&request_payload)
            .await;
        response.assert_status(axum::http::StatusCode::CREATED);
        tokens_ip1.push(response.json::<SubmissionResponse>().token);
    }

    // 3. Submit 1 long-running job from IP "2.2.2.2"
    let request_payload_ip2 = SubmissionRequest {
        language: "python".to_string(),
        source_code: "import time; time.sleep(0.2)".to_string(),
        stdin: "".to_string(),
        cpu_time_limit_ms: None,
        memory_limit_mb: None,
        wall_time_limit_ms: None,
        webhook_url: None,
    };
    let response_ip2 = server
        .post("/submissions")
        .add_header(
            axum::http::HeaderName::from_static("x-forwarded-for"),
            axum::http::HeaderValue::from_static("2.2.2.2"),
        )
        .json(&request_payload_ip2)
        .await;
    response_ip2.assert_status(axum::http::StatusCode::CREATED);
    let token_ip2 = response_ip2.json::<SubmissionResponse>().token;

    // 4. Poll status during execution.
    // - Since IP 1 is capped at 1 running job, only 1 of its 2 jobs should be Processing.
    // - IP 2 has 1 job, which should run immediately in Processing (since global cap is 4).
    let mut peak_ip1_processing = 0;
    let mut peak_ip2_processing = 0;
    let start_time = std::time::Instant::now();

    while start_time.elapsed() < Duration::from_millis(300) {
        let mut ip1_processing = 0;
        for token in &tokens_ip1 {
            let res = server.get(&format!("/submissions/{}", token)).await;
            res.assert_status_ok();
            let poll_res = res.json::<SubmissionResponse>();
            if poll_res.status.id == 2 {
                // Processing
                ip1_processing += 1;
            }
        }
        peak_ip1_processing = peak_ip1_processing.max(ip1_processing);

        let res_ip2 = server.get(&format!("/submissions/{}", token_ip2)).await;
        res_ip2.assert_status_ok();
        let poll_res_ip2 = res_ip2.json::<SubmissionResponse>();
        if poll_res_ip2.status.id == 2 {
            // Processing
            peak_ip2_processing = 1;
        }

        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    assert!(
        peak_ip1_processing <= 1,
        "IP 1 peak processing count was {} (exceeded cap 1)",
        peak_ip1_processing
    );
    assert_eq!(peak_ip2_processing, 1, "IP 2 job did not start processing");
}
