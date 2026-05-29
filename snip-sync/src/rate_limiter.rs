use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

pub struct RateLimiter {
    requests: Arc<Mutex<HashMap<String, Vec<Instant>>>>,
}

impl RateLimiter {
    pub fn new() -> Self {
        let requests: Arc<Mutex<HashMap<String, Vec<Instant>>>> =
            Arc::new(Mutex::new(HashMap::new()));

        let requests_clone = requests.clone();

        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(60)).await;
                let mut requests = requests_clone.lock().await;
                let now = Instant::now();
                requests.retain(|_, times| {
                    times.retain(|&t| now.duration_since(t) < Duration::from_secs(60));
                    !times.is_empty()
                });
            }
        });

        Self { requests }
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
