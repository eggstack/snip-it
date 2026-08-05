//! Multi-batch encrypted sync integration test.
//!
//! Verifies that `SyncClient::sync_encrypted` correctly uploads all
//! snippets when the collection requires multiple upload batches, and
//! that a repeated sync is convergent (no duplicate logical snippets).
//!
//! ```text
//! cargo test --test sync_multibatch -- --test-threads=1
//! ```

use snip_it::config::{SyncDirection, SyncSettings};
use snip_it::proto::Snippet;
use snip_it::sync::SyncClient;
use snip_sync::test_helpers::{build_test_service, start_test_server};

/// Enough snippets to force multiple upload batches with a reduced ceiling.
/// Each snippet has a ~10 KB command; encrypted they are ~13.5 KB. With a
/// ceiling of 100 KiB, each batch holds ~7 snippets, so 50 snippets
/// produce ~7 batches.
const MULTI_BATCH_COUNT: usize = 50;

/// Reduced ceiling to force multi-batch with a manageable number of
/// snippets. Must be large enough for at least one encrypted snippet
/// (~15 KB) but small enough that MULTI_BATCH_COUNT requires multiple
/// batches.
const TEST_BYTE_CEILING: usize = 100 * 1024; // 100 KiB

#[tokio::test(flavor = "multi_thread")]
async fn test_encrypted_sync_uploads_all_snippets_in_multiple_batches() {
    let service = build_test_service().await;
    let (addr, server_task, _captured) = start_test_server(service).await;
    let server_url = format!("http://{addr}");

    // Register a device.
    let (api_key, device_id) = SyncClient::register(server_url.clone())
        .await
        .expect("register should succeed");

    let mut client = build_sync_client(&server_url, &api_key).await;

    // Build enough snippets to require multiple batches under the
    // reduced ceiling.
    let now = chrono::Utc::now().timestamp();
    let snippets: Vec<Snippet> = (0..MULTI_BATCH_COUNT)
        .map(|i| Snippet {
            id: format!("mb-{i:04}", i = i),
            description: format!("multi-batch snippet {i}"),
            command: format!("echo snippet-{i} && {}", "x".repeat(10_000)),
            tags: vec![format!("batch{}", i % 3)],
            created_at: now,
            updated_at: now,
            device_id: device_id.clone(),
            deleted: false,
            encrypted: false,
        })
        .collect();

    // First sync: upload all snippets using a reduced ceiling to
    // force multi-batch uploads.
    let response = client
        .sync_encrypted_with_ceiling(snippets.clone(), 0, "", TEST_BYTE_CEILING)
        .await
        .expect("sync_encrypted should succeed for multi-batch upload");

    assert!(
        response.success,
        "sync should succeed, got: {}",
        response.message
    );

    // The server should have returned all snippets we sent.
    let returned_ids: Vec<&str> = response.snippets.iter().map(|s| s.id.as_str()).collect();
    for snippet in &snippets {
        assert!(
            returned_ids.contains(&snippet.id.as_str()),
            "server should contain snippet {} after multi-batch upload",
            snippet.id
        );
    }

    // No duplicate IDs in the response.
    let mut unique_ids: Vec<&str> = response.snippets.iter().map(|s| s.id.as_str()).collect();
    unique_ids.sort();
    unique_ids.dedup();
    assert_eq!(
        unique_ids.len(),
        response.snippets.len(),
        "response should contain no duplicate snippet IDs"
    );

    // Second sync: same snippets should be idempotent (no duplicates).
    let response2 = client
        .sync_encrypted_with_ceiling(snippets, 0, "", TEST_BYTE_CEILING)
        .await
        .expect("second sync should succeed");

    assert!(
        response2.success,
        "second sync should succeed, got: {}",
        response2.message
    );

    let mut ids2: Vec<&str> = response2.snippets.iter().map(|s| s.id.as_str()).collect();
    ids2.sort();
    ids2.dedup();
    assert_eq!(
        ids2.len(),
        response2.snippets.len(),
        "second sync response should contain no duplicate IDs"
    );
    assert_eq!(
        response2.snippets.len(),
        MULTI_BATCH_COUNT,
        "second sync should return all {} snippets",
        MULTI_BATCH_COUNT
    );

    server_task.abort();
}

/// Verify convergence after partial upload: sync a small batch, then a
/// larger batch that requires multiple upload batches, and confirm the
/// final server state contains all snippets with no duplicates.
#[tokio::test(flavor = "multi_thread")]
async fn test_sync_convergence_after_partial_upload() {
    let service = build_test_service().await;
    let (addr, server_task, _captured) = start_test_server(service).await;
    let server_url = format!("http://{addr}");

    let (api_key, device_id) = SyncClient::register(server_url.clone())
        .await
        .expect("register should succeed");

    let mut client = build_sync_client(&server_url, &api_key).await;

    let now = chrono::Utc::now().timestamp();

    // Sync a small batch that fits in one upload.
    let small_batch: Vec<Snippet> = (0..5)
        .map(|i| Snippet {
            id: format!("conv-{i}"),
            description: "convergence test".to_string(),
            command: "echo conv".to_string(),
            tags: vec![],
            created_at: now,
            updated_at: now,
            device_id: device_id.clone(),
            deleted: false,
            encrypted: false,
        })
        .collect();

    let resp1 = client
        .sync_encrypted(small_batch, 0, "")
        .await
        .expect("first sync should succeed");
    assert!(resp1.success);

    // Now sync a larger batch that requires multiple upload batches
    // under the reduced ceiling.
    let large_batch: Vec<Snippet> = (5..MULTI_BATCH_COUNT)
        .map(|i| Snippet {
            id: format!("conv-{i}"),
            description: "convergence test".to_string(),
            command: format!("echo conv-{i} && {}", "y".repeat(10_000)),
            tags: vec![],
            created_at: now,
            updated_at: now,
            device_id: device_id.clone(),
            deleted: false,
            encrypted: false,
        })
        .collect();

    let resp2 = client
        .sync_encrypted_with_ceiling(large_batch, 0, "", TEST_BYTE_CEILING)
        .await
        .expect("second sync should succeed");
    assert!(resp2.success);

    // Verify all snippets are present.
    assert_eq!(
        resp2.snippets.len(),
        MULTI_BATCH_COUNT,
        "should have all {} snippets after convergence",
        MULTI_BATCH_COUNT
    );

    // Verify no duplicates.
    let mut ids: Vec<&str> = resp2.snippets.iter().map(|s| s.id.as_str()).collect();
    ids.sort();
    ids.dedup();
    assert_eq!(
        ids.len(),
        MULTI_BATCH_COUNT,
        "no duplicate snippet IDs after convergence"
    );

    server_task.abort();
}

/// Simulate a partial failure mid-sync: the server crashes while the
/// client is uploading batches. After recovery (a fresh server), the
/// caller retries the full sync and all snippets converge with no
/// duplicates.
#[tokio::test(flavor = "multi_thread")]
async fn test_partial_failure_convergence() {
    // --- Phase 1: first server, register, start multi-batch sync. ---
    let service = build_test_service().await;
    let (addr, server_task, _captured) = start_test_server(service).await;
    let server_url = format!("http://{addr}");

    let (api_key, device_id) = SyncClient::register(server_url.clone())
        .await
        .expect("register should succeed");

    let mut client = build_sync_client(&server_url, &api_key).await;

    let now = chrono::Utc::now().timestamp();
    let snippets: Vec<Snippet> = (0..MULTI_BATCH_COUNT)
        .map(|i| Snippet {
            id: format!("pf-{i:04}", i = i),
            description: format!("partial-failure snippet {i}"),
            command: format!("echo snippet-{i} && {}", "x".repeat(10_000)),
            tags: vec![format!("batch{}", i % 3)],
            created_at: now,
            updated_at: now,
            device_id: device_id.clone(),
            deleted: false,
            encrypted: false,
        })
        .collect();

    // Spawn the sync in a background task so we can kill the server
    // while uploads are in flight.
    let snippets_clone = snippets.clone();
    let sync_task = tokio::spawn(async move {
        client
            .sync_encrypted_with_ceiling(snippets_clone, 0, "", TEST_BYTE_CEILING)
            .await
    });

    // Let the sync start sending batches before pulling the plug.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // Crash the server mid-sync.
    server_task.abort();
    let sync_result = tokio::time::timeout(std::time::Duration::from_secs(5), sync_task).await;

    // The sync must not have succeeded — the server died mid-upload.
    match sync_result {
        Ok(Ok(Ok(resp))) => {
            assert!(
                !resp.success,
                "sync should not report success when server died mid-upload"
            );
        }
        Ok(Ok(Err(_))) => { /* expected: gRPC/connection error */ }
        Ok(Err(_)) => { /* task panicked or was cancelled */ }
        Err(_) => { /* timed out — acceptable for a partial-failure test */ }
    }

    // --- Phase 2: recovery — fresh server, register, retry full sync. ---
    let service2 = build_test_service().await;
    let (addr2, server_task2, _captured2) = start_test_server(service2).await;
    let server_url2 = format!("http://{addr2}");

    let (api_key2, _device_id2) = SyncClient::register(server_url2.clone())
        .await
        .expect("register on recovery server should succeed");

    let mut client2 = build_sync_client(&server_url2, &api_key2).await;

    // Retry with the same cursor (0) — the caller does not advance
    // state after a failed sync.
    let response = client2
        .sync_encrypted_with_ceiling(snippets, 0, "", TEST_BYTE_CEILING)
        .await
        .expect("retry sync should succeed");

    assert!(
        response.success,
        "retry sync should succeed, got: {}",
        response.message
    );

    // All snippets must converge to the new server.
    assert_eq!(
        response.snippets.len(),
        MULTI_BATCH_COUNT,
        "recovery sync should return all {} snippets",
        MULTI_BATCH_COUNT
    );

    // No duplicate logical rows.
    let mut ids: Vec<&str> = response.snippets.iter().map(|s| s.id.as_str()).collect();
    ids.sort();
    ids.dedup();
    assert_eq!(
        ids.len(),
        MULTI_BATCH_COUNT,
        "recovery sync should contain no duplicate snippet IDs"
    );

    server_task2.abort();
}

async fn build_sync_client(server_url: &str, api_key: &str) -> SyncClient {
    let settings = SyncSettings {
        enabled: true,
        server_url: server_url.to_string(),
        api_key: api_key.to_string(),
        device_id: String::new(),
        sync_interval_minutes: 30,
        auto_sync: false,
        auto_sync_debounce_seconds: 2,
        auto_sync_failure: snip_it::config::AutoSyncFailureMode::Warn,
        auto_sync_max_delay_seconds: None,
        auto_sync_timeout_seconds: None,
        sync_direction: SyncDirection::Bidirectional,
        clipboard_auto_clear_seconds: None,
        sync_limit: None,
        credential_revision: 0,
    };
    SyncClient::create(settings)
        .await
        .expect("SyncClient::create should succeed against a plaintext loopback server")
}
