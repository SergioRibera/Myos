use std::borrow::Cow;

use freya::prelude::*;

use super::Action;
use super::press::use_press;
use crate::theme::use_theme;
use crate::tokens::{Dimen, Palette, Radius, Space, TextRole, TextRoleExt};

/// Training status of a muscle against its range. Shared by chips, alert
/// dots and the muscle map legend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Status {
    /// Below the range.
    Deficit,
    /// Above the range.
    Excess,
    /// Inside the range.
    InRange,
}

impl Status {
    /// Dot color of this status.
    #[must_use]
    pub const fn color(self, palette: &Palette) -> Color {
        match self {
            Self::Deficit => palette.deficit,
            Self::Excess => palette.excess,
            Self::InRange => palette.accent,
        }
    }
}

/// Variant of a [`Chip`] (Penpot property `Tipo`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ChipKind {
    /// Plain metadata ("45 min").
    #[default]
    Neutral,
    /// Chosen filter or option.
    Selected,
    /// A status with its colored dot.
    Status(Status),
}

/// Compact label (Penpot component `Chip`). Pressable when it has
/// [`Self::on_press`], e.g. as a filter.
#[derive(Clone, PartialEq)]
pub struct Chip {
    label: Cow<'static, str>,
    kind: ChipKind,
    on_press: Option<Action>,
}

impl Chip {
    /// A neutral chip.
    #[must_use]
    pub fn new(label: impl Into<Cow<'static, str>>) -> Self {
        Self {
            label: label.into(),
            kind: ChipKind::Neutral,
            on_press: None,
        }
    }

    /// Variant.
    #[must_use]
    pub const fn kind(mut self, kind: ChipKind) -> Self {
        self.kind = kind;
        self
    }

    /// Shortcut for [`ChipKind::Selected`] when `selected`, neutral otherwise.
    #[must_use]
    pub const fn selected(self, selected: bool) -> Self {
        self.kind(if selected {
            ChipKind::Selected
        } else {
            ChipKind::Neutral
        })
    }

    /// Shortcut for [`ChipKind::Status`].
    #[must_use]
    pub const fn status(self, status: Status) -> Self {
        self.kind(ChipKind::Status(status))
    }

    /// Makes the chip pressable.
    #[must_use]
    pub fn on_press(mut self, on_press: impl Into<Action>) -> Self {
        self.on_press = Some(on_press.into());
        self
    }
}

impl Component for Chip {
    fn render(&self) -> impl IntoElement {
        let palette = use_theme().palette;
        let press = use_press();
        let selected = self.kind == ChipKind::Selected;
        let (fill, ink) = match (selected, press.active()) {
            (true, false) => (palette.cta, palette.on_cta),
            (true, true) => (palette.cta_pressed, palette.on_cta),
            (false, false) => (palette.surface_raised, palette.ink_muted),
            (false, true) => (palette.surface_sunken, palette.ink_muted),
        };
        let dot = match self.kind {
            ChipKind::Status(status) => Some(status.color(palette)),
            ChipKind::Neutral | ChipKind::Selected => None,
        };
        let body = rect()
            .height(Size::px(Dimen::CHIP))
            .padding((0., Space::MD))
            .horizontal()
            .cross_align(Alignment::Center)
            .spacing(6.)
            .corner_radius(Radius::CHIP)
            .background(fill)
            .border(press.focus_border(palette.accent, false))
            .maybe_child(dot.map(|color| {
                rect()
                    .width(Size::px(8.))
                    .height(Size::px(8.))
                    .corner_radius(4.)
                    .background(color)
            }))
            .child(
                label()
                    .text(self.label.clone())
                    .text_role(TextRole::Tag)
                    .color(ink)
                    .max_lines(1),
            );
        let role = if self.on_press.is_some() {
            AccessibilityRole::Button
        } else {
            AccessibilityRole::Label
        };
        press.wire(body, role, true, self.on_press.as_ref())
    }
}
