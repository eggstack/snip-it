use super::sync_settings::migrate_plaintext_api_key;
use super::toml_cache::{
    TOML_CACHE, compute_crc32, strip_integrity_line, toml_cache_key, verify_integrity,
};
use super::*;

#[test]
fn test_sync_settings_default() {
    let settings = SyncSettings::default();

    assert!(!settings.enabled);
    assert_eq!(settings.server_url, DEFAULT_SERVER_URL);
    assert!(settings.api_key.is_empty());
    assert!(settings.device_id.is_empty());
    assert_eq!(settings.sync_interval_minutes, 30);
    assert!(!settings.auto_sync);
    assert_eq!(settings.auto_sync_debounce_seconds, 2);
    assert_eq!(settings.auto_sync_failure, AutoSyncFailureMode::Warn);
    assert_eq!(settings.sync_direction, SyncDirection::Push);
    assert_eq!(settings.sync_limit, None);
}

#[test]
fn test_sync_limit_rejects_non_positive_values() {
    for limit in ["0", "-1"] {
        let content = format!(
            "enabled = false\nserver_url = \"https://sync.example.com\"\nsync_interval_minutes = 30\nsync_limit = {limit}\n"
        );
        assert!(
            toml::from_str::<SyncSettings>(&content).is_err(),
            "sync_limit = {limit} should be rejected"
        );
    }
}

#[test]
fn test_sync_settings_serialization() {
    // Ensure keychain is bypassed in CI environments without a keychain
    #[cfg(feature = "test-support")]
    unsafe {
        std::env::set_var("SNP_ALLOW_PLAINTEXT_API_KEY", "true");
    }
    let settings = SyncSettings {
        enabled: true,
        server_url: "https://sync.example.com".to_string(),
        api_key: if cfg!(feature = "test-support") {
            "test-key-123"
        } else {
            ""
        }
        .to_string(),
        device_id: "device-456".to_string(),
        sync_interval_minutes: 60,
        auto_sync: true,
        auto_sync_debounce_seconds: 5,
        auto_sync_failure: AutoSyncFailureMode::Error,
        auto_sync_max_delay_seconds: Some(60),
        auto_sync_timeout_seconds: None,
        sync_direction: SyncDirection::Bidirectional,
        clipboard_auto_clear_seconds: Some(30),
        sync_limit: Some(2000),
        credential_revision: 0,
    };

    let toml_str = toml::to_string_pretty(&settings).unwrap();
    assert!(toml_str.contains("enabled = true"));
    assert!(toml_str.contains("server_url = \"https://sync.example.com\""));
    #[cfg(feature = "test-support")]
    assert!(
        toml_str.contains("api_key = \"@keychain\"")
            || toml_str.contains("api_key = \"test-key-123\"")
    );
    #[cfg(not(feature = "test-support"))]
    assert!(toml_str.contains("api_key = \"\""));
    assert!(toml_str.contains("device_id = \"device-456\""));
    assert!(toml_str.contains("sync_interval_minutes = 60"));
    assert!(toml_str.contains("auto_sync = true"));
    assert!(toml_str.contains("auto_sync_debounce_seconds = 5"));
    assert!(toml_str.contains("auto_sync_failure = \"error\""));
    assert!(toml_str.contains("sync_direction = \"Bidirectional\""));
}

#[test]
fn test_verify_integrity_no_header_returns_true() {
    // Legacy config files without an integrity header should be accepted
    // to prevent data loss on upgrade from older versions.
    let content = "[sync]\nenabled = true\n";
    assert!(verify_integrity(content));
}

#[test]
fn test_invalidate_toml_cache_does_not_duplicate_insertion_order() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let path = temp_dir.path().join("cache-churn.toml");
    std::fs::write(&path, "value = 1\n").unwrap();

    let key = toml_cache_key(&path);
    for _ in 0..10 {
        invalidate_toml_cache(&path);
        let _ = cached_read_toml(&path).unwrap();
    }

    let cache = TOML_CACHE.lock().unwrap();
    let occurrences = cache.insertion_order.iter().filter(|k| **k == key).count();
    assert_eq!(occurrences, 1);
    assert_eq!(cache.entries.get(&key).map(|e| e.len), Some(10));
}

#[cfg(unix)]
#[test]
fn test_toml_cache_key_canonicalizes_symlink_aliases() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let path = temp_dir.path().join("real.toml");
    let alias = temp_dir.path().join("alias.toml");
    std::fs::write(&path, "value = 1\n").unwrap();
    std::os::unix::fs::symlink(&path, &alias).unwrap();

    assert_eq!(toml_cache_key(&path), toml_cache_key(&alias));
}

#[test]
fn test_verify_integrity_valid_header() {
    let body = "[sync]\nenabled = true";
    let checksum = compute_crc32(body);
    let content = format!("# integrity: {checksum}\n{body}");
    assert!(verify_integrity(&content));
}

#[test]
fn test_verify_integrity_invalid_header() {
    let body = "[sync]\nenabled = true";
    let content = format!("# integrity: 999999\n{body}");
    assert!(!verify_integrity(&content));
}

#[test]
fn test_verify_integrity_tampered_body() {
    let original = "[sync]\nenabled = true";
    let checksum = compute_crc32(original);
    let tampered = "[sync]\nenabled = false";
    let content = format!("# integrity: {checksum}\n{tampered}");
    assert!(!verify_integrity(&content));
}

#[test]
fn test_verify_integrity_preserves_exact_body() {
    let body = "[sync]\n# integrity: user-authored comment\nenabled = true\n";
    let checksum = compute_crc32(body);
    let content = format!("# integrity: {checksum}\n{body}");

    assert!(verify_integrity(&content));
    assert_eq!(strip_integrity_line(&content), body);
}

#[test]
fn test_verify_integrity_malformed_header_fails() {
    let content = "# integrity: not-a-checksum\n[sync]\nenabled = true\n";
    assert!(!verify_integrity(content));
}

#[test]
fn test_keychain_migration_preserves_in_memory_api_key() {
    let mut settings = SyncSettings::default();
    settings.api_key = "test-key-123".to_string();

    migrate_plaintext_api_key(
        &settings,
        |api_key| {
            assert_eq!(api_key, "test-key-123");
            Ok(())
        },
        |saved_settings| {
            assert_eq!(saved_settings.api_key, KEYCHAIN_MARKER);
            Ok(())
        },
    );

    assert_eq!(settings.api_key, "test-key-123");
}

#[test]
fn test_auto_sync_debounce_clamped() {
    let mut settings = SyncSettings::default();
    assert_eq!(
        settings.auto_sync_debounce(),
        std::time::Duration::from_secs(2)
    );

    settings.auto_sync_debounce_seconds = 0;
    assert_eq!(
        settings.auto_sync_debounce(),
        std::time::Duration::from_secs(0)
    );

    settings.auto_sync_debounce_seconds = 300;
    assert_eq!(
        settings.auto_sync_debounce(),
        std::time::Duration::from_secs(300)
    );

    // Overflow clamped to max
    settings.auto_sync_debounce_seconds = u64::MAX;
    assert_eq!(
        settings.auto_sync_debounce(),
        std::time::Duration::from_secs(300)
    );
}

#[test]
fn test_auto_sync_failure_mode_default() {
    let settings = SyncSettings::default();
    assert_eq!(settings.auto_sync_failure, AutoSyncFailureMode::Warn);
}

#[test]
fn test_auto_sync_failure_mode_display_roundtrip() {
    let modes = vec![
        AutoSyncFailureMode::Ignore,
        AutoSyncFailureMode::Warn,
        AutoSyncFailureMode::Error,
    ];
    for mode in &modes {
        let s = mode.to_string();
        let parsed: AutoSyncFailureMode = s.parse().unwrap();
        assert_eq!(*mode, parsed);
    }
}

#[test]
fn test_auto_sync_failure_mode_invalid() {
    let result = "bogus".parse::<AutoSyncFailureMode>();
    assert!(result.is_err());
}

#[test]
fn test_old_config_without_auto_sync_fields_loads_defaults() {
    let content = r#"
[settings.sync]
enabled = true
server_url = "https://sync.example.com"
api_key = "test-key"
sync_interval_minutes = 15
auto_sync = true
sync_direction = "Bidirectional"
"#;
    // Old configs without auto_sync_debounce_seconds/auto_sync_failure should load defaults
    let config: SyncConfigFile = toml::from_str(content).unwrap();
    let settings = config.settings.sync;
    assert!(settings.auto_sync);
    assert_eq!(settings.auto_sync_debounce_seconds, 2); // default
    assert_eq!(settings.auto_sync_failure, AutoSyncFailureMode::Warn); // default
}

#[test]
fn test_full_config_roundtrip() {
    // Ensure keychain is bypassed in CI environments without a keychain
    #[cfg(feature = "test-support")]
    unsafe {
        std::env::set_var("SNP_ALLOW_PLAINTEXT_API_KEY", "true");
    }
    let settings = SyncSettings {
        enabled: true,
        server_url: "https://sync.example.com".to_string(),
        api_key: if cfg!(feature = "test-support") {
            "test-key"
        } else {
            ""
        }
        .to_string(),
        device_id: "device-1".to_string(),
        sync_interval_minutes: 15,
        auto_sync: true,
        auto_sync_debounce_seconds: 5,
        auto_sync_failure: AutoSyncFailureMode::Error,
        auto_sync_max_delay_seconds: Some(120),
        auto_sync_timeout_seconds: None,
        sync_direction: SyncDirection::Bidirectional,
        clipboard_auto_clear_seconds: Some(30),
        sync_limit: Some(500),
        credential_revision: 0,
    };
    let toml_str = toml::to_string_pretty(&settings).unwrap();
    // Use from_str directly to avoid keychain lookup
    let roundtripped: SyncSettings = toml::from_str(&toml_str).unwrap_or_else(|_| {
        // If keychain lookup fails, parse with a plaintext fallback
        let fallback = toml_str.replace("api_key = \"@keychain\"", "api_key = \"test-key\"");
        toml::from_str(&fallback).unwrap()
    });
    assert!(roundtripped.auto_sync);
    assert_eq!(roundtripped.auto_sync_debounce_seconds, 5);
    assert_eq!(roundtripped.auto_sync_failure, AutoSyncFailureMode::Error);
    assert_eq!(roundtripped.sync_direction, SyncDirection::Bidirectional);
}

#[test]
fn test_unrelated_settings_preserved() {
    // Ensure keychain is bypassed in CI environments without a keychain
    #[cfg(feature = "test-support")]
    unsafe {
        std::env::set_var("SNP_ALLOW_PLAINTEXT_API_KEY", "true");
    }
    let settings = SyncSettings {
        enabled: true,
        server_url: "https://sync.example.com".to_string(),
        api_key: if cfg!(feature = "test-support") {
            "test-key"
        } else {
            ""
        }
        .to_string(),
        device_id: "device-1".to_string(),
        sync_interval_minutes: 15,
        auto_sync: true,
        auto_sync_debounce_seconds: 10,
        auto_sync_failure: AutoSyncFailureMode::Ignore,
        auto_sync_max_delay_seconds: None,
        auto_sync_timeout_seconds: None,
        sync_direction: SyncDirection::Push,
        clipboard_auto_clear_seconds: Some(60),
        sync_limit: Some(500),
        credential_revision: 0,
    };
    let toml_str = toml::to_string_pretty(&settings).unwrap();
    // Verify unrelated fields are present
    assert!(toml_str.contains("enabled = true"));
    assert!(toml_str.contains("sync_interval_minutes = 15"));
    assert!(toml_str.contains("clipboard_auto_clear_seconds = 60"));
    assert!(toml_str.contains("sync_limit = 500"));
    assert!(toml_str.contains("sync_direction = \"Push\""));
}
