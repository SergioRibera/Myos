//! Desktop entry point. Mobile targets get an empty `main` so the same
//! manifest builds the `cdylib`, the `staticlib` and this binary.

#[cfg(not(any(
    target_os = "android",
    target_os = "ios",
    target_os = "tvos",
    target_os = "watchos",
    target_os = "visionos",
)))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use myos::{app::App, desktop};

    let runtime = desktop::start()?;

    // `istmo::app_icon!()` holds the `[app.icon]` PNG for the window icon.
    App::new(runtime).run_desktop();
    Ok(())
}

#[cfg(any(
    target_os = "android",
    target_os = "ios",
    target_os = "tvos",
    target_os = "watchos",
    target_os = "visionos",
))]
#[allow(
    clippy::missing_const_for_fn,
    reason = "fires only for the test harness build; `main` cannot be `const`"
)]
fn main() {}
