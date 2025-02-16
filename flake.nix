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
            cargoHash = "sha256-FVtzROPo5md18zgOiwAa3QeHsL1u/07vX7vKwFKGbKc=";
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
