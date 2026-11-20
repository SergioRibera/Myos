use std::borrow::Cow;

use freya::prelude::*;

use super::Action;
use super::press::use_press;
use crate::icon::{Icon, IconName};
use crate::motion::use_press_fill;
use crate::theme::use_theme;
use crate::tokens::{Radius, Space, Stroke, TextRole, TextRoleExt};

/// Large single-choice option with icon, title and description, used in
/// onboarding and settings (Penpot component `Opción de selección`).
#[derive(Clone, PartialEq)]
pub struct SelectOption {
    icon: IconName,
    title: Cow<'static, str>,
    description: Option<Cow<'static, str>>,
    selected: bool,
    enabled: bool,
    on_press: Option<Action>,
    layout: LayoutData,
}

impl SelectOption {
    /// Unselected option.
    #[must_use]
    pub fn new(icon: IconName, title: impl Into<Cow<'static, str>>) -> Self {
        Self {
            icon,
            title: title.into(),
            description: None,
            selected: false,
            enabled: true,
            on_press: None,
            layout: LayoutData::default(),
        }
        .width(Size::fill())
    }

    /// Line under the title.
    #[must_use]
    pub fn description(mut self, description: impl Into<Cow<'static, str>>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Selected state.
    #[must_use]
    pub const fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// Disabled options are dimmed and ignore input.
    #[must_use]
    pub const fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Called when chosen.
    #[must_use]
    pub fn on_press(mut self, on_press: impl Into<Action>) -> Self {
        self.on_press = Some(on_press.into());
        self
    }
}

impl LayoutExt for SelectOption {
    fn layout_mut(&mut self) -> &mut LayoutData {
        &mut self.layout
    }
}

impl ContainerExt for SelectOption {}

impl Component for SelectOption {
    fn render(&self) -> impl IntoElement {
        let palette = use_theme().palette;
        let press = use_press();
        let (fill, outline, outline_width) = if self.selected {
            (palette.surface_raised, palette.ink, Stroke::STRONG)
        } else if self.enabled && press.active() {
            (palette.surface_raised, palette.line, Stroke::HAIRLINE)
        } else {
            (palette.surface, palette.line, Stroke::HAIRLINE)
        };
        let border = press.focus_border(palette.accent, true).unwrap_or_else(|| {
            Border::new()
                .fill(outline)
                .width(outline_width)
                .alignment(BorderAlignment::Inner)
        });
        let icon_tile = rect()
            .width(Size::px(40.))
            .height(Size::px(40.))
            .center()
            .corner_radius(14.)
            .background(palette.ground)
            .child(Icon::new(self.icon).size(22.).color(if self.selected {
                palette.accent
            } else {
                palette.ink_muted
            }));
        let text = rect()
            .width(Size::flex(1.))
            .spacing(2.)
            .child(
                label()
                    .text(self.title.clone())
                    .text_role(TextRole::BodyStrong)
                    .color(palette.ink)
                    .max_lines(1),
            )
            .maybe_child(self.description.clone().map(|description| {
                label()
                    .text(description)
                    .text_role(TextRole::Caption)
                    .color(palette.ink_subtle)
                    .max_lines(2)
                    .text_overflow(TextOverflow::Ellipsis)
            }));
        let check = if self.selected {
            rect()
                .width(Size::px(22.))
                .height(Size::px(22.))
                .center()
                .corner_radius(11.)
                .background(palette.ink)
                .child(
                    Icon::new(IconName::Check)
                        .size(14.)
                        .weight(2.)
                        .color(palette.ground),
                )
        } else {
            rect()
                .width(Size::px(22.))
                .height(Size::px(22.))
                .corner_radius(11.)
                .border(
                    Border::new()
                        .fill(palette.ink_subtle)
                        .width(Stroke::REGULAR)
                        .alignment(BorderAlignment::Inner),
                )
        };
        let body = rect()
            .layout(self.layout.clone())
            .padding((Space::LG, Space::LG))
            .horizontal()
            .content(Content::flex())
            .cross_align(Alignment::Center)
            .spacing(Space::MD)
            .corner_radius(Radius::CONTROL)
            .background(use_press_fill(fill))
            .border(border)
            .child(icon_tile)
            .child(text)
            .child(check)
            .a11y_builder({
                let selected = self.selected;
                move |node| node.set_selected(selected)
            });
        press.wire(
            body,
            AccessibilityRole::RadioButton,
            self.enabled,
            self.on_press.as_ref(),
        )
    }
}
