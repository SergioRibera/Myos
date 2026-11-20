use std::borrow::Cow;

use freya::prelude::*;

use super::Action;
use super::press::use_press;
use crate::theme::ThemeMode;
use crate::theme::use_theme;
use crate::tokens::{Brand, Palette, Radius, Space, Stroke, TextRole, TextRoleExt};

/// One choice of Ajustes → Apariencia, with a preview of that appearance
/// (Penpot component `Tarjeta de tema`). Previews use fixed brand colors,
/// whatever the current theme.
#[derive(Clone, PartialEq)]
pub struct ThemeCard {
    mode: ThemeMode,
    label: Cow<'static, str>,
    selected: bool,
    on_press: Option<Action>,
}

impl ThemeCard {
    /// Card for `mode`, labelled in Spanish ("Sistema", "Claro", "Oscuro").
    #[must_use]
    pub const fn new(mode: ThemeMode) -> Self {
        let label = match mode {
            ThemeMode::System => "Sistema",
            ThemeMode::Light => "Claro",
            ThemeMode::Dark => "Oscuro",
        };
        Self {
            mode,
            label: Cow::Borrowed(label),
            selected: false,
            on_press: None,
        }
    }

    /// Replace the label.
    #[must_use]
    pub fn label(mut self, label: impl Into<Cow<'static, str>>) -> Self {
        self.label = label.into();
        self
    }

    /// The mode in use.
    #[must_use]
    pub const fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// Choose this mode.
    #[must_use]
    pub fn on_press(mut self, on_press: impl Into<Action>) -> Self {
        self.on_press = Some(on_press.into());
        self
    }
}

/// Miniature screen: ground, a text bar and an accent bar.
fn preview_pane(palette: &Palette, width: Size, bar: f32, accent: bool) -> Rect {
    rect()
        .width(width)
        .height(Size::fill())
        .padding(Space::MD)
        .spacing(6.)
        .background(palette.ground)
        .child(
            rect()
                .width(Size::px(bar))
                .height(Size::px(6.))
                .corner_radius(3.)
                .background(palette.ink),
        )
        .maybe_child(accent.then(|| {
            rect()
                .width(Size::px(30.))
                .height(Size::px(6.))
                .corner_radius(3.)
                .background(palette.accent)
        }))
}

impl Component for ThemeCard {
    fn render(&self) -> impl IntoElement {
        let palette = use_theme().palette;
        let press = use_press();
        let preview = rect()
            .width(Size::fill())
            .height(Size::px(84.))
            .horizontal()
            .corner_radius(Radius::TILE)
            .overflow(Overflow::Clip)
            .background(Brand::CARBON);
        let preview = match self.mode {
            ThemeMode::Dark => preview.child(preview_pane(&Palette::DARK, Size::fill(), 50., true)),
            ThemeMode::Light => {
                preview.child(preview_pane(&Palette::LIGHT, Size::fill(), 50., true))
            }
            ThemeMode::System => preview
                .child(preview_pane(
                    &Palette::LIGHT,
                    Size::percent(50.),
                    28.,
                    false,
                ))
                .child(preview_pane(&Palette::DARK, Size::percent(50.), 28., false)),
        };
        let (outline, outline_width) = if self.selected {
            (palette.ink, Stroke::STRONG)
        } else {
            (palette.line, Stroke::HAIRLINE)
        };
        let radio = rect()
            .width(Size::px(18.))
            .height(Size::px(18.))
            .corner_radius(9.)
            .border(
                Border::new()
                    .fill(if self.selected {
                        palette.ink
                    } else {
                        palette.ink_subtle
                    })
                    .width(if self.selected { 5. } else { Stroke::REGULAR })
                    .alignment(BorderAlignment::Inner),
            );
        let body = rect()
            .width(Size::px(110.))
            .padding(Space::MD)
            .spacing(10.)
            .cross_align(Alignment::Center)
            .corner_radius(Radius::CONTROL)
            .background(if press.active() {
                palette.surface_raised
            } else {
                palette.surface
            })
            .border(press.focus_border(palette.accent, true).unwrap_or_else(|| {
                Border::new()
                    .fill(outline)
                    .width(outline_width)
                    .alignment(BorderAlignment::Inner)
            }))
            .child(preview)
            .child(
                label()
                    .text(self.label.clone())
                    .text_role(TextRole::BodyStrong)
                    .color(palette.ink),
            )
            .child(radio)
            .a11y_builder({
                let selected = self.selected;
                move |node| node.set_selected(selected)
            });
        press.wire(
            body,
            AccessibilityRole::RadioButton,
            true,
            self.on_press.as_ref(),
        )
    }
}
