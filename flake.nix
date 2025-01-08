{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs =
    inputs:
    with inputs;
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs {
          inherit system;
        };
        fs = pkgs.lib.fileset;
      in
      rec {
        packages = {
          # replace hello-world with your package name
          imgserv = pkgs.rustPlatform.buildRustPackage {
            pname = "imgserv";
            version = "0.1.0";
            src = fs.toSource {
              root = ./.;
              fileset = fs.unions [
                ./Cargo.toml
                ./Cargo.lock
                ./src
                ./build.rs
                ./data
                ./migrations
                ./.sqlx
              ];
            };
            # currently impossible due to tree magic dependency, waiting for upstream release
            # cargoHash = "";
            cargoLock = {
              lockFile = ./Cargo.lock;
              outputHashes = {
                "tree_magic_mini-3.1.6" = "sha256-IJ2tVnPb+NmsrGUnfIuRgMIYAi8j+4dtrEXQAN0wA4s=";
              };
            };
          };
          default = packages.imgserv;
        };
        devShells.default = pkgs.mkShell {
          nativeBuildInputs = with pkgs; [
            cargo
            sqlx-cli
            rustc
            rust-analyzer
            rustfmt
            clippy
          ];
        };
      }
    );
}
