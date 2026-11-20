use std::borrow::Cow;

use freya::prelude::*;

use super::Action;
use super::press::use_press;
use crate::icon::{Icon, IconName};
use crate::theme::use_theme;
use crate::tokens::{Palette, Radius, Space, TextRole, TextRoleExt};

/// What an [`AlertCard`] reports (Penpot property `Tipo`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AlertKind {
    /// A muscle below its range.
    Deficit,
    /// A muscle above its range.
    Excess,
    /// An unbalanced pair (push / pull, quads / hamstrings).
    Imbalance,
    /// Back in range.
    Resolved,
}

impl AlertKind {
    const fn dot(self, palette: &Palette) -> Color {
        match self {
            Self::Deficit => palette.deficit,
            Self::Excess | Self::Imbalance => palette.excess,
            Self::Resolved => palette.accent,
        }
    }
}

/// One-line finding with a detail line, opening its explanation on press
/// (Penpot component `Tarjeta de alerta`).
#[derive(Clone, PartialEq)]
pub struct AlertCard {
    kind: AlertKind,
    title: Cow<'static, str>,
    detail: Cow<'static, str>,
    on_press: Option<Action>,
    layout: LayoutData,
}

impl AlertCard {
    /// Card for `kind` with its title and detail line.
    #[must_use]
    pub fn new(
        kind: AlertKind,
        title: impl Into<Cow<'static, str>>,
        detail: impl Into<Cow<'static, str>>,
    ) -> Self {
        Self {
            kind,
            title: title.into(),
            detail: detail.into(),
            on_press: None,
            layout: LayoutData::default(),
        }
        .width(Size::fill())
    }

    /// Opens the detail; shows the chevron.
    #[must_use]
    pub fn on_press(mut self, on_press: impl Into<Action>) -> Self {
        self.on_press = Some(on_press.into());
        self
    }
}

impl LayoutExt for AlertCard {
    fn layout_mut(&mut self) -> &mut LayoutData {
        &mut self.layout
    }
}

impl ContainerExt for AlertCard {}

impl Component for AlertCard {
    fn render(&self) -> impl IntoElement {
        let palette = use_theme().palette;
        let press = use_press();
        let fill = if press.active() {
            palette.surface_raised
        } else {
            palette.surface
        };
        let text = rect()
            .width(Size::flex(1.))
            .spacing(3.)
            .child(
                label()
                    .text(self.title.clone())
                    .text_role(TextRole::BodyStrong)
                    .color(palette.ink)
                    .max_lines(1)
                    .text_overflow(TextOverflow::Ellipsis),
            )
            .child(
                label()
                    .text(self.detail.clone())
                    .text_role(TextRole::Detail)
                    .color(palette.ink_subtle)
                    .max_lines(2)
                    .text_overflow(TextOverflow::Ellipsis),
            );
        let body = rect()
            .layout(self.layout.clone())
            .padding((Space::LG, Space::LG))
            .horizontal()
            .content(Content::flex())
            .cross_align(Alignment::Center)
            .spacing(Space::MD)
            .corner_radius(Radius::ALERT)
            .background(fill)
            .border(press.focus_border(palette.accent, true))
            .child(
                rect()
                    .width(Size::px(10.))
                    .height(Size::px(10.))
                    .corner_radius(5.)
                    .background(self.kind.dot(palette)),
            )
            .child(text)
            .maybe_child(self.on_press.as_ref().map(|_| {
                Icon::new(IconName::ChevronRight)
                    .size(20.)
                    .color(palette.ink_subtle)
            }));
        press.wire(
            body,
            AccessibilityRole::Button,
            true,
            self.on_press.as_ref(),
        )
    }
}
