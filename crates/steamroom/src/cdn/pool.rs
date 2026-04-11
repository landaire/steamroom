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

    fn find(&self, server: &CdnServer) -> Option<&ServerState> {
        self.servers
            .iter()
            .find(|s| s.server.host == server.host && s.server.port == server.port)
    }
}
