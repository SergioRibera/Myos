{
  description = "myos: Workout manager app";

  # Personal attic cache: builds already pushed by CI (or another machine)
  # are downloaded instead of rebuilt.
  nixConfig = {
    extra-substituters = [ "https://cache.sergioribera.rs/main" ];
    extra-trusted-public-keys = [ "main:vFI3N1JP9edRFwBwdk9ebUKTIPWK9R1ECbkdA7Q593M=" ];
  };

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    nix-bundle-app = {
      url = "github:SergioRibera/nix-bundle-app";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      nixpkgs,
      flake-utils,
      rust-overlay,
      nix-bundle-app,
      ...
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs {
          inherit system;
          config = {
            allowUnfree = true;
            android_sdk.accept_license = true;
          };
          overlays = [
            (import rust-overlay)
            nix-bundle-app.overlays.default
          ];
        };
        lib = pkgs.lib;
        targets = import ./nix/targets.nix;

        # What the release workflow fans out over (`nix run .#matrix`).
        matrix =
          let
            entries = lib.mapAttrsToList (system: t: t // { inherit system; }) targets.desktop;
          in
          builtins.toJSON {
            nix = map (t: { inherit (t) system runner; }) (builtins.filter (t: t.kind == "nix") entries);
            cross = map (t: { inherit (t) system runner; }) (builtins.filter (t: t.kind == "cross") entries);
            android.abis = lib.concatStringsSep "," targets.android.abis;
          };

        app = import ./nix/package.nix { inherit pkgs lib; };
        inherit (app) pname version;

        # The app identity lives in istmo.toml, like on Android and iOS.
        manifest = (fromTOML (builtins.readFile ./istmo.toml)).app;
        icon = manifest.icon.desktop or manifest.icon.image or null;

        info = {
          name = pname;
          displayName = manifest.name;
          inherit version;
          license = "MIT OR Apache-2.0";
          maintainer = "Sergio Ribera <contact@sergioribera.rs>";
          description = "Workout manager app";
          homepage = "https://github.com/SergioRibera/myos";
          bundleId = manifest.id;
          downloadUrl = "https://github.com/SergioRibera/myos/releases/download/v${version}";
          desktopEntries = [
            {
              name = manifest.name;
              # Named after the bundle id: Wayland matches the window's app
              # id against it.
              fileName = manifest.id;
              exec = "/opt/${pname}/bin/${pname} %F";
              categories = [ "Utility" ];
            }
          ];
        }
        // lib.optionalAttrs (icon != null) { icon = ./. + "/${icon}"; };

        bundler = pkgs.nixBundleApp;
        # `tar.gz` → `tar_gz`: keeps attr names CLI-friendly (`.#bundle-tar_gz`).
        attrOf = format: lib.replaceStrings [ "." ] [ "_" ] format;
        mkBundles =
          {
            drv,
            formats,
            target ? null,
          }:
          let
            built = lib.genAttrs formats (
              format:
              bundler.bundle {
                inherit
                  drv
                  format
                  info
                  target
                  ;
              }
            );
          in
          lib.mapAttrs' (format: value: lib.nameValuePair "bundle-${attrOf format}" value) built
          // {
            bundles = pkgs.linkFarm "${pname}-${version}-${system}-bundles" (
              lib.mapAttrsToList (name: path: { inherit name path; }) built
            );
          };

        nativeTarget = targets.desktop.${system} or null;
        nativeBundles = lib.optionalAttrs (nativeTarget != null && nativeTarget.kind == "nix") (mkBundles {
          drv = app.unwrapped;
          inherit (nativeTarget) formats;
        });

        # Windows, cross-built from Linux (nix/windows.nix): `.#windows-bundles`.
        windowsBundles = lib.optionalAttrs (system == "x86_64-linux" && targets.desktop ? x86_64-windows) (
          let
            drv = import ./nix/windows.nix {
              inherit
                pkgs
                lib
                pname
                version
                ;
              inherit (app) src lockFile;
            };
          in
          {
            "${pname}-windows" = drv;
          }
          // lib.mapAttrs' (name: value: lib.nameValuePair "windows-${name}" value) (mkBundles {
            inherit drv;
            inherit (targets.desktop.x86_64-windows) formats;
            target = {
              arch = "x86_64";
              os = "windows";
            };
          })
        );
      in
      {
        packages = {
          default = app.wrapped;
          ${pname} = app.wrapped;
          "${pname}-unwrapped" = app.unwrapped;
        }
        // nativeBundles
        // windowsBundles;

        checks = import ./nix/checks.nix { inherit pkgs lib app; };

        apps.matrix = {
          type = "app";
          program = toString (pkgs.writeShellScript "matrix" "echo '${matrix}'");
        };

        # Devshell: `just`, the desktop shell's wayland/x11 libraries and
        # the utilities the mobile builds shell out to.
        devShells.default = pkgs.mkShell {
          packages = with pkgs; [
            just
            pkg-config
            libGL
            wayland
            libxkbcommon
            libX11
            libXcursor
            libXrandr
            libXi
            nodejs
            pnpm
            # Sync server tests start a throwaway cluster (initdb / pg_ctl).
            postgresql
            # myos-store builds a vendored OpenSSL for SQLCipher.
            perl
            claude-code
            # Android tooling normally comes from Android Studio / a Docker image;
            # keep this shell focused on desktop dev + cargo.
            # iOS builds require Xcode + xcodegen on a macOS host.
          ];
          LD_LIBRARY_PATH = lib.makeLibraryPath (
            with pkgs;
            [
              libGL
              vulkan-loader
              wayland
              libxkbcommon
              libX11
              libXcursor
              libXrandr
              libXi
            ]
          );
        };

        formatter = pkgs.nixfmt-rfc-style;
      }
    );
}
