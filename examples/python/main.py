"""List files in Spacewar (app 480) using the steam-ffi Python bindings."""

from steam_ffi_ext import SteamSession


def format_size(size: int) -> str:
    if size < 1024:
        return f"{size} B"
    elif size < 1024 * 1024:
        return f"{size / 1024:.1f} KB"
    elif size < 1024 * 1024 * 1024:
        return f"{size / (1024 * 1024):.1f} MB"
    else:
        return f"{size / (1024 * 1024 * 1024):.2f} GB"


def main():
    print("Connecting to Steam anonymously...")
    session = SteamSession.connect_anonymous()

    print("Listing files in Spacewar (app 480, depot 481)...")
    files = session.list_depot_files(480, 481, "public")

    print(f"\n{'Filename':<40} {'Size':>10}")
    print("-" * 52)

    total_size = 0
    for i in range(len(files)):
        name = files.get_name(i)
        size = files.get_size(i)
        is_dir = files.is_directory(i)

        if is_dir:
            print(f"{name + '/':<40} {'<DIR>':>10}")
        else:
            print(f"{name:<40} {format_size(size):>10}")
            total_size += size

    print("-" * 52)
    print(f"{'Total':<40} {format_size(total_size):>10}")
    print(f"{len(files)} files")


if __name__ == "__main__":
    main()
