{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-24.05";
    nixpkgs-unstable.url = "github:nixos/nixpkgs/nixos-unstable";
    cargo2nix = {
      url = "github:cargo2nix/cargo2nix/release-0.11.0";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    flake-utils.follows = "cargo2nix/flake-utils";
  };

  outputs =
    inputs:
    with inputs;
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ cargo2nix.overlays.default ];
        };
        unstable = import nixpkgs-unstable {
          inherit system;
        };

        rustPkgs = pkgs.rustBuilder.makePackageSet {
          rustVersion = "1.75.0";
          packageFun = import ./Cargo.nix;
        };

      in
      rec {
        packages = {
          # replace hello-world with your package name
          imgserv = (rustPkgs.workspace.imgserv { });
          default = packages.imgserv;
        };
        devShells.default = unstable.mkShell {
          nativeBuildInputs = with unstable; [
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
