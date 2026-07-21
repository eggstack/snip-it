//! **Layer: Application**
//!
//! Read-only status projection for `snp status` and doctor commands.
//!
//! Captures a point-in-time snapshot of local libraries, sync state,
//! pending operations, and execution status.

use crate::auto_sync::execution_lock::{self, ExecutionLockContents};
use crate::auto_sync::lock::{self, WorkerLockContents};
use crate::auto_sync::pending::{self, PendingError, PendingState};
use crate::auto_sync::status::{self, AutoSyncStatus, StatusRead};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const SNAPSHOT_SCHEMA: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusSnapshot {
    pub schema: u32,
    pub generated_at_unix_ms: u64,
    pub config_root: PathBuf,
    pub log_dir: PathBuf,
    pub local: LocalSummary,
    pub sync: SyncSummary,
    pub pending: PendingSummary,
    pub attempt: AttemptSummary,
    pub execution: ExecutionSummary,
    pub diagnostics: Vec<StatusDiagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalSummary {
    pub libraries: usize,
    pub snippets: usize,
    pub primary_library: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum SyncConfigurationState {
    NotConfigured,
    Configured,
    ConfiguredAutoSyncDisabled,
    LoadFailed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncSummary {
    pub configuration: SyncConfigurationState,
    pub top_level: TopLevelSyncState,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum TopLevelSyncState {
    CorruptOrInaccessible,
    LiveExecution { pid: u32, started_at_unix_ms: u64 },
    PendingAttentionRequired,
    PendingRetryBackoff { next_attempt_at_unix_ms: u64 },
    PendingAwaitingScheduling,
    ConfiguredAndCurrent,
    ConfiguredAutoSyncDisabled,
    NotConfigured,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum PendingStateView {
    None,
    Pending {
        generation: u64,
        created_at_unix_ms: u64,
    },
    Corrupt {
        reason_code: String,
    },
    Inaccessible {
        reason_code: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingSummary {
    pub state: PendingStateView,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum AttemptStateView {
    NeverAttempted,
    Succeeded,
    RetryScheduled,
    AttentionRequired,
    Deferred,
    Corrupt,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttemptSummary {
    pub state: AttemptStateView,
    pub last_attempt_generation: u64,
    pub last_attempt_at_unix_ms: u64,
    pub last_success_at_unix_ms: u64,
    pub last_failure_class: String,
    pub consecutive_failures: u32,
    pub next_attempt_at_unix_ms: u64,
    pub attention_required: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ProcessStateView {
    Idle,
    Live { pid: u32, started_at_unix_ms: u64 },
    DeadStale { pid: u32 },
    Malformed,
    Inaccessible,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionSummary {
    pub execution_lock: ProcessStateView,
    pub worker_lock: ProcessStateView,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusDiagnostic {
    pub severity: DiagnosticSeverity,
    pub code: String,
    pub message: String,
    pub remediation: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum DiagnosticSeverity {
    Info,
    Warning,
    Error,
}

fn unix_now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub fn capture_snapshot() -> StatusSnapshot {
    let state_dir = crate::auto_sync::paths::state_dir();
    let config_root = crate::utils::config::get_config_dir();
    let log_dir = crate::logging::get_default_log_dir();
    let generated_at = unix_now_ms();

    let sync_config = sync_configuration_state();
    let pending_view = pending_state_view(&state_dir);
    let (exec_view, worker_view) = execution_state_view(&state_dir);
    let execution = ExecutionSummary {
        execution_lock: exec_view,
        worker_lock: worker_view,
    };

    let status_read = status::read_status_typed(&state_dir);
    let attempt = match &status_read {
        StatusRead::Valid(s) => attempt_from_status(s),
        StatusRead::Corrupt(_) => AttemptSummary {
            state: AttemptStateView::Corrupt,
            ..default_attempt()
        },
        StatusRead::Missing => AttemptSummary {
            state: AttemptStateView::NeverAttempted,
            ..default_attempt()
        },
    };

    let top_level = derive_top_level(&pending_view, &execution, &attempt, &sync_config);
    let sync = SyncSummary {
        configuration: sync_config,
        top_level,
    };

    let pending = PendingSummary {
        state: pending_view,
    };

    let local = capture_local_summary();

    let snapshot = StatusSnapshot {
        schema: SNAPSHOT_SCHEMA,
        generated_at_unix_ms: generated_at,
        config_root,
        log_dir,
        local,
        sync,
        pending,
        attempt,
        execution,
        diagnostics: Vec::new(),
    };

    let mut snapshot = snapshot;
    snapshot.diagnostics = collect_diagnostics(&snapshot);
    snapshot
}

pub fn sync_configuration_state() -> SyncConfigurationState {
    let settings = crate::config::load_sync_settings();
    match settings {
        Ok(s) => {
            if !s.enabled {
                SyncConfigurationState::NotConfigured
            } else if !s.auto_sync {
                SyncConfigurationState::ConfiguredAutoSyncDisabled
            } else {
                SyncConfigurationState::Configured
            }
        }
        Err(_) => SyncConfigurationState::LoadFailed,
    }
}

pub fn pending_state_view(state_dir: &Path) -> PendingStateView {
    match pending::read_state_from_dir(state_dir) {
        Ok(PendingState {
            generation,
            created_at_unix_ms,
            ..
        }) => PendingStateView::Pending {
            generation,
            created_at_unix_ms,
        },
        Err(PendingError::NotFound) => PendingStateView::None,
        Err(PendingError::IntegrityMismatch { .. }) | Err(PendingError::Corrupted(_)) => {
            PendingStateView::Corrupt {
                reason_code: "integrity_or_parse_failure".to_string(),
            }
        }
        Err(PendingError::Io(_)) => PendingStateView::Inaccessible {
            reason_code: "io_error".to_string(),
        },
        Err(PendingError::Deserialize(_)) => PendingStateView::Corrupt {
            reason_code: "deserialization_failure".to_string(),
        },
        Err(PendingError::Lock(_)) => PendingStateView::Inaccessible {
            reason_code: "lock_contention".to_string(),
        },
        Err(PendingError::Serialize(_)) => PendingStateView::Corrupt {
            reason_code: "serialization_failure".to_string(),
        },
    }
}

pub fn attempt_state_view(status: &AutoSyncStatus) -> AttemptStateView {
    if status.last_attempt_at_unix_ms == 0 {
        return AttemptStateView::NeverAttempted;
    }
    if status.attention_required {
        return AttemptStateView::AttentionRequired;
    }
    if status.last_success_at_unix_ms >= status.last_attempt_at_unix_ms
        && status.last_success_at_unix_ms > 0
    {
        return AttemptStateView::Succeeded;
    }
    if status.next_attempt_at_unix_ms > 0 {
        let now = unix_now_ms();
        if now < status.next_attempt_at_unix_ms {
            return AttemptStateView::RetryScheduled;
        }
        return AttemptStateView::Deferred;
    }
    if status.last_success_at_unix_ms > 0 {
        return AttemptStateView::Succeeded;
    }
    if status.consecutive_failures > 0 {
        return AttemptStateView::RetryScheduled;
    }
    AttemptStateView::Succeeded
}

pub fn execution_state_view(state_dir: &Path) -> (ProcessStateView, ProcessStateView) {
    let exec_view = inspect_execution_lock(state_dir);
    let worker_view = inspect_worker_lock(state_dir);
    (exec_view, worker_view)
}

fn inspect_execution_lock(state_dir: &Path) -> ProcessStateView {
    let path = execution_lock::execution_lock_path(state_dir);
    match std::fs::read_to_string(&path) {
        Ok(content) => {
            if content.trim().is_empty() {
                return ProcessStateView::Idle;
            }
            match toml::from_str::<ExecutionLockContents>(&content) {
                Ok(contents) => {
                    if execution_lock::process_alive(contents.pid) {
                        ProcessStateView::Live {
                            pid: contents.pid,
                            started_at_unix_ms: contents.started_at_unix_ms,
                        }
                    } else {
                        ProcessStateView::DeadStale { pid: contents.pid }
                    }
                }
                Err(_) => ProcessStateView::Malformed,
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => ProcessStateView::Idle,
        Err(_) => ProcessStateView::Inaccessible,
    }
}

fn inspect_worker_lock(state_dir: &Path) -> ProcessStateView {
    let path = lock::lock_path(state_dir);
    match std::fs::read_to_string(&path) {
        Ok(content) => {
            if content.trim().is_empty() {
                return ProcessStateView::Idle;
            }
            match toml::from_str::<WorkerLockContents>(&content) {
                Ok(contents) => {
                    if execution_lock::process_alive(contents.pid) {
                        ProcessStateView::Live {
                            pid: contents.pid,
                            started_at_unix_ms: contents.started_at_unix_ms,
                        }
                    } else {
                        ProcessStateView::DeadStale { pid: contents.pid }
                    }
                }
                Err(_) => ProcessStateView::Malformed,
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => ProcessStateView::Idle,
        Err(_) => ProcessStateView::Inaccessible,
    }
}

pub fn derive_top_level(
    pending: &PendingStateView,
    execution: &ExecutionSummary,
    attempt: &AttemptSummary,
    sync_config: &SyncConfigurationState,
) -> TopLevelSyncState {
    if matches!(
        pending,
        PendingStateView::Corrupt { .. } | PendingStateView::Inaccessible { .. }
    ) {
        return TopLevelSyncState::CorruptOrInaccessible;
    }

    if let ProcessStateView::Live {
        pid,
        started_at_unix_ms,
    } = &execution.execution_lock
    {
        return TopLevelSyncState::LiveExecution {
            pid: *pid,
            started_at_unix_ms: *started_at_unix_ms,
        };
    }

    if let PendingStateView::Pending { .. } = pending {
        if attempt.attention_required {
            return TopLevelSyncState::PendingAttentionRequired;
        }
        if attempt.next_attempt_at_unix_ms > 0 {
            let now = unix_now_ms();
            if now < attempt.next_attempt_at_unix_ms {
                return TopLevelSyncState::PendingRetryBackoff {
                    next_attempt_at_unix_ms: attempt.next_attempt_at_unix_ms,
                };
            }
        }
        return TopLevelSyncState::PendingAwaitingScheduling;
    }

    match sync_config {
        SyncConfigurationState::Configured => TopLevelSyncState::ConfiguredAndCurrent,
        SyncConfigurationState::ConfiguredAutoSyncDisabled => {
            TopLevelSyncState::ConfiguredAutoSyncDisabled
        }
        SyncConfigurationState::NotConfigured => TopLevelSyncState::NotConfigured,
        SyncConfigurationState::LoadFailed => TopLevelSyncState::NotConfigured,
    }
}

pub fn collect_diagnostics(snapshot: &StatusSnapshot) -> Vec<StatusDiagnostic> {
    let mut diagnostics = Vec::new();

    match &snapshot.sync.configuration {
        SyncConfigurationState::LoadFailed => {
            diagnostics.push(StatusDiagnostic {
                severity: DiagnosticSeverity::Error,
                code: "CONFIG_LOAD_FAILED".to_string(),
                message: "Failed to load sync configuration".to_string(),
                remediation: Some("Check ~/.config/snp/sync.toml for syntax errors".to_string()),
            });
        }
        SyncConfigurationState::NotConfigured => {
            diagnostics.push(StatusDiagnostic {
                severity: DiagnosticSeverity::Info,
                code: "NOT_CONFIGURED".to_string(),
                message: "Sync is not configured".to_string(),
                remediation: Some("Run `snp register` to configure sync".to_string()),
            });
        }
        _ => {}
    }

    match &snapshot.pending.state {
        PendingStateView::Corrupt { reason_code } => {
            diagnostics.push(StatusDiagnostic {
                severity: DiagnosticSeverity::Error,
                code: "PENDING_CORRUPT".to_string(),
                message: format!("Pending state is corrupt: {reason_code}"),
                remediation: Some("Remove the pending marker and retry".to_string()),
            });
        }
        PendingStateView::Inaccessible { reason_code } => {
            diagnostics.push(StatusDiagnostic {
                severity: DiagnosticSeverity::Warning,
                code: "PENDING_INACCESSIBLE".to_string(),
                message: format!("Pending state inaccessible: {reason_code}"),
                remediation: None,
            });
        }
        _ => {}
    }

    match &snapshot.execution.execution_lock {
        ProcessStateView::DeadStale { pid } => {
            diagnostics.push(StatusDiagnostic {
                severity: DiagnosticSeverity::Warning,
                code: "EXECUTION_LOCK_STALE".to_string(),
                message: format!("Execution lock held by dead process {pid}"),
                remediation: Some("Lock will be reclaimed on next sync attempt".to_string()),
            });
        }
        ProcessStateView::Malformed => {
            diagnostics.push(StatusDiagnostic {
                severity: DiagnosticSeverity::Warning,
                code: "EXECUTION_LOCK_MALFORMED".to_string(),
                message: "Execution lock file is malformed".to_string(),
                remediation: Some("Lock will be reclaimed on next sync attempt".to_string()),
            });
        }
        ProcessStateView::Inaccessible => {
            diagnostics.push(StatusDiagnostic {
                severity: DiagnosticSeverity::Warning,
                code: "EXECUTION_LOCK_INACCESSIBLE".to_string(),
                message: "Execution lock file is inaccessible".to_string(),
                remediation: None,
            });
        }
        _ => {}
    }

    match &snapshot.execution.worker_lock {
        ProcessStateView::DeadStale { pid } => {
            diagnostics.push(StatusDiagnostic {
                severity: DiagnosticSeverity::Warning,
                code: "WORKER_LOCK_STALE".to_string(),
                message: format!("Worker lock held by dead process {pid}"),
                remediation: Some("Lock will be reclaimed on next sync attempt".to_string()),
            });
        }
        ProcessStateView::Malformed => {
            diagnostics.push(StatusDiagnostic {
                severity: DiagnosticSeverity::Warning,
                code: "WORKER_LOCK_MALFORMED".to_string(),
                message: "Worker lock file is malformed".to_string(),
                remediation: Some("Lock will be reclaimed on next sync attempt".to_string()),
            });
        }
        ProcessStateView::Inaccessible => {
            diagnostics.push(StatusDiagnostic {
                severity: DiagnosticSeverity::Warning,
                code: "WORKER_LOCK_INACCESSIBLE".to_string(),
                message: "Worker lock file is inaccessible".to_string(),
                remediation: None,
            });
        }
        _ => {}
    }

    match snapshot.attempt.state {
        AttemptStateView::AttentionRequired => {
            diagnostics.push(StatusDiagnostic {
                severity: DiagnosticSeverity::Error,
                code: "ATTENTION_REQUIRED".to_string(),
                message: format!(
                    "Sync requires attention: {}",
                    snapshot.attempt.last_failure_class
                ),
                remediation: Some("Run `snp sync` or fix the underlying issue".to_string()),
            });
        }
        AttemptStateView::Corrupt => {
            diagnostics.push(StatusDiagnostic {
                severity: DiagnosticSeverity::Error,
                code: "STATUS_CORRUPT".to_string(),
                message: "Auto-sync status file is corrupt".to_string(),
                remediation: Some("Remove auto-sync-status.toml and retry".to_string()),
            });
        }
        _ => {}
    }

    diagnostics.sort_by(|a, b| {
        a.severity
            .cmp(&b.severity)
            .then_with(|| a.code.cmp(&b.code))
    });

    diagnostics
}

fn attempt_from_status(status: &AutoSyncStatus) -> AttemptSummary {
    let state = attempt_state_view(status);
    AttemptSummary {
        state,
        last_attempt_generation: status.last_attempt_generation,
        last_attempt_at_unix_ms: status.last_attempt_at_unix_ms,
        last_success_at_unix_ms: status.last_success_at_unix_ms,
        last_failure_class: status.last_failure_class.clone(),
        consecutive_failures: status.consecutive_failures,
        next_attempt_at_unix_ms: status.next_attempt_at_unix_ms,
        attention_required: status.attention_required,
        message: status.message.clone(),
    }
}

fn default_attempt() -> AttemptSummary {
    AttemptSummary {
        state: AttemptStateView::NeverAttempted,
        last_attempt_generation: 0,
        last_attempt_at_unix_ms: 0,
        last_success_at_unix_ms: 0,
        last_failure_class: String::new(),
        consecutive_failures: 0,
        next_attempt_at_unix_ms: 0,
        attention_required: false,
        message: String::new(),
    }
}

fn capture_local_summary() -> LocalSummary {
    match crate::library::LibraryManager::new() {
        Ok(mgr) => {
            let libs = mgr.list_libraries().len();
            let primary = mgr.get_primary_library().map(|l| l.filename.clone());
            let snippets = count_snippets(&mgr);
            LocalSummary {
                libraries: libs,
                snippets,
                primary_library: primary,
            }
        }
        Err(_) => LocalSummary {
            libraries: 0,
            snippets: 0,
            primary_library: None,
        },
    }
}

fn count_snippets(mgr: &crate::library::LibraryManager) -> usize {
    let mut total = 0;
    for lib_meta in mgr.list_libraries() {
        let path = mgr
            .get_libraries_dir()
            .join(format!("{}.toml", lib_meta.filename));
        if let Ok(snippets) = crate::library::load_library(&path) {
            total += snippets.snippets.len();
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auto_sync::execution_lock::{ExecutionLockContents, execution_lock_path};
    use crate::auto_sync::lock::{WorkerLockContents, lock_path};
    use crate::auto_sync::pending::{PendingSnapshot, record_pending_mutation};
    use crate::auto_sync::policy::{FailureClass, MutationKind};
    use crate::auto_sync::status;
    use tempfile::TempDir;

    fn make_empty_state_dir() -> TempDir {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path()).unwrap();
        dir
    }

    #[test]
    fn test_not_configured() {
        let dir = make_empty_state_dir();
        let pending = pending_state_view(dir.path());
        let execution = ExecutionSummary {
            execution_lock: ProcessStateView::Idle,
            worker_lock: ProcessStateView::Idle,
        };
        let attempt = default_attempt();
        let top = derive_top_level(
            &pending,
            &execution,
            &attempt,
            &SyncConfigurationState::NotConfigured,
        );
        assert_eq!(top, TopLevelSyncState::NotConfigured);
    }

    #[test]
    fn test_configured_and_current() {
        let dir = make_empty_state_dir();
        let pending = pending_state_view(dir.path());
        let execution = ExecutionSummary {
            execution_lock: ProcessStateView::Idle,
            worker_lock: ProcessStateView::Idle,
        };
        let attempt = AttemptSummary {
            state: AttemptStateView::Succeeded,
            last_attempt_at_unix_ms: 1000,
            last_success_at_unix_ms: 1000,
            ..default_attempt()
        };
        let top = derive_top_level(
            &pending,
            &execution,
            &attempt,
            &SyncConfigurationState::Configured,
        );
        assert_eq!(top, TopLevelSyncState::ConfiguredAndCurrent);
    }

    #[test]
    fn test_auto_sync_disabled() {
        let dir = make_empty_state_dir();
        let pending = pending_state_view(dir.path());
        let execution = ExecutionSummary {
            execution_lock: ProcessStateView::Idle,
            worker_lock: ProcessStateView::Idle,
        };
        let attempt = default_attempt();
        let top = derive_top_level(
            &pending,
            &execution,
            &attempt,
            &SyncConfigurationState::ConfiguredAutoSyncDisabled,
        );
        assert_eq!(top, TopLevelSyncState::ConfiguredAutoSyncDisabled);
    }

    #[test]
    fn test_pending_awaiting_debounce() {
        let dir = make_empty_state_dir();
        record_pending_mutation(
            dir.path(),
            PendingSnapshot::Mutation {
                kind: MutationKind::SnippetCreate,
            },
        )
        .unwrap();
        let pending = pending_state_view(dir.path());
        let execution = ExecutionSummary {
            execution_lock: ProcessStateView::Idle,
            worker_lock: ProcessStateView::Idle,
        };
        let attempt = default_attempt();
        let top = derive_top_level(
            &pending,
            &execution,
            &attempt,
            &SyncConfigurationState::Configured,
        );
        assert_eq!(top, TopLevelSyncState::PendingAwaitingScheduling);
    }

    #[test]
    fn test_pending_with_active_execution() {
        let dir = make_empty_state_dir();
        record_pending_mutation(
            dir.path(),
            PendingSnapshot::Mutation {
                kind: MutationKind::SnippetCreate,
            },
        )
        .unwrap();
        let contents = ExecutionLockContents {
            pid: std::process::id(),
            started_at_unix_ms: 5000,
            nonce: "test".to_string(),
        };
        let serialized = toml::to_string_pretty(&contents).unwrap();
        std::fs::write(execution_lock_path(dir.path()), serialized).unwrap();

        let pending = pending_state_view(dir.path());
        let (exec_view, worker_view) = execution_state_view(dir.path());
        let execution = ExecutionSummary {
            execution_lock: exec_view,
            worker_lock: worker_view,
        };
        let attempt = default_attempt();
        let top = derive_top_level(
            &pending,
            &execution,
            &attempt,
            &SyncConfigurationState::Configured,
        );
        match top {
            TopLevelSyncState::LiveExecution { pid, .. } => {
                assert_eq!(pid, std::process::id());
            }
            other => panic!("expected LiveExecution, got {other:?}"),
        }
    }

    #[test]
    fn test_pending_retry_backoff() {
        let dir = make_empty_state_dir();
        record_pending_mutation(
            dir.path(),
            PendingSnapshot::Mutation {
                kind: MutationKind::SnippetCreate,
            },
        )
        .unwrap();
        let pending = pending_state_view(dir.path());
        let execution = ExecutionSummary {
            execution_lock: ProcessStateView::Idle,
            worker_lock: ProcessStateView::Idle,
        };
        let future_ms = unix_now_ms() + 60_000;
        status::record_failure(
            dir.path(),
            1,
            FailureClass::TransientNetwork,
            4,
            1,
            future_ms,
            "connection failed",
            0,
        )
        .unwrap();
        let status_read = status::read_status_typed(dir.path());
        let attempt = match status_read {
            StatusRead::Valid(ref s) => attempt_from_status(s),
            _ => default_attempt(),
        };
        let top = derive_top_level(
            &pending,
            &execution,
            &attempt,
            &SyncConfigurationState::Configured,
        );
        assert!(matches!(top, TopLevelSyncState::PendingRetryBackoff { .. }));
    }

    #[test]
    fn test_attention_required() {
        let dir = make_empty_state_dir();
        record_pending_mutation(
            dir.path(),
            PendingSnapshot::Mutation {
                kind: MutationKind::SnippetCreate,
            },
        )
        .unwrap();
        let pending = pending_state_view(dir.path());
        let execution = ExecutionSummary {
            execution_lock: ProcessStateView::Idle,
            worker_lock: ProcessStateView::Idle,
        };
        status::record_failure(
            dir.path(),
            1,
            FailureClass::Authentication,
            3,
            1,
            0,
            "bad api key",
            0,
        )
        .unwrap();
        let status_read = status::read_status_typed(dir.path());
        let attempt = match status_read {
            StatusRead::Valid(ref s) => attempt_from_status(s),
            _ => default_attempt(),
        };
        let top = derive_top_level(
            &pending,
            &execution,
            &attempt,
            &SyncConfigurationState::Configured,
        );
        assert_eq!(top, TopLevelSyncState::PendingAttentionRequired);
    }

    #[test]
    fn test_corrupt_pending() {
        let dir = make_empty_state_dir();
        let pending_path = crate::auto_sync::paths::pending_marker(dir.path());
        std::fs::write(&pending_path, "not valid toml {{{").unwrap();
        let pending = pending_state_view(dir.path());
        assert!(matches!(pending, PendingStateView::Corrupt { .. }));
        let execution = ExecutionSummary {
            execution_lock: ProcessStateView::Idle,
            worker_lock: ProcessStateView::Idle,
        };
        let attempt = default_attempt();
        let top = derive_top_level(
            &pending,
            &execution,
            &attempt,
            &SyncConfigurationState::Configured,
        );
        assert_eq!(top, TopLevelSyncState::CorruptOrInaccessible);
    }

    #[test]
    fn test_corrupt_status() {
        let dir = make_empty_state_dir();
        let status_path = crate::auto_sync::paths::status_file(dir.path());
        std::fs::write(&status_path, "not valid toml {{{").unwrap();
        let status_read = status::read_status_typed(dir.path());
        assert!(matches!(status_read, StatusRead::Corrupt(_)));
        let attempt = default_attempt();
        assert_eq!(attempt.state, AttemptStateView::NeverAttempted);
    }

    #[test]
    fn test_inaccessible_pending() {
        let dir = make_empty_state_dir();
        let pending = pending_state_view(dir.path());
        assert_eq!(pending, PendingStateView::None);
        let path = dir.path().join("nonexistent");
        let pending = pending_state_view(&path);
        assert_eq!(pending, PendingStateView::None);
    }

    #[test]
    fn test_io_error_pending() {
        let dir = make_empty_state_dir();
        let pending_path = crate::auto_sync::paths::pending_marker(dir.path());
        std::fs::create_dir(&pending_path).unwrap();
        let pending = pending_state_view(dir.path());
        match &pending {
            PendingStateView::Inaccessible { .. } => {}
            other => panic!("expected Inaccessible, got {other:?}"),
        }
    }

    #[test]
    fn test_live_execution_lock() {
        let dir = make_empty_state_dir();
        let contents = ExecutionLockContents {
            pid: std::process::id(),
            started_at_unix_ms: 1000,
            nonce: "test-nonce".to_string(),
        };
        let serialized = toml::to_string_pretty(&contents).unwrap();
        std::fs::write(execution_lock_path(dir.path()), serialized).unwrap();
        let view = inspect_execution_lock(dir.path());
        match view {
            ProcessStateView::Live { pid, .. } => assert_eq!(pid, std::process::id()),
            other => panic!("expected Live, got {other:?}"),
        }
    }

    #[test]
    fn test_dead_execution_lock() {
        let dir = make_empty_state_dir();
        let contents = ExecutionLockContents {
            pid: 1,
            started_at_unix_ms: 1000,
            nonce: "dead".to_string(),
        };
        let serialized = toml::to_string_pretty(&contents).unwrap();
        std::fs::write(execution_lock_path(dir.path()), serialized).unwrap();
        let view = inspect_execution_lock(dir.path());
        assert!(matches!(view, ProcessStateView::DeadStale { pid: 1 }));
    }

    #[test]
    fn test_malformed_execution_lock() {
        let dir = make_empty_state_dir();
        std::fs::write(execution_lock_path(dir.path()), "garbage").unwrap();
        let view = inspect_execution_lock(dir.path());
        assert_eq!(view, ProcessStateView::Malformed);
    }

    #[test]
    fn test_idle_when_no_lock_file() {
        let dir = make_empty_state_dir();
        let view = inspect_execution_lock(dir.path());
        assert_eq!(view, ProcessStateView::Idle);
    }

    #[test]
    fn test_empty_lock_file_is_idle() {
        let dir = make_empty_state_dir();
        std::fs::write(execution_lock_path(dir.path()), "").unwrap();
        let view = inspect_execution_lock(dir.path());
        assert_eq!(view, ProcessStateView::Idle);
    }

    #[test]
    fn test_worker_lock_live() {
        let dir = make_empty_state_dir();
        let contents = WorkerLockContents {
            pid: std::process::id(),
            started_at_unix_ms: 2000,
            nonce: "wnonce".to_string(),
        };
        let serialized = toml::to_string_pretty(&contents).unwrap();
        std::fs::write(lock_path(dir.path()), serialized).unwrap();
        let view = inspect_worker_lock(dir.path());
        match view {
            ProcessStateView::Live { pid, .. } => assert_eq!(pid, std::process::id()),
            other => panic!("expected Live, got {other:?}"),
        }
    }

    #[test]
    fn test_worker_lock_dead() {
        let dir = make_empty_state_dir();
        let contents = WorkerLockContents {
            pid: 1,
            started_at_unix_ms: 2000,
            nonce: "dead".to_string(),
        };
        let serialized = toml::to_string_pretty(&contents).unwrap();
        std::fs::write(lock_path(dir.path()), serialized).unwrap();
        let view = inspect_worker_lock(dir.path());
        assert!(matches!(view, ProcessStateView::DeadStale { pid: 1 }));
    }

    #[test]
    fn test_pending_generation_newer_than_last_success() {
        let dir = make_empty_state_dir();
        record_pending_mutation(
            dir.path(),
            PendingSnapshot::Mutation {
                kind: MutationKind::SnippetCreate,
            },
        )
        .unwrap();
        status::record_success(dir.path(), 1, "ok").unwrap();
        record_pending_mutation(
            dir.path(),
            PendingSnapshot::Mutation {
                kind: MutationKind::SnippetUpdate,
            },
        )
        .unwrap();
        let pending = pending_state_view(dir.path());
        match &pending {
            PendingStateView::Pending { generation, .. } => assert_eq!(*generation, 2),
            other => panic!("expected Pending, got {other:?}"),
        }
        let execution = ExecutionSummary {
            execution_lock: ProcessStateView::Idle,
            worker_lock: ProcessStateView::Idle,
        };
        let status_read = status::read_status_typed(dir.path());
        let attempt = match status_read {
            StatusRead::Valid(ref s) => attempt_from_status(s),
            _ => default_attempt(),
        };
        let top = derive_top_level(
            &pending,
            &execution,
            &attempt,
            &SyncConfigurationState::Configured,
        );
        assert_eq!(top, TopLevelSyncState::PendingAwaitingScheduling);
    }

    #[test]
    fn test_deterministic_diagnostics_ordering() {
        let dir = make_empty_state_dir();
        let snapshot = StatusSnapshot {
            schema: SNAPSHOT_SCHEMA,
            generated_at_unix_ms: 0,
            config_root: dir.path().to_path_buf(),
            log_dir: dir.path().join("logs"),
            local: LocalSummary {
                libraries: 0,
                snippets: 0,
                primary_library: None,
            },
            sync: SyncSummary {
                configuration: SyncConfigurationState::LoadFailed,
                top_level: TopLevelSyncState::NotConfigured,
            },
            pending: PendingSummary {
                state: PendingStateView::Corrupt {
                    reason_code: "test".to_string(),
                },
            },
            attempt: AttemptSummary {
                state: AttemptStateView::AttentionRequired,
                last_failure_class: "authentication".to_string(),
                ..default_attempt()
            },
            execution: ExecutionSummary {
                execution_lock: ProcessStateView::Malformed,
                worker_lock: ProcessStateView::Idle,
            },
            diagnostics: Vec::new(),
        };
        let diags = collect_diagnostics(&snapshot);
        assert!(!diags.is_empty());
        for i in 1..diags.len() {
            assert!(
                diags[i - 1].severity <= diags[i].severity
                    || (diags[i - 1].severity == diags[i].severity
                        && diags[i - 1].code <= diags[i].code),
                "diagnostics not sorted: {:?} before {:?}",
                diags[i - 1],
                diags[i]
            );
        }
    }

    #[test]
    fn test_attempt_state_never_attempted() {
        let status = AutoSyncStatus::default();
        assert_eq!(
            attempt_state_view(&status),
            AttemptStateView::NeverAttempted
        );
    }

    #[test]
    fn test_attempt_state_succeeded() {
        let status = AutoSyncStatus {
            last_attempt_at_unix_ms: 1000,
            last_success_at_unix_ms: 1000,
            ..AutoSyncStatus::default()
        };
        assert_eq!(attempt_state_view(&status), AttemptStateView::Succeeded);
    }

    #[test]
    fn test_attempt_state_retry_scheduled() {
        let status = AutoSyncStatus {
            last_attempt_at_unix_ms: 1000,
            last_success_at_unix_ms: 0,
            next_attempt_at_unix_ms: unix_now_ms() + 60_000,
            consecutive_failures: 1,
            ..AutoSyncStatus::default()
        };
        assert_eq!(
            attempt_state_view(&status),
            AttemptStateView::RetryScheduled
        );
    }

    #[test]
    fn test_attempt_state_attention_required() {
        let status = AutoSyncStatus {
            last_attempt_at_unix_ms: 1000,
            attention_required: true,
            ..AutoSyncStatus::default()
        };
        assert_eq!(
            attempt_state_view(&status),
            AttemptStateView::AttentionRequired
        );
    }

    #[test]
    fn test_attempt_state_deferred() {
        let status = AutoSyncStatus {
            last_attempt_at_unix_ms: 1000,
            last_success_at_unix_ms: 0,
            next_attempt_at_unix_ms: 100,
            consecutive_failures: 1,
            ..AutoSyncStatus::default()
        };
        assert_eq!(attempt_state_view(&status), AttemptStateView::Deferred);
    }

    #[test]
    fn test_snapshot_schema_version() {
        assert_eq!(SNAPSHOT_SCHEMA, 1);
    }

    #[test]
    fn test_snapshot_json_roundtrip() {
        let snapshot = StatusSnapshot {
            schema: SNAPSHOT_SCHEMA,
            generated_at_unix_ms: 12345,
            config_root: PathBuf::from("/tmp/test"),
            log_dir: PathBuf::from("/tmp/test/logs"),
            local: LocalSummary {
                libraries: 2,
                snippets: 42,
                primary_library: Some("snippets".to_string()),
            },
            sync: SyncSummary {
                configuration: SyncConfigurationState::Configured,
                top_level: TopLevelSyncState::ConfiguredAndCurrent,
            },
            pending: PendingSummary {
                state: PendingStateView::None,
            },
            attempt: AttemptSummary {
                state: AttemptStateView::Succeeded,
                last_attempt_generation: 5,
                last_attempt_at_unix_ms: 1000,
                last_success_at_unix_ms: 1000,
                last_failure_class: String::new(),
                consecutive_failures: 0,
                next_attempt_at_unix_ms: 0,
                attention_required: false,
                message: "ok".to_string(),
            },
            execution: ExecutionSummary {
                execution_lock: ProcessStateView::Idle,
                worker_lock: ProcessStateView::Idle,
            },
            diagnostics: Vec::new(),
        };
        let json = serde_json::to_string(&snapshot).unwrap();
        let restored: StatusSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.schema, snapshot.schema);
        assert_eq!(restored.local.snippets, 42);
        assert_eq!(
            restored.sync.top_level,
            TopLevelSyncState::ConfiguredAndCurrent
        );
    }

    #[test]
    fn test_capture_snapshot_does_not_panic() {
        let _snapshot = capture_snapshot();
    }

    #[test]
    fn test_diagnostics_empty_for_clean_state() {
        let dir = make_empty_state_dir();
        let snapshot = StatusSnapshot {
            schema: SNAPSHOT_SCHEMA,
            generated_at_unix_ms: 0,
            config_root: dir.path().to_path_buf(),
            log_dir: dir.path().join("logs"),
            local: LocalSummary {
                libraries: 0,
                snippets: 0,
                primary_library: None,
            },
            sync: SyncSummary {
                configuration: SyncConfigurationState::Configured,
                top_level: TopLevelSyncState::ConfiguredAndCurrent,
            },
            pending: PendingSummary {
                state: PendingStateView::None,
            },
            attempt: default_attempt(),
            execution: ExecutionSummary {
                execution_lock: ProcessStateView::Idle,
                worker_lock: ProcessStateView::Idle,
            },
            diagnostics: Vec::new(),
        };
        let diags = collect_diagnostics(&snapshot);
        assert!(diags.is_empty());
    }

    #[test]
    fn test_diagnostics_severity_order() {
        let dir = make_empty_state_dir();
        let snapshot = StatusSnapshot {
            schema: SNAPSHOT_SCHEMA,
            generated_at_unix_ms: 0,
            config_root: dir.path().to_path_buf(),
            log_dir: dir.path().join("logs"),
            local: LocalSummary {
                libraries: 0,
                snippets: 0,
                primary_library: None,
            },
            sync: SyncSummary {
                configuration: SyncConfigurationState::NotConfigured,
                top_level: TopLevelSyncState::NotConfigured,
            },
            pending: PendingSummary {
                state: PendingStateView::None,
            },
            attempt: default_attempt(),
            execution: ExecutionSummary {
                execution_lock: ProcessStateView::Idle,
                worker_lock: ProcessStateView::Idle,
            },
            diagnostics: Vec::new(),
        };
        let diags = collect_diagnostics(&snapshot);
        assert!(!diags.is_empty());
        assert!(diags.iter().all(|d| d.severity == DiagnosticSeverity::Info));
    }

    #[test]
    fn test_lock_inaccessible() {
        let dir = make_empty_state_dir();
        let path = execution_lock_path(dir.path());
        std::fs::write(&path, "garbage content").unwrap();
        let view = inspect_execution_lock(dir.path());
        assert_eq!(view, ProcessStateView::Malformed);
    }
}
