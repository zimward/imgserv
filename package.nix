{ rustPlatform }:
rustPlatform.buildRustPackage {
  version = "0.1.0";
  pname = "imgserv";
  src = ./.;
  cargoHash = "sha256-7xbPwsFzede+evb+TIh/F7lyuAeWMl0JR3nA2jPyArk=";
}
