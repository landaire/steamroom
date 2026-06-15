/// Lancache detection and URL rewriting for local CDN caches.
pub mod lancache;
/// Lock-free CDN server pool with health tracking and round-robin selection.
pub mod pool;
/// CDN server address representation.
pub mod server;

pub use self::pool::CdnServerPool;
pub use self::server::CdnServer;
use crate::depot::ChunkId;
use crate::depot::DepotId;
use crate::depot::ManifestId;
use crate::error::Error;
use bytes::Bytes;

/// HTTP client for downloading manifests and chunks from Steam's CDN.
/// Validates response status codes and parses `Retry-After` headers.
/// Supports lancache proxying via [`with_lancache`](Self::with_lancache).
pub struct CdnClient {
    client: reqwest::Client,
    lancache_ip: Option<std::net::IpAddr>,
}

impl CdnClient {
    pub fn new() -> Result<Self, Error> {
        let client = reqwest::Client::builder().build().map_err(Error::Http)?;
        Ok(Self {
            client,
            lancache_ip: None,
        })
    }

    pub fn with_lancache(mut self) -> Self {
        self.lancache_ip = lancache::detect();
        self
    }

    pub fn is_lancache(&self) -> bool {
        self.lancache_ip.is_some()
    }

    pub async fn download_manifest(
        &self,
        server: &CdnServer,
        depot_id: DepotId,
        manifest_id: ManifestId,
        request_code: u64,
        cdn_auth_token: Option<&str>,
    ) -> Result<Bytes, Error> {
        let path = format!(
            "/depot/{}/manifest/{}/5/{}",
            depot_id.0, manifest_id.0, request_code
        );
        let url = self.build_url(server, &path, cdn_auth_token);
        let resp = self.build_request(server, &url).send().await?;
        Self::check_response(resp).await
    }

    pub async fn download_chunk(
        &self,
        server: &CdnServer,
        depot_id: DepotId,
        chunk_id: &ChunkId,
        cdn_auth_token: Option<&str>,
    ) -> Result<Bytes, Error> {
        let path = format!("/depot/{}/chunk/{}", depot_id.0, chunk_id);
        let url = self.build_url(server, &path, cdn_auth_token);
        let resp = self.build_request(server, &url).send().await?;
        Self::check_response(resp).await
    }

    /// Download a manifest with CDN pool rotation and fallback. Picks a server
    /// from `pool` and, on a transport or CDN error, cools that server down and
    /// retries with the next healthy one. Tries each server at most once and
    /// returns the last error if every server fails.
    ///
    /// Prefer this over [`download_manifest`](Self::download_manifest) whenever a
    /// [`CdnServerPool`] is available: Steam's directory lists internal hosts
    /// (e.g. `*.valve.org`, reporting load 0 so they sort first) that do not
    /// resolve publicly, so a single-server fetch deterministically hits an
    /// unreachable host.
    pub async fn download_manifest_pooled(
        &self,
        pool: &CdnServerPool,
        depot_id: DepotId,
        manifest_id: ManifestId,
        request_code: u64,
        cdn_auth_token: Option<&str>,
    ) -> Result<Bytes, Error> {
        let mut last_err = None;
        for _ in 0..pool.len() {
            let (server, wait) = pool.pick();
            if !wait.is_zero() {
                tracing::warn!(
                    server = %server.host,
                    wait_secs = wait.as_secs_f32(),
                    "all CDN servers in cooldown, waiting"
                );
                tokio::time::sleep(wait).await;
            }
            match self
                .download_manifest(server, depot_id, manifest_id, request_code, cdn_auth_token)
                .await
            {
                Ok(data) => {
                    pool.report_success(server);
                    return Ok(data);
                }
                Err(e) => {
                    let retry_after = match &e {
                        Error::CdnStatus { retry_after, .. } => *retry_after,
                        _ => None,
                    }
                    .map(std::time::Duration::from_secs);
                    tracing::warn!(
                        server = %server.host,
                        error = %e,
                        "manifest download failed, trying another CDN server"
                    );
                    pool.report_failure(server, retry_after);
                    last_err = Some(e);
                }
            }
        }
        // `CdnServerPool::new` asserts the pool is non-empty, so the loop runs at
        // least once and `last_err` is always set when we reach this point.
        match last_err {
            Some(e) => Err(e),
            None => unreachable!("CdnServerPool is non-empty by construction"),
        }
    }

    async fn check_response(resp: reqwest::Response) -> Result<Bytes, Error> {
        let status = resp.status();
        if status == reqwest::StatusCode::OK {
            return Ok(resp.bytes().await?);
        }
        let retry_after = resp
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok());
        Err(Error::CdnStatus {
            status,
            retry_after,
        })
    }

    fn build_url(&self, server: &CdnServer, path: &str, cdn_auth_token: Option<&str>) -> String {
        if let Some(ip) = self.lancache_ip {
            lancache::build_url(ip, server, path)
        } else {
            server.build_url(path, cdn_auth_token)
        }
    }

    fn build_request(&self, server: &CdnServer, url: &str) -> reqwest::RequestBuilder {
        let mut req = self.client.get(url);
        if self.lancache_ip.is_some() {
            req = req.header("Host", lancache::host_header(server));
        }
        req
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cdn::pool::CdnServerPool;
    use tokio::io::AsyncReadExt;
    use tokio::io::AsyncWriteExt;
    use tokio::net::TcpListener;

    fn http_server(host: &str, port: u16) -> CdnServer {
        CdnServer {
            host: host.to_string(),
            port,
            https: false,
            vhost: host.to_string(),
        }
    }

    /// Regression: Steam's directory lists internal hosts (e.g. `*.valve.org`,
    /// load=0) that sort first but do not resolve, so the pooled download must
    /// skip a failing server and fall back to a healthy one.
    #[tokio::test]
    async fn download_manifest_pooled_falls_back_past_failing_server() {
        let body = b"decompressed-manifest-bytes".to_vec();

        // Healthy server: a minimal HTTP/1.1 responder that returns 200.
        let good = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let good_port = good.local_addr().unwrap().port();
        let resp_body = body.clone();
        tokio::spawn(async move {
            let (mut sock, _) = good.accept().await.unwrap();
            let mut buf = [0u8; 2048];
            let _ = sock.read(&mut buf).await.unwrap();
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
                resp_body.len()
            );
            sock.write_all(head.as_bytes()).await.unwrap();
            sock.write_all(&resp_body).await.unwrap();
            sock.flush().await.unwrap();
        });

        // Dead server: bind then drop so the port refuses connections.
        let dead = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let dead_port = dead.local_addr().unwrap().port();
        drop(dead);

        // Dead server first, mirroring the load=0 internal host that sorts first.
        let pool = CdnServerPool::new(vec![
            http_server("127.0.0.1", dead_port),
            http_server("127.0.0.1", good_port),
        ]);

        let cdn = CdnClient::new().unwrap();
        let data = cdn
            .download_manifest_pooled(&pool, DepotId(700331), ManifestId(123), 5, None)
            .await
            .expect("should fall back to the healthy server");
        assert_eq!(data.as_ref(), body.as_slice());
    }
}
