//! **Layer: Domain/Core**
//!
//! The **owned-record lock protocol** — one implementation of the
//! `create_new` + ownership-record protocol used by the transaction lock
//! (`transaction.lock`) and the local-data lock (`local-data.lock`).
//!
//! Both callers used to carry their own copy of the protocol. They differ
//! only in their *wait policy*, which is a **required parameter** of
//! [`acquire_owned_lock`]: a caller cannot inherit the other caller's
//! contract by omission.
//!
//! # Protocol
//!
//! 1. Create the lock file with `create_new(true)`.
//! 2. Write the pre-serialized record to the open handle immediately, then
//!    `sync_all` it. A sync failure is a warning, not an error. If the write
//!    fails, the created file is **removed**: a 0-byte leftover would make
//!    every later acquirer spin through the empty-record ladder.
//! 3. If the file already exists, read and classify the record:
//!    - empty content → bounded retry ladder, then quarantine;
//!    - malformed (or partially written) content → bounded retry ladder,
//!      then quarantine;
//!    - a dead owner → reclaim through an ownership-verified quarantine;
//!    - a live owner → the caller's wait policy decides.
//!
//! # Ownership rule
//!
//! A lock is reclaimed **only** when the recorded start token and the
//! observed start token are both present and differ (the PID-reuse case).
//! An unverifiable owner — no recorded token, no observed token, or matching
//! tokens — is a **live** owner and is never reclaimed.
//!
//! # Release
//!
//! The guard releases through
//! [`remove_owned_lock_file`](crate::utils::process::remove_owned_lock_file):
//! the lock file is removed only when the record still carries this owner's
//! nonce, PID, and start token.

use crate::error::{SnipError, SnipResult};
use crate::utils::process::{is_process_alive, remove_owned_lock_file};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

/// Process identity for lock ownership verification.
///
/// Contains the PID and a start-time token that uniquely identifies a
/// process incarnation. This prevents PID-reuse attacks where a new
/// process inherits the same PID as a dead lock owner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessIdentity {
    /// Process ID.
    pub pid: u32,
    /// Start-time token identifying this process incarnation.
    /// `None` when the platform does not support start-time detection.
    pub start_token: Option<String>,
}

impl ProcessIdentity {
    /// Get the current process identity with start-time token.
    pub fn current() -> ProcessIdentity {
        current_process_identity()
    }

    /// Observe the identity of the process identified by `pid`.
    ///
    /// Returns `Some(identity)` if the process is alive (with start token
    /// when observable), or `None` if the process is dead or cannot be
    /// queried. A live PID whose start identity cannot be observed still
    /// returns `Some` with `start_token: None` — callers must treat this
    /// conservatively as a live owner.
    pub fn observe(pid: u32) -> Option<ProcessIdentity> {
        if !is_process_alive(pid) {
            return None;
        }
        Some(ProcessIdentity {
            pid,
            start_token: get_process_start_token(pid),
        })
    }
}

/// Get the current process identity with start-time token.
pub fn current_process_identity() -> ProcessIdentity {
    let pid = std::process::id();
    ProcessIdentity {
        pid,
        start_token: get_process_start_token(pid),
    }
}

/// Get a start-time token for the given PID.
///
/// On Linux, reads the process start time from `/proc/<pid>/stat` (field 22,
/// in clock ticks since boot). On macOS, uses `sysctl` with `KERN_PROC_PID`.
/// On Windows, uses `GetProcessTimes`. Returns `None` if the start identity
/// cannot be determined.
#[cfg(target_os = "linux")]
fn get_process_start_token(pid: u32) -> Option<String> {
    let stat_path = format!("/proc/{pid}/stat");
    let content = fs::read_to_string(&stat_path).ok()?;
    // Field 22 (1-indexed) is `starttime`. The comm field (field 2) may
    // contain spaces or parens, so find the last `)` and count from there.
    parse_linux_proc_start_token(&content)
}

#[cfg(target_os = "linux")]
fn parse_linux_proc_start_token(stat: &str) -> Option<String> {
    let after_comm = stat.rfind(')')?;
    let fields: Vec<&str> = stat.get(after_comm + 2..)?.split_whitespace().collect();
    fields.get(19).map(|value| (*value).to_owned())
}

#[cfg(target_os = "macos")]
fn get_process_start_token(pid: u32) -> Option<String> {
    use libc::{PROC_PIDTBSDINFO, c_int, proc_bsdinfo, proc_pidinfo};

    let mut info: proc_bsdinfo = unsafe { std::mem::zeroed() };
    let ret = unsafe {
        proc_pidinfo(
            pid as c_int,
            PROC_PIDTBSDINFO,
            0,
            &mut info as *mut _,
            std::mem::size_of::<proc_bsdinfo>() as c_int,
        )
    };

    if ret <= 0 {
        return None;
    }

    // pbi_start_tvsec and pbi_start_tvusec give the process start time
    Some(format!(
        "{}.{:06}",
        info.pbi_start_tvsec, info.pbi_start_tvusec
    ))
}

#[cfg(windows)]
fn get_process_start_token(pid: u32) -> Option<String> {
    use windows_sys::Win32::Foundation::{CloseHandle, FILETIME, HANDLE};
    use windows_sys::Win32::System::Threading::{
        GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    unsafe {
        let handle: HANDLE = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return None;
        }
        let mut creation_time: FILETIME = std::mem::zeroed();
        let mut exit_time: FILETIME = std::mem::zeroed();
        let mut kernel_time: FILETIME = std::mem::zeroed();
        let mut user_time: FILETIME = std::mem::zeroed();
        let success = GetProcessTimes(
            handle,
            &mut creation_time,
            &mut exit_time,
            &mut kernel_time,
            &mut user_time,
        );
        CloseHandle(handle);
        if success == 0 {
            return None;
        }
        // FILETIME is in 100-nanosecond intervals since January 1, 1601 (UTC)
        let creation =
            ((creation_time.dwHighDateTime as u64) << 32) | (creation_time.dwLowDateTime as u64);
        Some(creation.to_string())
    }
}

/// The ownership identity carried by an owned-record lock file.
///
/// This is the subset of a lock record the shared protocol needs: the
/// process that owns the lock and the token that proves the process
/// incarnation. The record payload itself stays with the caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OwnedLockOwner {
    /// Process ID of the lock owner.
    pub pid: u32,
    /// Random nonce that distinguishes two acquisitions by the same PID.
    pub nonce: String,
    /// Start-time token of the owner's process incarnation.
    pub start_token: Option<String>,
}

/// Diagnostic labels for one owned-record lock file.
///
/// Every label is supplied by the caller so the shared protocol keeps the
/// caller's exact diagnostic text: the two locks are reported differently
/// ("Transaction lock held" vs "Local data lock held").
#[derive(Debug, Clone, Copy)]
pub(crate) struct OwnedLockNaming {
    /// Human-readable lock name used in log messages ("transaction lock").
    pub label: &'static str,
    /// File-name prefix of the quarantine files ("transaction.lock.quarantine.").
    pub quarantine_prefix: &'static str,
    /// I/O operation label used when the record write fails.
    pub write_operation: &'static str,
    /// I/O operation label used when the existing record cannot be read.
    pub read_operation: &'static str,
    /// I/O operation label used when the lock file cannot be created.
    pub acquire_operation: &'static str,
    /// I/O operation label used when the record cannot be read before quarantine.
    pub quarantine_read_operation: &'static str,
    /// I/O operation label used when the quarantine rename fails.
    pub quarantine_operation: &'static str,
    /// Info message logged when the owner's PID was reused.
    pub pid_reuse_message: &'static str,
}

/// Builds the error reported when a live owner holds the lock.
pub(crate) type LiveOwnerError<R> = fn(&R, &Path) -> SnipError;

/// Builds the error reported when a wait window elapses.
pub(crate) type TimeoutError = fn(&Path) -> SnipError;

/// The wait policy of an owned-record lock acquisition.
///
/// The policy is a **required** parameter: there is no default, so a caller
/// cannot silently inherit the other lock's contract. The two variants
/// encode the three places where the two locks historically differed:
///
/// | Arm | [`FailFast`](Self::FailFast) | [`WaitForProgress`](Self::WaitForProgress) |
/// |-----|------------------------------|------------------------------------------|
/// | live owner | returns the held-lock error immediately | extends the deadline on every hand-off, then times out |
/// | unreadable record | flat deadline, then a timeout error | no deadline |
/// | empty record | no deadline | deadline, then a distinct timeout error |
pub(crate) enum OwnedLockWaitPolicy<R> {
    /// Fail fast when a live owner holds the lock.
    ///
    /// The flat deadline bounds the acquisition loop: it is measured from
    /// the start of the loop and applies to the arm that cannot read the
    /// existing record. The empty-record arm is bounded by the retry ladder
    /// instead.
    FailFast {
        /// Flat deadline for the whole acquisition.
        flat_timeout: Duration,
        /// Error returned when a live owner holds the lock.
        live_owner_error: LiveOwnerError<R>,
        /// Error returned when the flat deadline elapses.
        read_timeout_error: TimeoutError,
    },
    /// Wait for the lock to change hands.
    ///
    /// The deadline measures time *without progress* rather than elapsed
    /// time: it is extended every time the lock changes hands, so a tail
    /// waiter behind a healthy queue is not told the lock is "held". A hard
    /// cap keeps a genuinely stuck owner from waiting forever.
    WaitForProgress {
        /// Window granted while the lock keeps changing hands.
        base_wait: Duration,
        /// Hard cap on the total wait.
        max_wait: Duration,
        /// Ceiling of the exponential poll backoff.
        max_backoff: Duration,
        /// Error returned when the lock stops changing hands.
        live_owner_error: LiveOwnerError<R>,
        /// Error returned when the lock record stays empty.
        empty_timeout_error: TimeoutError,
    },
}

/// Mutable wait state carried across acquisition-loop iterations.
struct WaitState {
    /// Flat deadline of the whole acquisition (`FailFast` only).
    flat_deadline: Option<Instant>,
    /// Progress-extended deadline (`WaitForProgress` only).
    deadline: Option<Instant>,
    /// Hard cap for the progress-extended deadline (`WaitForProgress` only).
    hard_deadline: Option<Instant>,
    /// Owner key observed on the previous poll (`WaitForProgress` only).
    last_seen_owner: Option<String>,
    /// Current poll backoff.
    backoff: Duration,
}

impl<R> OwnedLockWaitPolicy<R> {
    /// Build the initial wait state for this policy.
    fn new_state(&self) -> WaitState {
        let started = Instant::now();
        match self {
            OwnedLockWaitPolicy::FailFast { flat_timeout, .. } => WaitState {
                flat_deadline: Some(started + *flat_timeout),
                deadline: None,
                hard_deadline: None,
                last_seen_owner: None,
                backoff: Duration::from_millis(10),
            },
            OwnedLockWaitPolicy::WaitForProgress {
                base_wait,
                max_wait,
                ..
            } => WaitState {
                flat_deadline: None,
                deadline: Some(started + *base_wait),
                hard_deadline: Some(started + *max_wait),
                last_seen_owner: None,
                backoff: Duration::from_millis(10),
            },
        }
    }

    /// Handle the arm in which the existing record could not be read.
    ///
    /// The lock may have been removed between the failed create and the
    /// read, so this arm retries. `FailFast` additionally bounds the retry
    /// with its flat deadline; `WaitForProgress` does not.
    fn on_unreadable_record(&self, lock_path: &Path, state: &WaitState) -> SnipResult<()> {
        match self {
            OwnedLockWaitPolicy::FailFast {
                read_timeout_error, ..
            } => {
                if state
                    .flat_deadline
                    .is_some_and(|deadline| Instant::now() >= deadline)
                {
                    return Err(read_timeout_error(lock_path));
                }
                Ok(())
            }
            OwnedLockWaitPolicy::WaitForProgress { .. } => Ok(()),
        }
    }

    /// Handle the arm in which the existing record stayed empty.
    ///
    /// `FailFast` does not bound this arm: the retry ladder plus the
    /// quarantine step resolve it. `WaitForProgress` bounds it with the
    /// progress-extended deadline and reports a distinct error.
    fn on_empty_record(&self, lock_path: &Path, state: &WaitState) -> SnipResult<()> {
        match self {
            OwnedLockWaitPolicy::FailFast { .. } => Ok(()),
            OwnedLockWaitPolicy::WaitForProgress {
                empty_timeout_error,
                ..
            } => {
                if state
                    .deadline
                    .is_some_and(|deadline| Instant::now() >= deadline)
                {
                    return Err(empty_timeout_error(lock_path));
                }
                Ok(())
            }
        }
    }

    /// Handle the arm in which the owner is alive (or cannot be disproved).
    ///
    /// `FailFast` returns the caller's held-lock error immediately.
    /// `WaitForProgress` extends the deadline whenever the lock changes
    /// hands, sleeps with the bounded backoff, and returns the caller's
    /// timeout error once the lock stops changing hands.
    fn on_live_owner(
        &self,
        existing: &R,
        owner: &OwnedLockOwner,
        lock_path: &Path,
        state: &mut WaitState,
    ) -> SnipResult<()> {
        match self {
            OwnedLockWaitPolicy::FailFast {
                live_owner_error, ..
            } => Err(live_owner_error(existing, lock_path)),
            OwnedLockWaitPolicy::WaitForProgress {
                base_wait,
                max_backoff,
                live_owner_error,
                ..
            } => {
                // The lock changed hands since the previous poll: the queue
                // is making progress, so the waiter gets a fresh window
                // instead of timing out on a healthy lock.
                let owner_key = format!("{}:{}", owner.pid, owner.nonce);
                if state.last_seen_owner.as_deref() != Some(owner_key.as_str()) {
                    state.last_seen_owner = Some(owner_key);
                    let extended = Instant::now() + *base_wait;
                    let hard_deadline = state.hard_deadline.unwrap_or(extended);
                    state.deadline = Some(extended.min(hard_deadline));
                }
                if state
                    .deadline
                    .is_some_and(|deadline| Instant::now() >= deadline)
                {
                    return Err(live_owner_error(existing, lock_path));
                }
                std::thread::sleep(state.backoff);
                state.backoff = (state.backoff * 2).min(*max_backoff);
                Ok(())
            }
        }
    }
}

/// Everything the shared protocol needs for one acquisition.
pub(crate) struct OwnedLockRequest<'a, R> {
    /// Path of the lock file.
    pub lock_path: &'a Path,
    /// The ownership record; stored in the returned guard.
    pub record: &'a R,
    /// Pre-serialized record content written into the lock file.
    pub record_content: &'a str,
    /// Diagnostic labels and quarantine naming of this lock.
    pub naming: &'a OwnedLockNaming,
    /// The wait policy — required, never defaulted.
    pub policy: OwnedLockWaitPolicy<R>,
}

/// Guard holding an owned-record lock.
///
/// The record payload type `R` is supplied by the caller: the guard stores
/// the record so the release step can verify ownership against it.
///
/// Releases the lock on `Drop` by removing the lock file **only** when it
/// still carries this owner's ownership record.
#[derive(Debug)]
pub struct OwnedLockGuard<R> {
    /// Path of the lock file.
    pub(crate) lock_path: PathBuf,
    /// The ownership record written into the lock file.
    pub(crate) info: R,
    /// Extracts the ownership identity from the record.
    owner_of: fn(&R) -> OwnedLockOwner,
}

impl<R> Drop for OwnedLockGuard<R> {
    fn drop(&mut self) {
        // Only remove if we still own the lock. Verify nonce, PID, and
        // start token (when present) through an opened handle, and confirm
        // via handle metadata that the verified file is still the one at
        // the path before unlinking — a concurrent quarantine + re-acquire
        // must not lose its lock file to our Drop.
        let owner = (self.owner_of)(&self.info);
        remove_owned_lock_file(
            &self.lock_path,
            &owner.nonce,
            owner.pid,
            owner.start_token.as_deref(),
        );
    }
}

/// Acquire an owned-record lock.
///
/// Runs the shared protocol: create the lock file with `create_new`, write
/// the record, classify an existing record, and reclaim a stale lock when
/// the conservative ownership rule allows it.
///
/// `owner_of` extracts the ownership identity from a parsed record; the
/// record payload type `R` is supplied by the caller so this module never
/// grows a field only one caller needs.
pub(crate) fn acquire_owned_lock<R>(
    request: &OwnedLockRequest<'_, R>,
    owner_of: fn(&R) -> OwnedLockOwner,
) -> SnipResult<OwnedLockGuard<R>>
where
    R: Clone + DeserializeOwned + PartialEq,
{
    let OwnedLockRequest {
        lock_path,
        record,
        record_content,
        naming,
        policy,
    } = request;

    let mut state = policy.new_state();
    let mut empty_retries = 0u32;
    let mut malformed_retries = 0u32;

    // Single acquisition loop: create_new, write immediately, classify the
    // existing owner.
    loop {
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(lock_path)
        {
            Ok(file) => {
                write_record(file, lock_path, record_content, naming)?;
                return Ok(OwnedLockGuard {
                    lock_path: lock_path.to_path_buf(),
                    info: (*record).clone(),
                    owner_of,
                });
            }
            Err(e)
                if e.kind() == std::io::ErrorKind::AlreadyExists
                // On Windows, a just-deleted file can briefly return
                // PermissionDenied when in a pending-delete state.
                // Treat it the same as AlreadyExists.
                || is_transient_lock_contention(e.kind()) =>
            {
                // Lock exists — read and classify the owner.
                // Handle TOCTOU: another writer may have removed the lock
                // between create_new failing and read_to_string.
                let content = match fs::read_to_string(lock_path) {
                    Ok(c) => c,
                    Err(e)
                        if e.kind() == std::io::ErrorKind::NotFound
                        // On Windows, a pending-delete file may be unreadable.
                        || is_transient_lock_contention(e.kind()) =>
                    {
                        // Lock was removed or is in a transient state — loop
                        // back and retry.
                        std::thread::sleep(Duration::from_millis(1));
                        policy.on_unreadable_record(lock_path, &state)?;
                        continue;
                    }
                    Err(e) => {
                        return Err(SnipError::io_error(
                            naming.read_operation,
                            lock_path.to_path_buf(),
                            e,
                        ));
                    }
                };

                let existing: R = match toml::from_str(&content) {
                    Ok(info) => {
                        // A readable record resets the ladder below: a waiter
                        // that sees scattered empty reads over a long
                        // legitimate acquisition must not eventually quarantine
                        // a perfectly good lock.
                        empty_retries = 0;
                        malformed_retries = 0;
                        info
                    }
                    Err(_) if content.trim().is_empty() => {
                        // Empty file — normally another writer just called
                        // create_new but hasn't written yet. Retry briefly,
                        // then reclaim a file left behind by a crashed writer.
                        // Without this ladder a 0-byte leftover (failed
                        // write_all, or SIGKILL between create_new and
                        // write_all) would spin forever: the record can
                        // never become valid, so no reclaim path is reachable.
                        empty_retries = empty_retries.saturating_add(1);
                        let is_old = fs::metadata(lock_path)
                            .and_then(|m| m.modified())
                            .ok()
                            .and_then(|t| SystemTime::now().duration_since(t).ok())
                            .is_some_and(|age| age > Duration::from_millis(100));
                        if empty_retries > 200 || (empty_retries > 50 && is_old) {
                            tracing::warn!(
                                "Empty {} lock record did not become valid; quarantining",
                                naming.label
                            );
                            quarantine_owned_lock::<R>(lock_path, None, naming)?;
                            empty_retries = 0;
                        } else {
                            policy.on_empty_record(lock_path, &state)?;
                        }
                        std::thread::sleep(Duration::from_millis(1));
                        continue;
                    }
                    Err(_) => {
                        // Malformed *or* partially written. A record that is
                        // only partly written looks malformed too, so it gets
                        // the same bounded ladder as the empty arm rather than
                        // being quarantined out from under a live owner.
                        malformed_retries = malformed_retries.saturating_add(1);
                        if malformed_retries > 200 {
                            tracing::warn!(
                                "Malformed {} lock record did not become valid; quarantining",
                                naming.label
                            );
                            quarantine_owned_lock::<R>(lock_path, None, naming)?;
                            malformed_retries = 0;
                        }
                        std::thread::sleep(Duration::from_millis(1));
                        continue;
                    }
                };

                let existing_owner = owner_of(&existing);

                // Observe the process identified by the existing lock record.
                // This queries the recorded PID — NOT the current process.
                match ProcessIdentity::observe(existing_owner.pid) {
                    None => {
                        // Owner process is dead — reclaim immediately.
                        tracing::info!(
                            pid = existing_owner.pid,
                            "Reclaiming stale {} (owner process is dead)",
                            naming.label
                        );
                        quarantine_owned_lock(lock_path, Some(&existing), naming)?;
                        continue;
                    }
                    Some(observed) => {
                        // Owner is alive. Refuse if we cannot verify ownership
                        // (conservative policy):
                        // - the recorded start token is None (old lock without
                        //   a token)
                        // - the observed start token is None (cannot observe
                        //   identity)
                        // - the start tokens match (same process)
                        // Only reclaim when both tokens are present and differ.
                        if existing_owner.start_token.is_none()
                            || observed.start_token.is_none()
                            || observed.start_token == existing_owner.start_token
                        {
                            // The owner is alive (or cannot be verified). The
                            // wait policy decides whether to wait or fail.
                            policy.on_live_owner(
                                &existing,
                                &existing_owner,
                                lock_path,
                                &mut state,
                            )?;
                            continue;
                        }
                        // PID reuse detected — the observed start token differs
                        // from the recorded start token.
                        tracing::info!(
                            pid = existing_owner.pid,
                            observed_token = ?observed.start_token,
                            recorded_token = ?existing_owner.start_token,
                            "{}", naming.pid_reuse_message
                        );
                        quarantine_owned_lock(lock_path, Some(&existing), naming)?;
                        continue;
                    }
                }
            }
            Err(e) => {
                return Err(SnipError::io_error(
                    naming.acquire_operation,
                    lock_path.to_path_buf(),
                    e,
                ));
            }
        }
    }
}

/// Write the pre-serialized record to the freshly created lock file.
///
/// The record is written to the open handle immediately, then synced before
/// the handle is dropped. This minimizes the empty-file window: a concurrent
/// reader that sees empty content will retry instead of quarantining.
///
/// A write failure removes the created file: the file was created by this
/// call and is empty, and leaving it behind makes every later acquirer spin
/// through the empty-content ladder. A sync failure is only a warning — the
/// acquisition still succeeds.
fn write_record(
    mut file: fs::File,
    lock_path: &Path,
    record_content: &str,
    naming: &OwnedLockNaming,
) -> SnipResult<()> {
    use std::io::Write;
    if let Err(e) = file.write_all(record_content.as_bytes()) {
        // The file was created by this call and is empty. Leaving it behind
        // makes every later acquirer spin through the empty-content ladder,
        // so drop it before reporting.
        drop(file);
        let _ = fs::remove_file(lock_path);
        return Err(SnipError::io_error(
            naming.write_operation,
            lock_path.to_path_buf(),
            e,
        ));
    }
    if let Err(e) = file.sync_all() {
        tracing::warn!(
            error = %e,
            "failed to sync {} lock record",
            naming.label
        );
    }
    Ok(())
}

/// Quarantine a stale or malformed lock by renaming it.
///
/// When `observed` is supplied, the record currently at `lock_path` must
/// still match it before the rename happens. Reading the record and checking
/// the owner's liveness are non-atomic, so without this check a writer that
/// decided to reclaim could rename away a *live* lock that another writer had
/// already reclaimed and re-acquired in the meantime — silently putting two
/// processes inside the critical section. Passing `None` (malformed or empty
/// record) skips verification because there is no owner identity to compare,
/// which is the same trade-off the release step makes for unparseable content.
///
/// If the lock file has already been quarantined by a concurrent writer
/// (race on stale-lock reclaim), the `NotFound` error is treated as success.
pub(crate) fn quarantine_owned_lock<R>(
    lock_path: &Path,
    observed: Option<&R>,
    naming: &OwnedLockNaming,
) -> SnipResult<PathBuf>
where
    R: DeserializeOwned + PartialEq,
{
    if let Some(expected) = observed {
        match fs::read_to_string(lock_path) {
            Ok(content) => match toml::from_str::<R>(&content) {
                Ok(current) if &current == expected => {}
                Ok(_) => {
                    // A different record is in place: the stale observation is
                    // obsolete and the current owner must not be disturbed.
                    tracing::debug!(
                        "{} record changed since it was observed; not quarantining",
                        naming.label
                    );
                    return Ok(lock_path.to_path_buf());
                }
                Err(_) => {
                    tracing::debug!(
                        "{} record is no longer parseable; not quarantining",
                        naming.label
                    );
                    return Ok(lock_path.to_path_buf());
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                return Err(SnipError::io_error(
                    naming.quarantine_read_operation,
                    lock_path.to_path_buf(),
                    e,
                ));
            }
        }
    }

    let quarantine_name = format!("{}{}", naming.quarantine_prefix, uuid::Uuid::new_v4());
    let quarantine_path = lock_path
        .parent()
        .unwrap_or(lock_path)
        .join(&quarantine_name);
    match fs::rename(lock_path, &quarantine_path) {
        Ok(()) => {
            // Opportunistic GC: without it, quarantine files accumulate
            // without bound under pathological spawn/kill churn.
            prune_quarantine_files(
                quarantine_path.parent().unwrap_or(&quarantine_path),
                naming.quarantine_prefix,
                QUARANTINE_RETENTION,
            );
            Ok(quarantine_path)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // Another writer already quarantined the lock — treat as success.
            tracing::debug!("{} already quarantined by another writer", naming.label);
            Ok(quarantine_path)
        }
        Err(e) => Err(SnipError::io_error(
            naming.quarantine_operation,
            quarantine_path.clone(),
            e,
        )),
    }
}

/// Whether a lock-file I/O error kind should be treated as transient lock
/// contention (retry) rather than a hard failure.
///
/// On Windows a just-deleted file can briefly surface `PermissionDenied`
/// while in a pending-delete state, so it retries like `AlreadyExists`.
/// On Unix a `PermissionDenied` here is a genuine misconfiguration (e.g.
/// wrong mode on the lock directory) and must fail fast instead of spinning
/// in the contention loop.
#[cfg(windows)]
pub(crate) fn is_transient_lock_contention(kind: std::io::ErrorKind) -> bool {
    kind == std::io::ErrorKind::PermissionDenied
}

/// Unix has no pending-delete aliasing: permission errors fail fast.
#[cfg(not(windows))]
pub(crate) fn is_transient_lock_contention(_kind: std::io::ErrorKind) -> bool {
    false
}

/// How long quarantine files are kept for debugging before GC removes them.
pub(crate) const QUARANTINE_RETENTION: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// Best-effort removal of quarantine files in `dir` older than `max_age`.
///
/// Every stale/malformed lock is quarantined under a fresh UUID name; without
/// GC, pathological spawn/kill churn would grow the state directory without
/// bound. Pruning runs whenever a new quarantine file is created (rare), so
/// the directory scan cost is negligible. Individual failures are logged and
/// ignored — GC must never break the acquisition path it is called from.
pub(crate) fn prune_quarantine_files(dir: &Path, prefix: &str, max_age: Duration) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let now = std::time::SystemTime::now();
    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                tracing::debug!(error = %e, "skipping unreadable quarantine entry");
                continue;
            }
        };
        let name = entry.file_name();
        let Some(name_str) = name.to_str() else {
            continue;
        };
        if !name_str.starts_with(prefix) {
            continue;
        }
        let age = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|modified| now.duration_since(modified).ok());
        match age {
            Some(age) if age >= max_age => match fs::remove_file(entry.path()) {
                Ok(()) => tracing::debug!(file = %name_str, "removed expired quarantine file"),
                Err(e) => tracing::debug!(
                    error = %e,
                    file = %name_str,
                    "could not remove expired quarantine file"
                ),
            },
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};
    use tempfile::TempDir;

    /// A minimal lock record used by the protocol tests.
    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    struct TestRecord {
        schema_version: u32,
        pid: u32,
        nonce: String,
        created_at_unix_ms: i64,
        #[serde(default)]
        start_token: Option<String>,
    }

    const TEST_NAMING: OwnedLockNaming = OwnedLockNaming {
        label: "test lock",
        quarantine_prefix: "test.lock.quarantine.",
        write_operation: "write test lock record",
        read_operation: "read existing test lock",
        acquire_operation: "acquire test lock",
        quarantine_read_operation: "read test lock before quarantine",
        quarantine_operation: "quarantine stale test lock",
        pid_reuse_message: "Test lock owner PID reused (start token mismatch), reclaiming",
    };

    /// The error a fail-fast caller reports for a live owner.
    fn held_error(_record: &TestRecord, _lock_path: &Path) -> SnipError {
        SnipError::runtime_error("Test lock held", Some("the owner is live"))
    }

    /// The error a wait-for-progress caller reports when the lock stops
    /// changing hands.
    fn wait_live_owner_error(_record: &TestRecord, _lock_path: &Path) -> SnipError {
        SnipError::runtime_error("Test lock timed out", Some("no progress"))
    }

    /// The error a fail-fast caller reports when its flat deadline elapses.
    fn wait_timeout_error(_lock_path: &Path) -> SnipError {
        SnipError::runtime_error("Test lock timed out", Some("no progress"))
    }

    /// The error a wait-for-progress caller reports for an empty record.
    fn empty_timeout_error(_lock_path: &Path) -> SnipError {
        SnipError::runtime_error("Test lock timed out", Some("the record stayed empty"))
    }

    fn test_owner(record: &TestRecord) -> OwnedLockOwner {
        OwnedLockOwner {
            pid: record.pid,
            nonce: record.nonce.clone(),
            start_token: record.start_token.clone(),
        }
    }

    fn test_record(nonce: &str, pid: u32, start_token: Option<&str>) -> TestRecord {
        TestRecord {
            schema_version: 1,
            pid,
            nonce: nonce.to_string(),
            created_at_unix_ms: 0,
            start_token: start_token.map(|token| token.to_string()),
        }
    }

    fn write_existing_record(path: &Path, record: &TestRecord) {
        fs::write(path, toml::to_string_pretty(record).unwrap()).unwrap();
    }

    fn count_quarantines(dir: &Path) -> usize {
        fs::read_dir(dir)
            .unwrap()
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("test.lock.quarantine.")
            })
            .count()
    }

    fn fail_fast_policy() -> OwnedLockWaitPolicy<TestRecord> {
        OwnedLockWaitPolicy::FailFast {
            flat_timeout: Duration::from_secs(30),
            live_owner_error: held_error,
            read_timeout_error: wait_timeout_error,
        }
    }

    fn wait_for_progress_policy(base_wait: Duration) -> OwnedLockWaitPolicy<TestRecord> {
        OwnedLockWaitPolicy::WaitForProgress {
            base_wait,
            max_wait: Duration::from_secs(5),
            max_backoff: Duration::from_millis(5),
            live_owner_error: wait_live_owner_error,
            empty_timeout_error,
        }
    }

    /// The Linux start token is read from `/proc/<pid>/stat` field 22.
    #[cfg(target_os = "linux")]
    #[test]
    fn linux_start_token_parser_reads_field_22() {
        let stat = "42 (name with ) parens) S f4 f5 f6 f7 f8 f9 f10 f11 f12 f13 f14 f15 f16 f17 f18 f19 f20 FIELD21 START22";
        assert_eq!(parse_linux_proc_start_token(stat), Some("START22".into()));
        assert_ne!(parse_linux_proc_start_token(stat), Some("FIELD21".into()));
        assert_eq!(parse_linux_proc_start_token("1 (short) S f3"), None);
    }

    /// The fail-fast policy reports a live owner immediately, with the
    /// caller's own error text.
    #[test]
    fn fail_fast_policy_reports_a_live_owner_immediately() {
        let dir = TempDir::new().unwrap();
        let lock_path = dir.path().join("test.lock");
        // The current process is alive; the recorded start token cannot be
        // verified, so the owner counts as live.
        write_existing_record(
            &lock_path,
            &test_record("owner-a", std::process::id(), None),
        );

        let record = test_record("mine", std::process::id(), None);
        let content = toml::to_string_pretty(&record).unwrap();
        let request = OwnedLockRequest {
            lock_path: &lock_path,
            record: &record,
            record_content: &content,
            naming: &TEST_NAMING,
            policy: fail_fast_policy(),
        };

        let started = Instant::now();
        let err = acquire_owned_lock(&request, test_owner).unwrap_err();
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "the fail-fast policy must not wait for a live owner"
        );
        assert_eq!(
            err.to_string(),
            "Runtime error: Test lock held: the owner is live"
        );
        assert!(
            lock_path.exists(),
            "a live owner's lock file must not be reclaimed"
        );
    }

    /// The wait-for-progress policy times out with the caller's own error
    /// text when the lock stops changing hands.
    #[test]
    fn wait_for_progress_policy_times_out_with_the_callers_error() {
        let dir = TempDir::new().unwrap();
        let lock_path = dir.path().join("test.lock");
        write_existing_record(
            &lock_path,
            &test_record("owner-a", std::process::id(), None),
        );

        let record = test_record("mine", std::process::id(), None);
        let content = toml::to_string_pretty(&record).unwrap();
        let request = OwnedLockRequest {
            lock_path: &lock_path,
            record: &record,
            record_content: &content,
            naming: &TEST_NAMING,
            policy: wait_for_progress_policy(Duration::from_millis(50)),
        };

        let err = acquire_owned_lock(&request, test_owner).unwrap_err();
        assert_eq!(
            err.to_string(),
            "Runtime error: Test lock timed out: no progress"
        );
        assert!(
            lock_path.exists(),
            "the live owner's lock file must survive the timeout"
        );
    }

    /// The wait-for-progress policy extends its deadline every time the lock
    /// changes hands, and only times out when it stops changing hands.
    #[test]
    fn wait_for_progress_policy_extends_the_deadline_on_hand_off() {
        let policy = wait_for_progress_policy(Duration::from_millis(120));
        let mut state = policy.new_state();
        let lock_path = Path::new("test.lock");

        let owner_a = test_record("owner-a", std::process::id(), None);
        let owner_a_identity = test_owner(&owner_a);

        // First observation: the owner changed hands relative to "nothing
        // seen yet", so the waiter gets a fresh window.
        policy
            .on_live_owner(&owner_a, &owner_a_identity, lock_path, &mut state)
            .unwrap();
        let first_deadline = state.deadline.expect("wait-for-progress tracks a deadline");
        assert!(first_deadline > Instant::now());

        // Same owner: no hand-off, so the deadline must not move.
        std::thread::sleep(Duration::from_millis(20));
        policy
            .on_live_owner(&owner_a, &owner_a_identity, lock_path, &mut state)
            .unwrap();
        assert_eq!(
            state.deadline,
            Some(first_deadline),
            "an unchanged owner must not extend the deadline"
        );

        // New owner: the lock changed hands, so the deadline is extended.
        let owner_b = test_record("owner-b", std::process::id(), None);
        let owner_b_identity = test_owner(&owner_b);
        policy
            .on_live_owner(&owner_b, &owner_b_identity, lock_path, &mut state)
            .unwrap();
        assert!(
            state.deadline.expect("deadline") > first_deadline,
            "a hand-off must extend the deadline"
        );

        // Once the (extended) deadline passes, the caller gets its timeout
        // error.
        std::thread::sleep(Duration::from_millis(130));
        let err = policy
            .on_live_owner(&owner_b, &owner_b_identity, lock_path, &mut state)
            .unwrap_err();
        assert_eq!(
            err.to_string(),
            "Runtime error: Test lock timed out: no progress"
        );
    }

    /// The wait-for-progress policy bounds the empty-record arm with its
    /// deadline and reports its own error.
    #[test]
    fn wait_for_progress_policy_bounds_the_empty_record_arm() {
        let dir = TempDir::new().unwrap();
        let lock_path = dir.path().join("test.lock");
        fs::write(&lock_path, "").unwrap();

        let record = test_record("mine", std::process::id(), None);
        let content = toml::to_string_pretty(&record).unwrap();
        let request = OwnedLockRequest {
            lock_path: &lock_path,
            record: &record,
            record_content: &content,
            naming: &TEST_NAMING,
            policy: wait_for_progress_policy(Duration::from_millis(30)),
        };

        let err = acquire_owned_lock(&request, test_owner).unwrap_err();
        assert_eq!(
            err.to_string(),
            "Runtime error: Test lock timed out: the record stayed empty"
        );
    }

    /// An unverifiable owner — a record without a start token — is a live
    /// owner: the lock is never reclaimed.
    #[test]
    fn an_unverifiable_owner_is_treated_as_a_live_owner() {
        let dir = TempDir::new().unwrap();
        let lock_path = dir.path().join("test.lock");
        write_existing_record(
            &lock_path,
            &test_record("owner-a", std::process::id(), None),
        );

        let record = test_record("mine", std::process::id(), None);
        let content = toml::to_string_pretty(&record).unwrap();
        let request = OwnedLockRequest {
            lock_path: &lock_path,
            record: &record,
            record_content: &content,
            naming: &TEST_NAMING,
            policy: fail_fast_policy(),
        };

        let err = acquire_owned_lock(&request, test_owner).unwrap_err();
        assert_eq!(
            err.to_string(),
            "Runtime error: Test lock held: the owner is live"
        );
        let content = fs::read_to_string(&lock_path).unwrap();
        assert!(
            content.contains("owner-a"),
            "an unverifiable owner's lock must survive"
        );
        assert_eq!(count_quarantines(dir.path()), 0);
    }

    /// A record whose recorded and observed start tokens both differ is a
    /// PID reuse and is reclaimed.
    #[test]
    fn a_reused_pid_is_reclaimed() {
        if ProcessIdentity::current().start_token.is_none() {
            // The platform cannot supply start tokens; the PID-reuse rule
            // cannot be exercised here.
            return;
        }
        let dir = TempDir::new().unwrap();
        let lock_path = dir.path().join("test.lock");
        // The recorded start token differs from the observed one: the PID
        // was reused by a different process incarnation.
        write_existing_record(
            &lock_path,
            &test_record("owner-a", std::process::id(), Some("stale-start-token")),
        );

        let record = test_record("mine", std::process::id(), None);
        let content = toml::to_string_pretty(&record).unwrap();
        let request = OwnedLockRequest {
            lock_path: &lock_path,
            record: &record,
            record_content: &content,
            naming: &TEST_NAMING,
            policy: fail_fast_policy(),
        };

        let guard = acquire_owned_lock(&request, test_owner).unwrap();
        let content = fs::read_to_string(&lock_path).unwrap();
        assert!(
            content.contains("mine"),
            "the stale lock must be replaced by our own record"
        );
        assert_eq!(
            count_quarantines(dir.path()),
            1,
            "the stale lock must be quarantined"
        );
        drop(guard);
    }

    /// A failed record write removes the created file: a 0-byte leftover
    /// would make every later acquirer spin through the empty ladder.
    #[test]
    fn a_failed_record_write_removes_the_created_file() {
        let dir = TempDir::new().unwrap();
        let lock_path = dir.path().join("test.lock");
        fs::write(&lock_path, "placeholder").unwrap();
        // A read-only handle cannot be written to, so the record write fails.
        let handle = fs::OpenOptions::new().read(true).open(&lock_path).unwrap();

        let err =
            write_record(handle, &lock_path, "schema_version = 1\n", &TEST_NAMING).unwrap_err();
        assert!(
            !lock_path.exists(),
            "a failed record write must remove the created file"
        );
        assert!(
            err.to_string().contains("write test lock record"),
            "unexpected error: {err}"
        );
    }
}
