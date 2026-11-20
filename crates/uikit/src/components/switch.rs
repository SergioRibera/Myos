use freya::prelude::*;

use super::Action;
use super::press::use_press;
use crate::theme::use_theme;

/// On / off toggle (Penpot component `Switch`). The caller owns the value
/// and flips it in [`Self::on_toggle`].
#[derive(Clone, PartialEq)]
pub struct Switch {
    on: bool,
    enabled: bool,
    on_toggle: Option<Action>,
}

impl Switch {
    /// Switch showing `on`.
    #[must_use]
    pub const fn new(on: bool) -> Self {
        Self {
            on,
            enabled: true,
            on_toggle: None,
        }
    }

    /// Disabled switches are dimmed and ignore input.
    #[must_use]
    pub const fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Called on press; flip the value here.
    #[must_use]
    pub fn on_toggle(mut self, on_toggle: impl Into<Action>) -> Self {
        self.on_toggle = Some(on_toggle.into());
        self
    }
}

impl Component for Switch {
    fn render(&self) -> impl IntoElement {
        let palette = use_theme().palette;
        let press = use_press();
        let (track, knob) = if self.on {
            (palette.cta, palette.on_cta)
        } else {
            (palette.surface_sunken, palette.ink_subtle)
        };
        let body = rect()
            .width(Size::px(48.))
            .height(Size::px(28.))
            .padding(3.)
            .horizontal()
            .cross_align(Alignment::Center)
            .main_align(if self.on {
                Alignment::End
            } else {
                Alignment::Start
            })
            .corner_radius(14.)
            .background(track)
            .border(press.focus_border(palette.accent, false))
            .child(
                rect()
                    .width(Size::px(22.))
                    .height(Size::px(22.))
                    .corner_radius(11.)
                    .background(knob),
            )
            .a11y_builder({
                let on = self.on;
                move |node| node.set_toggled(on.into())
            });
        press.wire(
            body,
            AccessibilityRole::Switch,
            self.enabled,
            self.on_toggle.as_ref(),
        )
    }
}
