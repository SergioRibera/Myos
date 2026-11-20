//! `cargo run -p myos-uikit --example gallery`: every component, dark and
//! light side by side.

mod view;

use freya::prelude::*;
use myos_uikit::LaunchConfigExt;

fn main() {
    launch(
        LaunchConfig::new().with_myos_fonts().with_window(
            WindowConfig::new(app)
                .with_title("Myos UI kit")
                .with_size(900., 1000.),
        ),
    );
}

fn app() -> impl IntoElement {
    ScrollView::new().child(view::gallery())
}
