use std::borrow::Cow;

use freya::prelude::*;

use super::Action;
use super::press::use_press;
use crate::icon::{Icon, IconName};
use crate::theme::use_theme;
use crate::tokens::{Palette, Radius, Space, Stroke, TextRole, TextRoleExt};

/// Progress of one set in the live session (Penpot property `Estado`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum SetState {
    /// Not reached yet.
    #[default]
    Pending,
    /// The set being performed now.
    Active,
    /// Logged.
    Done,
}

/// One row of the set table: number, previous result, weight, reps, effort
/// and the done check (Penpot component `Fila de serie`). Values come
/// formatted; the row only lays them out.
#[derive(Clone, PartialEq)]
pub struct SetRow {
    number: u16,
    previous: Option<Cow<'static, str>>,
    weight: Cow<'static, str>,
    reps: Cow<'static, str>,
    effort: Option<Cow<'static, str>>,
    state: SetState,
    on_toggle: Option<Action>,
}

impl SetRow {
    /// Row `number` with its target or logged weight and reps.
    #[must_use]
    pub fn new(
        number: u16,
        weight: impl Into<Cow<'static, str>>,
        reps: impl Into<Cow<'static, str>>,
    ) -> Self {
        Self {
            number,
            previous: None,
            weight: weight.into(),
            reps: reps.into(),
            effort: None,
            state: SetState::Pending,
            on_toggle: None,
        }
    }

    /// Last session's result for this set ("22 × 9").
    #[must_use]
    pub fn previous(mut self, previous: impl Into<Cow<'static, str>>) -> Self {
        self.previous = Some(previous.into());
        self
    }

    /// RIR or RPE once logged; shows "·" while empty.
    #[must_use]
    pub fn effort(mut self, effort: impl Into<Cow<'static, str>>) -> Self {
        self.effort = Some(effort.into());
        self
    }

    /// Progress.
    #[must_use]
    pub const fn state(mut self, state: SetState) -> Self {
        self.state = state;
        self
    }

    /// Called by the check; mark done or undo here.
    #[must_use]
    pub fn on_toggle(mut self, on_toggle: impl Into<Action>) -> Self {
        self.on_toggle = Some(on_toggle.into());
        self
    }
}

struct RowColors {
    fill: Color,
    outline: Option<Color>,
    number: Color,
    value: Color,
}

impl SetState {
    const fn colors(self, palette: &Palette) -> RowColors {
        match self {
            Self::Pending => RowColors {
                fill: palette.surface,
                outline: None,
                number: palette.ink_subtle,
                value: palette.ink_subtle,
            },
            Self::Active => RowColors {
                fill: palette.surface_raised,
                outline: Some(palette.ink),
                number: palette.accent,
                value: palette.ink,
            },
            Self::Done => RowColors {
                fill: palette.surface,
                outline: None,
                number: palette.ink,
                value: palette.ink,
            },
        }
    }
}

fn cell(text: Cow<'static, str>, width: Size, role: TextRole, color: Color) -> Label {
    label()
        .width(width)
        .text(text)
        .text_role(role)
        .font_tabular()
        .text_align(TextAlign::Center)
        .color(color)
        .max_lines(1)
}

impl Component for SetRow {
    fn render(&self) -> impl IntoElement {
        let palette = use_theme().palette;
        let colors = self.state.colors(palette);
        let effort = self.effort.clone().unwrap_or(Cow::Borrowed("·"));
        let effort_color = if self.effort.is_some() {
            colors.value
        } else {
            palette.ink_subtle
        };
        rect()
            .width(Size::fill())
            .height(Size::px(44.))
            .padding((0., Space::MD))
            .horizontal()
            .content(Content::flex())
            .cross_align(Alignment::Center)
            .spacing(Space::SM)
            .corner_radius(Radius::ROW)
            .background(colors.fill)
            .border(colors.outline.map(|fill| {
                Border::new()
                    .fill(fill)
                    .width(Stroke::REGULAR)
                    .alignment(BorderAlignment::Inner)
            }))
            .child(cell(
                self.number.to_string().into(),
                Size::px(20.),
                TextRole::BodyStrong,
                colors.number,
            ))
            .child(cell(
                self.previous.clone().unwrap_or(Cow::Borrowed("—")),
                Size::flex(1.),
                TextRole::Detail,
                palette.ink_subtle,
            ))
            .child(cell(
                self.weight.clone(),
                Size::px(48.),
                TextRole::BodyStrong,
                colors.value,
            ))
            .child(cell(
                self.reps.clone(),
                Size::px(40.),
                TextRole::BodyStrong,
                colors.value,
            ))
            .child(cell(
                effort,
                Size::px(32.),
                TextRole::BodyStrong,
                effort_color,
            ))
            .child(DoneCheck {
                state: self.state,
                on_toggle: self.on_toggle.clone(),
            })
    }
}

/// The 28 px circle at the end of a [`SetRow`].
#[derive(Clone, PartialEq)]
struct DoneCheck {
    state: SetState,
    on_toggle: Option<Action>,
}

impl Component for DoneCheck {
    fn render(&self) -> impl IntoElement {
        let palette = use_theme().palette;
        let press = use_press();
        let base = rect()
            .width(Size::px(28.))
            .height(Size::px(28.))
            .center()
            .corner_radius(14.);
        let ring = match self.state {
            SetState::Done => None,
            SetState::Active => Some(palette.ink_muted),
            SetState::Pending if press.active() => Some(palette.ink_muted),
            SetState::Pending => Some(palette.surface_sunken),
        };
        let border = press.focus_border(palette.accent, false).or_else(|| {
            ring.map(|fill| {
                Border::new()
                    .fill(fill)
                    .width(Stroke::REGULAR)
                    .alignment(BorderAlignment::Inner)
            })
        });
        let body = base
            .border(border)
            .maybe(self.state == SetState::Done, |el| {
                el.background(palette.ground).child(
                    Icon::new(IconName::Check)
                        .size(18.)
                        .weight(2.6)
                        .color(palette.accent),
                )
            })
            .a11y_builder({
                let done = self.state == SetState::Done;
                move |node| node.set_toggled(done.into())
            });
        press.wire(
            body,
            AccessibilityRole::CheckBox,
            true,
            self.on_toggle.as_ref(),
        )
    }
}
