use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let proto_dir = PathBuf::from("proto/steam");

    let protos: Vec<PathBuf> = [
        "steammessages_base.proto",
        "steammessages_unified_base.steamclient.proto",
        "enums.proto",
        "enums_clientserver.proto",
        "enums_productinfo.proto",
        "steammessages_auth.steamclient.proto",
        "steammessages_contentsystem.steamclient.proto",
        "steammessages_clientserver_login.proto",
        "steammessages_clientserver.proto",
        "steammessages_clientserver_2.proto",
        "steammessages_clientserver_appinfo.proto",
        "steammessages_clientserver_friends.proto",
        "steammessages_clientserver_uds.proto",
        "steammessages_player.steamclient.proto",
        "steammessages_publishedfile.steamclient.proto",
        "steammessages_twofactor.steamclient.proto",
        "steammessages_cloud.steamclient.proto",
        "steammessages_client_objects.proto",
        "content_manifest.proto",
        "encrypted_app_ticket.proto",
        "clientmetrics.proto",
        "offline_ticket.proto",
        "steammessages_workshop.steamclient.proto",
    ]
    .iter()
    .map(|p| proto_dir.join(p))
    .collect();

    // Use protox (pure Rust) to parse .proto files into a FileDescriptorSet,
    // then feed that to prost-build. No protoc binary needed.
    let file_descriptors = protox::compile(
        &proto_paths_as_str(&protos),
        &[proto_dir.to_str().unwrap()],
    )?;

    prost_build::Config::new()
        .compile_fds(file_descriptors)?;

    Ok(())
}

fn proto_paths_as_str(paths: &[PathBuf]) -> Vec<&str> {
    paths.iter().filter_map(|p| p.to_str()).collect()
}
