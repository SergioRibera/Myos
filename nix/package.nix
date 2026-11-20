# Desktop build of the app for the host system.
#
# Needs a committed Cargo.lock (`cargo generate-lockfile` once). Git
# dependencies, e.g. a `[patch.crates-io]` pointing at an istmo checkout,
# also need their hash in `cargoLock.outputHashes`.
{
  pkgs,
  lib,
}:
let
  manifest = lib.importTOML ../Cargo.toml;
  pname = manifest.package.name;
  inherit (manifest.package) version;

  # Only what the desktop binary needs: a change under android/ or ios/
  # does not rebuild it.
  lockFile =
    assert lib.assertMsg (builtins.pathExists ../Cargo.lock)
      "Cargo.lock is missing: run `cargo generate-lockfile` and commit it (Nix builds read the locked versions).";
    ../Cargo.lock;

  src = lib.fileset.toSource {
    root = ../.;
    fileset = lib.fileset.unions (
      [
        ../Cargo.toml
        lockFile
        ../build.rs
        ../istmo.toml
        ../src
        # Workspace members: cargo resolves every manifest, and the app links
        # the core and store crates.
        ../crates
      ]
      ++ lib.optional (builtins.pathExists ../assets) ../assets
    );
  };

  # dlopen'd by winit / glutin / wgpu, so the Nix wrapper puts them on
  # LD_LIBRARY_PATH. Packaged bundles leave them to the host, where the
  # GPU driver stack must win.
  dlopenLibs = with pkgs; [
    libGL
    vulkan-loader
    wayland
    libxkbcommon
    libX11
    libXcursor
    libXrandr
    libXi
  ];

  unwrapped = pkgs.rustPlatform.buildRustPackage {
    inherit pname version src;
    cargoLock = { inherit lockFile; };
    # perl: the vendored OpenSSL behind SQLCipher (myos-store).
    nativeBuildInputs = [
      pkgs.pkg-config
      pkgs.perl
    ];
    # Plugins with system libraries add theirs here (the camera plugin:
    # `llvmPackages.libclang` for bindgen and `LIBCLANG_PATH`).
    buildInputs =
      lib.optionals pkgs.stdenv.hostPlatform.isLinux (
        with pkgs;
        [
          fontconfig
          freetype
          libxcb
        ]
        ++ dlopenLibs
      )
      ++ lib.optionals pkgs.stdenv.hostPlatform.isDarwin [ pkgs.libiconv ];
    # Only the binary ships; the cdylib and staticlib are the mobile entry.
    cargoBuildFlags = [
      "--bin"
      pname
    ];
    # Tests run in CI with cargo; they need no packaging.
    doCheck = false;
    meta.mainProgram = pname;
  };

  wrapped =
    if pkgs.stdenv.hostPlatform.isLinux then
      pkgs.runCommand "${pname}-${version}"
        {
          nativeBuildInputs = [ pkgs.makeWrapper ];
          meta.mainProgram = pname;
          passthru = { inherit unwrapped; };
        }
        ''
          mkdir -p $out/bin
          makeWrapper ${unwrapped}/bin/${pname} $out/bin/${pname} \
            --prefix LD_LIBRARY_PATH : ${lib.makeLibraryPath dlopenLibs}
        ''
    else
      unwrapped;
in
{
  inherit
    pname
    version
    src
    lockFile
    unwrapped
    wrapped
    ;
}
