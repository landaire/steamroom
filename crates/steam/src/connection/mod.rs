pub mod encryption;
pub mod framing;

use std::net::SocketAddr;
use crate::error::Error;

#[derive(Clone, Debug)]
pub struct CmServer {
    pub addr: CmServerAddr,
    pub protocol: Protocol,
}

#[derive(Clone, Debug)]
pub enum CmServerAddr {
    Resolved(SocketAddr),
    Dns { host: String, port: u16 },
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum Protocol {
    Tcp,
    WebSocket,
}

pub static DEFAULT_CM_SERVERS: &[&str] = &[
    "cm1-ord1.cm.steampowered.com",
    "cm2-ord1.cm.steampowered.com",
    "cm1-iad1.cm.steampowered.com",
    "cm2-iad1.cm.steampowered.com",
];

pub fn default_cm_servers() -> Vec<CmServer> {
    DEFAULT_CM_SERVERS
        .iter()
        .map(|host| CmServer {
            addr: CmServerAddr::Dns {
                host: (*host).to_owned(),
                port: 27017,
            },
            protocol: Protocol::Tcp,
        })
        .collect()
}

pub async fn fetch_cm_servers() -> Result<Vec<CmServer>, Error> {
    todo!()
}
