# Cross build of the app for `x86_64-pc-windows-msvc` from Linux.
#
# The MSVC target is linked with LLVM (clang-cl + lld-link) against the
# Windows SDK and CRT that `xwin` downloads from Microsoft, pinned below.
# MSVC rather than mingw because Skia-based UI crates only ship prebuilt
# binaries for it. No Windows runner involved.
{
  pkgs,
  lib,
  pname,
  version,
  src,
  lockFile,
}:
let
  target = "x86_64-pc-windows-msvc";
  targetEnv = lib.toUpper (builtins.replaceStrings [ "-" ] [ "_" ] target);
  llvm = pkgs.llvmPackages;

  toolchain = pkgs.rust-bin.stable.latest.minimal.override { targets = [ target ]; };
  rustPlatform = pkgs.makeRustPlatform {
    cargo = toolchain;
    rustc = toolchain;
  };

  # Windows SDK + Visual C++ CRT headers and import libraries. Bump both
  # versions together after `xwin --accept-license list`, then refresh
  # `outputHash`.
  xwinSdk = pkgs.stdenvNoCC.mkDerivation {
    pname = "xwin-sdk";
    version = "10.0.26100-14.44.17.14";
    dontUnpack = true;
    nativeBuildInputs = [
      pkgs.xwin
      pkgs.cacert
    ];
    SSL_CERT_FILE = "${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt";
    buildCommand = ''
      export HOME=$TMPDIR
      xwin --accept-license --arch x86_64 --cache-dir $TMPDIR/xwin \
        --manifest-version 17 --sdk-version 10.0.26100 --crt-version 14.44.17.14 \
        splat --output $TMPDIR/splat
      # xwin links from its cache into the output; keep both on one filesystem.
      cp -r $TMPDIR/splat $out
    '';
    outputHashMode = "recursive";
    outputHashAlgo = "sha256";
    outputHash = "sha256-UFQjsFVBwcF/9e9tVFoG0Z1JySxyTnFqoaRwr/tUWzA=";
  };

  # clang-cl with the MSVC include tree, for build scripts that compile C
  # for the target.
  clangCl = pkgs.writeShellScriptBin "clang-cl" ''
    exec ${llvm.clang-unwrapped}/bin/clang-cl --target=${target} \
      /imsvc ${xwinSdk}/crt/include \
      /imsvc ${xwinSdk}/sdk/include/ucrt \
      /imsvc ${xwinSdk}/sdk/include/um \
      /imsvc ${xwinSdk}/sdk/include/shared \
      "$@"
  '';

  # SQLCipher's libcrypto (nix/openssl-msvc.nix).
  openssl = import ./openssl-msvc.nix { inherit pkgs xwinSdk; };

  libDirs = [
    "${xwinSdk}/crt/lib/x86_64"
    "${xwinSdk}/sdk/lib/um/x86_64"
    "${xwinSdk}/sdk/lib/ucrt/x86_64"
  ];
in
pkgs.stdenv.mkDerivation {
  pname = "${pname}-windows";
  inherit version src;

  cargoDeps = rustPlatform.importCargoLock { inherit lockFile; };
  nativeBuildInputs = [
    toolchain
    rustPlatform.cargoSetupHook
  ];

  env = {
    "CARGO_TARGET_${targetEnv}_LINKER" = "${llvm.lld}/bin/lld-link";
    # Static CRT: the exe needs no VC++ redistributable.
    "CARGO_TARGET_${targetEnv}_RUSTFLAGS" = lib.concatStringsSep " " (
      [ "-Ctarget-feature=+crt-static" ]
      ++ map (d: "-Lnative=${d}") libDirs
      # What the static libcrypto calls into.
      ++ map (l: "-ldylib=${l}") [
        "ws2_32"
        "crypt32"
        "advapi32"
        "user32"
        "bcrypt"
      ]
    );
    "CC_${target}" = "${clangCl}/bin/clang-cl";
    "CXX_${target}" = "${clangCl}/bin/clang-cl";
    "AR_${target}" = "${llvm.llvm}/bin/llvm-lib";
    "RC_${target}" = "${llvm.llvm}/bin/llvm-rc";
    # SQLCipher links `crypto` from here (`bundled-sqlcipher` on Windows).
    "${targetEnv}_OPENSSL_LIB_DIR" = "${openssl}/lib";
    "${targetEnv}_OPENSSL_INCLUDE_DIR" = "${openssl}/include";
  };

  buildPhase = ''
    runHook preBuild
    cargo build --release --frozen --target ${target} --bin ${pname}
    runHook postBuild
  '';

  installPhase = ''
    runHook preInstall
    install -Dm755 target/${target}/release/${pname}.exe $out/bin/${pname}.exe
    runHook postInstall
  '';

  passthru = { inherit xwinSdk openssl; };
}
