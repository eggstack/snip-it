//! Cross-process event sink for worker/executor lifecycle evidence.
//!
//! Provides a JSON-lines channel that worker and executor child processes
//! can write to during test runs. The test side reads the file to assert
//! on lifecycle events without timing-dependent coordination.

#![allow(dead_code)]
//!
//! # Usage
//!
//! - The **test** creates an [`EventSink`] and optionally clears it.
//! - Child processes are passed the [`EventWriter`] path (via env or arg)
//!   and use [`EventWriter::write`] to emit structured events.
//! - After the child exits the test calls [`EventSink::read_all`] or one
//!   of the `wait_for_*` helpers to inspect what happened.

use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

// ---------------------------------------------------------------------------
// EventRecord
// ---------------------------------------------------------------------------

#[allow(dead_code)]
/// A single JSON-lines event written by a child process.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventRecord {
    /// Schema version (starts at 1).
    pub schema: u32,
    /// Monotonic sequence number per writer instance.
    pub seq: u64,
    /// Component that emitted the event (`"worker"`, `"executor"`, `"test"`).
    pub component: String,
    /// Lifecycle event name (e.g. `"started"`, `"sync_completed"`).
    pub event: String,
    /// PID of the writing process.
    pub pid: u32,
    /// Optional generation marker.
    pub generation: Option<u64>,
    /// Wall-clock timestamp in milliseconds since Unix epoch.
    pub at_unix_ms: u64,
    /// Optional JSON-encoded extra detail.
    pub detail: Option<String>,
}

// ---------------------------------------------------------------------------
// event_sink_path
// ---------------------------------------------------------------------------

/// Return the canonical path for the event-sink JSONL file inside a state dir.
pub fn event_sink_path(state_dir: &Path) -> PathBuf {
    state_dir.join("test-events.jsonl")
}

// ---------------------------------------------------------------------------
// EventSink (test-side reader)
// ---------------------------------------------------------------------------

/// Test-side handle that reads events from the JSON-lines file.
pub struct EventSink {
    path: PathBuf,
}

impl EventSink {
    /// Create a new sink pointing to `<state_dir>/test-events.jsonl`.
    pub fn new(state_dir: &Path) -> Self {
        Self {
            path: event_sink_path(state_dir),
        }
    }

    /// Read every event recorded so far.
    pub fn read_all(&self) -> Vec<EventRecord> {
        Self::read_lines(&self.path)
    }

    /// Block until an event matching `component` and `event` appears, or
    /// `timeout` elapses. Returns `None` on timeout.
    pub fn wait_for_event(
        &self,
        component: &str,
        event: &str,
        timeout: Duration,
    ) -> Option<EventRecord> {
        let deadline = Instant::now() + timeout;
        loop {
            for rec in self.read_all() {
                if rec.component == component && rec.event == event {
                    return Some(rec);
                }
            }
            if Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Like [`wait_for_event`](Self::wait_for_event) but also requires the
    /// generation field to match `r#gen`.
    pub fn wait_for_generation(
        &self,
        component: &str,
        event: &str,
        r#gen: u64,
        timeout: Duration,
    ) -> Option<EventRecord> {
        let deadline = Instant::now() + timeout;
        loop {
            for rec in self.read_all() {
                if rec.component == component && rec.event == event && rec.generation == Some(r#gen)
                {
                    return Some(rec);
                }
            }
            if Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Count events matching `component` and `event`.
    pub fn count_events(&self, component: &str, event: &str) -> usize {
        self.read_all()
            .iter()
            .filter(|r| r.component == component && r.event == event)
            .count()
    }

    /// Truncate the event file so subsequent reads start fresh.
    pub fn clear(&self) {
        if let Ok(f) = OpenOptions::new()
            .create(true)
            .truncate(true)
            .open(&self.path)
        {
            drop(f);
        }
    }

    // -- private ----------------------------------------------------------

    fn read_lines(path: &Path) -> Vec<EventRecord> {
        let file = match File::open(path) {
            Ok(f) => f,
            Err(_) => return Vec::new(),
        };
        let reader = BufReader::new(file);
        reader
            .lines()
            .map_while(Result::ok)
            .filter(|l| !l.trim().is_empty())
            .filter_map(|l| serde_json::from_str(&l).ok())
            .collect()
    }
}

// ---------------------------------------------------------------------------
// EventWriter (child-side writer)
// ---------------------------------------------------------------------------

/// Child-side handle that appends JSON-lines events to the shared file.
///
/// The writer opens the file in **append** mode on each [`write`](Self::write)
/// call so multiple processes can safely emit events concurrently.
pub struct EventWriter {
    path: PathBuf,
    seq: AtomicU64,
}

impl EventWriter {
    /// Create a new writer pointing to `<state_dir>/test-events.jsonl`.
    pub fn new(state_dir: &Path) -> Self {
        Self {
            path: event_sink_path(state_dir),
            seq: AtomicU64::new(1),
        }
    }

    /// Append a single event line.
    ///
    /// The file is opened in append mode per call for process safety.
    pub fn write(
        &self,
        component: &str,
        event: &str,
        pid: u32,
        generation: Option<u64>,
        detail: Option<String>,
    ) {
        let rec = EventRecord {
            schema: 1,
            seq: self.seq.fetch_add(1, Ordering::Relaxed),
            component: component.to_owned(),
            event: event.to_owned(),
            pid,
            generation,
            at_unix_ms: unix_ms(),
            detail,
        };
        let mut line = serde_json::to_string(&rec).unwrap_or_default();
        line.push('\n');

        if let Ok(mut f) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
        {
            let _ = f.write_all(line.as_bytes());
            let _ = f.flush();
        }
    }
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
