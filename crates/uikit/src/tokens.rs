//! Design tokens: the numbers behind every component.
//!
//! Source of truth is the Penpot file "Myos - Workout" (token sets `Base`,
//! `Tema/Oscuro`, `Tema/Claro` and the library typographies). Components only
//! read the semantic layer exposed here, never raw hex values.

mod palette;
mod typography;

use std::time::Duration;

pub use palette::{Brand, Palette};
pub use typography::{FONT_FAMILY, TextRole, TextRoleExt, TextStyle};

/// Spacing scale, 4 px base unit (Penpot set `Base`, tokens `espacio.*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Space;

impl Space {
    /// 4 px.
    pub const XS: f32 = 4.;
    /// 8 px.
    pub const SM: f32 = 8.;
    /// 12 px.
    pub const MD: f32 = 12.;
    /// 16 px: card padding and the screen gutter on mobile.
    pub const LG: f32 = 16.;
    /// 20 px.
    pub const XL: f32 = 20.;
    /// 24 px: gap between sections.
    pub const XXL: f32 = 24.;
    /// 32 px.
    pub const XXXL: f32 = 32.;
}

/// Corner radius scale (Penpot tokens `radio.*` plus the values the
/// components use).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Radius;

impl Radius {
    /// Inner tiles nested in a card (icon tile, explanation box).
    pub const TILE: f32 = 12.;
    /// Set rows.
    pub const ROW: f32 = 14.;
    /// Chips: half their 32 px height.
    pub const CHIP: f32 = 16.;
    /// Alert cards.
    pub const ALERT: f32 = 18.;
    /// Selectable controls (options, theme cards, suggestions).
    pub const CONTROL: f32 = 20.;
    /// Content cards.
    pub const CARD: f32 = 24.;
    /// Buttons and steppers: half their 56 px height.
    pub const BUTTON: f32 = 28.;
}

/// Fixed sizes shared by several components.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Dimen;

impl Dimen {
    /// Minimum touch target on mobile.
    pub const TOUCH: f32 = 48.;
    /// Regular button and stepper height.
    pub const BUTTON: f32 = 56.;
    /// Small (inline) button height.
    pub const BUTTON_SMALL: f32 = 28.;
    /// Chip height.
    pub const CHIP: f32 = 32.;
    /// Default icon size.
    pub const ICON: f32 = 24.;
    /// Width the mobile layouts are drawn at.
    pub const MOBILE_WIDTH: f32 = 390.;
}

/// Stroke widths.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stroke;

impl Stroke {
    /// Hairline borders (cards, options).
    pub const HAIRLINE: f32 = 1.;
    /// Outlined controls (ghost buttons, rows, pending checks).
    pub const REGULAR: f32 = 1.5;
    /// Selected outlines and the keyboard focus ring.
    pub const STRONG: f32 = 2.;
}

/// Motion durations. Short on purpose: the app is used between sets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Motion;

impl Motion {
    /// Press feedback, toggles.
    pub const FAST: Duration = Duration::from_millis(120);
    /// Expanding a card, switching a view.
    pub const BASE: Duration = Duration::from_millis(200);
    /// Screen transitions.
    pub const SLOW: Duration = Duration::from_millis(320);
}

/// Opacity applied to disabled controls.
pub const DISABLED_OPACITY: f32 = 0.4;
