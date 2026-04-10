use super::server::CdnServer;
use crate::depot::DepotId;

pub fn detect() -> Option<std::net::IpAddr> {
    todo!()
}

pub fn build_url(lancache_ip: std::net::IpAddr, server: &CdnServer, path: &str) -> String {
    todo!()
}

pub fn host_header(server: &CdnServer) -> String {
    todo!()
}
