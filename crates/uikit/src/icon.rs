//! The Myos icon set: 24 × 24 outline icons drawn for the Penpot proposal,
//! stroked with `currentColor` so one file serves every theme and state.

use freya::prelude::*;

use crate::tokens::Dimen;

macro_rules! icons {
    ($($(#[$doc:meta])* $variant:ident => $file:literal,)+) => {
        /// Every icon in the set.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum IconName {
            $($(#[$doc])* $variant,)+
        }

        impl IconName {
            /// Every icon, in declaration order.
            pub const ALL: &[Self] = &[$(Self::$variant,)+];

            /// SVG source of this icon.
            #[must_use]
            pub const fn svg(self) -> &'static [u8] {
                match self {
                    $(Self::$variant => include_bytes!(concat!(
                        env!("CARGO_MANIFEST_DIR"), "/assets/icons/", $file, ".svg"
                    )),)+
                }
            }
        }
    };
}

icons! {
    /// Notifications.
    Bell => "bell",
    /// Navigate forward, open detail.
    ChevronRight => "chevron_right",
    /// Navigate back.
    Back => "back",
    /// Rest timer.
    Timer => "timer",
    /// Strength exercise, equipment.
    Dumbbell => "dumbbell",
    /// Start a session or a video.
    Play => "play",
    /// "Hoy" tab.
    Home => "home",
    /// "Mapa" tab, the body.
    Body => "body",
    /// Add.
    Plus => "plus",
    /// Remove one, decrement.
    Minus => "minus",
    /// "Progreso" tab.
    Progress => "progress",
    /// "Yo" tab, profile.
    Profile => "profile",
    /// Running.
    Run => "run",
    /// Swimming.
    Swim => "swim",
    /// Calibration weeks.
    Calibration => "calibration",
    /// Nutrition.
    Apple => "apple",
    /// Body weight.
    Scale => "scale",
    /// Sync in progress or done.
    Sync => "sync",
    /// No connection.
    WifiOff => "wifi_off",
    /// Done, selected.
    Check => "check",
    /// Swap an exercise.
    Swap => "swap",
    /// Records.
    Trophy => "trophy",
    /// Plan, events.
    Calendar => "calendar",
    /// Explanation.
    Info => "info",
    /// Search.
    Search => "search",
    /// Suggestion.
    Bolt => "bolt",
    /// Settings.
    Settings => "settings",
    /// Clients, organization.
    Users => "users",
    /// Import, upload.
    Upload => "upload",
    /// Warning, error.
    Alert => "alert",
    /// Streak, intensity.
    Flame => "flame",
    /// Share by QR.
    Qr => "qr",
    /// Share.
    Share => "share",
    /// Reorder handle.
    Drag => "drag",
    /// Filter.
    Filter => "filter",
    /// Share by link.
    Link => "link",
}

/// One icon at a given size and color.
///
/// ```rust,no_run
/// # use freya::prelude::*;
/// # use myos_uikit::{Icon, IconName, use_theme};
/// fn app() -> impl IntoElement {
///     let theme = use_theme();
///     Icon::new(IconName::Bolt).size(16.).color(theme.palette.accent)
/// }
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Icon {
    name: IconName,
    size: f32,
    color: Color,
    weight: f32,
}

impl Icon {
    /// A 24 px icon in black; set [`Self::color`] from the theme.
    #[must_use]
    pub const fn new(name: IconName) -> Self {
        Self {
            name,
            size: Dimen::ICON,
            color: Color::BLACK,
            weight: 2.,
        }
    }

    /// Box size in logical px; the drawing scales with it.
    #[must_use]
    pub const fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }

    /// Stroke color.
    #[must_use]
    pub const fn color(mut self, color: Color) -> Self {
        self.color = color;
        self
    }

    /// Visible stroke width in logical px, kept constant across sizes
    /// (default 2).
    #[must_use]
    pub const fn weight(mut self, weight: f32) -> Self {
        self.weight = weight;
        self
    }
}

impl Component for Icon {
    fn render(&self) -> impl IntoElement {
        // The SVG strokes are in 24-unit viewBox space and scale with the box.
        let stroke_width = self.weight * Dimen::ICON / self.size;
        SvgViewer::new(self.name.svg())
            .color(self.color)
            .stroke_width(stroke_width)
            .width(Size::px(self.size))
            .height(Size::px(self.size))
    }
}
