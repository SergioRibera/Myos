use std::borrow::Cow;

use freya::prelude::*;

use super::Action;
use super::press::use_press;
use crate::icon::{Icon, IconName};
use crate::theme::use_theme;
use crate::tokens::{Dimen, Space, Stroke, TextRole, TextRoleExt};

/// One destination of the [`TabBar`] (Penpot component `Ítem de tab`).
#[derive(Clone, PartialEq)]
pub struct TabItem {
    icon: IconName,
    label: Cow<'static, str>,
    active: bool,
    on_press: Option<Action>,
}

impl TabItem {
    /// Inactive tab.
    #[must_use]
    pub fn new(icon: IconName, label: impl Into<Cow<'static, str>>) -> Self {
        Self {
            icon,
            label: label.into(),
            active: false,
            on_press: None,
        }
    }

    /// The current destination.
    #[must_use]
    pub const fn active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }

    /// Navigate here.
    #[must_use]
    pub fn on_press(mut self, on_press: impl Into<Action>) -> Self {
        self.on_press = Some(on_press.into());
        self
    }
}

impl Component for TabItem {
    fn render(&self) -> impl IntoElement {
        let palette = use_theme().palette;
        let press = use_press();
        let (icon, ink, weight) = if self.active {
            (palette.accent, palette.ink, 700)
        } else if press.active() {
            (palette.ink_muted, palette.ink_muted, 600)
        } else {
            (palette.ink_subtle, palette.ink_subtle, 600)
        };
        let body = rect()
            .width(Size::px(72.))
            .height(Size::px(64.))
            .center()
            .spacing(Space::XS)
            .corner_radius(Space::LG)
            .border(press.focus_border(palette.accent, true))
            .child(Icon::new(self.icon).color(icon))
            .child(
                label()
                    .text(self.label.clone())
                    .text_role(TextRole::Label)
                    .letter_spacing(0.)
                    .font_weight(weight)
                    .text_align(TextAlign::Center)
                    .color(ink)
                    .max_lines(1),
            )
            .a11y_builder({
                let active = self.active;
                move |node| node.set_selected(active)
            });
        press.wire(body, AccessibilityRole::Tab, true, self.on_press.as_ref())
    }
}

/// Bottom navigation of the mobile app: tabs around a highlighted central
/// "register" action (decision 7A).
#[derive(Clone, PartialEq, Default)]
pub struct TabBar {
    tabs: Vec<TabItem>,
    center: Option<(Cow<'static, str>, Action)>,
}

impl TabBar {
    /// Empty bar.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a tab. Tabs split evenly around the center action.
    #[must_use]
    pub fn tab(mut self, tab: TabItem) -> Self {
        self.tabs.push(tab);
        self
    }

    /// The central action; `label` is its accessible name.
    #[must_use]
    pub fn center_action(
        mut self,
        label: impl Into<Cow<'static, str>>,
        on_press: impl Into<Action>,
    ) -> Self {
        self.center = Some((label.into(), on_press.into()));
        self
    }
}

impl Component for TabBar {
    fn render(&self) -> impl IntoElement {
        let palette = use_theme().palette;
        let half = self.tabs.len().div_ceil(2);
        let (leading, trailing) = self.tabs.split_at(half);
        rect()
            .width(Size::fill())
            .padding((Space::XS, Space::SM))
            .horizontal()
            .content(Content::flex())
            .cross_align(Alignment::Center)
            .main_align(Alignment::SpaceAround)
            .background(palette.surface)
            .border(
                Border::new()
                    .fill(palette.line)
                    .width((Stroke::HAIRLINE, 0., 0., 0.))
                    .alignment(BorderAlignment::Inner),
            )
            .a11y_role(AccessibilityRole::TabList)
            .children(leading.iter().cloned().map(IntoElement::into_element))
            .maybe_child(
                self.center
                    .clone()
                    .map(|(name, on_press)| CenterAction { name, on_press }),
            )
            .children(trailing.iter().cloned().map(IntoElement::into_element))
    }
}

/// The round call-to-action in the middle of the [`TabBar`].
#[derive(Clone, PartialEq)]
struct CenterAction {
    name: Cow<'static, str>,
    on_press: Action,
}

impl Component for CenterAction {
    fn render(&self) -> impl IntoElement {
        let palette = use_theme().palette;
        let press = use_press();
        let body = rect()
            .width(Size::px(Dimen::BUTTON))
            .height(Size::px(Dimen::BUTTON))
            .center()
            .corner_radius(Dimen::BUTTON / 2.)
            .background(if press.active() {
                palette.cta_pressed
            } else {
                palette.cta
            })
            .border(press.focus_border(palette.accent, false))
            .child(
                Icon::new(IconName::Plus)
                    .size(26.)
                    .weight(2.4)
                    .color(palette.on_cta),
            )
            .a11y_alt(self.name.to_string());
        press.wire(body, AccessibilityRole::Button, true, Some(&self.on_press))
    }
}

/// Compact bar of secondary screens: back and title on the same row, an
/// optional trailing action.
#[derive(Clone, PartialEq)]
pub struct NavBar {
    title: Cow<'static, str>,
    back_label: Cow<'static, str>,
    on_back: Option<Action>,
    trailing: Option<Element>,
}

impl NavBar {
    /// Bar titled `title`.
    #[must_use]
    pub fn new(title: impl Into<Cow<'static, str>>) -> Self {
        Self {
            title: title.into(),
            back_label: Cow::Borrowed("Volver"),
            on_back: None,
            trailing: None,
        }
    }

    /// Shows the back button.
    #[must_use]
    pub fn on_back(mut self, on_back: impl Into<Action>) -> Self {
        self.on_back = Some(on_back.into());
        self
    }

    /// Accessible name of the back button (default "Volver").
    #[must_use]
    pub fn back_label(mut self, back_label: impl Into<Cow<'static, str>>) -> Self {
        self.back_label = back_label.into();
        self
    }

    /// Element at the end of the row (an icon button, a chip).
    #[must_use]
    pub fn trailing(mut self, trailing: impl IntoElement) -> Self {
        self.trailing = Some(trailing.into_element());
        self
    }
}

impl Component for NavBar {
    fn render(&self) -> impl IntoElement {
        let palette = use_theme().palette;
        rect()
            .width(Size::fill())
            .height(Size::px(Dimen::BUTTON))
            .padding((0., Space::SM))
            .horizontal()
            .content(Content::flex())
            .cross_align(Alignment::Center)
            .spacing(Space::XS)
            .maybe_child(self.on_back.clone().map(|on_press| BackButton {
                name: self.back_label.clone(),
                on_press,
            }))
            .child(
                label()
                    .width(Size::flex(1.))
                    .text(self.title.clone())
                    .text_role(TextRole::Title2)
                    .color(palette.ink)
                    .max_lines(1)
                    .text_overflow(TextOverflow::Ellipsis),
            )
            .maybe_child(self.trailing.clone())
    }
}

/// Back arrow with a 48 px target.
#[derive(Clone, PartialEq)]
struct BackButton {
    name: Cow<'static, str>,
    on_press: Action,
}

impl Component for BackButton {
    fn render(&self) -> impl IntoElement {
        let palette = use_theme().palette;
        let press = use_press();
        let body = rect()
            .width(Size::px(Dimen::TOUCH))
            .height(Size::px(Dimen::TOUCH))
            .center()
            .corner_radius(Dimen::TOUCH / 2.)
            .map(
                press.active().then_some(palette.surface_raised),
                BackgroundExt::background,
            )
            .border(press.focus_border(palette.accent, true))
            .child(Icon::new(IconName::Back).color(palette.ink))
            .a11y_alt(self.name.to_string());
        press.wire(body, AccessibilityRole::Button, true, Some(&self.on_press))
    }
}
