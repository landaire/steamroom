use super::server::CdnServer;

pub fn detect() -> Option<std::net::IpAddr> {
    // Lancache detection: resolve "lancache.steamcontent.com" via DNS
    // If it resolves to a private IP, there's a lancache on the LAN
    use std::net::ToSocketAddrs;
    let addrs = "lancache.steamcontent.com:80".to_socket_addrs().ok()?;
    for addr in addrs {
        let ip = addr.ip();
        if is_private(&ip) {
            return Some(ip);
        }
    }
    None
}

pub fn build_url(lancache_ip: std::net::IpAddr, _server: &CdnServer, path: &str) -> String {
    format!("http://{lancache_ip}{path}")
}

pub fn host_header(server: &CdnServer) -> String {
    if server.vhost.is_empty() {
        server.host.clone()
    } else {
        server.vhost.clone()
    }
}

fn is_private(ip: &std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => {
            v4.is_private() || v4.is_loopback() || v4.is_link_local()
        }
        std::net::IpAddr::V6(v6) => v6.is_loopback(),
    }
}
