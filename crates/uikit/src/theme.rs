//! Appearance: the user's choice (Claro / Oscuro / Sistema) resolved against
//! the operating system into one [`Palette`].

use freya::prelude::{Platform, PreferredTheme, State, try_consume_context, use_provide_context};

use crate::tokens::Palette;

/// What the user picked in Ajustes → Apariencia. Defaults to following the
/// system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ThemeMode {
    /// Follow the operating system.
    #[default]
    System,
    /// Always light.
    Light,
    /// Always dark.
    Dark,
}

/// The appearance actually on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Appearance {
    /// Light grounds, olive accent.
    Light,
    /// Carbón grounds, lime accent.
    Dark,
}

impl ThemeMode {
    /// Resolve against the appearance the operating system prefers.
    #[must_use]
    pub const fn resolve(self, system: Appearance) -> Appearance {
        match self {
            Self::System => system,
            Self::Light => Appearance::Light,
            Self::Dark => Appearance::Dark,
        }
    }
}

impl From<PreferredTheme> for Appearance {
    fn from(theme: PreferredTheme) -> Self {
        match theme {
            PreferredTheme::Light => Self::Light,
            PreferredTheme::Dark => Self::Dark,
        }
    }
}

/// Resolved theme handed to components. Cheap to copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    /// Appearance on screen.
    pub appearance: Appearance,
    /// Colors of that appearance.
    pub palette: &'static Palette,
}

impl From<Appearance> for Theme {
    fn from(appearance: Appearance) -> Self {
        let palette = match appearance {
            Appearance::Light => &Palette::LIGHT,
            Appearance::Dark => &Palette::DARK,
        };
        Self {
            appearance,
            palette,
        }
    }
}

/// Provide the user's [`ThemeMode`] to every component below. Call it once in
/// the root component; write to the returned state when the user changes the
/// setting.
#[expect(
    clippy::must_use_candidate,
    reason = "a root that never changes the mode at runtime ignores the state"
)]
pub fn use_provide_theme_mode(initial: ThemeMode) -> State<ThemeMode> {
    use_provide_context(|| State::create(initial))
}

/// The theme for the current window. Re-renders the caller when the user's
/// mode or the system appearance changes. Without a provider it follows the
/// system.
#[must_use]
pub fn use_theme() -> Theme {
    let mode =
        try_consume_context::<State<ThemeMode>>().map_or(ThemeMode::System, |mode| *mode.read());
    let system = Appearance::from(*Platform::get().current_window().preferred_theme.read());
    Theme::from(mode.resolve(system))
}
