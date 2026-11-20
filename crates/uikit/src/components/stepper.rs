use std::borrow::Cow;

use freya::prelude::*;

use super::Action;
use super::press::use_press;
use crate::icon::{Icon, IconName};
use crate::theme::use_theme;
use crate::tokens::{Dimen, Radius, Space, TextRole, TextRoleExt};

/// − value + control for weight, reps and portions (Penpot component
/// `Stepper`). The caller owns the value and formats it ("24 kg").
#[derive(Clone, PartialEq)]
pub struct Stepper {
    value: Cow<'static, str>,
    at_min: bool,
    at_max: bool,
    on_decrement: Option<Action>,
    on_increment: Option<Action>,
    layout: LayoutData,
}

impl Stepper {
    /// Stepper showing `value`.
    #[must_use]
    pub fn new(value: impl Into<Cow<'static, str>>) -> Self {
        Self {
            value: value.into(),
            at_min: false,
            at_max: false,
            on_decrement: None,
            on_increment: None,
            layout: LayoutData::default(),
        }
        .width(Size::px(224.))
    }

    /// Disables − (Penpot state `En el mínimo`).
    #[must_use]
    pub const fn at_min(mut self, at_min: bool) -> Self {
        self.at_min = at_min;
        self
    }

    /// Disables +.
    #[must_use]
    pub const fn at_max(mut self, at_max: bool) -> Self {
        self.at_max = at_max;
        self
    }

    /// Called by −.
    #[must_use]
    pub fn on_decrement(mut self, on_decrement: impl Into<Action>) -> Self {
        self.on_decrement = Some(on_decrement.into());
        self
    }

    /// Called by +.
    #[must_use]
    pub fn on_increment(mut self, on_increment: impl Into<Action>) -> Self {
        self.on_increment = Some(on_increment.into());
        self
    }
}

impl LayoutExt for Stepper {
    fn layout_mut(&mut self) -> &mut LayoutData {
        &mut self.layout
    }
}

impl ContainerExt for Stepper {}

impl Component for Stepper {
    fn render(&self) -> impl IntoElement {
        let palette = use_theme().palette;
        rect()
            .layout(self.layout.clone())
            .height(Size::px(Dimen::BUTTON))
            .padding(Space::XS)
            .horizontal()
            .content(Content::flex())
            .cross_align(Alignment::Center)
            .corner_radius(Radius::BUTTON)
            .background(palette.surface)
            .child(StepButton {
                icon: IconName::Minus,
                enabled: !self.at_min,
                on_press: self.on_decrement.clone(),
            })
            .child(
                label()
                    .width(Size::flex(1.))
                    .text(self.value.clone())
                    .text_role(TextRole::Title2)
                    .font_tabular()
                    .text_align(TextAlign::Center)
                    .color(palette.ink)
                    .max_lines(1),
            )
            .child(StepButton {
                icon: IconName::Plus,
                enabled: !self.at_max,
                on_press: self.on_increment.clone(),
            })
    }
}

/// The round − / + of a [`Stepper`].
#[derive(Clone, PartialEq)]
struct StepButton {
    icon: IconName,
    enabled: bool,
    on_press: Option<Action>,
}

impl Component for StepButton {
    fn render(&self) -> impl IntoElement {
        let palette = use_theme().palette;
        let press = use_press();
        let fill = if self.enabled && press.active() {
            palette.surface_sunken
        } else {
            palette.surface_raised
        };
        let body = rect()
            .width(Size::px(Dimen::TOUCH))
            .height(Size::px(Dimen::TOUCH))
            .center()
            .corner_radius(Dimen::TOUCH / 2.)
            .background(fill)
            .border(press.focus_border(palette.accent, true))
            .child(Icon::new(self.icon).weight(2.4).color(palette.ink));
        press.wire(
            body,
            AccessibilityRole::Button,
            self.enabled,
            self.on_press.as_ref(),
        )
    }
}
