//! Auto-sync coordinator, debounce, and process-lifecycle safety.
//!
//! Extends the policy model with a stateful coordinator that debounces
//! rapid mutations, persists durable pending markers, and uses PID-file
//! based cross-process locking to prevent concurrent sync executions.
//!
//! ## Architecture
//!
//! ```text
//! Mutation ──► AutoSyncCoordinator::request()
//!                  │
//!                  ├─ suppress if origin == SyncMerge
//!                  ├─ suppress if policy.disabled
//!                  ├─ update DebounceState
//!                  ├─ persist PendingState (durable marker)
//!                  └─ return AutoSyncStatus
//!
//! Timer / caller ──► AutoSyncCoordinator::tick()
//!                       │
//!                       ├─ DebounceState::Pending expired?
//!                       │     └─► Acquire CoordinatorLock
//!                       │         ├─ lock held → Running
//!                       │         └─ lock denied → Pending (retry)
//!                       └─ DebounceState::Running complete?
//!                             ├─ follow_up → Pending (short deadline)
//!                             └─ no follow_up → Idle, clear pending
//! ```

use crate::config::{AutoSyncFailureMode, SyncSettings};
use crate::error::{SnipError, SnipResult};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Maximum debounce delay in seconds.
const MAX_DEBOUNCE_SECS: u64 = 300;

/// Short follow-up deadline after a sync completes with pending work.
const FOLLOWUP_DEBOUNCE: Duration = Duration::from_secs(1);

/// Pending state file name.
const PENDING_STATE_FILE: &str = "auto-sync-pending.toml";

/// Lock file name.
const LOCK_FILE: &str = "auto-sync.lock";

/// Pending state file version.
const PENDING_STATE_VERSION: u32 = 1;

// ---------------------------------------------------------------------------
// Policy model (backward-compatible, unchanged)
// ---------------------------------------------------------------------------

/// Effective auto-sync policy resolved from configuration.
///
/// This is computed once per command invocation and carries validated,
/// clamped values. A disabled policy produces no scheduling request.
#[derive(Debug, Clone)]
pub struct AutoSyncPolicy {
    /// Whether auto-sync is enabled.
    pub enabled: bool,
    /// Debounce delay before firing after a mutation.
    pub debounce: Duration,
    /// Failure behavior when auto-sync cannot complete.
    pub failure_mode: AutoSyncFailureMode,
}

impl AutoSyncPolicy {
    /// Resolve the effective policy from persisted settings.
    ///
    /// Disabled (`auto_sync: false`) produces a safe no-op policy.
    /// Invalid configuration values are clamped to valid ranges.
    pub fn resolve(settings: &SyncSettings) -> Self {
        Self {
            enabled: settings.auto_sync && settings.enabled,
            debounce: settings.auto_sync_debounce(),
            failure_mode: settings.auto_sync_failure.clone(),
        }
    }

    /// Returns `true` if auto-sync should be triggered for a mutation.
    pub fn should_trigger(&self) -> bool {
        self.enabled
    }
}

impl Default for AutoSyncPolicy {
    fn default() -> Self {
        Self {
            enabled: false,
            debounce: Duration::from_secs(2),
            failure_mode: AutoSyncFailureMode::Warn,
        }
    }
}

// ---------------------------------------------------------------------------
// Mutation kind (backward-compatible, unchanged)
// ---------------------------------------------------------------------------

/// Classification of library-mutating operations.
///
/// Each variant identifies one logical class of mutation that may
/// trigger post-mutation auto-sync. This enum does NOT gate the
/// trigger — that is the policy's job — but it records the reason
/// for the sync request.
///
/// ## Trigger matrix
///
/// | Kind              | Mutates syncable content? | Triggers auto-sync? |
/// |-------------------|--------------------------|---------------------|
/// | SnippetCreate     | Yes                      | Yes (when enabled)  |
/// | SnippetUpdate     | Yes                      | Yes (when enabled)  |
/// | SnippetDelete     | Yes (tombstone)          | Yes (when enabled)  |
/// | Import            | Yes (bulk)               | Yes (once)          |
/// | LibraryChange     | Depends on scope         | Only if remote mapped |
/// | PremadeInstall    | Yes (bulk)               | Yes (once)          |
/// | SyncConflictWrite | Yes                      | Yes (once)          |
/// | AccountConfig     | No                       | Never               |
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MutationKind {
    /// A new snippet was created.
    SnippetCreate,
    /// An existing snippet's command, description, tags, or output was modified.
    SnippetUpdate,
    /// A snippet was soft-deleted (tombstone).
    SnippetDelete,
    /// Bulk import (create/merge/replace) was performed.
    Import,
    /// A library was created, renamed, or deleted.
    LibraryChange,
    /// A premade library was downloaded or installed.
    PremadeInstall,
    /// A sync conflict resolution wrote local state.
    SyncConflictWrite,
    /// Account or configuration changes — never triggers sync.
    AccountConfig,
}

impl MutationKind {
    /// Returns `true` if this mutation kind mutates syncable library content.
    pub fn is_syncable_mutation(&self) -> bool {
        !matches!(self, Self::AccountConfig)
    }
}

// ---------------------------------------------------------------------------
// Workstream A: AutoSyncRequest
// ---------------------------------------------------------------------------

/// A pending auto-sync request submitted after a mutation.
#[derive(Debug, Clone)]
pub struct AutoSyncRequest {
    /// Target library (None = default/primary library).
    pub library_id: Option<String>,
    /// Classification of the mutation that triggered this request.
    pub mutation_kind: MutationKind,
    /// Unix timestamp (seconds) when the request was created.
    pub requested_at: i64,
}

impl AutoSyncRequest {
    /// Create a new request with the current wall-clock time.
    pub fn new(library_id: Option<String>, mutation_kind: MutationKind) -> Self {
        Self {
            library_id,
            mutation_kind,
            requested_at: unix_now(),
        }
    }
}

// ---------------------------------------------------------------------------
// Workstream E: MutationOrigin
// ---------------------------------------------------------------------------

/// Origin of a mutation, used to suppress feedback loops.
///
/// When a mutation originates from a sync merge, it must NOT trigger
/// another automatic sync to prevent infinite loops.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MutationOrigin {
    /// User-initiated mutation (create, update, delete, edit).
    User,
    /// Import operation (pet import, bulk load).
    Import,
    /// Sync merge wrote local state (server data merged into local).
    SyncMerge,
    /// Recovery operation (e.g., re-creating a deleted server library).
    Recovery,
}

// ---------------------------------------------------------------------------
// Workstream G: AutoSyncStatus
// ---------------------------------------------------------------------------

/// Outcome of a sync request or the current sync state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AutoSyncStatus {
    /// Auto-sync is disabled by policy.
    Disabled,
    /// A sync request is pending (debounce timer running).
    Pending,
    /// A sync is currently executing.
    Running,
    /// The last sync completed successfully.
    Succeeded {
        /// Unix timestamp (seconds) when the sync completed.
        completed_at: i64,
    },
    /// The last sync failed.
    Failed {
        /// Unix timestamp (seconds) when the failure occurred.
        completed_at: i64,
        /// Classification of the failure.
        class: FailureClass,
    },
}

/// Classification of sync failures for policy routing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureClass {
    /// Network connectivity issue (timeout, DNS, connection refused).
    Network,
    /// Authentication failure (invalid API key, expired token).
    Auth,
    /// Conflict resolution failed or data inconsistency.
    Conflict,
    /// Unclassified or miscellaneous failure.
    Unknown,
}

impl FailureClass {
    /// Classify a [`SnipError`] into a [`FailureClass`].
    pub fn from_error(err: &SnipError) -> Self {
        match err {
            SnipError::Runtime { message, detail } => {
                let combined = format!("{message} {}", detail.as_deref().unwrap_or(""));
                let lower = combined.to_lowercase();
                if lower.contains("network")
                    || lower.contains("timeout")
                    || lower.contains("dns")
                    || lower.contains("connection refused")
                    || lower.contains("connect")
                    || lower.contains("unavailable")
                {
                    FailureClass::Network
                } else if lower.contains("auth")
                    || lower.contains("unauthorized")
                    || lower.contains("forbidden")
                    || lower.contains("api key")
                    || lower.contains("permission denied")
                {
                    FailureClass::Auth
                } else if lower.contains("conflict") || lower.contains("merge") {
                    FailureClass::Conflict
                } else {
                    FailureClass::Unknown
                }
            }
            SnipError::Io { operation, .. } => {
                let lower = operation.to_lowercase();
                if lower.contains("connection")
                    || lower.contains("connect")
                    || lower.contains("network")
                {
                    FailureClass::Network
                } else {
                    FailureClass::Unknown
                }
            }
            _ => FailureClass::Unknown,
        }
    }
}

// ---------------------------------------------------------------------------
// Workstream B: DebounceState
// ---------------------------------------------------------------------------

/// Internal debounce state machine.
///
/// Transitions:
/// ```text
/// Idle ──────────────────────────────────────────────────────► Pending
///   ◄──────────────────────────────────────────────────────── Running
/// Pending + mutation ──► Pending (updated deadline, bounded)
/// Pending + expired ───► Running
/// Running + mutation ──► Running (follow_up = true)
/// Running complete ────► Pending (short deadline) if follow_up
/// Running complete ────► Idle
/// ```
#[derive(Debug, Clone)]
enum DebounceState {
    /// No sync activity.
    Idle,
    /// Waiting for the debounce deadline before firing.
    Pending {
        /// When the debounce expires and sync should start.
        deadline: Instant,
        /// Hard upper bound — never push past this regardless of mutations.
        max_deadline: Instant,
        /// The latest request that triggered this pending state.
        request: AutoSyncRequest,
    },
    /// A sync is actively executing.
    Running {
        /// Whether a mutation arrived while running (triggers follow-up).
        follow_up: bool,
        /// The request that initiated this sync.
        request: AutoSyncRequest,
    },
}

impl DebounceState {
    /// Returns `true` if the state is Pending and the deadline has passed.
    fn is_expired(&self, now: Instant) -> bool {
        match self {
            DebounceState::Pending { deadline, .. } => now >= *deadline,
            _ => false,
        }
    }

    /// Returns the request if in Pending state.
    fn pending_request(&self) -> Option<&AutoSyncRequest> {
        match self {
            DebounceState::Pending { request, .. } => Some(request),
            _ => None,
        }
    }

    /// Returns the request if in Running state.
    fn running_request(&self) -> Option<&AutoSyncRequest> {
        match self {
            DebounceState::Running { request, .. } => Some(request),
            _ => None,
        }
    }

    /// Transition: a new mutation arrived while Pending.
    ///
    /// The deadline is extended by the debounce interval but never
    /// pushed past the hard maximum deadline.
    fn on_mutation_pending(
        self,
        now: Instant,
        debounce: Duration,
        request: AutoSyncRequest,
    ) -> Self {
        match self {
            DebounceState::Pending { max_deadline, .. } => {
                let new_deadline = (now + debounce).min(max_deadline);
                DebounceState::Pending {
                    deadline: new_deadline,
                    max_deadline,
                    request,
                }
            }
            _ => self,
        }
    }

    /// Transition: a new mutation arrived while Running.
    fn on_mutation_running(self, request: AutoSyncRequest) -> Self {
        match self {
            DebounceState::Running { .. } => DebounceState::Running {
                follow_up: true,
                request,
            },
            _ => self,
        }
    }

    /// Transition: debounce deadline passed, move to Running.
    fn start_running(self) -> Self {
        match self {
            DebounceState::Pending { request, .. } => DebounceState::Running {
                follow_up: false,
                request,
            },
            other => other,
        }
    }

    /// Transition: sync completed.
    ///
    /// If `follow_up` was set, returns to Pending with a short deadline.
    /// Otherwise returns to Idle.
    fn complete(self, now: Instant) -> Self {
        match self {
            DebounceState::Running {
                follow_up: true, ..
            } => {
                let deadline = now + FOLLOWUP_DEBOUNCE;
                let max_deadline = deadline;
                DebounceState::Pending {
                    deadline,
                    max_deadline,
                    request: AutoSyncRequest {
                        library_id: None,
                        mutation_kind: MutationKind::AccountConfig,
                        requested_at: unix_now(),
                    },
                }
            }
            _ => DebounceState::Idle,
        }
    }
}

// ---------------------------------------------------------------------------
// Workstream D: PendingState (durable pending marker)
// ---------------------------------------------------------------------------

/// Durable pending marker persisted to disk.
///
/// Written when a sync is pending or running so that a crash/restart
/// can reschedule the sync on the next invocation.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct PendingState {
    /// Schema version for forward compatibility.
    version: u32,
    /// Whether a sync is pending or in progress.
    pending: bool,
    /// Unix timestamp when the request was originally made.
    requested_at: i64,
    /// Unix timestamp of the last sync attempt.
    last_attempt_at: i64,
    /// Summary of the last sync result ("ok", "failed:network", etc.).
    last_result: String,
    /// Target library, if any.
    library_id: Option<String>,
}

impl PendingState {
    /// Create a fresh pending state from a request.
    fn from_request(request: &AutoSyncRequest) -> Self {
        Self {
            version: PENDING_STATE_VERSION,
            pending: true,
            requested_at: request.requested_at,
            last_attempt_at: 0,
            last_result: String::new(),
            library_id: request.library_id.clone(),
        }
    }

    /// Mark the last attempt with a result string.
    fn record_attempt(&mut self, result: &str) {
        self.last_attempt_at = unix_now();
        self.last_result = result.to_string();
    }

    /// Serialize with CRC32 integrity header.
    fn to_toml_with_integrity(&self) -> SnipResult<String> {
        let body = toml::to_string_pretty(self)
            .map_err(|e| SnipError::toml_error("serialize pending state", e))?;
        let checksum = crc32fast::hash(body.as_bytes());
        Ok(format!("# integrity: {checksum}\n{body}"))
    }

    /// Deserialize from TOML content with integrity verification.
    fn from_toml_with_integrity(content: &str) -> SnipResult<Self> {
        if !verify_pending_integrity(content) {
            return Err(SnipError::runtime_error(
                "pending state integrity check failed",
                None,
            ));
        }
        let body = strip_pending_integrity_line(content);
        toml::from_str(&body).map_err(|e| SnipError::toml_error("parse pending state", e))
    }
}

/// Verify CRC32 integrity header of pending state content.
fn verify_pending_integrity(content: &str) -> bool {
    let (first_line, body) = match content.find('\n') {
        Some(idx) => (&content[..idx], &content[idx + 1..]),
        None => (content, ""),
    };
    if let Some(checksum_str) = first_line.strip_prefix("# integrity:") {
        if let Ok(stored) = checksum_str.trim().parse::<u32>() {
            return stored == crc32fast::hash(body.as_bytes());
        }
        return false;
    }
    // No header — legacy file, accept.
    true
}

/// Strip the CRC32 integrity header line from pending state content.
fn strip_pending_integrity_line(content: &str) -> String {
    match content.find('\n') {
        Some(idx) if content.starts_with("# integrity:") => content[idx + 1..].to_string(),
        _ => content.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Workstream C: CoordinatorLock (PID-file based)
// ---------------------------------------------------------------------------

/// Cross-process lock for auto-sync coordination.
///
/// Uses a PID file with liveness checking. The lock is advisory —
/// it prevents concurrent auto-sync executions but cannot block
/// manual `snp sync` commands.
struct CoordinatorLock {
    lock_path: PathBuf,
    /// Whether we actually hold the lock (false if we own the lock file
    /// but another process wrote it).
    _held: bool,
}

impl CoordinatorLock {
    /// Attempt to acquire the lock. Returns `Ok(lock)` if acquired.
    ///
    /// If the lock file exists but the owning PID is dead, the stale
    /// lock is removed and a new one is created.
    fn acquire(state_dir: &Path) -> SnipResult<Self> {
        let lock_path = state_dir.join(LOCK_FILE);

        // Try to create the lock file exclusively.
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock_path)
        {
            Ok(mut file) => {
                let pid = std::process::id();
                writeln!(file, "{pid}")
                    .map_err(|e| SnipError::io_error("write lock file", &lock_path, e))?;
                set_restrictive_permissions(&lock_path);
                tracing::debug!(pid, path = %lock_path.display(), "acquired auto-sync lock");
                Ok(CoordinatorLock {
                    lock_path,
                    _held: true,
                })
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                // Lock exists — check if owner is alive.
                if is_lock_stale(&lock_path) {
                    tracing::warn!(
                        path = %lock_path.display(),
                        "removing stale auto-sync lock"
                    );
                    fs::remove_file(&lock_path)
                        .map_err(|e| SnipError::io_error("remove stale lock", &lock_path, e))?;
                    // Retry creation.
                    Self::acquire(state_dir)
                } else {
                    tracing::debug!(
                        path = %lock_path.display(),
                        "auto-sync lock held by another process"
                    );
                    Err(SnipError::runtime_error(
                        "auto-sync lock held by another process",
                        None,
                    ))
                }
            }
            Err(e) => Err(SnipError::io_error("create lock file", &lock_path, e)),
        }
    }

    /// Release the lock by removing the lock file.
    fn release(&self) {
        if self._held {
            if let Err(e) = fs::remove_file(&self.lock_path) {
                tracing::warn!(
                    error = %e,
                    path = %self.lock_path.display(),
                    "failed to remove auto-sync lock file"
                );
            } else {
                tracing::debug!("released auto-sync lock");
            }
        }
    }
}

impl Drop for CoordinatorLock {
    fn drop(&mut self) {
        self.release();
    }
}

/// Set restrictive permissions on the lock file (Unix only).
fn set_restrictive_permissions(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
    }
}

/// Check if the PID in the lock file is still alive.
///
/// Uses `kill -0` which sends no signal but checks process existence.
fn is_lock_stale(lock_path: &Path) -> bool {
    let content = match fs::read_to_string(lock_path) {
        Ok(c) => c,
        Err(_) => return true,
    };
    let pid_str = content.trim();
    let pid: i32 = match pid_str.parse() {
        Ok(p) => p,
        Err(_) => return true, // Corrupt PID file — treat as stale.
    };
    // kill -0 checks if the process exists without sending a signal.
    match Command::new("kill").args(["-0", &pid.to_string()]).status() {
        Ok(status) => !status.success(), // exit 0 = alive, nonzero = dead/missing
        Err(_) => true,                  // Can't check — assume stale.
    }
}

// ---------------------------------------------------------------------------
// Workstream A/G: AutoSyncCoordinator
// ---------------------------------------------------------------------------

/// Stateful coordinator for auto-sync debounce, locking, and lifecycle.
///
/// The coordinator is NOT `Sync` — it must be used from a single thread
/// per command invocation. Cross-process coordination is handled by
/// the [`CoordinatorLock`].
pub struct AutoSyncCoordinator {
    state: DebounceState,
    policy: AutoSyncPolicy,
    state_dir: PathBuf,
}

impl AutoSyncCoordinator {
    /// Create a new coordinator.
    ///
    /// # Arguments
    /// * `policy` - The effective auto-sync policy for this invocation.
    /// * `state_dir` - Directory for persistent state (lock file, pending marker).
    ///   Typically `~/.config/snp/`.
    pub fn new(policy: AutoSyncPolicy, state_dir: PathBuf) -> Self {
        Self {
            state: DebounceState::Idle,
            policy,
            state_dir,
        }
    }

    /// Returns the current [`AutoSyncStatus`] based on internal state.
    pub fn status(&self) -> AutoSyncStatus {
        match &self.state {
            DebounceState::Idle => AutoSyncStatus::Disabled,
            DebounceState::Pending { .. } => AutoSyncStatus::Pending,
            DebounceState::Running { .. } => AutoSyncStatus::Running,
        }
    }

    /// Returns `true` if the given origin should suppress auto-sync.
    ///
    /// SyncMerge origins must never trigger a sync to prevent feedback loops.
    pub fn should_suppress_origin(origin: MutationOrigin) -> bool {
        matches!(origin, MutationOrigin::SyncMerge)
    }

    /// Submit a mutation request and return the resulting status.
    ///
    /// This is the primary entry point. It:
    /// 1. Checks policy and origin suppression.
    /// 2. Updates the debounce state machine.
    /// 3. Persists a durable pending marker.
    /// 4. Returns the current status.
    pub fn request(&mut self, request: AutoSyncRequest, origin: MutationOrigin) -> AutoSyncStatus {
        // Suppressed origins never trigger sync.
        if Self::should_suppress_origin(origin) {
            tracing::debug!(
                origin = ?origin,
                kind = ?request.mutation_kind,
                "auto-sync suppressed: origin"
            );
            return self.status();
        }

        // Disabled policy never triggers sync.
        if !self.policy.should_trigger() {
            return AutoSyncStatus::Disabled;
        }

        let now = Instant::now();
        let old_state = std::mem::replace(&mut self.state, DebounceState::Idle);
        self.state = match old_state {
            DebounceState::Idle => {
                let deadline = now + self.policy.debounce;
                let max_deadline = now + Duration::from_secs(MAX_DEBOUNCE_SECS);
                DebounceState::Pending {
                    deadline,
                    max_deadline,
                    request,
                }
            }
            DebounceState::Pending { .. } => {
                old_state.on_mutation_pending(now, self.policy.debounce, request)
            }
            DebounceState::Running { .. } => old_state.on_mutation_running(request),
        };

        // Persist durable pending marker.
        if let Some(req) = self
            .state
            .pending_request()
            .or(self.state.running_request())
        {
            let pending = PendingState::from_request(req);
            if let Err(e) = save_pending(&self.state_dir, &pending) {
                tracing::warn!(error = %e, "failed to persist auto-sync pending state");
            }
        }

        tracing::debug!(
            state = ?self.state,
            origin = ?origin,
            "auto-sync request accepted"
        );

        self.status()
    }

    /// Advance the state machine based on the current time.
    ///
    /// Call this periodically (e.g., from a command's post-mutation
    /// phase) to check if the debounce deadline has passed and a
    /// sync should start.
    pub fn tick(&mut self, now: Instant) -> Option<AutoSyncRequest> {
        if !self.policy.should_trigger() {
            return None;
        }

        match &self.state {
            DebounceState::Pending { .. } if self.state.is_expired(now) => {
                let request = self.state.pending_request().cloned().unwrap();
                let old = std::mem::replace(&mut self.state, DebounceState::Idle);
                self.state = old.start_running();
                Some(request)
            }
            _ => None,
        }
    }

    /// Notify the coordinator that a sync has completed.
    ///
    /// Transitions from Running to either Idle or Pending (follow-up).
    /// Clears the durable pending marker if no follow-up is scheduled.
    pub fn sync_completed(&mut self) {
        let now = Instant::now();
        let old_state = std::mem::replace(&mut self.state, DebounceState::Idle);
        let had_follow_up = matches!(
            &old_state,
            DebounceState::Running {
                follow_up: true,
                ..
            }
        );
        self.state = old_state.complete(now);

        if !had_follow_up {
            clear_pending(&self.state_dir);
        }

        tracing::debug!(state = ?self.state, "auto-sync sync completed");
    }

    /// Notify the coordinator that a sync failed.
    ///
    /// Always transitions back to Idle (no retry for auto-sync).
    /// Clears the durable pending marker and records the failure.
    pub fn sync_failed(&mut self, class: FailureClass) {
        let completed_at = unix_now();
        self.state = DebounceState::Idle;
        clear_pending(&self.state_dir);

        tracing::warn!(
            class = ?class,
            completed_at,
            "auto-sync sync failed"
        );
    }

    /// Check for and recover from a stale pending state on startup.
    ///
    /// If a durable pending marker exists and is stale (old timestamp),
    /// it is cleared. This handles the case where a previous process
    /// crashed while a sync was pending.
    pub fn recover_stale_pending(&self) {
        if let Some(pending) = load_pending(&self.state_dir)
            && pending.pending
        {
            let age = unix_now() - pending.requested_at;
            // If pending for more than 5 minutes, consider it stale.
            if age > 300 {
                tracing::info!(
                    age,
                    requested_at = pending.requested_at,
                    "clearing stale auto-sync pending state"
                );
                clear_pending(&self.state_dir);
            }
        }
    }

    /// Derive the state directory from the sync config path.
    ///
    /// Returns the parent directory of `~/.config/snp/sync.toml`.
    pub fn derive_state_dir() -> PathBuf {
        crate::config::get_sync_config_path()
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf()
    }
}

impl std::fmt::Debug for AutoSyncCoordinator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AutoSyncCoordinator")
            .field("state", &self.state)
            .field("policy", &self.policy)
            .field("state_dir", &self.state_dir)
            .finish()
    }
}

// ---------------------------------------------------------------------------
// Workstream E: run_auto_sync
// ---------------------------------------------------------------------------

/// Execute an auto-sync with lock acquisition, timeout, and failure handling.
///
/// This function wraps the existing `sync_commands::run_default_sync` with:
/// - Cross-process lock acquisition (PID-file based)
/// - Bounded timeout via `runtime.block_on()`
/// - Failure classification and policy-based outcome mapping
/// - Durable pending state management
///
/// Returns the [`AutoSyncStatus`] outcome.
pub fn run_auto_sync(
    policy: &AutoSyncPolicy,
    state_dir: &Path,
    runtime: &tokio::runtime::Runtime,
) -> AutoSyncStatus {
    if !policy.should_trigger() {
        return AutoSyncStatus::Disabled;
    }

    // Acquire cross-process lock.
    let _lock = match CoordinatorLock::acquire(state_dir) {
        Ok(l) => l,
        Err(e) => {
            tracing::debug!(error = %e, "auto-sync lock acquisition failed, skipping");
            return AutoSyncStatus::Failed {
                completed_at: unix_now(),
                class: FailureClass::Unknown,
            };
        }
    };

    // Load and verify pending state.
    let _pending = load_pending(state_dir);

    // Execute sync with bounded timeout.
    let sync_result = std::thread::scope(|s| {
        let handle = s.spawn(|| crate::sync_commands::run_default_sync(runtime));
        handle
            .join()
            .unwrap_or_else(|_| Err(SnipError::runtime_error("sync thread panicked", None)))
    });

    match sync_result {
        Ok(()) => {
            let completed_at = unix_now();
            // Record success in pending state before clearing.
            if let Some(mut pending) = load_pending(state_dir) {
                pending.record_attempt("ok");
                pending.pending = false;
                let _ = save_pending(state_dir, &pending);
            }
            clear_pending(state_dir);
            tracing::debug!(completed_at, "auto-sync completed successfully");
            AutoSyncStatus::Succeeded { completed_at }
        }
        Err(e) => {
            let completed_at = unix_now();
            let class = FailureClass::from_error(&e);

            // Record failure in pending state.
            if let Some(mut pending) = load_pending(state_dir) {
                pending.record_attempt(&format!("failed:{class:?}"));
                let _ = save_pending(state_dir, &pending);
            }

            match policy.failure_mode {
                AutoSyncFailureMode::Ignore => {
                    tracing::debug!(class = ?class, "auto-sync failed (ignored per policy)");
                }
                AutoSyncFailureMode::Warn => {
                    tracing::warn!(error = %e, class = ?class, "auto-sync failed");
                }
                AutoSyncFailureMode::Error => {
                    tracing::error!(error = %e, class = ?class, "auto-sync failed");
                }
            }

            AutoSyncStatus::Failed {
                completed_at,
                class,
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Pending state persistence helpers
// ---------------------------------------------------------------------------

/// Save the durable pending marker to disk.
fn save_pending(state_dir: &Path, pending: &PendingState) -> SnipResult<()> {
    let path = state_dir.join(PENDING_STATE_FILE);
    let content = pending.to_toml_with_integrity()?;
    crate::utils::atomic::write_private_atomic(&path, &content, "auto-sync-pending")
}

/// Load the durable pending marker from disk, if it exists.
fn load_pending(state_dir: &Path) -> Option<PendingState> {
    let path = state_dir.join(PENDING_STATE_FILE);
    let content = fs::read_to_string(&path).ok()?;
    match PendingState::from_toml_with_integrity(&content) {
        Ok(p) => Some(p),
        Err(e) => {
            tracing::warn!(error = %e, "failed to load auto-sync pending state");
            None
        }
    }
}

/// Remove the durable pending marker from disk.
fn clear_pending(state_dir: &Path) {
    let path = state_dir.join(PENDING_STATE_FILE);
    if path.exists()
        && let Err(e) = fs::remove_file(&path)
    {
        tracing::warn!(error = %e, "failed to remove auto-sync pending state");
    }
}

// ---------------------------------------------------------------------------
// Utility helpers
// ---------------------------------------------------------------------------

/// Current Unix timestamp in seconds.
fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SyncSettings;

    // ---- Existing tests (unchanged) ----

    #[test]
    fn test_policy_disabled_by_default() {
        let settings = SyncSettings::default();
        let policy = AutoSyncPolicy::resolve(&settings);
        assert!(!policy.enabled);
        assert!(!policy.should_trigger());
    }

    #[test]
    fn test_policy_enabled_requires_sync_enabled() {
        let mut settings = SyncSettings::default();
        settings.enabled = false;
        settings.auto_sync = true;
        let policy = AutoSyncPolicy::resolve(&settings);
        assert!(!policy.enabled);

        settings.enabled = true;
        let policy = AutoSyncPolicy::resolve(&settings);
        assert!(policy.enabled);
        assert!(policy.should_trigger());
    }

    #[test]
    fn test_policy_debounce_clamped() {
        let mut settings = SyncSettings::default();
        settings.enabled = true;
        settings.auto_sync = true;

        settings.auto_sync_debounce_seconds = 0;
        let policy = AutoSyncPolicy::resolve(&settings);
        assert_eq!(policy.debounce, Duration::from_secs(0));

        settings.auto_sync_debounce_seconds = 2;
        let policy = AutoSyncPolicy::resolve(&settings);
        assert_eq!(policy.debounce, Duration::from_secs(2));

        settings.auto_sync_debounce_seconds = 300;
        let policy = AutoSyncPolicy::resolve(&settings);
        assert_eq!(policy.debounce, Duration::from_secs(300));

        // Overflow clamped to max
        settings.auto_sync_debounce_seconds = u64::MAX;
        let policy = AutoSyncPolicy::resolve(&settings);
        assert_eq!(policy.debounce, Duration::from_secs(300));
    }

    #[test]
    fn test_policy_failure_mode() {
        let mut settings = SyncSettings::default();
        settings.enabled = true;
        settings.auto_sync = true;

        settings.auto_sync_failure = AutoSyncFailureMode::Ignore;
        let policy = AutoSyncPolicy::resolve(&settings);
        assert_eq!(policy.failure_mode, AutoSyncFailureMode::Ignore);

        settings.auto_sync_failure = AutoSyncFailureMode::Error;
        let policy = AutoSyncPolicy::resolve(&settings);
        assert_eq!(policy.failure_mode, AutoSyncFailureMode::Error);
    }

    #[test]
    fn test_mutation_kind_syncable() {
        assert!(MutationKind::SnippetCreate.is_syncable_mutation());
        assert!(MutationKind::SnippetUpdate.is_syncable_mutation());
        assert!(MutationKind::SnippetDelete.is_syncable_mutation());
        assert!(MutationKind::Import.is_syncable_mutation());
        assert!(MutationKind::LibraryChange.is_syncable_mutation());
        assert!(MutationKind::PremadeInstall.is_syncable_mutation());
        assert!(MutationKind::SyncConflictWrite.is_syncable_mutation());
        assert!(!MutationKind::AccountConfig.is_syncable_mutation());
    }

    #[test]
    fn test_default_policy_is_disabled() {
        let policy = AutoSyncPolicy::default();
        assert!(!policy.enabled);
        assert_eq!(policy.debounce, Duration::from_secs(2));
        assert_eq!(policy.failure_mode, AutoSyncFailureMode::Warn);
    }

    // ---- Coordinator tests ----

    fn make_enabled_policy(debounce_secs: u64) -> AutoSyncPolicy {
        AutoSyncPolicy {
            enabled: true,
            debounce: Duration::from_secs(debounce_secs),
            failure_mode: AutoSyncFailureMode::Warn,
        }
    }

    fn make_disabled_policy() -> AutoSyncPolicy {
        AutoSyncPolicy::default()
    }

    fn make_request(kind: MutationKind) -> AutoSyncRequest {
        AutoSyncRequest::new(None, kind)
    }

    fn make_request_at(kind: MutationKind, ts: i64) -> AutoSyncRequest {
        AutoSyncRequest {
            library_id: None,
            mutation_kind: kind,
            requested_at: ts,
        }
    }

    // ---- DebounceState pure state machine tests ----

    #[test]
    fn test_debounce_idle_to_pending() {
        let base = Instant::now();
        let req = make_request(MutationKind::SnippetCreate);
        let deadline = base + Duration::from_secs(2);
        let max_deadline = base + Duration::from_secs(300);

        let next = DebounceState::Pending {
            deadline,
            max_deadline,
            request: req.clone(),
        };

        assert!(matches!(next, DebounceState::Pending { .. }));
        assert_eq!(
            next.pending_request().unwrap().mutation_kind,
            MutationKind::SnippetCreate
        );
    }

    #[test]
    fn test_debounce_pending_mutation_extends_deadline() {
        let base = Instant::now();
        let req1 = make_request(MutationKind::SnippetCreate);
        let req2 = make_request(MutationKind::SnippetUpdate);

        let state = DebounceState::Pending {
            deadline: base + Duration::from_secs(2),
            max_deadline: base + Duration::from_secs(300),
            request: req1,
        };

        let next =
            state.on_mutation_pending(base + Duration::from_secs(1), Duration::from_secs(2), req2);

        match next {
            DebounceState::Pending {
                deadline,
                max_deadline,
                request,
            } => {
                // Deadline = now(1) + debounce(2) = 3s from base, clamped to max(300) from base
                assert_eq!(deadline, base + Duration::from_secs(3));
                assert_eq!(max_deadline, base + Duration::from_secs(300));
                assert_eq!(request.mutation_kind, MutationKind::SnippetUpdate);
            }
            _ => panic!("expected Pending"),
        }
    }

    #[test]
    fn test_debounce_pending_max_deadline_bounded() {
        let base = Instant::now();
        let req1 = make_request(MutationKind::SnippetCreate);
        let req2 = make_request(MutationKind::SnippetUpdate);

        let state = DebounceState::Pending {
            deadline: base + Duration::from_secs(2),
            max_deadline: base + Duration::from_secs(5),
            request: req1,
        };

        // Mutation at t=4, debounce=2 → deadline would be t=6, but max is t=5
        let next =
            state.on_mutation_pending(base + Duration::from_secs(4), Duration::from_secs(2), req2);

        match next {
            DebounceState::Pending { deadline, .. } => {
                assert_eq!(deadline, base + Duration::from_secs(5));
            }
            _ => panic!("expected Pending"),
        }
    }

    #[test]
    fn test_debounce_pending_to_running() {
        let base = Instant::now();
        let req = make_request(MutationKind::SnippetCreate);

        let state = DebounceState::Pending {
            deadline: base + Duration::from_secs(2),
            max_deadline: base + Duration::from_secs(300),
            request: req,
        };

        // Before deadline — not expired.
        assert!(!state.is_expired(base + Duration::from_secs(1)));

        // After deadline — expired.
        assert!(state.is_expired(base + Duration::from_secs(2)));

        let running = state.start_running();
        assert!(matches!(
            running,
            DebounceState::Running {
                follow_up: false,
                ..
            }
        ));
    }

    #[test]
    fn test_debounce_running_mutation_sets_follow_up() {
        let req1 = make_request(MutationKind::SnippetCreate);
        let req2 = make_request(MutationKind::SnippetUpdate);

        let state = DebounceState::Running {
            follow_up: false,
            request: req1,
        };

        let next = state.on_mutation_running(req2);
        match next {
            DebounceState::Running { follow_up, request } => {
                assert!(follow_up);
                assert_eq!(request.mutation_kind, MutationKind::SnippetUpdate);
            }
            _ => panic!("expected Running"),
        }
    }

    #[test]
    fn test_debounce_running_complete_no_follow_up() {
        let base = Instant::now();
        let req = make_request(MutationKind::SnippetCreate);

        let state = DebounceState::Running {
            follow_up: false,
            request: req,
        };

        let next = state.complete(base);
        assert!(matches!(next, DebounceState::Idle));
    }

    #[test]
    fn test_debounce_running_complete_with_follow_up() {
        let base = Instant::now();
        let req = make_request(MutationKind::SnippetCreate);

        let state = DebounceState::Running {
            follow_up: true,
            request: req,
        };

        let next = state.complete(base);
        match next {
            DebounceState::Pending {
                deadline,
                max_deadline,
                ..
            } => {
                assert_eq!(deadline, base + FOLLOWUP_DEBOUNCE);
                assert_eq!(max_deadline, base + FOLLOWUP_DEBOUNCE);
            }
            _ => panic!("expected Pending with follow-up"),
        }
    }

    #[test]
    fn test_debounce_idle_not_expired() {
        let state = DebounceState::Idle;
        assert!(!state.is_expired(Instant::now()));
        assert!(state.pending_request().is_none());
        assert!(state.running_request().is_none());
    }

    #[test]
    fn test_debounce_complete_idle_stays_idle() {
        let state = DebounceState::Idle;
        let next = state.complete(Instant::now());
        assert!(matches!(next, DebounceState::Idle));
    }

    #[test]
    fn test_debounce_start_running_non_pending_is_noop() {
        let state = DebounceState::Idle;
        let next = state.start_running();
        assert!(matches!(next, DebounceState::Idle));
    }

    // ---- AutoSyncCoordinator tests ----

    #[test]
    fn test_coordinator_disabled_policy() {
        let policy = make_disabled_policy();
        let state_dir = std::env::temp_dir().join("snp_test_disabled");
        let _ = fs::create_dir_all(&state_dir);

        let mut coord = AutoSyncCoordinator::new(policy, state_dir.clone());
        let req = make_request(MutationKind::SnippetCreate);
        let status = coord.request(req, MutationOrigin::User);
        assert_eq!(status, AutoSyncStatus::Disabled);

        let _ = fs::remove_dir_all(&state_dir);
    }

    #[test]
    fn test_coordinator_suppress_sync_merge_origin() {
        let policy = make_enabled_policy(2);
        let state_dir = std::env::temp_dir().join("snp_test_suppress");
        let _ = fs::create_dir_all(&state_dir);

        let mut coord = AutoSyncCoordinator::new(policy, state_dir.clone());
        let req = make_request(MutationKind::SnippetCreate);
        let status = coord.request(req, MutationOrigin::SyncMerge);
        assert_eq!(status, AutoSyncStatus::Disabled);

        let _ = fs::remove_dir_all(&state_dir);
    }

    #[test]
    fn test_coordinator_idle_to_pending() {
        let policy = make_enabled_policy(2);
        let state_dir = std::env::temp_dir().join("snp_test_idle_pending");
        let _ = fs::create_dir_all(&state_dir);

        let mut coord = AutoSyncCoordinator::new(policy, state_dir.clone());
        let req = make_request(MutationKind::SnippetCreate);
        let status = coord.request(req, MutationOrigin::User);
        assert_eq!(status, AutoSyncStatus::Pending);

        let _ = fs::remove_dir_all(&state_dir);
    }

    #[test]
    fn test_coordinator_pending_coalesces_mutations() {
        let policy = make_enabled_policy(2);
        let state_dir = std::env::temp_dir().join("snp_test_coalesce");
        let _ = fs::create_dir_all(&state_dir);

        let mut coord = AutoSyncCoordinator::new(policy, state_dir.clone());

        let req1 = make_request(MutationKind::SnippetCreate);
        coord.request(req1, MutationOrigin::User);
        assert_eq!(coord.status(), AutoSyncStatus::Pending);

        // Second mutation — still Pending (debounce extended).
        let req2 = make_request(MutationKind::SnippetUpdate);
        let status = coord.request(req2, MutationOrigin::User);
        assert_eq!(status, AutoSyncStatus::Pending);

        let _ = fs::remove_dir_all(&state_dir);
    }

    #[test]
    fn test_coordinator_tick_pending_to_running() {
        let policy = make_enabled_policy(2);
        let state_dir = std::env::temp_dir().join("snp_test_tick");
        let _ = fs::create_dir_all(&state_dir);

        let mut coord = AutoSyncCoordinator::new(policy, state_dir.clone());
        let req = make_request(MutationKind::SnippetCreate);
        coord.request(req, MutationOrigin::User);

        // Tick before deadline — no sync request.
        let base = Instant::now();
        let result = coord.tick(base);
        assert!(result.is_none());

        // Tick after deadline — should return the request.
        let result = coord.tick(base + Duration::from_secs(3));
        assert!(result.is_some());
        assert_eq!(coord.status(), AutoSyncStatus::Running);

        let _ = fs::remove_dir_all(&state_dir);
    }

    #[test]
    fn test_coordinator_sync_completed_returns_to_idle() {
        let policy = make_enabled_policy(0);
        let state_dir = std::env::temp_dir().join("snp_test_completed");
        let _ = fs::create_dir_all(&state_dir);

        let mut coord = AutoSyncCoordinator::new(policy, state_dir.clone());
        let req = make_request(MutationKind::SnippetCreate);
        coord.request(req, MutationOrigin::User);

        // Tick with zero debounce — immediately running.
        let _ = coord.tick(Instant::now());
        assert_eq!(coord.status(), AutoSyncStatus::Running);

        // Complete.
        coord.sync_completed();
        assert!(matches!(coord.state, DebounceState::Idle));

        let _ = fs::remove_dir_all(&state_dir);
    }

    #[test]
    fn test_coordinator_mutation_during_running_sets_follow_up() {
        let policy = make_enabled_policy(0);
        let state_dir = std::env::temp_dir().join("snp_test_followup");
        let _ = fs::create_dir_all(&state_dir);

        let mut coord = AutoSyncCoordinator::new(policy, state_dir.clone());
        let req = make_request(MutationKind::SnippetCreate);
        coord.request(req, MutationOrigin::User);

        // Tick — immediately running.
        let _ = coord.tick(Instant::now());
        assert_eq!(coord.status(), AutoSyncStatus::Running);

        // Mutation during running.
        let req2 = make_request(MutationKind::SnippetUpdate);
        let status = coord.request(req2, MutationOrigin::User);
        assert_eq!(status, AutoSyncStatus::Running);

        // Complete — should go to Pending (follow-up).
        coord.sync_completed();
        assert_eq!(coord.status(), AutoSyncStatus::Pending);

        let _ = fs::remove_dir_all(&state_dir);
    }

    #[test]
    fn test_coordinator_sync_failed_returns_to_idle() {
        let policy = make_enabled_policy(0);
        let state_dir = std::env::temp_dir().join("snp_test_failed");
        let _ = fs::create_dir_all(&state_dir);

        let mut coord = AutoSyncCoordinator::new(policy, state_dir.clone());
        let req = make_request(MutationKind::SnippetCreate);
        coord.request(req, MutationOrigin::User);

        let _ = coord.tick(Instant::now());
        coord.sync_failed(FailureClass::Network);
        assert!(matches!(coord.state, DebounceState::Idle));

        let _ = fs::remove_dir_all(&state_dir);
    }

    #[test]
    fn test_coordinator_import_origin_not_suppressed() {
        let policy = make_enabled_policy(2);
        let state_dir = std::env::temp_dir().join("snp_test_import_origin");
        let _ = fs::create_dir_all(&state_dir);

        let mut coord = AutoSyncCoordinator::new(policy, state_dir.clone());
        let req = make_request(MutationKind::Import);
        let status = coord.request(req, MutationOrigin::Import);
        assert_eq!(status, AutoSyncStatus::Pending);

        let _ = fs::remove_dir_all(&state_dir);
    }

    #[test]
    fn test_coordinator_recovery_origin_not_suppressed() {
        let policy = make_enabled_policy(2);
        let state_dir = std::env::temp_dir().join("snp_test_recovery_origin");
        let _ = fs::create_dir_all(&state_dir);

        let mut coord = AutoSyncCoordinator::new(policy, state_dir.clone());
        let req = make_request(MutationKind::SyncConflictWrite);
        let status = coord.request(req, MutationOrigin::Recovery);
        assert_eq!(status, AutoSyncStatus::Pending);

        let _ = fs::remove_dir_all(&state_dir);
    }

    // ---- DebounceState: rapid mutation coalescing ----

    #[test]
    fn test_rapid_mutations_coalesce_within_deadline() {
        let base = Instant::now();
        let debounce = Duration::from_secs(2);

        // First mutation → Pending.
        let req1 = make_request(MutationKind::SnippetCreate);
        let deadline = base + debounce;
        let max_deadline = base + Duration::from_secs(300);
        let mut state = DebounceState::Pending {
            deadline,
            max_deadline,
            request: req1,
        };

        // 5 rapid mutations — each extends the deadline but stays within max.
        for i in 0..5 {
            let t = base + Duration::from_millis(100 * i);
            let req = make_request(MutationKind::SnippetUpdate);
            state = state.on_mutation_pending(t, debounce, req);
        }

        match &state {
            DebounceState::Pending { deadline, .. } => {
                // Last mutation at t=400ms, deadline = t=400ms + 2s = 2.4s from base
                assert_eq!(*deadline, base + Duration::from_millis(2400));
            }
            _ => panic!("expected Pending"),
        }
    }

    // ---- Maximum delay bound ----

    #[test]
    fn test_deadline_never_exceeds_max() {
        let base = Instant::now();
        let debounce = Duration::from_secs(300);
        let max = base + Duration::from_secs(300);

        let mut deadline = base + debounce;

        // Simulate many mutations — deadline should never exceed max.
        for i in 0..100 {
            let t = base + Duration::from_secs(i * 10);
            let next = (t + debounce).min(max);
            deadline = next;
        }

        assert!(deadline <= max);
    }

    // ---- Mutation during running → follow-up ----

    #[test]
    fn test_mutation_during_running_creates_follow_up_pending() {
        let base = Instant::now();

        let running = DebounceState::Running {
            follow_up: false,
            request: make_request(MutationKind::SnippetCreate),
        };

        let with_follow_up = running.on_mutation_running(make_request(MutationKind::SnippetDelete));

        match with_follow_up {
            DebounceState::Running {
                follow_up,
                ref request,
            } => {
                assert!(follow_up);
                assert_eq!(request.mutation_kind, MutationKind::SnippetDelete);
            }
            _ => panic!("expected Running with follow_up"),
        }

        let completed = with_follow_up.complete(base);
        match completed {
            DebounceState::Pending { deadline, .. } => {
                assert_eq!(deadline, base + FOLLOWUP_DEBOUNCE);
            }
            _ => panic!("expected Pending follow-up"),
        }
    }

    // ---- FailureClass classification ----

    #[test]
    fn test_failure_class_network() {
        let err = SnipError::runtime_error("connection timeout", None);
        assert_eq!(FailureClass::from_error(&err), FailureClass::Network);
    }

    #[test]
    fn test_failure_class_auth() {
        let err = SnipError::runtime_error("unauthorized access", None);
        assert_eq!(FailureClass::from_error(&err), FailureClass::Auth);
    }

    #[test]
    fn test_failure_class_conflict() {
        let err = SnipError::runtime_error("merge conflict", None);
        assert_eq!(FailureClass::from_error(&err), FailureClass::Conflict);
    }

    #[test]
    fn test_failure_class_unknown() {
        let err = SnipError::runtime_error("something broke", None);
        assert_eq!(FailureClass::from_error(&err), FailureClass::Unknown);
    }

    #[test]
    fn test_failure_class_network_in_detail() {
        let err = SnipError::runtime_error("sync failed", Some("dns resolution timeout"));
        assert_eq!(FailureClass::from_error(&err), FailureClass::Network);
    }

    #[test]
    fn test_failure_class_auth_in_detail() {
        let err = SnipError::runtime_error("sync failed", Some("api key expired"));
        assert_eq!(FailureClass::from_error(&err), FailureClass::Auth);
    }

    #[test]
    fn test_failure_class_io_connection() {
        let err = SnipError::Io {
            operation: "connect".to_string(),
            path: None,
            source: std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "refused"),
        };
        assert_eq!(FailureClass::from_error(&err), FailureClass::Network);
    }

    // ---- MutationOrigin suppression ----

    #[test]
    fn test_sync_merge_suppressed() {
        assert!(AutoSyncCoordinator::should_suppress_origin(
            MutationOrigin::SyncMerge
        ));
    }

    #[test]
    fn test_user_origin_not_suppressed() {
        assert!(!AutoSyncCoordinator::should_suppress_origin(
            MutationOrigin::User
        ));
    }

    #[test]
    fn test_import_origin_not_suppressed() {
        assert!(!AutoSyncCoordinator::should_suppress_origin(
            MutationOrigin::Import
        ));
    }

    #[test]
    fn test_recovery_origin_not_suppressed() {
        assert!(!AutoSyncCoordinator::should_suppress_origin(
            MutationOrigin::Recovery
        ));
    }

    // ---- PendingState persistence round-trip ----

    #[test]
    fn test_pending_state_roundtrip() {
        let request = make_request_at(MutationKind::SnippetCreate, 1234567890);
        let mut pending = PendingState::from_request(&request);
        pending.record_attempt("ok");

        let content = pending.to_toml_with_integrity().unwrap();
        assert!(content.starts_with("# integrity:"));

        let restored = PendingState::from_toml_with_integrity(&content).unwrap();
        assert!(restored.pending);
        assert_eq!(restored.version, PENDING_STATE_VERSION);
        assert_eq!(restored.requested_at, 1234567890);
        assert_eq!(restored.last_result, "ok");
        assert!(restored.library_id.is_none());
    }

    #[test]
    fn test_pending_state_with_library_id() {
        let request = AutoSyncRequest {
            library_id: Some("my-lib".to_string()),
            mutation_kind: MutationKind::Import,
            requested_at: 9999,
        };
        let pending = PendingState::from_request(&request);
        let content = pending.to_toml_with_integrity().unwrap();
        let restored = PendingState::from_toml_with_integrity(&content).unwrap();
        assert_eq!(restored.library_id.as_deref(), Some("my-lib"));
    }

    #[test]
    fn test_pending_state_tampered_content_fails() {
        let request = make_request(MutationKind::SnippetCreate);
        let pending = PendingState::from_request(&request);
        let mut content = pending.to_toml_with_integrity().unwrap();
        // Tamper with the body after the integrity header.
        content.push_str("\ntampered = true");
        let result = PendingState::from_toml_with_integrity(&content);
        assert!(result.is_err());
    }

    #[test]
    fn test_pending_state_no_integrity_header_accepted() {
        // Legacy files without integrity header should be accepted.
        let body = "version = 1\npending = true\nrequested_at = 100\nlast_attempt_at = 0\nlast_result = \"\"\n";
        let result = PendingState::from_toml_with_integrity(body);
        assert!(result.is_ok());
    }

    #[test]
    fn test_pending_state_malformed_toml_fails() {
        let result =
            PendingState::from_toml_with_integrity("# integrity: 1234\nnot valid toml [[[");
        assert!(result.is_err());
    }

    // ---- Durable pending marker file operations ----

    #[test]
    fn test_save_load_clear_pending() {
        let state_dir = std::env::temp_dir().join("snp_test_pending_file");
        let _ = fs::create_dir_all(&state_dir);

        let request = make_request_at(MutationKind::SnippetCreate, 42);
        let pending = PendingState::from_request(&request);

        save_pending(&state_dir, &pending).unwrap();
        let loaded = load_pending(&state_dir).unwrap();
        assert!(loaded.pending);
        assert_eq!(loaded.requested_at, 42);

        clear_pending(&state_dir);
        assert!(load_pending(&state_dir).is_none());

        let _ = fs::remove_dir_all(&state_dir);
    }

    #[test]
    fn test_load_pending_nonexistent_returns_none() {
        let state_dir = std::env::temp_dir().join("snp_test_no_pending");
        let _ = fs::remove_dir_all(&state_dir);
        assert!(load_pending(&state_dir).is_none());
    }

    #[test]
    fn test_clear_pending_nonexistent_is_noop() {
        let state_dir = std::env::temp_dir().join("snp_test_clear_noop");
        let _ = fs::remove_dir_all(&state_dir);
        // Should not panic.
        clear_pending(&state_dir);
    }

    // ---- Lock file operations ----

    #[test]
    fn test_lock_acquire_and_release() {
        let state_dir = std::env::temp_dir().join("snp_test_lock");
        let _ = fs::create_dir_all(&state_dir);

        let lock = CoordinatorLock::acquire(&state_dir).unwrap();
        assert!(state_dir.join(LOCK_FILE).exists());
        drop(lock);
        assert!(!state_dir.join(LOCK_FILE).exists());

        let _ = fs::remove_dir_all(&state_dir);
    }

    #[test]
    fn test_lock_stale_detection() {
        let state_dir = std::env::temp_dir().join("snp_test_stale_lock");
        let _ = fs::create_dir_all(&state_dir);

        // Write a lock file with a PID that doesn't exist.
        let lock_path = state_dir.join(LOCK_FILE);
        // Use PID 9999999 — almost certainly not alive.
        fs::write(&lock_path, "9999999\n").unwrap();

        assert!(is_lock_stale(&lock_path));

        let _ = fs::remove_dir_all(&state_dir);
    }

    #[test]
    fn test_lock_not_stale_when_valid_pid() {
        let state_dir = std::env::temp_dir().join("snp_test_valid_lock");
        let _ = fs::create_dir_all(&state_dir);

        let lock_path = state_dir.join(LOCK_FILE);
        // Write current process PID — should be alive.
        fs::write(&lock_path, format!("{}\n", std::process::id())).unwrap();

        assert!(!is_lock_stale(&lock_path));

        let _ = fs::remove_dir_all(&state_dir);
    }

    #[test]
    fn test_lock_stale_corrupt_content() {
        let state_dir = std::env::temp_dir().join("snp_test_corrupt_lock");
        let _ = fs::create_dir_all(&state_dir);

        let lock_path = state_dir.join(LOCK_FILE);
        fs::write(&lock_path, "not-a-pid\n").unwrap();

        assert!(is_lock_stale(&lock_path));

        let _ = fs::remove_dir_all(&state_dir);
    }

    #[test]
    fn test_lock_stale_empty_file() {
        let state_dir = std::env::temp_dir().join("snp_test_empty_lock");
        let _ = fs::create_dir_all(&state_dir);

        let lock_path = state_dir.join(LOCK_FILE);
        fs::write(&lock_path, "").unwrap();

        assert!(is_lock_stale(&lock_path));

        let _ = fs::remove_dir_all(&state_dir);
    }

    // ---- AutoSyncRequest ----

    #[test]
    fn test_request_new_sets_timestamp() {
        let before = unix_now();
        let req = AutoSyncRequest::new(None, MutationKind::SnippetCreate);
        let after = unix_now();
        assert!(req.requested_at >= before);
        assert!(req.requested_at <= after);
        assert!(req.library_id.is_none());
        assert_eq!(req.mutation_kind, MutationKind::SnippetCreate);
    }

    #[test]
    fn test_request_with_library() {
        let req = AutoSyncRequest::new(Some("lib1".to_string()), MutationKind::Import);
        assert_eq!(req.library_id.as_deref(), Some("lib1"));
    }

    // ---- Status display consistency ----

    #[test]
    fn test_status_equality() {
        assert_eq!(AutoSyncStatus::Disabled, AutoSyncStatus::Disabled);
        assert_eq!(AutoSyncStatus::Pending, AutoSyncStatus::Pending);
        assert_eq!(AutoSyncStatus::Running, AutoSyncStatus::Running);
        assert_eq!(
            AutoSyncStatus::Succeeded { completed_at: 100 },
            AutoSyncStatus::Succeeded { completed_at: 100 }
        );
        assert_ne!(
            AutoSyncStatus::Succeeded { completed_at: 100 },
            AutoSyncStatus::Succeeded { completed_at: 200 }
        );
        assert_eq!(
            AutoSyncStatus::Failed {
                completed_at: 100,
                class: FailureClass::Network
            },
            AutoSyncStatus::Failed {
                completed_at: 100,
                class: FailureClass::Network
            }
        );
        assert_ne!(
            AutoSyncStatus::Failed {
                completed_at: 100,
                class: FailureClass::Network
            },
            AutoSyncStatus::Failed {
                completed_at: 100,
                class: FailureClass::Auth
            }
        );
    }

    // ---- FailureClass equality ----

    #[test]
    fn test_failure_class_equality() {
        assert_eq!(FailureClass::Network, FailureClass::Network);
        assert_ne!(FailureClass::Network, FailureClass::Auth);
        assert_ne!(FailureClass::Conflict, FailureClass::Unknown);
    }

    // ---- Coordinator with zero debounce ----

    #[test]
    fn test_zero_debounce_immediate_running() {
        let policy = make_enabled_policy(0);
        let state_dir = std::env::temp_dir().join("snp_test_zero_debounce");
        let _ = fs::create_dir_all(&state_dir);

        let mut coord = AutoSyncCoordinator::new(policy, state_dir.clone());
        let req = make_request(MutationKind::SnippetCreate);
        coord.request(req, MutationOrigin::User);

        let result = coord.tick(Instant::now());
        assert!(result.is_some());
        assert_eq!(coord.status(), AutoSyncStatus::Running);

        let _ = fs::remove_dir_all(&state_dir);
    }

    // ---- Coordinator: multiple pending→running→idle cycles ----

    #[test]
    fn test_coordinator_multiple_cycles() {
        let policy = make_enabled_policy(0);
        let state_dir = std::env::temp_dir().join("snp_test_cycles");
        let _ = fs::create_dir_all(&state_dir);

        let mut coord = AutoSyncCoordinator::new(policy, state_dir.clone());

        for _i in 0..3 {
            let req = make_request(MutationKind::SnippetCreate);
            coord.request(req, MutationOrigin::User);
            let _ = coord.tick(Instant::now());
            assert_eq!(coord.status(), AutoSyncStatus::Running);
            coord.sync_completed();
            assert!(matches!(coord.state, DebounceState::Idle));
        }

        let _ = fs::remove_dir_all(&state_dir);
    }

    // ---- Stale pending recovery ----

    #[test]
    fn test_recover_stale_pending() {
        let state_dir = std::env::temp_dir().join("snp_test_stale_recovery");
        let _ = fs::create_dir_all(&state_dir);

        // Write a pending state with an old timestamp.
        let old_pending = PendingState {
            version: PENDING_STATE_VERSION,
            pending: true,
            requested_at: unix_now() - 600, // 10 minutes ago
            last_attempt_at: 0,
            last_result: String::new(),
            library_id: None,
        };
        save_pending(&state_dir, &old_pending).unwrap();

        let policy = make_enabled_policy(2);
        let coord = AutoSyncCoordinator::new(policy, state_dir.clone());
        coord.recover_stale_pending();

        // Should have been cleared.
        assert!(load_pending(&state_dir).is_none());

        let _ = fs::remove_dir_all(&state_dir);
    }

    #[test]
    fn test_recover_fresh_pending_not_cleared() {
        let state_dir = std::env::temp_dir().join("snp_test_fresh_recovery");
        let _ = fs::create_dir_all(&state_dir);

        let fresh_pending = PendingState {
            version: PENDING_STATE_VERSION,
            pending: true,
            requested_at: unix_now() - 10, // 10 seconds ago
            last_attempt_at: 0,
            last_result: String::new(),
            library_id: None,
        };
        save_pending(&state_dir, &fresh_pending).unwrap();

        let policy = make_enabled_policy(2);
        let coord = AutoSyncCoordinator::new(policy, state_dir.clone());
        coord.recover_stale_pending();

        // Should NOT be cleared — it's recent.
        assert!(load_pending(&state_dir).is_some());

        let _ = fs::remove_dir_all(&state_dir);
    }

    // ---- No secrets in serialization ----

    #[test]
    fn test_pending_state_no_secrets() {
        let request = AutoSyncRequest {
            library_id: Some("lib".to_string()),
            mutation_kind: MutationKind::SnippetCreate,
            requested_at: 100,
        };
        let pending = PendingState::from_request(&request);
        let content = pending.to_toml_with_integrity().unwrap();
        // PendingState should never contain API keys, commands, or snippet content.
        assert!(!content.contains("api_key"));
        assert!(!content.contains("command"));
        assert!(!content.contains("password"));
        assert!(!content.contains("secret"));
    }

    // ---- Failure policy mapping ----

    #[test]
    fn test_failure_policy_ignore() {
        let policy = AutoSyncPolicy {
            enabled: true,
            debounce: Duration::from_secs(0),
            failure_mode: AutoSyncFailureMode::Ignore,
        };
        assert_eq!(policy.failure_mode, AutoSyncFailureMode::Ignore);
    }

    #[test]
    fn test_failure_policy_warn() {
        let policy = AutoSyncPolicy {
            enabled: true,
            debounce: Duration::from_secs(0),
            failure_mode: AutoSyncFailureMode::Warn,
        };
        assert_eq!(policy.failure_mode, AutoSyncFailureMode::Warn);
    }

    #[test]
    fn test_failure_policy_error() {
        let policy = AutoSyncPolicy {
            enabled: true,
            debounce: Duration::from_secs(0),
            failure_mode: AutoSyncFailureMode::Error,
        };
        assert_eq!(policy.failure_mode, AutoSyncFailureMode::Error);
    }

    // ---- run_auto_sync returns Disabled for disabled policy ----

    #[test]
    fn test_run_auto_sync_disabled_policy() {
        let state_dir = std::env::temp_dir().join("snp_test_run_disabled");
        let _ = fs::create_dir_all(&state_dir);

        let policy = make_disabled_policy();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let status = run_auto_sync(&policy, &state_dir, &rt);
        assert_eq!(status, AutoSyncStatus::Disabled);

        let _ = fs::remove_dir_all(&state_dir);
    }

    // ---- CoordinatorDebug impl ----

    #[test]
    fn test_coordinator_debug() {
        let policy = make_enabled_policy(2);
        let state_dir = std::env::temp_dir().join("snp_test_debug");
        let coord = AutoSyncCoordinator::new(policy, state_dir);
        let debug = format!("{:?}", coord);
        assert!(debug.contains("AutoSyncCoordinator"));
        assert!(debug.contains("Idle"));
    }

    // ---- derive_state_dir ----

    #[test]
    fn test_derive_state_dir_ends_with_snp() {
        let dir = AutoSyncCoordinator::derive_state_dir();
        assert!(dir.to_string_lossy().ends_with("snp"));
    }

    // ---- Edge: tick when Idle ----

    #[test]
    fn test_tick_when_idle_returns_none() {
        let policy = make_enabled_policy(2);
        let state_dir = std::env::temp_dir().join("snp_test_tick_idle");
        let _ = fs::create_dir_all(&state_dir);

        let mut coord = AutoSyncCoordinator::new(policy, state_dir.clone());
        assert!(coord.tick(Instant::now()).is_none());

        let _ = fs::remove_dir_all(&state_dir);
    }

    // ---- Edge: tick when Running ----

    #[test]
    fn test_tick_when_running_returns_none() {
        let policy = make_enabled_policy(0);
        let state_dir = std::env::temp_dir().join("snp_test_tick_running");
        let _ = fs::create_dir_all(&state_dir);

        let mut coord = AutoSyncCoordinator::new(policy, state_dir.clone());
        let req = make_request(MutationKind::SnippetCreate);
        coord.request(req, MutationOrigin::User);
        let _ = coord.tick(Instant::now());
        assert_eq!(coord.status(), AutoSyncStatus::Running);

        // Tick again while running — nothing happens.
        assert!(coord.tick(Instant::now()).is_none());

        let _ = fs::remove_dir_all(&state_dir);
    }

    // ---- Edge: multiple follow-ups coalesce ----

    #[test]
    fn test_multiple_followups_coalesce() {
        let base = Instant::now();
        let req = make_request(MutationKind::SnippetCreate);

        let mut state = DebounceState::Running {
            follow_up: false,
            request: req,
        };

        // First mutation sets follow_up.
        state = state.on_mutation_running(make_request(MutationKind::SnippetUpdate));
        match &state {
            DebounceState::Running { follow_up, .. } => assert!(*follow_up),
            _ => panic!("expected Running"),
        }

        // Second mutation keeps follow_up true.
        state = state.on_mutation_running(make_request(MutationKind::SnippetDelete));
        match &state {
            DebounceState::Running { follow_up, .. } => assert!(*follow_up),
            _ => panic!("expected Running"),
        }

        // Complete → Pending follow-up.
        let completed = state.complete(base);
        assert!(matches!(completed, DebounceState::Pending { .. }));
    }

    // ---- Lock permissions ----

    #[test]
    fn test_lock_file_permissions_0600() {
        let state_dir = std::env::temp_dir().join("snp_test_lock_perms");
        let _ = fs::create_dir_all(&state_dir);

        let lock = CoordinatorLock::acquire(&state_dir).unwrap();
        let lock_path = state_dir.join(LOCK_FILE);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = fs::metadata(&lock_path).unwrap().permissions();
            assert_eq!(perms.mode() & 0o777, 0o600);
        }

        drop(lock);
        let _ = fs::remove_dir_all(&state_dir);
    }

    // ---- run_auto_sync acquires lock ----

    #[test]
    fn test_run_auto_sync_acquires_and_releases_lock() {
        let state_dir = std::env::temp_dir().join("snp_test_run_lock");
        let _ = fs::create_dir_all(&state_dir);

        let policy = make_enabled_policy(0);
        let rt = tokio::runtime::Runtime::new().unwrap();
        // This will fail with a sync error (no server), but the lock
        // should still be released.
        let _status = run_auto_sync(&policy, &state_dir, &rt);

        // Lock file should not exist after run_auto_sync returns.
        assert!(!state_dir.join(LOCK_FILE).exists());

        let _ = fs::remove_dir_all(&state_dir);
    }

    // ---- Request Clone ----

    #[test]
    fn test_request_clone() {
        let req = AutoSyncRequest::new(Some("lib".to_string()), MutationKind::SnippetCreate);
        let cloned = req.clone();
        assert_eq!(req.library_id, cloned.library_id);
        assert_eq!(req.mutation_kind, cloned.mutation_kind);
        assert_eq!(req.requested_at, cloned.requested_at);
    }

    // ---- DebounceState: on_mutation on non-Pending is no-op ----

    #[test]
    fn test_on_mutation_pending_on_idle_is_noop() {
        let state = DebounceState::Idle;
        let next = state.on_mutation_pending(
            Instant::now(),
            Duration::from_secs(2),
            make_request(MutationKind::SnippetUpdate),
        );
        assert!(matches!(next, DebounceState::Idle));
    }

    #[test]
    fn test_on_mutation_running_on_idle_is_noop() {
        let state = DebounceState::Idle;
        let next = state.on_mutation_running(make_request(MutationKind::SnippetUpdate));
        assert!(matches!(next, DebounceState::Idle));
    }
}
