use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

pub struct RateLimiter {
    requests: Arc<Mutex<HashMap<String, Vec<Instant>>>>,
    #[allow(dead_code)]
    shutdown_tx: Arc<tokio::sync::oneshot::Sender<()>>,
}

impl RateLimiter {
    pub fn new() -> Self {
        let requests: Arc<Mutex<HashMap<String, Vec<Instant>>>> =
            Arc::new(Mutex::new(HashMap::new()));

        let requests_clone = requests.clone();
        let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel();

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(60)) => {
                        let mut requests = requests_clone.lock().await;
                        let now = Instant::now();
                        requests.retain(|_, times| {
                            times.retain(|&t| now.duration_since(t) < Duration::from_secs(60));
                            !times.is_empty()
                        });
                    }
                    _ = &mut shutdown_rx => {
                        tracing::debug!("Rate limiter cleanup task shutting down");
                        break;
                    }
                }
            }
        });

        Self {
            requests,
            shutdown_tx: Arc::new(shutdown_tx),
        }
    }

    pub async fn allow(&self, key: &str, max_requests: usize, window: Duration) -> bool {
        let now = Instant::now();
        let mut requests = self.requests.lock().await;

        let entry = requests.entry(key.to_string()).or_default();

        entry.retain(|&t| now.duration_since(t) < window);

        if entry.len() >= max_requests {
            return false;
        }

        entry.push(now);
        true
    }
}
