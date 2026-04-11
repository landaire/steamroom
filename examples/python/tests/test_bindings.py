"""Smoke tests for the steam_ffi_ext Python bindings."""

import pytest


def test_import():
    import steam_ffi_ext


def test_session_class_exists():
    from steam_ffi_ext import SteamSession
    assert hasattr(SteamSession, "connect_anonymous")
    assert hasattr(SteamSession, "connect_with_token")


def test_manifest_file_list_class_exists():
    from steam_ffi_ext import ManifestFileList
    assert hasattr(ManifestFileList, "get_name")
    assert hasattr(ManifestFileList, "get_size")
    assert hasattr(ManifestFileList, "is_directory")


@pytest.mark.integration
def test_connect_anonymous_and_list_files():
    """Connect to Steam and list Spacewar files. Requires network."""
    from steam_ffi_ext import SteamSession

    session = SteamSession.connect_anonymous()
    files = session.list_depot_files(480, 481, "public")

    assert len(files) > 0

    names = [files.get_name(i) for i in range(len(files))]
    assert any("steam_api" in n.lower() for n in names)

    for i in range(len(files)):
        size = files.get_size(i)
        assert size >= 0
