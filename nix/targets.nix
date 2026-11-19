# Release matrix: the single source of truth for what gets built where.
#
# `nix run .#matrix` serialises it into the JSON the release workflow fans
# out over, and the site reads it (`site/src/data/targets.json`) to
# list the downloads. Adding a platform, format or ABI here is the only
# change needed to ship it.
{
  # Desktop app, bundled by nix-bundle-app.
  #
  # kind = "nix"   → built and bundled natively on `runner`.
  # kind = "cross" → cross-built from Linux on `runner` (nix/windows.nix).
  desktop = {
    x86_64-linux = {
      kind = "nix";
      runner = "ubuntu-latest";
      formats = [
        "appimage"
        "deb"
        "rpm"
        "tar.gz"
      ];
    };
    aarch64-linux = {
      kind = "nix";
      runner = "ubuntu-24.04-arm";
      formats = [
        "appimage"
        "deb"
        "rpm"
        "tar.gz"
      ];
    };
    aarch64-darwin = {
      kind = "nix";
      runner = "macos-latest";
      formats = [
        "app"
        "dmg"
        "tar.gz"
      ];
    };
    x86_64-windows = {
      kind = "cross";
      runner = "ubuntu-latest";
      formats = [
        "nsis"
        "msi"
        "zip"
      ];
    };
  };

  # Android APKs: one per ABI plus a universal one (`-Pistmo.abis`).
  android = {
    abis = [
      "arm64-v8a"
      "x86_64"
    ];
  };
}
