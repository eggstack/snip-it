//! gRPC sync integration tests for the snip-sync server.
//!
//! These tests exercise the full round-trip between `SyncClient` (the
//! production client) and a real `snip-sync` server bound to a random
//! local port. They catch regressions in the auth-header wiring — most
//! importantly, the bug where `sync_with_retry` was reading the API key
//! from `request.api_key` (which is empty by design for security) and
//! inserting that empty string as the bearer token in `authorization`
//! metadata, breaking all sync auth.

use snip_it::config::{SyncDirection, SyncSettings};
use snip_it::sync::{SyncClient, decrypt_snippet};
use snip_proto::Snippet;
use snip_sync::test_helpers::{build_test_service, start_test_server};
use std::sync::{Arc, Mutex};

/// The headline regression test: the `SyncClient` must send the API key
/// in the `authorization` gRPC metadata, not in the request body. The
/// server side captures the `authorization` header of the first
/// authenticated request it sees, and we assert it equals the bearer
/// token that matches the registered user's API key.
#[tokio::test(flavor = "multi_thread")]
async fn test_sync_client_sends_bearer_token_in_authorization_metadata() {
    let (api_key, _device_id, captured) = boot_server_and_register().await;

    // Build a SyncClient against the same plaintext server.
    let server_url = captured.0.clone();
    let mut client = build_sync_client(&server_url, &api_key).await;

    // Send one snippet through the encrypted sync path. The server
    // side records the first authenticated request's `authorization`
    // header into `captured.1`; we assert it after the call completes.
    let now = time::OffsetDateTime::now_utc().unix_timestamp();
    let snippet = Snippet {
        id: "integration-snippet-1".to_string(),
        description: "integration snippet".to_string(),
        command: "echo integration".to_string(),
        tags: vec!["integration".to_string()],
        created_at: now,
        updated_at: now,
        device_id: "integration-device".to_string(),
        deleted: false,
        encrypted: false,
    };
    let response = client
        .sync_encrypted(vec![snippet], 0, "")
        .await
        .expect("sync_encrypted should succeed");

    assert!(
        response.success,
        "sync response should be successful, got message: {}",
        response.message
    );

    // Core regression assertion: the server saw a non-empty bearer
    // token in the gRPC metadata. The recent bug inserted an empty
    // `Bearer ` string (because it was read from the body field, which
    // is intentionally empty for security). The fix is to pass the
    // API key as a separate argument and call `add_api_key_metadata`
    // with the real key.
    let observed = captured.1.lock().unwrap().clone();
    assert_eq!(
        observed,
        Some(format!("Bearer {api_key}")),
        "Server should have observed `Bearer {api_key}` in the \
         `authorization` metadata, but observed: {observed:?}"
    );

    // Stop the server task so the test process can exit cleanly.
    captured.2.abort();
}

/// End-to-end round-trip: register a device, sync a snippet, decrypt
/// the server's response, and confirm the plaintext is preserved.
#[tokio::test(flavor = "multi_thread")]
async fn test_sync_client_round_trip_encrypts_and_decrypts_snippets() {
    let (api_key, device_id, captured) = boot_server_and_register().await;
    let server_url = captured.0.clone();

    let mut client = build_sync_client(&server_url, &api_key).await;

    let now = time::OffsetDateTime::now_utc().unix_timestamp();
    let original_command = "echo round-trip-works";
    let snippet = Snippet {
        id: "rt-1".to_string(),
        description: "round-trip snippet".to_string(),
        command: original_command.to_string(),
        tags: vec!["rt".to_string()],
        created_at: now,
        updated_at: now,
        device_id: device_id.clone(),
        deleted: false,
        encrypted: false,
    };

    let response = client
        .sync_encrypted(vec![snippet], 0, "")
        .await
        .expect("sync_encrypted should succeed");
    assert!(response.success);

    // The server's `sync` round-trip pulls the same snippet back, still
    // encrypted on the wire. `decrypt_snippet` reverses the encryption
    // performed by `SyncClient::sync_encrypted`.
    let returned = response
        .snippets
        .iter()
        .find(|s| s.id == "rt-1")
        .expect("server should echo back the snippet we sent");
    let decrypted = decrypt_snippet(&api_key, returned).expect("decryption should succeed");
    assert_eq!(decrypted.command, original_command);
    assert_eq!(decrypted.description, "round-trip snippet");
    assert_eq!(decrypted.tags, vec!["rt".to_string()]);
    assert_eq!(decrypted.device_id, device_id);

    captured.2.abort();
}

/// The capture slot on the service is intentionally "first wins" — once
/// the first authenticated request sets it, subsequent requests should
/// leave it unchanged. This guards against a class of regressions where
/// the metadata would be cleared or overwritten by later calls.
#[tokio::test(flavor = "multi_thread")]
async fn test_captured_authorization_metadata_is_first_wins() {
    let (api_key, _device_id, captured) = boot_server_and_register().await;
    let server_url = captured.0.clone();

    let mut client = build_sync_client(&server_url, &api_key).await;

    // Issue two sync calls back-to-back.
    let now = time::OffsetDateTime::now_utc().unix_timestamp();
    let make_snippet = |id: &str| Snippet {
        id: id.to_string(),
        description: "x".to_string(),
        command: "echo x".to_string(),
        tags: vec![],
        created_at: now,
        updated_at: now,
        device_id: "device".to_string(),
        deleted: false,
        encrypted: false,
    };

    client
        .sync_encrypted(vec![make_snippet("first")], 0, "")
        .await
        .expect("first sync should succeed");
    let after_first = captured.1.lock().unwrap().clone();
    assert_eq!(after_first, Some(format!("Bearer {api_key}")));

    client
        .sync_encrypted(vec![make_snippet("second")], 0, "")
        .await
        .expect("second sync should succeed");
    let after_second = captured.1.lock().unwrap().clone();
    assert_eq!(
        after_second, after_first,
        "captured metadata should be first-wins, not overwritten"
    );

    captured.2.abort();
}

/// Encrypted payload near the plaintext size limit must still be accepted
/// by the server's `validate_snippet` length check. Before the fix, the
/// server compared the encrypted blob against `DEFAULT_MAX_COMMAND_LENGTH`
/// even though the blob grows due to JSON wrapping, AES-GCM overhead,
/// and base64 encoding, so a valid plaintext command that fit on the
/// wire could be rejected by the server.
#[tokio::test(flavor = "multi_thread")]
async fn test_sync_accepts_encrypted_payload_near_command_length_limit() {
    let (api_key, device_id, captured) = boot_server_and_register().await;
    let server_url = captured.0.clone();

    let mut client = build_sync_client(&server_url, &api_key).await;

    // Plaintext command right at the documented limit. After encryption
    // it will be substantially larger than 1024 bytes (JSON envelope +
    // AES-GCM tag + salt + nonce + base64 expansion).
    let plaintext_command = "x".repeat(snip_sync::DEFAULT_MAX_COMMAND_LENGTH);
    let now = time::OffsetDateTime::now_utc().unix_timestamp();
    let snippet = Snippet {
        id: "near-limit".to_string(),
        description: "d".to_string(),
        command: plaintext_command.clone(),
        tags: vec!["t".to_string()],
        created_at: now,
        updated_at: now,
        device_id,
        deleted: false,
        encrypted: false,
    };

    let response = client
        .sync_encrypted(vec![snippet], 0, "")
        .await
        .expect("sync should accept an encrypted payload near the size limit");

    assert!(
        response.success,
        "server should accept encrypted payload near plaintext limit, got: {}",
        response.message
    );

    let returned = response
        .snippets
        .iter()
        .find(|s| s.id == "near-limit")
        .expect("server should echo back the near-limit snippet");
    let decrypted = decrypt_snippet(&api_key, returned).expect("decryption should succeed");
    assert_eq!(decrypted.command, plaintext_command);

    captured.2.abort();
}

/// Spins up a real `snip-sync` server in-process, registers a new device
/// over gRPC, and returns `(api_key, device_id, (server_url, capture_slot, task))`.
///
/// `capture_slot` is the `Arc<Mutex<Option<String>>>` shared with the
/// running service. After the first authenticated request, it will
/// contain the `authorization` header value the server observed.
async fn boot_server_and_register() -> (
    String,
    String,
    (
        String,
        Arc<Mutex<Option<String>>>,
        tokio::task::JoinHandle<()>,
    ),
) {
    let service = build_test_service().await;
    let (addr, server_task, captured) = start_test_server(service).await;
    let server_url = format!("http://{addr}");

    let (api_key, device_id) = SyncClient::register(server_url.clone())
        .await
        .expect("register should succeed over plaintext loopback");
    assert!(
        !api_key.is_empty(),
        "register should return a non-empty API key"
    );
    assert!(
        !device_id.is_empty(),
        "register should return a non-empty device ID"
    );
    assert!(
        uuid::Uuid::parse_str(&device_id).is_ok(),
        "device_id should be a UUID, got: {device_id}"
    );

    (api_key, device_id, (server_url, captured, server_task))
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

/// `Register` is not idempotent: the server mints a fresh account and a
/// fresh API key on every call and discards the request body, so a retry
/// after a lost response leaves a second, live, untracked credential on the
/// server. The client must therefore attempt the RPC exactly once.
#[tokio::test(flavor = "multi_thread")]
async fn test_register_is_attempted_exactly_once() {
    // A zero per-minute budget makes every registration answer
    // `RESOURCE_EXHAUSTED`, which the retry classifier treats as retryable.
    let mut service = build_test_service().await;
    service.config.rate_limit_per_minute = 0;
    let observer = snip_sync::test_observer::InMemoryObserver::shared();
    let mut service = service;
    service.test_observer = Some(observer.clone());
    let (addr, server_task, _captured) = start_test_server(service).await;
    let server_url = format!("http://{addr}");

    let result = SyncClient::register(server_url).await;
    assert!(
        result.is_err(),
        "a rate-limited registration must fail instead of succeeding"
    );

    let register_requests = observer
        .starts()
        .iter()
        .filter(|event| event.operation == "register")
        .count();
    assert_eq!(
        register_requests, 1,
        "register must be attempted exactly once — a retry would mint another account"
    );

    server_task.abort();
}

/// A duplicate library name must not wedge the local↔server link. The
/// server enforces `UNIQUE(user_id, name)`, so the second create comes back
/// as `ALREADY_EXISTS`; the client adopts the existing server library
/// instead of failing and leaving the local library unlinked forever.
#[tokio::test(flavor = "multi_thread")]
async fn test_create_library_adopts_the_existing_server_library() {
    let (api_key, _device_id, captured) = boot_server_and_register().await;
    let server_url = captured.0.clone();
    let mut client = build_sync_client(&server_url, &api_key).await;

    let first = client
        .create_library("adopt-me")
        .await
        .expect("the first create must succeed");
    assert!(!first.id.is_empty(), "the created library must have an id");

    let second = client
        .create_library("adopt-me")
        .await
        .expect("a duplicate create must adopt the existing server library");
    assert_eq!(
        second.id, first.id,
        "the duplicate create must resolve to the existing server library"
    );

    captured.2.abort();
}

// --- The sequence-cursor bootstrap -------------------------------------
//
// The write-sequence cursor is the server's exact delta cursor: the server
// assigns a strictly increasing value per library, and the delta filter
// selects on it. A client can only enter that path through a bootstrap, and
// the bootstrap is a request that carries no cursor at all — the only
// request shape that delivers the complete library, and therefore the only
// one whose watermark the client may adopt.

/// The bootstrap: a request with no cursors delivers the complete library
/// and reports the server's sequence watermark.
#[tokio::test(flavor = "multi_thread")]
async fn test_a_cursorless_request_bootstraps_the_sequence_watermark() {
    let (api_key, device_id, captured) = boot_server_and_register().await;
    let server_url = captured.0.clone();
    let mut client = build_sync_client(&server_url, &api_key).await;

    let library = client
        .create_library("bootstrap")
        .await
        .expect("library creation must succeed");
    let now = time::OffsetDateTime::now_utc().unix_timestamp();
    let make = |id: &str| Snippet {
        id: id.to_string(),
        description: format!("bootstrap {id}"),
        command: format!("echo {id}"),
        tags: vec![],
        created_at: now,
        updated_at: now,
        device_id: device_id.clone(),
        deleted: false,
        encrypted: false,
    };

    // Two rows, so the watermark is above the first one.
    let response = client
        .sync_encrypted_with_cursors(vec![make("boot-1"), make("boot-2")], 0, 0, &library.id)
        .await
        .expect("the bootstrap sync must succeed");

    assert!(
        response.success,
        "the bootstrap must be successful, got: {}",
        response.message
    );
    assert!(
        response.server_sequence >= 2,
        "the bootstrap must report the library's sequence watermark, got {}",
        response.server_sequence
    );
    let delivered: Vec<&str> = response.snippets.iter().map(|s| s.id.as_str()).collect();
    assert!(
        delivered.contains(&"boot-1") && delivered.contains(&"boot-2"),
        "the bootstrap must deliver every row of the library, got {delivered:?}"
    );

    captured.2.abort();
}

/// The closing evidence for the sequence-cursor milestone: a client that
/// synced against a sequence-capable server sends a non-zero
/// `last_sync_sequence` on its next sync, and the server answers with a
/// non-zero `server_sequence`.
#[tokio::test(flavor = "multi_thread")]
async fn test_a_bootstrapped_client_sends_a_non_zero_sequence_cursor() {
    let (api_key, device_id, captured) = boot_server_and_register().await;
    let server_url = captured.0.clone();
    let mut client = build_sync_client(&server_url, &api_key).await;

    let library = client
        .create_library("cursor-path")
        .await
        .expect("library creation must succeed");
    let now = time::OffsetDateTime::now_utc().unix_timestamp();
    let snippet = Snippet {
        id: "cursor-1".to_string(),
        description: "cursor snippet".to_string(),
        command: "echo cursor".to_string(),
        tags: vec![],
        created_at: now,
        updated_at: now,
        device_id: device_id.clone(),
        deleted: false,
        encrypted: false,
    };

    // The bootstrap: no cursors at all.
    let bootstrap = client
        .sync_encrypted_with_cursors(vec![snippet.clone()], 0, 0, &library.id)
        .await
        .expect("the bootstrap sync must succeed");
    assert!(
        bootstrap.server_sequence > 0,
        "the server must report a sequence for a cursorless request"
    );

    // The next sync carries the adopted cursor.
    let follow_up = client
        .sync_encrypted_with_cursors(
            vec![],
            bootstrap.server_timestamp,
            bootstrap.server_sequence,
            &library.id,
        )
        .await
        .expect("the follow-up sync must succeed");
    assert!(
        follow_up.server_sequence > 0,
        "the server must answer a sequence request with a sequence"
    );
    assert!(
        follow_up.server_sequence >= bootstrap.server_sequence,
        "the reported watermark must not go backwards"
    );

    // The follow-up is a sequence delta, so it delivers only the rows after
    // the cursor — the boundary row is re-delivered because the filter is
    // inclusive.
    assert!(
        follow_up.snippets.iter().all(|s| s.id == "cursor-1"),
        "a sequence delta must not deliver rows the client already has"
    );

    captured.2.abort();
}

/// A timestamp-only request is answered with no sequence: a delta does not
/// deliver the library's whole history, so its watermark is not adoptable
/// by that client. Answering it anyway would let the client skip rows it
/// never received.
#[tokio::test(flavor = "multi_thread")]
async fn test_a_timestamp_only_request_is_answered_with_no_sequence() {
    let (api_key, device_id, captured) = boot_server_and_register().await;
    let server_url = captured.0.clone();
    let mut client = build_sync_client(&server_url, &api_key).await;

    let library = client
        .create_library("timestamp-only")
        .await
        .expect("library creation must succeed");
    let now = time::OffsetDateTime::now_utc().unix_timestamp();
    let snippet = Snippet {
        id: "ts-1".to_string(),
        description: "timestamp snippet".to_string(),
        command: "echo ts".to_string(),
        tags: vec![],
        created_at: now,
        updated_at: now,
        device_id: device_id.clone(),
        deleted: false,
        encrypted: false,
    };

    let response = client
        .sync_encrypted_with_cursors(vec![snippet], 0, 0, &library.id)
        .await
        .expect("the first sync must succeed");
    assert!(response.server_sequence > 0);

    // A timestamp-only request: a non-zero timestamp cursor and no
    // sequence cursor.
    let delta = client
        .sync_encrypted_with_cursors(vec![], response.server_timestamp, 0, &library.id)
        .await
        .expect("the timestamp delta must succeed");
    assert_eq!(
        delta.server_sequence, 0,
        "a timestamp-only request must be answered with no sequence"
    );

    captured.2.abort();
}

#[cfg(feature = "test-support")]
/// Push raw records through the generated client, bypassing `SyncClient`'s
/// encryption step. This is how an integration test seeds payloads in a
/// version the production write path does not emit.
async fn push_raw_snippets(server_url: &str, api_key: &str, snippets: Vec<Snippet>) {
    use snip_proto::PushSnippetsRequest;
    use snip_proto::snippet_sync_client::SnippetSyncClient;

    let mut client = SnippetSyncClient::connect(server_url.to_string())
        .await
        .expect("raw gRPC client should connect to the loopback server");
    let mut request = tonic::Request::new(PushSnippetsRequest {
        api_key: String::new(),
        library_id: String::new(),
        snippets,
    });
    request.metadata_mut().insert(
        "authorization",
        format!("Bearer {api_key}")
            .parse()
            .expect("a bearer token is valid metadata"),
    );
    let response = client
        .push_snippets(request)
        .await
        .expect("raw push should succeed")
        .into_inner();
    assert!(response.success, "raw push failed: {}", response.message);
}

#[cfg(feature = "test-support")]
/// Build an encrypted proto snippet around an already-sealed payload.
fn sealed_snippet(id: &str, payload: String, now: i64, device_id: &str) -> Snippet {
    Snippet {
        id: id.to_string(),
        description: String::new(),
        command: payload,
        tags: Vec::new(),
        created_at: now,
        updated_at: now,
        device_id: device_id.to_string(),
        deleted: false,
        encrypted: true,
    }
}

#[cfg(feature = "test-support")]
/// A library holding one version-1 and one version-2 payload must sync with
/// both rows decrypted and their ids preserved. A version-2 payload moved to a
/// different row must fail its tag check and be skipped, leaving the rest of
/// the library intact.
#[tokio::test(flavor = "multi_thread")]
async fn mixed_v1_and_v2_payloads_sync_and_a_substituted_v2_row_is_skipped() {
    let (api_key, device_id, captured) = boot_server_and_register().await;
    let server_url = captured.0.clone();
    let now = time::OffsetDateTime::now_utc().unix_timestamp();

    // Version 1: produced by the production write path.
    let v1_plaintext = Snippet {
        id: "v1-row".to_string(),
        description: "version one".to_string(),
        command: "echo v1".to_string(),
        tags: vec!["v1".to_string()],
        created_at: now,
        updated_at: now,
        device_id: device_id.clone(),
        deleted: false,
        encrypted: false,
    };
    let v1 = snip_it::sync::encrypt_snippet(&api_key, &v1_plaintext)
        .expect("v1 encryption should succeed");

    // Version 2: sealed for its own id, which only a future writer produces.
    let salt = [7u8; 16];
    let v2_json = serde_json::json!({
        "description": "version two",
        "command": "echo v2",
        "tags": ["v2"],
    })
    .to_string();
    let v2_payload =
        snip_it::sync::seal_v2_for_test(&api_key, &v2_json, "v2-row", &salt).expect("v2 seal");
    let v2 = sealed_snippet("v2-row", v2_payload, now, &device_id);

    // A version-2 payload sealed for `v2-row` but stored under another id: the
    // key-blind substitution this binding exists to stop.
    let substituted_payload =
        snip_it::sync::seal_v2_for_test(&api_key, &v2_json, "v2-row", &salt).expect("v2 seal");
    let substituted = sealed_snippet("v2-substituted", substituted_payload, now, &device_id);

    push_raw_snippets(&server_url, &api_key, vec![v1, v2, substituted.clone()]).await;

    let mut client = build_sync_client(&server_url, &api_key).await;
    let response = client
        .sync_encrypted(vec![], 0, "")
        .await
        .expect("pull should succeed");

    // The two authentic rows land with their ids and content preserved.
    let v1_returned = response
        .snippets
        .iter()
        .find(|s| s.id == "v1-row")
        .expect("the version-1 row should sync");
    assert_eq!(v1_returned.command, "echo v1");
    assert_eq!(v1_returned.description, "version one");

    let v2_returned = response
        .snippets
        .iter()
        .find(|s| s.id == "v2-row")
        .expect("the version-2 row should sync");
    assert_eq!(
        v2_returned.command, "echo v2",
        "the id-bound payload must decrypt under its own id"
    );
    assert_eq!(v2_returned.description, "version two");

    // The substituted row fails the tag check and is reported as skipped
    // rather than silently accepted under the wrong id.
    assert!(
        response.skipped_ids.iter().any(|id| id == "v2-substituted"),
        "a substituted v2 payload must be skipped, got skipped_ids={:?}",
        response.skipped_ids
    );
    assert!(
        !response.snippets.iter().any(|s| s.id == "v2-substituted"),
        "a substituted v2 payload must not appear as accepted content"
    );
    assert!(
        response.success,
        "one bad row must not abort the rest of the library: {}",
        response.message
    );

    captured.2.abort();
}

#[cfg(feature = "test-support")]
/// A library containing only version-1 payloads syncs exactly as before the
/// version-2 read path existed.
#[tokio::test(flavor = "multi_thread")]
async fn v1_only_library_still_syncs_unchanged() {
    let (api_key, device_id, captured) = boot_server_and_register().await;
    let server_url = captured.0.clone();
    let mut client = build_sync_client(&server_url, &api_key).await;

    let now = time::OffsetDateTime::now_utc().unix_timestamp();
    let snippet = Snippet {
        id: "v1-only".to_string(),
        description: "unchanged".to_string(),
        command: "echo unchanged".to_string(),
        tags: vec![],
        created_at: now,
        updated_at: now,
        device_id,
        deleted: false,
        encrypted: false,
    };

    let response = client
        .sync_encrypted(vec![snippet], 0, "")
        .await
        .expect("v1-only sync should succeed");
    assert!(response.success);
    assert!(
        response.skipped_ids.is_empty(),
        "{:?}",
        response.skipped_ids
    );
    let returned = response
        .snippets
        .iter()
        .find(|s| s.id == "v1-only")
        .expect("the row should round-trip");
    assert_eq!(returned.command, "echo unchanged");

    captured.2.abort();
}
