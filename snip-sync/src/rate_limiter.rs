use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;

#[derive(Clone)]
struct WindowEntry {
    window_start: u64,
    count: usize,
}

pub struct RateLimiter {
    windows: Arc<Mutex<HashMap<String, WindowEntry>>>,
    shutdown_tx: Arc<tokio::sync::oneshot::Sender<()>>,
    db_pool: Option<sqlx::SqlitePool>,
    persist: bool,
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

impl RateLimiter {
    pub fn new() -> Self {
        Self::new_inner(None, false)
    }

    pub fn new_with_db(pool: sqlx::SqlitePool, persist: bool) -> Self {
        Self::new_inner(Some(pool), persist)
    }

    fn new_inner(db_pool: Option<sqlx::SqlitePool>, persist: bool) -> Self {
        let windows: Arc<Mutex<HashMap<String, WindowEntry>>> =
            Arc::new(Mutex::new(HashMap::new()));

        let windows_clone = windows.clone();
        let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel();

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(60)) => {
                        let mut windows = windows_clone.lock().await;
                        let now = now_secs();
                        windows.retain(|_, entry| now.saturating_sub(entry.window_start) < 60);
                    }
                    _ = &mut shutdown_rx => {
                        tracing::debug!("Rate limiter cleanup task shutting down");
                        break;
                    }
                }
            }
        });

        Self {
            windows,
            shutdown_tx: Arc::new(shutdown_tx),
            db_pool,
            persist,
        }
    }

    pub async fn load_state(&self) {
        let pool = match &self.db_pool {
            Some(p) => p,
            None => return,
        };
        if !self.persist {
            return;
        }

        let rows: Vec<(String, i64, i64)> =
            match sqlx::query_as("SELECT peer_ip, window_start, request_count FROM rate_limits")
                .fetch_all(pool)
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!("Failed to load rate limit state: {}", e);
                    return;
                }
            };

        let now = now_secs();
        let mut windows = self.windows.lock().await;
        let mut loaded = 0u32;

        for (peer_ip, window_start, request_count) in rows {
            let ws = window_start as u64;
            if now.saturating_sub(ws) >= 60 {
                continue;
            }
            windows.insert(
                peer_ip,
                WindowEntry {
                    window_start: ws,
                    count: request_count as usize,
                },
            );
            loaded += 1;
        }

        tracing::info!("Loaded rate limit state for {} peers", loaded);
    }

    pub async fn save_state(&self) {
        let pool = match &self.db_pool {
            Some(p) => p,
            None => return,
        };
        if !self.persist {
            return;
        }

        let windows = self.windows.lock().await;

        for (key, entry) in windows.iter() {
            if entry.count == 0 {
                continue;
            }
            if let Err(e) = sqlx::query(
                "INSERT INTO rate_limits (peer_ip, window_start, request_count) VALUES (?, ?, ?)
                 ON CONFLICT(peer_ip) DO UPDATE SET window_start = excluded.window_start, request_count = excluded.request_count",
            )
            .bind(key)
            .bind(entry.window_start as i64)
            .bind(entry.count as i64)
            .execute(pool)
            .await
            {
                tracing::warn!("Failed to save rate limit entry for {}: {}", key, e);
            }
        }
    }

    pub fn start_persistence_task(self: &Arc<Self>) {
        if !self.persist || self.db_pool.is_none() {
            return;
        }

        let limiter = Arc::clone(self);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(30));
            loop {
                interval.tick().await;
                limiter.save_state().await;
            }
        });
    }

    pub async fn allow(&self, key: &str, max_requests: usize, window: Duration) -> bool {
        let now = now_secs();
        let window_secs = window.as_secs();
        let mut windows = self.windows.lock().await;

        let entry = windows
            .entry(key.to_string())
            .or_insert_with(|| WindowEntry {
                window_start: now,
                count: 0,
            });

        if now.saturating_sub(entry.window_start) >= window_secs {
            entry.window_start = now;
            entry.count = 0;
        }

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
