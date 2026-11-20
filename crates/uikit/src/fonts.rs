//! Figtree, embedded so the app looks the same on every platform.
//!
//! SIL Open Font License 1.1, Copyright 2022 The Figtree Project Authors
//! (`assets/fonts/OFL.txt`). Only the weights the type scale uses are
//! shipped: 500, 600, 700, 800.

use crate::tokens::FONT_FAMILY;

macro_rules! font {
    ($file:literal) => {
        include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/fonts/", $file))
    };
}

/// Every embedded face. Each one registers under [`FONT_FAMILY`]; Skia picks
/// the face by weight.
pub const FACES: [&[u8]; 4] = [
    font!("Figtree-Medium.ttf"),
    font!("Figtree-SemiBold.ttf"),
    font!("Figtree-Bold.ttf"),
    font!("Figtree-ExtraBold.ttf"),
];

/// Font license text, for the credits screen.
pub const LICENSE: &str =
    include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/fonts/OFL.txt"));

/// Registers the Myos fonts on a Freya launch configuration.
#[cfg(feature = "winit")]
pub trait LaunchConfigExt {
    /// Embed Figtree and make it the default family.
    #[must_use]
    fn with_myos_fonts(self) -> Self;
}

#[cfg(feature = "winit")]
impl LaunchConfigExt for freya::prelude::LaunchConfig {
    fn with_myos_fonts(self) -> Self {
        FACES
            .into_iter()
            .fold(self, |config, face| config.with_font(FONT_FAMILY, face))
            .with_default_font(FONT_FAMILY)
    }
}
