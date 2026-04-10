pub mod encryption;
pub mod framing;

use std::net::SocketAddr;
use serde::Deserialize;
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

#[derive(Deserialize)]
struct CmListResponse {
    response: CmListResponseInner,
}

#[derive(Deserialize)]
struct CmListResponseInner {
    #[serde(default)]
    serverlist: Vec<String>,
    #[serde(default)]
    serverlist_websockets: Vec<String>,
}

pub async fn fetch_cm_servers() -> Result<Vec<CmServer>, Error> {
    let url = "https://api.steampowered.com/ISteamDirectory/GetCMListForConnect/v1/?cellid=0";
    let client = reqwest::Client::new();
    let resp: CmListResponse = client.get(url).send().await?.json().await?;

    let mut servers = Vec::new();

    for entry in &resp.response.serverlist {
        if let Some(server) = parse_cm_entry(entry, Protocol::Tcp) {
            servers.push(server);
        }
    }

    for entry in &resp.response.serverlist_websockets {
        if let Some(server) = parse_cm_entry(entry, Protocol::WebSocket) {
            servers.push(server);
        }
    }

    if servers.is_empty() {
        return Ok(default_cm_servers());
    }

    Ok(servers)
}

fn parse_cm_entry(entry: &str, protocol: Protocol) -> Option<CmServer> {
    if let Ok(addr) = entry.parse::<SocketAddr>() {
        Some(CmServer {
            addr: CmServerAddr::Resolved(addr),
            protocol,
        })
    } else if let Some((host, port_str)) = entry.rsplit_once(':') {
        let port = port_str.parse().ok()?;
        Some(CmServer {
            addr: CmServerAddr::Dns {
                host: host.to_owned(),
                port,
            },
            protocol,
        })
    } else {
        None
    }
}
