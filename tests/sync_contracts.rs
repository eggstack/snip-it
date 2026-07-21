mod support;

use snip_it::auto_sync::executor::effective_sync_direction;
use snip_it::config::{SyncDirection, SyncSettings};

#[test]
fn test_push_is_default_direction() {
    let settings = SyncSettings::default();
    assert_eq!(settings.sync_direction, SyncDirection::Push);
}

#[test]
fn test_push_only_cli_overrides_config() {
    let settings = SyncSettings::default();
    assert_eq!(
        effective_sync_direction(&settings, true, false),
        SyncDirection::Push
    );
}

#[test]
fn test_pull_only_cli_overrides_config() {
    let settings = SyncSettings::default();
    assert_eq!(
        effective_sync_direction(&settings, false, true),
        SyncDirection::Pull
    );
}

#[test]
fn test_config_fallback_when_no_cli_override() {
    let mut settings = SyncSettings::default();
    settings.sync_direction = SyncDirection::Bidirectional;
    assert_eq!(
        effective_sync_direction(&settings, false, false),
        SyncDirection::Bidirectional
    );
}

#[test]
fn test_push_cli_overrides_pull_config() {
    let mut settings = SyncSettings::default();
    settings.sync_direction = SyncDirection::Pull;
    assert_eq!(
        effective_sync_direction(&settings, true, false),
        SyncDirection::Push
    );
}

#[test]
fn test_pull_cli_overrides_push_config() {
    let mut settings = SyncSettings::default();
    settings.sync_direction = SyncDirection::Push;
    assert_eq!(
        effective_sync_direction(&settings, false, true),
        SyncDirection::Pull
    );
}

#[test]
fn test_direction_json_roundtrip() {
    for dir in [
        SyncDirection::Push,
        SyncDirection::Pull,
        SyncDirection::Bidirectional,
    ] {
        let json = serde_json::to_string(&dir).unwrap();
        let recovered: SyncDirection = serde_json::from_str(&json).unwrap();
        assert_eq!(recovered, dir, "roundtrip failed for {dir:?}");
    }
}

#[test]
fn test_local_output_field_not_in_proto() {
    let proto_snippet = snip_it::proto::Snippet::default();
    let json_str = format!("{:?}", proto_snippet);
    assert!(
        !json_str.contains("output"),
        "ProtoSnippet Debug output must not contain 'output' field (it is local-only): {json_str}"
    );
}

#[test]
fn test_sync_settings_auto_sync_defaults() {
    let settings = SyncSettings::default();
    assert!(!settings.auto_sync);
    assert_eq!(settings.auto_sync_debounce_seconds, 2);
}
