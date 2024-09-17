{ rustPlatform }:
rustPlatform.buildRustPackage {
  version = "0.1.0";
  pname = "imgserv";
  src = ./.;
  cargoHash = "sha256-CJWSfEr2QJfstUBB8kLOcITBG9yRA5vzPdhY05o6s64=";
}
