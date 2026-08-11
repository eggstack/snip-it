//! Multi-batch encrypted sync integration test.
//!
//! Verifies that `SyncClient::sync_encrypted` correctly uploads all
//! snippets when the collection requires multiple upload batches, and
//! that a repeated sync is convergent (no duplicate logical snippets).
//!
//! The all-encryption-failed accounting regression and the typed
//! batch-context unit tests now live in `src/sync.rs` next to the
//! private helpers they exercise (`sync_encrypted_with_test_encrypt`
//! and `add_batch_context`).
//!
//! ```text
//! cargo test --test sync_multibatch -- --test-threads=1
//! ```

use std::path::PathBuf;
use std::sync::Arc;

use snip_it::config::{SyncDirection, SyncSettings};
use snip_it::sync::SyncClient;
use snip_proto::Snippet;
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
        push_fail_after: Arc::new(std::sync::atomic::AtomicU32::new(u32::MAX)),
        push_fail_counter: Arc::new(std::sync::atomic::AtomicU32::new(0)),
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
///
/// Uses the deterministic push failure seam (push_fail_after) instead
/// of timing-based server abort.
#[tokio::test(flavor = "multi_thread")]
async fn test_partial_failure_convergence() {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("test.db");
    let db_path_str = db_path.to_str().unwrap();

    // Phase 1: Start server with push_fail_after=2 (fail on 3rd push batch).
    let service = build_file_service(db_path_str).await;
    service
        .push_fail_after
        .store(2, std::sync::atomic::Ordering::SeqCst);
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

    // Sync should fail deterministically on the 3rd push batch.
    let sync_result = client
        .sync_encrypted_with_ceiling(snippets.clone(), 0, "", TEST_BYTE_CEILING)
        .await;
    assert!(
        sync_result.is_err(),
        "first sync should fail due to push failure injection"
    );

    // Phase 2: The server retained partial state. Verify the DB has
    // more than zero and fewer than all expected snippets.
    {
        let db = snip_sync::db::Database::connect(db_path_str, 1)
            .await
            .unwrap();
        let user_id = db
            .get_user_by_api_key(&api_key)
            .await
            .unwrap()
            .expect("user should exist");
        let default_lib = db.get_default_library(&user_id).await.unwrap();
        let (rows, _total) = db
            .get_snippets(&user_id, &default_lib, 0, 200, 0, false)
            .await
            .unwrap();
        assert!(
            !rows.is_empty(),
            "server should have retained some snippets after partial failure"
        );
        assert!(
            rows.len() < MULTI_BATCH_COUNT,
            "server should NOT have all {} snippets after partial failure, got {}",
            MULTI_BATCH_COUNT,
            rows.len()
        );
    }

    // Phase 3: Stop the server, restart with push failure disabled.
    server_task.abort();
    let _ = tokio::time::timeout(std::time::Duration::from_secs(5), server_task).await;

    let service2 = build_file_service(db_path_str).await;
    // push_fail_after defaults to u32::MAX (no failure).
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

    // Phase 4: Verify idempotent convergence with a second retry.
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

    // Phase 5: Verify the database has exactly one row per expected ID.
    {
        let db = snip_sync::db::Database::connect(db_path_str, 1)
            .await
            .unwrap();
        let user_id = db
            .get_user_by_api_key(&api_key)
            .await
            .unwrap()
            .expect("user should exist");
        let default_lib = db.get_default_library(&user_id).await.unwrap();
        let (rows, _total) = db
            .get_snippets(&user_id, &default_lib, 0, 200, 0, false)
            .await
            .unwrap();
        let mut db_ids: Vec<&str> = rows.iter().map(|s| s.id.as_str()).collect();
        db_ids.sort();
        db_ids.dedup();
        assert_eq!(
            db_ids.len(),
            MULTI_BATCH_COUNT,
            "database should have exactly {} unique snippet IDs",
            MULTI_BATCH_COUNT
        );
    }

    server_task2.abort();
}

// ── Zero-batch and pull-only tests ──────────────────────────────

/// Empty local input against empty remote should succeed with no panic.
#[tokio::test(flavor = "multi_thread")]
async fn test_zero_batch_empty_local_empty_remote() {
    let service = build_file_service("sqlite::memory:").await;
    let (addr, server_task, _captured) = start_test_server(service).await;
    let server_url = format!("http://{addr}");

    let (api_key, _device_id) = SyncClient::register(server_url.clone())
        .await
        .expect("register should succeed");

    let mut client = build_sync_client(&server_url, &api_key).await;

    let response = client
        .sync_encrypted_with_ceiling(vec![], 0, "", TEST_BYTE_CEILING)
        .await
        .expect("zero-batch sync should succeed");

    assert!(response.success);
    assert!(
        response.snippets.is_empty(),
        "empty remote should return no snippets"
    );

    server_task.abort();
}

/// Seed remote state, then sync with empty local input and verify all
/// remote IDs are returned (pull-only path).
#[tokio::test(flavor = "multi_thread")]
async fn test_zero_batch_pull_only_seeded_remote() {
    let service = build_file_service("sqlite::memory:").await;
    let (addr, server_task, _captured) = start_test_server(service).await;
    let server_url = format!("http://{addr}");

    let (api_key, device_id) = SyncClient::register(server_url.clone())
        .await
        .expect("register should succeed");

    // Seed the server with some snippets via a normal sync.
    let mut setup_client = build_sync_client(&server_url, &api_key).await;
    let now = chrono::Utc::now().timestamp();
    let seed_snippets: Vec<Snippet> = (0..10)
        .map(|i| Snippet {
            id: format!("seed-{i:02}"),
            description: format!("seed snippet {i}"),
            command: format!("echo seed-{i}"),
            tags: vec![],
            created_at: now,
            updated_at: now,
            device_id: device_id.clone(),
            deleted: false,
            encrypted: false,
        })
        .collect();

    let seed_resp = setup_client
        .sync_encrypted(seed_snippets, 0, "")
        .await
        .expect("seed sync should succeed");
    assert!(seed_resp.success);

    // Now sync with empty local input — should pull all remote snippets.
    let mut pull_client = build_sync_client(&server_url, &api_key).await;
    let response = pull_client
        .sync_encrypted_with_ceiling(vec![], 0, "", TEST_BYTE_CEILING)
        .await
        .expect("pull-only sync should succeed");

    assert!(response.success);
    let mut returned_ids: Vec<&str> = response.snippets.iter().map(|s| s.id.as_str()).collect();
    returned_ids.sort();
    assert_eq!(
        returned_ids.len(),
        10,
        "pull-only sync should return all 10 seeded snippets"
    );
    for i in 0..10 {
        let expected_id = format!("seed-{i:02}");
        assert!(
            returned_ids.contains(&expected_id.as_str()),
            "missing seed snippet {expected_id}"
        );
    }

    server_task.abort();
}

/// Zero-batch sync retrieves more than one remote page when the page
/// limit is small enough to require pagination.
#[tokio::test(flavor = "multi_thread")]
async fn test_zero_batch_pagination() {
    let service = build_file_service("sqlite::memory:").await;
    let (addr, server_task, _captured) = start_test_server(service).await;
    let server_url = format!("http://{addr}");

    let (api_key, device_id) = SyncClient::register(server_url.clone())
        .await
        .expect("register should succeed");

    // Seed 20 snippets via a normal sync (small enough to fit in one batch).
    let mut setup_client = build_sync_client(&server_url, &api_key).await;
    let now = chrono::Utc::now().timestamp();
    let seed_snippets: Vec<Snippet> = (0..20)
        .map(|i| Snippet {
            id: format!("page-{i:02}"),
            description: format!("pagination snippet {i}"),
            command: format!("echo page-{i}"),
            tags: vec![],
            created_at: now,
            updated_at: now,
            device_id: device_id.clone(),
            deleted: false,
            encrypted: false,
        })
        .collect();

    let seed_resp = setup_client
        .sync_encrypted(seed_snippets, 0, "")
        .await
        .expect("seed sync should succeed");
    assert!(seed_resp.success);

    // Now pull with a small sync_limit to force multi-page pagination.
    // The server's default page limit is 100, but we can set a smaller
    // limit via the SyncSettings to force multiple round trips.
    let settings = SyncSettings {
        enabled: true,
        server_url: server_url.clone(),
        api_key: api_key.clone(),
        device_id: String::new(),
        sync_interval_minutes: 30,
        auto_sync: false,
        auto_sync_debounce_seconds: 2,
        auto_sync_failure: snip_it::config::AutoSyncFailureMode::Warn,
        auto_sync_max_delay_seconds: None,
        auto_sync_timeout_seconds: None,
        sync_direction: SyncDirection::Bidirectional,
        clipboard_auto_clear_seconds: None,
        sync_limit: Some(5), // Force pages of 5
        credential_revision: 0,
    };
    let mut paginated_client = SyncClient::create(settings)
        .await
        .expect("SyncClient::create should succeed");

    let response = paginated_client
        .sync_encrypted_with_ceiling(vec![], 0, "", TEST_BYTE_CEILING)
        .await
        .expect("paginated pull should succeed");

    assert!(response.success);
    let mut returned_ids: Vec<&str> = response.snippets.iter().map(|s| s.id.as_str()).collect();
    returned_ids.sort();
    assert_eq!(
        returned_ids.len(),
        20,
        "paginated pull should retrieve all 20 snippets across multiple pages"
    );

    server_task.abort();
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
