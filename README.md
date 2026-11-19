# myos

Workout manager app

Multi-platform application built on [istmo](https://crates.io/crates/istmo):
one Rust crate drives Android iOS desktop.

Author: Sergio Ribera <contact@sergioribera.rs>
License: MIT OR Apache-2.0

## Selected features

- Archetype: `app`
- Platforms: android ios desktop
- CI: github-actions 
- Plugins: DeepLinks SafeArea istmo-share 
- UI framework: `freya` (see `src/app.rs`)
- Desktop packaging: nix-bundle-app (`nix build .#bundles`, every format of `nix/targets.nix` for the host)
- Publish targets: github-release 

Requires Rust 1.89 or newer.

## Quickstart

```sh
cargo build         # also runs build.rs: Kotlin / Swift glue, istmo doctor
git add Cargo.lock   # Nix builds and releases read the locked versions
just run            # desktop shell
just build-android  # debug APK (Gradle runs cargo for every ABI)
just build-ios      # xcodegen + simulator build (Xcode runs cargo)
just bundle-desktop # deb/rpm/appimage/dmg/msi via nix-bundle-app
```

## How it fits together

- `istmo.toml` is the single source of truth for the app id, name,
  version, icon and minimum OS versions.
- `build.rs` (`istmo_build::emit()`) runs on every cargo build. It reads
  `istmo.toml` and the manifest of every istmo plugin crate the app
  depends on, and generates the native glue: dispatchers, codecs and an
  `IstmoPluginRegistry` that registers every plugin backend. Warnings
  prefixed `istmo doctor:` point at setup mistakes.
- `src/lib.rs` declares the runtime (`istmo::runtime!`) and the
  `#[istmo::mobile_app]` entry point. Plugin crates are wired
  automatically; built-in `istmo::plugins` clients are listed there.
- `src/app.rs` holds the shared app state and UI.
- `src/main.rs` + `src/desktop.rs` run the same app on an in-process runtime;
  plugins with a desktop backend run in-process, calls to the others fail
  with a log line until you emulate them.
- `android/` is a plain Gradle project. The `io.github.sergioribera.istmo`
  plugin builds the crate for every ABI, links the plugins' Kotlin sources,
  manifests and dependencies, and applies `[app]`. The launcher activity is
  `dev.istmo.runtime.IstmoGameActivity`; subclass it to add Kotlin code.
  The repo ships no Gradle wrapper: run `gradle wrapper` once in
  `android/`, and `./gradlew istmoDoctor` to check the toolchain.
- `ios/` is an xcodegen project. Run one `cargo build` (or `cargo check`)
  before `xcodegen generate`: it writes `ios/MyosApp/istmo-plugins.yml`
  (plugin sources, app identity, link settings, the cargo pre-build phase)
  and `ios/.istmo/`. `main.swift` is `IstmoApp.run()`.

Adding a plugin crate is a `cargo add istmo-<plugin>`: the next build
wires its client in Rust and registers its native backend.

## Releases

`nix/targets.nix` lists what gets built: desktop systems and their
formats, and the Android ABIs. `nix run .#matrix` turns it into the
matrix `.github/workflows/release.yml` fans out over, so shipping a new
format is a change to that file only.

Pushing a `v*` tag (its version must match `Cargo.toml`):

- builds every desktop format with Nix and nix-bundle-app on each system
  (Windows is cross-built from Linux with the MSVC SDK, `nix/windows.nix`);
- builds one APK per ABI plus a universal one (`-Pistmo.abis`), signed when
  the `ANDROID_KEYSTORE_B64` / `ANDROID_STORE_PASSWORD` /
  `ANDROID_KEY_ALIAS` / `ANDROID_KEY_PASSWORD` secrets exist;
- publishes the GitHub release with `SHA256SUMS`, notes from git-cliff
  (`cliff.toml`; a hand-written `release-notes/<tag>.md` wins) and
  `latest.json`: every file with platform, arch, kind, size and sha256,
  ready for an in-app update check. Tags with a `-` are pre-releases.

`site/` is the landing page: it offers the file for the visitor's
platform, reading the formats from `nix/targets.nix` and the files from
the newest release. See `site/README.md`.

## Continuous integration

`.github/workflows/ci.yml` runs `cargo fmt --check`, `cargo clippy` and
`cargo test --all-targets`, plus an MSRV check, an Android APK build, an iOS simulator build. Release
workflows fire on `v*` tags.

