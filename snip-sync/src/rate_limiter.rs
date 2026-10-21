use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

/// Maximum number of rate limiter entries to prevent memory exhaustion from
/// requests with many distinct API keys or peer addresses.
const MAX_ENTRIES: usize = 100_000;

/// Smallest window the limiter will use. A zero-length window would reset
/// the window on every call (see `allow`), which silently disables rate
/// limiting, so it is clamped to one second instead.
const MIN_WINDOW_SECS: u64 = 1;

#[derive(Clone)]
struct WindowEntry {
    /// Monotonic instant the current window started. A monotonic clock is
    /// used on purpose: a wall-clock step backwards (NTP correction, VM
    /// snapshot restore) must not pin every client inside a full window.
    window_start: Instant,
    count: usize,
}

struct RateLimiterState {
    windows: HashMap<String, WindowEntry>,
    oldest_first: BTreeSet<(Instant, String)>,
}

/// Bounded, process-local request windows. Windows intentionally reset when
/// the server restarts; rate-limit continuity is not part of the server's
/// persisted control-plane state.
pub struct RateLimiter {
    windows: Arc<Mutex<RateLimiterState>>,
}

impl RateLimiter {
    pub fn new() -> Self {
        Self {
            windows: Arc::new(Mutex::new(RateLimiterState {
                windows: HashMap::new(),
                oldest_first: BTreeSet::new(),
            })),
        }
    }

    pub async fn allow(&self, key: &str, max_requests: usize, window: Duration) -> bool {
        // A zero request budget can never be satisfied. Return early so no
        // window state is created for a key that is never allowed.
        if max_requests == 0 {
            return false;
        }
        // A zero-length window would expire on every call, which disables
        // rate limiting entirely. Clamp it to the smallest window that still
        // enforces the limit.
        let window_secs = window.as_secs().max(MIN_WINDOW_SECS);

        let now = Instant::now();
        let mut windows = self.windows.lock().await;

        if !windows.windows.contains_key(key)
            && windows.windows.len() >= MAX_ENTRIES
            && let Some((_, oldest_key)) = windows.oldest_first.pop_first()
        {
            windows.windows.remove(&oldest_key);
        }

        if let Some(window_start) = windows.windows.get(key).map(|entry| entry.window_start) {
            if now.saturating_duration_since(window_start).as_secs() >= window_secs {
                windows
                    .oldest_first
                    .remove(&(window_start, key.to_string()));
                let entry = windows.windows.get_mut(key).expect("entry exists");
                entry.window_start = now;
                entry.count = 0;
                windows.oldest_first.insert((now, key.to_string()));
            }
        } else {
            windows.windows.insert(
                key.to_string(),
                WindowEntry {
                    window_start: now,
                    count: 0,
                },
            );
            windows.oldest_first.insert((now, key.to_string()));
        }

        let entry = windows.windows.get_mut(key).expect("entry inserted above");
        if entry.count >= max_requests {
            return false;
        }

        entry.count += 1;
        true
    }

    /// Test-only accessor for the start of a key's current window.
    #[cfg(test)]
    async fn window_start_for(&self, key: &str) -> Option<Instant> {
        let windows = self.windows.lock().await;
        windows.windows.get(key).map(|entry| entry.window_start)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_allow_within_limit() {
        let limiter = RateLimiter::new();
        assert!(
            limiter
                .allow("192.168.1.1", 5, Duration::from_secs(60))
                .await
        );
        assert!(
            limiter
                .allow("192.168.1.1", 5, Duration::from_secs(60))
                .await
        );
        assert!(
            limiter
                .allow("192.168.1.1", 5, Duration::from_secs(60))
                .await
        );
    }

    #[tokio::test]
    async fn test_deny_at_limit() {
        let limiter = RateLimiter::new();
        for _ in 0..3 {
            assert!(limiter.allow("10.0.0.1", 3, Duration::from_secs(60)).await);
        }
        assert!(!limiter.allow("10.0.0.1", 3, Duration::from_secs(60)).await);
    }

    #[tokio::test]
    async fn test_separate_keys_independent() {
        let limiter = RateLimiter::new();
        for _ in 0..2 {
            assert!(limiter.allow("key-a", 2, Duration::from_secs(60)).await);
        }
        assert!(!limiter.allow("key-a", 2, Duration::from_secs(60)).await);
        assert!(limiter.allow("key-b", 2, Duration::from_secs(60)).await);
    }

    #[tokio::test]
    async fn test_window_expiry_resets_count() {
        let limiter = RateLimiter::new();
        for _ in 0..2 {
            assert!(
                limiter
                    .allow("expire-test", 2, Duration::from_secs(1))
                    .await
            );
        }
        assert!(
            !limiter
                .allow("expire-test", 2, Duration::from_secs(1))
                .await
        );
        tokio::time::sleep(Duration::from_secs(2)).await;
        assert!(
            limiter
                .allow("expire-test", 2, Duration::from_secs(1))
                .await
        );
    }

    #[tokio::test]
    async fn test_zero_max_requests_deny_all() {
        let limiter = RateLimiter::new();
        assert!(!limiter.allow("zero", 0, Duration::from_secs(60)).await);
    }

    #[tokio::test]
    async fn zero_length_window_still_enforces_the_limit() {
        let limiter = RateLimiter::new();
        // A zero-length window must not disable rate limiting: the limit is
        // still enforced inside the clamped (one second) window.
        assert!(limiter.allow("zero-window", 2, Duration::ZERO).await);
        assert!(limiter.allow("zero-window", 2, Duration::ZERO).await);
        assert!(!limiter.allow("zero-window", 2, Duration::ZERO).await);
    }

    #[tokio::test]
    async fn zero_length_window_with_zero_budget_denies_everything() {
        let limiter = RateLimiter::new();
        assert!(!limiter.allow("zero-zero", 0, Duration::ZERO).await);
    }

    #[tokio::test]
    async fn window_bookkeeping_uses_a_monotonic_clock() {
        let limiter = RateLimiter::new();
        assert!(limiter.allow("monotonic", 5, Duration::from_secs(60)).await);

        let window_start = limiter
            .window_start_for("monotonic")
            .await
            .expect("the window must exist after an allowed request");
        // The window start is a monotonic instant, so it can never be behind
        // the wall clock: a backwards clock step cannot pin the client
        // inside a full window.
        let now = Instant::now();
        assert!(
            window_start <= now,
            "window start {window_start:?} must not be in the future"
        );
        assert!(
            now.duration_since(window_start) < Duration::from_secs(5),
            "window start must track the monotonic clock"
        );
    }
}
