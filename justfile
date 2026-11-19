set shell := ["bash", "-cu"]

# Host build. Also runs build.rs, which regenerates the Kotlin / Swift glue.
build:
    cargo build --release

test:
    cargo test --all-targets

fmt:
    cargo fmt --all

lint:
    cargo fmt --all --check
    cargo clippy --all-targets -- -D warnings

# Android debug APK. The `io.github.sergioribera.istmo` Gradle plugin runs
# cargo for every ABI; `./gradlew istmoDoctor` checks the toolchain.
build-android:
    cd android && ./gradlew assembleDebug

# Play Store release bundle. Requires the ANDROID_RELEASE_* env vars.
release-android:
    cd android && ./gradlew bundleRelease

# iOS simulator build. `cargo check` writes the xcodegen fragment
# (`istmo-plugins.yml`) before the project is generated; Xcode's pre-build
# phase (`ios/.istmo/build-rust.sh`) then builds the Rust library.
build-ios:
    cargo check
    cd ios && xcodegen generate && \
        xcodebuild -project MyosApp.xcodeproj \
                   -scheme  MyosApp \
                   -destination 'generic/platform=iOS Simulator' build

# Desktop bundles for the host: every format `nix/targets.nix` lists for it.
bundle-desktop:
    nix build .#bundles

# Windows bundles, cross-built from Linux (x86_64-linux hosts only).
bundle-windows:
    nix build .#windows-bundles

# Run the desktop shell (in-process runtime + Rust hosts / emulator thread).
run:
    cargo run


# The release matrix (nix/targets.nix) as the workflow sees it.
matrix:
    nix run .#matrix | jq .

# Landing page: refresh its data from nix/targets.nix + the latest release, then serve it.
site:
    cd site && pnpm install && pnpm dev

