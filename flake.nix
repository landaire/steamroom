{
  description = "steamroom development shell";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, flake-utils, rust-overlay }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs { inherit system overlays; };
        rust = pkgs.rust-bin.stable.latest.default.override {
          extensions = [ "rust-src" "rust-analyzer" ];
        };
      in {
        devShells.default = pkgs.mkShell {
          buildInputs = [
            rust
            pkgs.pkg-config
            pkgs.openssl
            pkgs.hyperfine
            pkgs.dotnet-sdk_8 # for DepotDownloader (C#)
            pkgs.jujutsu
          ];

          shellHook = ''
            echo "steamroom dev shell"
            echo "  rust:      $(rustc --version)"
            echo "  hyperfine: $(hyperfine --version)"
            echo "  dotnet:    $(dotnet --version)"
          '';
        };
      }
    );
}
