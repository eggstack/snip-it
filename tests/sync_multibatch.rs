//! Multi-batch encrypted sync integration test.
//!
//! Verifies that `SyncClient::sync_encrypted` correctly uploads all
//! snippets when the collection requires multiple upload batches, and
//! that a repeated sync is convergent (no duplicate logical snippets).
//!
//! ```text
//! cargo test --test sync_multibatch -- --test-threads=1
//! ```

use std::path::PathBuf;
use std::sync::Arc;

use snip_it::config::{SyncDirection, SyncSettings};
use snip_it::proto::Snippet;
use snip_it::sync::SyncClient;
use snip_sync::db::Database;
use snip_sync::test_helpers::start_test_server;
use snip_sync::{Config, Metrics, PremadeManager, RateLimiter, SnipSyncService};

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

async fn build_file_service(db_path: &str) -> SnipSyncService {
    let db = Arc::new(Database::connect(db_path, 5).await.unwrap());
    let config = Config {
        grpc_host: "127.0.0.1".to_string(),
        grpc_port: 0,
        http_host: "127.0.0.1".to_string(),
        http_port: 0,
        db_path: db_path.to_string(),
        db_max_connections: 5,
        premade_dir: PathBuf::from("premade-libraries"),
        max_command_length: 1024,
        max_description_length: 1024,
        max_tags: 50,
        max_tag_length: 100,
        max_id_length: 128,
        max_device_id_length: 128,
        max_api_key_length: 512,
        request_timeout_secs: 30,
        grpc_max_message_size: 4 * 1024 * 1024,
        rate_limit_per_minute: 120,
        trusted_proxies: vec![],
        persist_rate_limits: false,
        metrics_username: None,
        metrics_password: None,
        cors_allowed_origins: vec![],
    };
    let metrics = Metrics::fallback();
    let rate_limiter = Arc::new(RateLimiter::new());
    let premade_manager = PremadeManager::new(PathBuf::from("premade-libraries"));

    SnipSyncService {
        db,
        rate_limiter,
        config,
        metrics,
        premade_manager,
        captured_auth_header: Arc::new(std::sync::Mutex::new(None)),
        test_observer: None,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_encrypted_sync_uploads_all_snippets_in_multiple_batches() {
    let service = build_file_service("sqlite::memory:").await;
    let (addr, server_task, _captured) = start_test_server(service).await;
    let server_url = format!("http://{addr}");

    let (api_key, device_id) = SyncClient::register(server_url.clone())
        .await
        .expect("register should succeed");

    let mut client = build_sync_client(&server_url, &api_key).await;

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

    let response = client
        .sync_encrypted_with_ceiling(snippets.clone(), 0, "", TEST_BYTE_CEILING)
        .await
        .expect("sync_encrypted should succeed for multi-batch upload");

    assert!(
        response.success,
        "sync should succeed, got: {}",
        response.message
    );

    let returned_ids: Vec<&str> = response.snippets.iter().map(|s| s.id.as_str()).collect();
    for snippet in &snippets {
        assert!(
            returned_ids.contains(&snippet.id.as_str()),
            "server should contain snippet {} after multi-batch upload",
            snippet.id
        );
    }

    let mut unique_ids: Vec<&str> = response.snippets.iter().map(|s| s.id.as_str()).collect();
    unique_ids.sort();
    unique_ids.dedup();
    assert_eq!(
        unique_ids.len(),
        response.snippets.len(),
        "response should contain no duplicate snippet IDs"
    );

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
    let service = build_file_service("sqlite::memory:").await;
    let (addr, server_task, _captured) = start_test_server(service).await;
    let server_url = format!("http://{addr}");

    let (api_key, device_id) = SyncClient::register(server_url.clone())
        .await
        .expect("register should succeed");

    let mut client = build_sync_client(&server_url, &api_key).await;

    let now = chrono::Utc::now().timestamp();

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

    assert_eq!(
        resp2.snippets.len(),
        MULTI_BATCH_COUNT,
        "should have all {} snippets after convergence",
        MULTI_BATCH_COUNT
    );

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

/// Verify retained-state convergence: after a partial failure where
/// some batches committed and others didn't, retrying with the same
/// credentials against the same database produces the correct result
/// with no duplicates.
#[tokio::test(flavor = "multi_thread")]
async fn test_partial_failure_convergence() {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("test.db");
    let db_path_str = db_path.to_str().unwrap();

    // Phase 1: Start server, register, upload batches, crash mid-sync.
    let service = build_file_service(db_path_str).await;
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

    // Spawn sync in background, then crash the server mid-upload.
    let snippets_clone = snippets.clone();
    let sync_task = tokio::spawn(async move {
        client
            .sync_encrypted_with_ceiling(snippets_clone, 0, "", TEST_BYTE_CEILING)
            .await
    });

    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    server_task.abort();
    let _ = tokio::time::timeout(std::time::Duration::from_secs(5), sync_task).await;

    // Phase 2: Restart against the same database file.
    let service2 = build_file_service(db_path_str).await;
    let (addr2, server_task2, _captured2) = start_test_server(service2).await;
    let server_url2 = format!("http://{addr2}");

    // Retry with the SAME API key and library identity.
    let mut client2 = build_sync_client(&server_url2, &api_key).await;

    let response = client2
        .sync_encrypted_with_ceiling(snippets.clone(), 0, "", TEST_BYTE_CEILING)
        .await
        .expect("retry sync should succeed");

    assert!(
        response.success,
        "retry sync should succeed, got: {}",
        response.message
    );

    // All snippets must converge — no duplicates from idempotent upserts.
    let mut ids: Vec<&str> = response.snippets.iter().map(|s| s.id.as_str()).collect();
    ids.sort();
    ids.dedup();
    assert_eq!(
        ids.len(),
        MULTI_BATCH_COUNT,
        "retained-state retry should contain all {} snippets with no duplicates",
        MULTI_BATCH_COUNT
    );

    // The caller does not advance the successful-sync cursor after failure.
    // Verify by syncing again with the same cursor.
    let mut client3 = build_sync_client(&server_url2, &api_key).await;
    let response2 = client3
        .sync_encrypted_with_ceiling(snippets, 0, "", TEST_BYTE_CEILING)
        .await
        .expect("second retry should succeed");

    let mut ids2: Vec<&str> = response2.snippets.iter().map(|s| s.id.as_str()).collect();
    ids2.sort();
    ids2.dedup();
    assert_eq!(
        ids2.len(),
        MULTI_BATCH_COUNT,
        "second retry should still have all {} snippets without duplication",
        MULTI_BATCH_COUNT
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
