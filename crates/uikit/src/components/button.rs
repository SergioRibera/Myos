use std::borrow::Cow;

use freya::prelude::*;

use super::Action;
use super::press::use_press;
use crate::icon::{Icon, IconName};
use crate::motion::use_press_fill;
use crate::theme::use_theme;
use crate::tokens::{Dimen, Palette, Space, Stroke, TextRole, TextRoleExt};

/// Visual weight of a [`Button`] (Penpot property `Tipo`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ButtonKind {
    /// The one main action of a screen.
    #[default]
    Primary,
    /// Other actions.
    Secondary,
    /// Low-emphasis actions, outlined.
    Ghost,
    /// Irreversible actions (delete data, revoke access).
    Destructive,
    /// Text only, for inline actions such as "Deshacer".
    Plain,
}

/// Height of a [`Button`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ButtonSize {
    /// 56 px, full-width actions.
    #[default]
    Regular,
    /// 28 px, actions inside cards.
    Small,
}

struct Colors {
    fill: Option<Color>,
    pressed: Color,
    border: Option<Color>,
    ink: Color,
}

impl ButtonKind {
    const fn colors(self, palette: &Palette, size: ButtonSize) -> Colors {
        match self {
            Self::Primary => Colors {
                fill: Some(palette.cta),
                pressed: palette.cta_pressed,
                border: None,
                ink: palette.on_cta,
            },
            Self::Secondary => Colors {
                fill: Some(palette.surface_raised),
                pressed: palette.surface_sunken,
                border: None,
                ink: palette.ink,
            },
            Self::Ghost => Colors {
                fill: None,
                pressed: palette.surface_raised,
                border: Some(palette.line),
                ink: match size {
                    ButtonSize::Regular => palette.ink,
                    ButtonSize::Small => palette.ink_muted,
                },
            },
            Self::Destructive => Colors {
                fill: Some(palette.danger_surface),
                pressed: palette.danger_surface_pressed,
                border: None,
                ink: palette.excess,
            },
            Self::Plain => Colors {
                fill: None,
                pressed: palette.surface_raised,
                border: None,
                ink: palette.ink_muted,
            },
        }
    }
}

/// Pill button (Penpot component `Botón`: Tipo × Estado × Tema).
///
/// Sizes to its label by default; `.expanded()` or `.width(..)` stretch it.
/// Height and padding are fixed by [`ButtonSize`].
///
/// ```rust,no_run
/// # use freya::prelude::*;
/// # use myos_uikit as ui;
/// fn app() -> impl IntoElement {
///     ui::Button::primary("Empezar sesión").expanded().on_press(|| println!("start"))
/// }
/// ```
#[derive(Clone, PartialEq)]
pub struct Button {
    label: Cow<'static, str>,
    kind: ButtonKind,
    size: ButtonSize,
    icon: Option<IconName>,
    enabled: bool,
    on_press: Option<Action>,
    layout: LayoutData,
}

impl Button {
    /// A regular button of `kind`.
    #[must_use]
    pub fn new(kind: ButtonKind, label: impl Into<Cow<'static, str>>) -> Self {
        Self {
            label: label.into(),
            kind,
            size: ButtonSize::Regular,
            icon: None,
            enabled: true,
            on_press: None,
            layout: LayoutData::default(),
        }
    }

    /// Main action.
    #[must_use]
    pub fn primary(label: impl Into<Cow<'static, str>>) -> Self {
        Self::new(ButtonKind::Primary, label)
    }

    /// Secondary action.
    #[must_use]
    pub fn secondary(label: impl Into<Cow<'static, str>>) -> Self {
        Self::new(ButtonKind::Secondary, label)
    }

    /// Low-emphasis action.
    #[must_use]
    pub fn ghost(label: impl Into<Cow<'static, str>>) -> Self {
        Self::new(ButtonKind::Ghost, label)
    }

    /// Irreversible action.
    #[must_use]
    pub fn destructive(label: impl Into<Cow<'static, str>>) -> Self {
        Self::new(ButtonKind::Destructive, label)
    }

    /// Text-only action.
    #[must_use]
    pub fn plain(label: impl Into<Cow<'static, str>>) -> Self {
        Self::new(ButtonKind::Plain, label)
    }

    /// Height.
    #[must_use]
    pub const fn size(mut self, size: ButtonSize) -> Self {
        self.size = size;
        self
    }

    /// Shortcut for [`ButtonSize::Small`].
    #[must_use]
    pub const fn small(self) -> Self {
        self.size(ButtonSize::Small)
    }

    /// Icon before the label.
    #[must_use]
    pub const fn icon(mut self, icon: IconName) -> Self {
        self.icon = Some(icon);
        self
    }

    /// Disabled buttons are dimmed and ignore input.
    #[must_use]
    pub const fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Called on click, tap, Enter or Space.
    #[must_use]
    pub fn on_press(mut self, on_press: impl Into<Action>) -> Self {
        self.on_press = Some(on_press.into());
        self
    }
}

impl LayoutExt for Button {
    fn layout_mut(&mut self) -> &mut LayoutData {
        &mut self.layout
    }
}

impl ContainerExt for Button {}

impl Component for Button {
    fn render(&self) -> impl IntoElement {
        let palette = use_theme().palette;
        let press = use_press();
        let colors = self.kind.colors(palette, self.size);
        let (height, padding, role, icon_size) = match self.size {
            ButtonSize::Regular => (Dimen::BUTTON, Space::XXL, TextRole::Action, 20.),
            ButtonSize::Small => (Dimen::BUTTON_SMALL, 14., TextRole::ActionSmall, 14.),
        };
        let fill = if self.enabled && press.active() {
            Some(colors.pressed)
        } else {
            colors.fill
        };
        let border = press.focus_border(palette.accent, false).or_else(|| {
            colors.border.map(|fill| {
                Border::new()
                    .fill(fill)
                    .width(Stroke::REGULAR)
                    .alignment(BorderAlignment::Inner)
            })
        });

        // Width, margin and position come from the caller; height and
        // padding belong to the size variant.
        let body = rect()
            .layout(self.layout.clone())
            .height(Size::px(height))
            .padding((0., padding))
            .horizontal()
            .center()
            .spacing(Space::SM)
            .corner_radius(height / 2.)
            .background(use_press_fill(
                fill.unwrap_or_else(|| colors.pressed.with_a(0)),
            ))
            .border(border)
            .maybe_child(
                self.icon
                    .map(|icon| Icon::new(icon).size(icon_size).color(colors.ink)),
            )
            .child(
                label()
                    .text(self.label.clone())
                    .text_role(role)
                    .color(colors.ink)
                    .max_lines(1),
            );
        press.wire(
            body,
            AccessibilityRole::Button,
            self.enabled,
            self.on_press.as_ref(),
        )
    }
}
