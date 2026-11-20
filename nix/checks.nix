# `nix flake check`: build and test the shared crates in the sandbox, the
# same way the release package builds (vendored crates, no network).
{
  pkgs,
  lib,
  app,
}:
let
  crates = [
    "myos-core"
    "myos-store"
  ];
  packageFlags = lib.concatMap (name: [
    "-p"
    name
  ]) crates;
in
{
  crates = pkgs.rustPlatform.buildRustPackage {
    pname = "${app.pname}-crates-check";
    inherit (app) version src;
    cargoLock.lockFile = app.lockFile;
    # perl: the vendored OpenSSL behind SQLCipher.
    nativeBuildInputs = [ pkgs.perl ];
    cargoBuildFlags = packageFlags;
    cargoTestFlags = packageFlags;
    doCheck = true;
    # Libraries only: nothing to install, the check is the build + tests.
    installPhase = ''
      runHook preInstall
      mkdir -p $out
      runHook postInstall
    '';
  };
}
