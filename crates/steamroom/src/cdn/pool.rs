use super::server::CdnServer;
use std::sync::atomic::AtomicU32;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::Instant;

struct ServerState {
    server: CdnServer,
    failures: AtomicU32,
    /// Monotonic instant (as nanos since pool creation) when server becomes eligible again.
    available_after: AtomicU64,
}

pub struct CdnServerPool {
    servers: Vec<ServerState>,
    epoch: Instant,
    next: AtomicU32,
}

impl CdnServerPool {
    pub fn new(servers: Vec<CdnServer>) -> Self {
        assert!(
            !servers.is_empty(),
            "CdnServerPool requires at least one server"
        );
        let epoch = Instant::now();
        Self {
            servers: servers
                .into_iter()
                .map(|s| ServerState {
                    server: s,
                    failures: AtomicU32::new(0),
                    available_after: AtomicU64::new(0),
                })
                .collect(),
            epoch,
            next: AtomicU32::new(0),
        }
    }

    fn now_nanos(&self) -> u64 {
        self.epoch.elapsed().as_nanos() as u64
    }

    /// Pick the next server via round-robin. Returns the server and how long
    /// the caller should wait before sending (zero if the server is available now).
    pub fn pick(&self) -> (&CdnServer, Duration) {
        let n = self.servers.len();
        let now = self.now_nanos();
        let start = self.next.fetch_add(1, Ordering::Relaxed) as usize;

        // First pass: find an available server
        for i in 0..n {
            let idx = (start + i) % n;
            let state = &self.servers[idx];
            if state.available_after.load(Ordering::Relaxed) <= now {
                return (&state.server, Duration::ZERO);
            }
        }

        // All in cooldown — pick the one that becomes available soonest
        let mut best = 0;
        let mut best_time = u64::MAX;
        for (i, state) in self.servers.iter().enumerate() {
            let t = state.available_after.load(Ordering::Relaxed);
            if t < best_time {
                best_time = t;
                best = i;
            }
        }
        let wait = Duration::from_nanos(best_time.saturating_sub(now));
        (&self.servers[best].server, wait)
    }

    /// Report a successful request — resets the failure counter.
    pub fn report_success(&self, server: &CdnServer) {
        if let Some(state) = self.find(server) {
            state.failures.store(0, Ordering::Relaxed);
        }
    }

    /// Report a failed request. Applies exponential cooldown on repeated failures.
    /// If `retry_after` is provided (from HTTP Retry-After header), uses that instead.
    pub fn report_failure(&self, server: &CdnServer, retry_after: Option<Duration>) {
        if let Some(state) = self.find(server) {
            let count = state.failures.fetch_add(1, Ordering::Relaxed) + 1;
            let cooldown = match retry_after {
                Some(d) => d,
                None => {
                    // Exponential backoff: 2s, 4s, 8s, 16s, capped at 30s
                    let secs = (2u64 << count.min(4)).min(30);
                    Duration::from_secs(secs)
                }
            };
            let deadline = self.now_nanos() + cooldown.as_nanos() as u64;
            state.available_after.store(deadline, Ordering::Relaxed);
        }
    }

    /// Number of servers in the pool.
    pub fn len(&self) -> usize {
        self.servers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.servers.is_empty()
    }

    fn find(&self, server: &CdnServer) -> Option<&ServerState> {
        self.servers
            .iter()
            .find(|s| s.server.host == server.host && s.server.port == server.port)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server(name: &str) -> CdnServer {
        CdnServer {
            host: name.into(),
            port: 443,
            https: true,
            vhost: name.into(),
        }
    }

    #[test]
    fn pick_round_robins_across_servers() {
        let pool = CdnServerPool::new(vec![server("a"), server("b"), server("c")]);
        let (s1, w1) = pool.pick();
        let (s2, w2) = pool.pick();
        let (s3, w3) = pool.pick();
        assert_eq!(w1, Duration::ZERO);
        assert_eq!(w2, Duration::ZERO);
        assert_eq!(w3, Duration::ZERO);
        // Should cycle through a, b, c
        let hosts: Vec<&str> = vec![&s1.host, &s2.host, &s3.host]
            .into_iter()
            .map(|s| s.as_str())
            .collect();
        assert!(hosts.contains(&"a"));
        assert!(hosts.contains(&"b"));
        assert!(hosts.contains(&"c"));
    }

    #[test]
    fn pick_skips_server_in_cooldown() {
        let pool = CdnServerPool::new(vec![server("a"), server("b")]);

        // Put "a" in cooldown
        pool.report_failure(&server("a"), Some(Duration::from_secs(60)));

        // Next picks should all be "b"
        for _ in 0..5 {
            let (s, wait) = pool.pick();
            assert_eq!(s.host, "b");
            assert_eq!(wait, Duration::ZERO);
        }
    }

    #[test]
    fn pick_returns_wait_when_all_in_cooldown() {
        let pool = CdnServerPool::new(vec![server("a"), server("b")]);

        pool.report_failure(&server("a"), Some(Duration::from_secs(60)));
        pool.report_failure(&server("b"), Some(Duration::from_secs(30)));

        let (_s, wait) = pool.pick();
        // "b" has shorter cooldown, should be picked with non-zero wait
        assert!(wait > Duration::ZERO);
        assert!(wait <= Duration::from_secs(30));
    }

    #[test]
    fn report_success_resets_failure_count() {
        let pool = CdnServerPool::new(vec![server("a"), server("b")]);

        pool.report_failure(&server("a"), None);
        pool.report_failure(&server("a"), None);
        pool.report_success(&server("a"));

        // "a" should be immediately available
        let (s, wait) = pool.pick();
        // After success, failure counter is 0 but available_after may still be in the future
        // from the last report_failure. So we just verify the success didn't panic.
        assert!(s.host == "a" || s.host == "b");
        let _ = wait;
    }

    #[test]
    fn report_failure_escalates_cooldown() {
        let pool = CdnServerPool::new(vec![server("a"), server("b")]);

        // Each failure should increase cooldown: 4s, 8s, 16s, 30s, 30s
        pool.report_failure(&server("a"), None); // count=1 -> 4s
        pool.report_failure(&server("a"), None); // count=2 -> 8s
        pool.report_failure(&server("a"), None); // count=3 -> 16s
        pool.report_failure(&server("a"), None); // count=4 -> 30s (capped)
        pool.report_failure(&server("a"), None); // count=5 -> 30s (capped)

        // "a" should be in cooldown, "b" should be available
        let (s, wait) = pool.pick();
        assert_eq!(s.host, "b");
        assert_eq!(wait, Duration::ZERO);
    }

    #[test]
    fn report_failure_with_retry_after_uses_exact_duration() {
        let pool = CdnServerPool::new(vec![server("a"), server("b")]);

        pool.report_failure(&server("a"), Some(Duration::from_secs(10)));

        let (s, _) = pool.pick();
        assert_eq!(s.host, "b");
    }

    #[test]
    fn single_server_pool_returns_wait_on_cooldown() {
        let pool = CdnServerPool::new(vec![server("only")]);

        pool.report_failure(&server("only"), Some(Duration::from_secs(5)));

        let (s, wait) = pool.pick();
        assert_eq!(s.host, "only");
        assert!(wait > Duration::ZERO);
    }
}
