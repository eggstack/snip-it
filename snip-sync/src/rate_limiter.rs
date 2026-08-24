use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;

/// Maximum number of rate limiter entries to prevent memory exhaustion from
/// requests with many distinct API keys or peer addresses.
const MAX_ENTRIES: usize = 100_000;

#[derive(Clone)]
struct WindowEntry {
    window_start: u64,
    count: usize,
}

struct RateLimiterState {
    windows: HashMap<String, WindowEntry>,
    oldest_first: BTreeSet<(u64, String)>,
}

/// Bounded, process-local request windows. Windows intentionally reset when
/// the server restarts; rate-limit continuity is not part of the server's
/// persisted control-plane state.
pub struct RateLimiter {
    windows: Arc<Mutex<RateLimiterState>>,
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
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
        let now = now_secs();
        let window_secs = window.as_secs();
        let mut windows = self.windows.lock().await;

        if !windows.windows.contains_key(key)
            && windows.windows.len() >= MAX_ENTRIES
            && let Some((_, oldest_key)) = windows.oldest_first.pop_first()
        {
            windows.windows.remove(&oldest_key);
        }

        if let Some(window_start) = windows.windows.get(key).map(|entry| entry.window_start) {
            if now.saturating_sub(window_start) >= window_secs {
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
}
