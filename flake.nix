{
  description = "A simple temporary image upload service";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs?ref=nixos-unstable-small";
  };

  outputs =
    { self, nixpkgs }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      forAllSystem = f: nixpkgs.lib.genAttrs systems (system: f system);
    in
    {
      packages = forAllSystem (system: {
        default = nixpkgs.legacyPackages.${system}.callPackage ./package.nix { };
      });

    };
}
