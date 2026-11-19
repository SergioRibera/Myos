//! `myos` — Workout manager app
//!
//! One crate, three shells:
//!
//! - Desktop binary — `src/main.rs`, on an in-process runtime (`src/desktop.rs`).
//! - Android `cdylib`, loaded by `IstmoGameActivity` — `mobile_main` below.
//! - iOS `staticlib`, linked into the Xcode app — same `mobile_main`.

pub mod app;

#[cfg(not(any(
    target_os = "android",
    target_os = "ios",
    target_os = "tvos",
    target_os = "watchos",
    target_os = "visionos",
)))]
pub mod desktop;

use istmo::plugins::DeepLinks;
use istmo::plugins::SafeArea;

// Plugin crates (`istmo-data-store`, …) are wired automatically from
// their manifests; built-in `istmo::plugins` clients are listed here.
istmo::runtime!(
    plugins: [
        DeepLinks,
        SafeArea,
    ],
);

#[cfg(any(
    target_os = "android",
    target_os = "ios",
    target_os = "tvos",
    target_os = "watchos",
    target_os = "visionos",
))]
#[istmo::mobile_app]
fn mobile_main() {
    #[cfg(target_os = "android")]
    android_logger::init_once(
        android_logger::Config::default().with_max_level(log::LevelFilter::Info),
    );

    let runtime = match istmo::Runtime::global() {
        Ok(runtime) => runtime,
        Err(err) => {
            log::error!("istmo runtime is not running: {err}");
            return;
        }
    };

    app::App::new(runtime).run_mobile();
}
