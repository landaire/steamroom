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

static DEFAULT_CM_ADDRS: &[&str] = &[
    "162.254.193.102:27017",
    "162.254.195.66:27017",
    "205.196.6.148:27017",
];

impl CmServer {
    pub fn defaults() -> Vec<Self> {
        DEFAULT_CM_ADDRS
            .iter()
            .filter_map(|entry| parse_cm_entry(entry, Protocol::Tcp))
            .collect()
    }

    pub async fn fetch() -> Result<Vec<Self>, Error> {
        fetch_cm_servers().await
    }
}

#[derive(Deserialize)]
struct CmListResponse {
    response: CmListResponseInner,
}

#[derive(Deserialize)]
struct CmListResponseInner {
    #[serde(default)]
    serverlist: Vec<CmServerEntry>,
}

#[derive(Deserialize)]
struct CmServerEntry {
    endpoint: String,
    #[serde(default)]
    r#type: String,
}

pub async fn fetch_cm_servers() -> Result<Vec<CmServer>, Error> {
    let url = "https://api.steampowered.com/ISteamDirectory/GetCMListForConnect/v1/?cellid=0";
    let client = reqwest::Client::new();
    let resp: CmListResponse = client.get(url).send().await?.json().await?;

    let mut servers = Vec::new();

    for entry in &resp.response.serverlist {
        let protocol = match entry.r#type.as_str() {
            "netfilter" => Protocol::Tcp,
            "websockets" => Protocol::WebSocket,
            _ => continue,
        };
        if let Some(server) = parse_cm_entry(&entry.endpoint, protocol) {
            servers.push(server);
        }
    }

    // Sort TCP servers first (we currently only support TCP)
    servers.sort_by_key(|s| match s.protocol {
        Protocol::Tcp => 0,
        Protocol::WebSocket => 1,
    });

    if servers.is_empty() {
        return Ok(CmServer::defaults());
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
