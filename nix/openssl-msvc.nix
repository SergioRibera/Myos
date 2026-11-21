# Static libcrypto for `x86_64-pc-windows-msvc`, cross-built from Linux, for
# SQLCipher on Windows.
#
# `openssl-src` (behind `bundled-sqlcipher-vendored-openssl`) drives MSVC
# builds through `nmake`, which does not exist here. OpenSSL's Unix makefile
# works instead: clang's GNU driver targets the MSVC ABI against the xwin
# headers, the `mingw64` platform config supplies the Windows code paths, and
# the archive (COFF objects) is installed as `crypto.lib` for lld-link.
{
  pkgs,
  xwinSdk,
}:
let
  llvm = pkgs.llvmPackages;
  cc = pkgs.writeShellScript "clang-msvc" ''
    exec ${llvm.clang-unwrapped}/bin/clang --target=x86_64-pc-windows-msvc \
      -fms-compatibility -fms-extensions -D_MT \
      -isystem ${xwinSdk}/crt/include \
      -isystem ${xwinSdk}/sdk/include/ucrt \
      -isystem ${xwinSdk}/sdk/include/um \
      -isystem ${xwinSdk}/sdk/include/shared \
      "$@"
  '';
  config = pkgs.writeText "myos-msvc.conf" ''
    my %targets = (
        "myos-msvc-x64" => {
            inherit_from   => [ "mingw64" ],
            cflags         => "-O2",
            ex_libs        => "",
            perlasm_scheme => "",
        },
    );
  '';
in
pkgs.stdenvNoCC.mkDerivation {
  pname = "openssl-msvc-x64";
  inherit (pkgs.openssl) version src;
  nativeBuildInputs = [
    pkgs.perl
    pkgs.gnumake
  ];
  # Only the crypto library: SQLCipher needs nothing else. No assembly (the
  # perlasm output targets GNU as / MASM, not this toolchain).
  configurePhase = ''
    runHook preConfigure
    patchShebangs Configure
    CC=${cc} AR=${llvm.llvm}/bin/llvm-ar RANLIB=${llvm.llvm}/bin/llvm-ranlib \
      ./Configure --config=${config} myos-msvc-x64 \
      no-asm no-shared no-module no-dso no-engine no-apps no-tests no-docs \
      --prefix=$out --libdir=lib
    runHook postConfigure
  '';
  buildPhase = ''
    runHook preBuild
    make -j$NIX_BUILD_CORES build_libs
    runHook postBuild
  '';
  installPhase = ''
    runHook preInstall
    install -Dm644 libcrypto.a $out/lib/crypto.lib
    mkdir -p $out/include
    cp -r include/openssl $out/include/
    runHook postInstall
  '';
}
