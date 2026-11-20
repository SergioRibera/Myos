//! Myos UI kit for [Freya](https://freyaui.dev): design tokens, the
//! Claro / Oscuro / Sistema theme, the Figtree type scale, the icon set and
//! the base components of the Penpot proposal ("Myos - Workout", page
//! `03 · Componentes`).
//!
//! Every component reads its colors from [`use_theme`], so it renders in both
//! appearances without per-call styling. Provide the user's choice once at
//! the root:
//!
//! ```rust,no_run
//! use freya::prelude::*;
//! use myos_uikit::{self as ui, LaunchConfigExt, ThemeMode};
//!
//! fn main() {
//!     launch(LaunchConfig::new().with_myos_fonts().with_window(WindowConfig::new(app)));
//! }
//!
//! fn app() -> impl IntoElement {
//!     ui::use_provide_theme_mode(ThemeMode::System);
//!     ui::Screen::new().child(ui::Button::primary("Empezar sesión"))
//! }
//! ```
//!
//! Names overlap with `freya::prelude` (`Button`, `Chip`, `Switch`…): import
//! the crate as `ui` and qualify, instead of glob-importing both.

pub mod components;
pub mod fonts;
pub mod icon;
pub mod theme;
pub mod tokens;

pub use components::*;
#[cfg(feature = "winit")]
pub use fonts::LaunchConfigExt;
pub use icon::{Icon, IconName};
pub use theme::{Appearance, Theme, ThemeMode, use_provide_theme_mode, use_theme};
pub use tokens::{
    Brand, DISABLED_OPACITY, Dimen, FONT_FAMILY, Motion, Palette, Radius, Space, Stroke, TextRole,
    TextRoleExt, TextStyle,
};
