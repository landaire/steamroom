use prost::Message;

pub struct SessionInner {
    pub rt: tokio::runtime::Runtime,
    pub client: steamroom::client::SteamClient<steamroom::client::LoggedIn>,
}

pub struct FileListInner {
    pub names: Vec<String>,
    pub sizes: Vec<u64>,
    pub dirs: Vec<bool>,
}

pub fn connect_anonymous() -> Result<SessionInner, String> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    let client = rt.block_on(do_connect_anon()).map_err(|e| e.to_string())?;
    Ok(SessionInner { rt, client })
}

pub fn connect_with_token(username: &str, token: &str) -> Result<SessionInner, String> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    let u = username.to_string();
    let t = token.to_string();
    let client = rt
        .block_on(do_connect_token(u, t))
        .map_err(|e| e.to_string())?;
    Ok(SessionInner { rt, client })
}

pub fn list_depot_files(
    session: &SessionInner,
    app_id: u32,
    depot_id: u32,
    branch: &str,
) -> Result<FileListInner, String> {
    session
        .rt
        .block_on(do_list_files(&session.client, app_id, depot_id, branch))
        .map_err(|e| e.to_string())
}

async fn ws_connect()
-> Result<steamroom::client::SteamClient<steamroom::client::Encrypted>, steamroom::error::Error> {
    let servers = steamroom::connection::CmServer::fetch()
        .await
        .unwrap_or_else(|_| steamroom::connection::CmServer::defaults());
    let ws = servers
        .iter()
        .find(|s| s.protocol == steamroom::connection::Protocol::WebSocket)
        .or_else(|| servers.first())
        .ok_or_else(|| {
            steamroom::error::Error::Connection(
                steamroom::error::ConnectionError::DnsResolutionFailed,
            )
        })?;
    let transport = steamroom::transport::websocket::WebSocketTransport::connect(ws).await?;
    let (client, _) = steamroom::client::SteamClient::connect_ws(transport).await?;
    Ok(client)
}

async fn do_login(
    client: steamroom::client::SteamClient<steamroom::client::Encrypted>,
    logon: steamroom::generated::CMsgClientLogon,
    steam_id: u64,
) -> Result<steamroom::client::SteamClient<steamroom::client::LoggedIn>, steamroom::error::Error> {
    let client = client.prepare().await?;
    let body = logon.encode_to_vec();
    let mut msg = steamroom::client::msg::ClientMsg::with_body(
        steamroom::messages::EMsg::CLIENT_LOGON,
        &body,
    );
    msg.header.steamid = Some(steam_id);
    msg.header.client_sessionid = Some(0);
    let (logged_in, _) = client.login(msg).await?;
    Ok(logged_in)
}

async fn do_connect_anon()
-> Result<steamroom::client::SteamClient<steamroom::client::LoggedIn>, steamroom::error::Error> {
    let client = ws_connect().await?;
    let logon = steamroom::generated::CMsgClientLogon {
        protocol_version: Some(steamroom::client::PROTOCOL_VERSION),
        cell_id: Some(0),
        client_os_type: Some(20),
        ..Default::default()
    };
    do_login(
        client,
        logon,
        steamroom::types::SteamId::from_parts(1, 10, 0, 0).raw(),
    )
    .await
}

async fn do_connect_token(
    username: String,
    token: String,
) -> Result<steamroom::client::SteamClient<steamroom::client::LoggedIn>, steamroom::error::Error> {
    let client = ws_connect().await?;
    let logon = steamroom::generated::CMsgClientLogon {
        protocol_version: Some(steamroom::client::PROTOCOL_VERSION),
        cell_id: Some(0),
        client_os_type: Some(20),
        account_name: Some(username),
        access_token: Some(token),
        ..Default::default()
    };
    do_login(
        client,
        logon,
        steamroom::types::SteamId::from_parts(1, 1, 1, 0).raw(),
    )
    .await
}

async fn do_list_files(
    client: &steamroom::client::SteamClient<steamroom::client::LoggedIn>,
    app_id: u32,
    depot_id: u32,
    branch: &str,
) -> Result<FileListInner, Box<dyn std::error::Error + Send + Sync>> {
    let app = steamroom::depot::AppId(app_id);
    let depot = steamroom::depot::DepotId(depot_id);

    let tokens = client.pics_get_access_tokens(&[app]).await?;
    let token = tokens
        .into_iter()
        .next()
        .unwrap_or(steamroom::apps::AccessToken {
            app_id: app,
            token: 0,
        });
    let infos = client.pics_get_product_info(&[token]).await?;
    let info = infos.into_iter().next().ok_or("no product info")?;
    let details = steamroom::apps::AppDetails::from_key_values(app, info.key_values()?);
    let manifest_id = details
        .depot(depot)
        .and_then(|d| d.manifest(branch))
        .map(|m| m.manifest_id)
        .ok_or("manifest not found for depot/branch")?;

    let depot_key = client.get_depot_decryption_key(depot, app).await?;
    let request_code = client
        .get_manifest_request_code(app, depot, manifest_id, Some(branch), None)
        .await?
        .unwrap_or(0);

    let cdn_servers = client
        .get_cdn_servers(steamroom::depot::CellId(0), Some(5))
        .await?;
    if cdn_servers.is_empty() {
        return Err("no cdn servers".into());
    }
    let cdn_pool = steamroom::cdn::CdnServerPool::new(cdn_servers);
    let cdn = steamroom::cdn::CdnClient::new()?;
    let raw = cdn
        .download_manifest_pooled(&cdn_pool, depot, manifest_id, request_code, None)
        .await?;

    let bytes = decompress(&raw)?;
    let mut manifest = steamroom::depot::manifest::DepotManifest::parse(&bytes)?;
    if manifest.filenames_encrypted {
        manifest.decrypt_filenames(&depot_key)?;
    }

    let mut names = Vec::with_capacity(manifest.files.len());
    let mut sizes = Vec::with_capacity(manifest.files.len());
    let mut dirs = Vec::with_capacity(manifest.files.len());
    for f in &manifest.files {
        names.push(f.filename.clone());
        sizes.push(f.size);
        dirs.push(steamroom::enums::DepotFileFlags::from_bits_retain(f.flags).is_directory());
    }

    Ok(FileListInner { names, sizes, dirs })
}

fn decompress(data: &[u8]) -> Result<Vec<u8>, std::io::Error> {
    if data.len() > 2 && data[0] == 0x50 && data[1] == 0x4B {
        let cursor = std::io::Cursor::new(data);
        let mut archive = zip::ZipArchive::new(cursor)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        if archive.is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "empty",
            ));
        }
        let mut file = archive
            .by_index(0)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        let mut buf = Vec::new();
        std::io::Read::read_to_end(&mut file, &mut buf)?;
        Ok(buf)
    } else {
        Ok(data.to_vec())
    }
}
