use crate::error::SnipResult;
use crate::status_snapshot::{TopLevelSyncState, capture_snapshot};
use chrono::{Local, TimeZone, Utc};
use std::io::{self, Write};

pub fn run(json: bool, sync_only: bool) -> SnipResult<()> {
    let snapshot = capture_snapshot();

    if json {
        let json_str = serde_json::to_string_pretty(&snapshot).map_err(|e| {
            crate::error::SnipError::runtime_error(
                "failed to serialize snapshot",
                Some(&e.to_string()),
            )
        })?;
        let stdout = io::stdout();
        let mut handle = stdout.lock();
        writeln!(handle, "{json_str}").map_err(|e| {
            crate::error::SnipError::io_error("write stdout", std::path::PathBuf::new(), e)
        })?;
        return Ok(());
    }

    let mut out = String::new();

    if !sync_only {
        let local = &snapshot.local;
        let primary = local.primary_library.as_deref().unwrap_or("none");
        out.push_str(&format!(
            "Local: {} libraries, {} snippets; primary={}\n",
            local.libraries, local.snippets, primary,
        ));
    }

    out.push_str(&format_sync_line(&snapshot.sync.top_level));

    if let crate::status_snapshot::PendingStateView::Pending { generation, .. } =
        &snapshot.pending.state
    {
        out.push_str(&format!("Pending generation: {generation}\n"));
    }

    if snapshot.attempt.last_attempt_at_unix_ms > 0 {
        let dt = Utc
            .timestamp_millis_opt(snapshot.attempt.last_attempt_at_unix_ms as i64)
            .single()
            .map(|dt: chrono::DateTime<Utc>| dt.with_timezone(&Local));
        if let Some(local_dt) = dt {
            let class = if snapshot.attempt.last_failure_class.is_empty() {
                "unknown".to_string()
            } else {
                snapshot.attempt.last_failure_class.clone()
            };
            out.push_str(&format!("Last attempt: {class} at {local_dt}\n",));
        }
    }

    if snapshot.attempt.next_attempt_at_unix_ms > 0 {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        if snapshot.attempt.next_attempt_at_unix_ms > now_ms {
            let dt = Utc
                .timestamp_millis_opt(snapshot.attempt.next_attempt_at_unix_ms as i64)
                .single()
                .map(|dt: chrono::DateTime<Utc>| dt.with_timezone(&Local));
            if let Some(local_dt) = dt {
                out.push_str(&format!("Next retry: {local_dt}\n"));
            }
        }
    }

    if let Some(action) = format_action(&snapshot.sync.top_level) {
        out.push_str(&format!("{action}\n"));
    }

    out.push_str(&format!("Logs: {}\n", snapshot.log_dir.display()));

    let stdout = io::stdout();
    let mut handle = stdout.lock();
    write!(handle, "{out}").map_err(|e| {
        crate::error::SnipError::io_error("write stdout", std::path::PathBuf::new(), e)
    })?;

    Ok(())
}

fn format_sync_line(top: &TopLevelSyncState) -> String {
    match top {
        TopLevelSyncState::CorruptOrInaccessible => "Sync: corrupt or inaccessible state\n",
        TopLevelSyncState::LiveExecution { pid, .. } => {
            return format!("Sync: active (pid={pid})\n");
        }
        TopLevelSyncState::PendingAttentionRequired => "Sync: attention required\n",
        TopLevelSyncState::PendingRetryBackoff { .. } => "Sync: pending retry\n",
        TopLevelSyncState::PendingAwaitingScheduling => "Sync: pending\n",
        TopLevelSyncState::ConfiguredAndCurrent => "Sync: current\n",
        TopLevelSyncState::ConfiguredAutoSyncDisabled => "Sync: auto-sync disabled\n",
        TopLevelSyncState::NotConfigured => "Sync: not configured\n",
    }
    .to_string()
}

fn format_action(top: &TopLevelSyncState) -> Option<String> {
    match top {
        TopLevelSyncState::PendingAttentionRequired => {
            Some("Action: run `snp sync retry` to retry now".to_string())
        }
        TopLevelSyncState::PendingRetryBackoff {
            next_attempt_at_unix_ms,
        } => {
            let dt = Utc
                .timestamp_millis_opt(*next_attempt_at_unix_ms as i64)
                .single()
                .map(|dt: chrono::DateTime<Utc>| dt.with_timezone(&Local));
            dt.map(|local_dt| format!("Action: retry eligible at {local_dt}"))
        }
        TopLevelSyncState::CorruptOrInaccessible => {
            Some("Action: run `snp sync repair` to diagnose".to_string())
        }
        _ => None,
    }
}
