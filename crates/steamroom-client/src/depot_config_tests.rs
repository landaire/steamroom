use steamroom::depot::{DepotId, DepotKey, ManifestId};
use crate::depot_config::DepotConfig;

#[test]
fn round_trip_save_load() {
    let dir = tempfile::tempdir().unwrap();
    let depot_id = DepotId(481);
    let manifest_id = ManifestId(123456789);
    let key = DepotKey([0xAB; 32]);

    let mut config = DepotConfig::default();
    config.set_installed(depot_id, manifest_id, &key);
    config.save(dir.path()).unwrap();

    let loaded = DepotConfig::load(dir.path());
    let (loaded_mid, loaded_key) = loaded.get_installed(depot_id).unwrap();
    assert_eq!(loaded_mid, manifest_id);
    assert_eq!(loaded_key.0, key.0);
}

#[test]
fn get_installed_returns_none_for_missing_depot() {
    let config = DepotConfig::default();
    assert!(config.get_installed(DepotId(999)).is_none());
}

#[test]
fn load_returns_default_for_missing_file() {
    let dir = tempfile::tempdir().unwrap();
    let config = DepotConfig::load(dir.path());
    assert!(config.depots.is_empty());
}

#[test]
fn manifest_raw_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let depot_id = DepotId(481);
    let manifest_id = ManifestId(111);
    let data = b"PK\x03\x04fake-zip-data";

    DepotConfig::save_manifest_raw(dir.path(), depot_id, manifest_id, data).unwrap();

    let path = dir.path()
        .join(".depotdownloader")
        .join("manifests")
        .join("481_111.zip");
    assert_eq!(std::fs::read(&path).unwrap(), data);
}

#[test]
fn manifest_decompressed_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let depot_id = DepotId(481);
    let manifest_id = ManifestId(222);
    let data = b"protobuf-manifest-bytes";

    DepotConfig::save_manifest_decompressed(dir.path(), depot_id, manifest_id, data).unwrap();

    let loaded = DepotConfig::load_manifest_decompressed(dir.path(), depot_id, manifest_id);
    assert_eq!(loaded.unwrap(), data);
}

#[test]
fn load_manifest_decompressed_returns_none_for_missing() {
    let dir = tempfile::tempdir().unwrap();
    assert!(DepotConfig::load_manifest_decompressed(dir.path(), DepotId(1), ManifestId(1)).is_none());
}

#[test]
fn multiple_depots() {
    let dir = tempfile::tempdir().unwrap();
    let key_a = DepotKey([0x11; 32]);
    let key_b = DepotKey([0x22; 32]);

    let mut config = DepotConfig::default();
    config.set_installed(DepotId(100), ManifestId(1), &key_a);
    config.set_installed(DepotId(200), ManifestId(2), &key_b);
    config.save(dir.path()).unwrap();

    let loaded = DepotConfig::load(dir.path());
    let (mid_a, k_a) = loaded.get_installed(DepotId(100)).unwrap();
    let (mid_b, k_b) = loaded.get_installed(DepotId(200)).unwrap();
    assert_eq!(mid_a.0, 1);
    assert_eq!(mid_b.0, 2);
    assert_eq!(k_a.0, [0x11; 32]);
    assert_eq!(k_b.0, [0x22; 32]);
}

#[test]
fn overwrite_installed() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = DepotConfig::default();
    config.set_installed(DepotId(1), ManifestId(10), &DepotKey([0xAA; 32]));
    config.set_installed(DepotId(1), ManifestId(20), &DepotKey([0xBB; 32]));
    config.save(dir.path()).unwrap();

    let loaded = DepotConfig::load(dir.path());
    let (mid, key) = loaded.get_installed(DepotId(1)).unwrap();
    assert_eq!(mid.0, 20);
    assert_eq!(key.0, [0xBB; 32]);
}
