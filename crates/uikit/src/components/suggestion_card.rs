use std::borrow::Cow;

use freya::prelude::*;

use super::Action;
use super::button::Button;
use crate::icon::{Icon, IconName};
use crate::theme::use_theme;
use crate::tokens::{Radius, Space, Stroke, TextRole, TextRoleExt};

/// Where the user is with a suggestion (Penpot property `Estado`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum SuggestionState {
    /// Proposed.
    #[default]
    Normal,
    /// "¿Por qué?" opened: shows the data behind it.
    Explained,
    /// "No aplica": collapsed, with undo.
    Dismissed,
}

/// Fixed copy of a [`SuggestionCard`]. Defaults are the Spanish (tuteo) UI
/// copy of the proposal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuggestionLabels {
    /// Tag above the title.
    pub tag: Cow<'static, str>,
    /// Accept button.
    pub accept: Cow<'static, str>,
    /// Explain button.
    pub explain: Cow<'static, str>,
    /// Dismiss button.
    pub dismiss: Cow<'static, str>,
    /// Undo a dismissal.
    pub undo: Cow<'static, str>,
}

impl Default for SuggestionLabels {
    fn default() -> Self {
        Self {
            tag: Cow::Borrowed("Sugerencia"),
            accept: Cow::Borrowed("Agregar"),
            explain: Cow::Borrowed("¿Por qué?"),
            dismiss: Cow::Borrowed("No aplica"),
            undo: Cow::Borrowed("Deshacer"),
        }
    }
}

/// A suggestion, presented as one: accept, ask why or dismiss with a
/// reason (Penpot component `Tarjeta de sugerencia`).
#[derive(Clone, PartialEq)]
pub struct SuggestionCard {
    title: Cow<'static, str>,
    body: Cow<'static, str>,
    explanation: Option<Cow<'static, str>>,
    dismissal: Option<Cow<'static, str>>,
    state: SuggestionState,
    labels: SuggestionLabels,
    on_accept: Option<Action>,
    on_explain: Option<Action>,
    on_dismiss: Option<Action>,
    on_undo: Option<Action>,
}

impl SuggestionCard {
    /// Suggestion with its title and one-line reason.
    #[must_use]
    pub fn new(title: impl Into<Cow<'static, str>>, body: impl Into<Cow<'static, str>>) -> Self {
        Self {
            title: title.into(),
            body: body.into(),
            explanation: None,
            dismissal: None,
            state: SuggestionState::Normal,
            labels: SuggestionLabels::default(),
            on_accept: None,
            on_explain: None,
            on_dismiss: None,
            on_undo: None,
        }
    }

    /// Data behind the suggestion, shown in [`SuggestionState::Explained`]:
    /// sets counted, range used, confidence.
    #[must_use]
    pub fn explanation(mut self, explanation: impl Into<Cow<'static, str>>) -> Self {
        self.explanation = Some(explanation.into());
        self
    }

    /// Note shown in [`SuggestionState::Dismissed`] in place of the body.
    #[must_use]
    pub fn dismissal(mut self, dismissal: impl Into<Cow<'static, str>>) -> Self {
        self.dismissal = Some(dismissal.into());
        self
    }

    /// Current state.
    #[must_use]
    pub const fn state(mut self, state: SuggestionState) -> Self {
        self.state = state;
        self
    }

    /// Replace the fixed copy.
    #[must_use]
    pub fn labels(mut self, labels: SuggestionLabels) -> Self {
        self.labels = labels;
        self
    }

    /// "Agregar".
    #[must_use]
    pub fn on_accept(mut self, on_accept: impl Into<Action>) -> Self {
        self.on_accept = Some(on_accept.into());
        self
    }

    /// "¿Por qué?"; toggle the explained state here.
    #[must_use]
    pub fn on_explain(mut self, on_explain: impl Into<Action>) -> Self {
        self.on_explain = Some(on_explain.into());
        self
    }

    /// "No aplica"; ask for the reason here.
    #[must_use]
    pub fn on_dismiss(mut self, on_dismiss: impl Into<Action>) -> Self {
        self.on_dismiss = Some(on_dismiss.into());
        self
    }

    /// "Deshacer" after a dismissal.
    #[must_use]
    pub fn on_undo(mut self, on_undo: impl Into<Action>) -> Self {
        self.on_undo = Some(on_undo.into());
        self
    }
}

impl SuggestionCard {
    /// Accept / explain / dismiss, or undo once dismissed.
    fn actions(&self) -> Rect {
        if self.state == SuggestionState::Dismissed {
            return rect().child(with_action(
                Button::plain(self.labels.undo.clone()).small(),
                self.on_undo.as_ref(),
            ));
        }
        let width = Size::px(100.);
        rect()
            .horizontal()
            .spacing(Space::SM)
            .child(with_action(
                Button::secondary(self.labels.accept.clone())
                    .small()
                    .width(width.clone()),
                self.on_accept.as_ref(),
            ))
            .child(with_action(
                Button::ghost(self.labels.explain.clone())
                    .small()
                    .width(width.clone()),
                self.on_explain.as_ref(),
            ))
            .child(with_action(
                Button::ghost(self.labels.dismiss.clone())
                    .small()
                    .width(width),
                self.on_dismiss.as_ref(),
            ))
    }
}

/// Attach `action` to `button` when there is one.
fn with_action(button: Button, action: Option<&Action>) -> Button {
    match action {
        Some(action) => button.on_press(action.clone()),
        None => button,
    }
}

impl Component for SuggestionCard {
    fn render(&self) -> impl IntoElement {
        let palette = use_theme().palette;
        let dismissed = self.state == SuggestionState::Dismissed;
        let tag = rect()
            .height(Size::px(24.))
            .padding((0., Space::SM))
            .horizontal()
            .cross_align(Alignment::Center)
            .spacing(Space::XS)
            .corner_radius(Radius::TILE)
            .background(palette.surface)
            .child(
                Icon::new(IconName::Bolt)
                    .size(16.)
                    .weight(2.2)
                    .color(palette.accent),
            )
            .child(
                label()
                    .text(self.labels.tag.clone())
                    .text_role(TextRole::ActionSmall)
                    .color(palette.ink_muted),
            );
        let title = label()
            .text(self.title.clone())
            .text_role(TextRole::BodyStrong)
            .color(if dismissed {
                palette.ink_subtle
            } else {
                palette.ink
            })
            .max_lines(2)
            .text_overflow(TextOverflow::Ellipsis);
        let body_text = if dismissed {
            self.dismissal.clone().unwrap_or_else(|| self.body.clone())
        } else {
            self.body.clone()
        };
        let body = label()
            .text(body_text)
            .text_role(TextRole::Caption)
            .line_height(TextRole::Caption.style().line_height)
            .color(palette.ink_subtle);
        let explanation = (self.state == SuggestionState::Explained)
            .then(|| self.explanation.clone())
            .flatten()
            .map(|text| {
                rect()
                    .width(Size::fill())
                    .padding(11.)
                    .corner_radius(Radius::TILE)
                    .background(palette.ground)
                    .child(
                        label()
                            .text(text)
                            .text_role(TextRole::Caption)
                            .line_height(TextRole::Caption.style().line_height)
                            .color(palette.ink_muted),
                    )
            });
        let actions = self.actions();
        rect()
            .width(Size::fill())
            .padding(14.)
            .spacing(Space::SM)
            .corner_radius(Radius::CONTROL)
            .border(
                Border::new()
                    .fill(palette.line)
                    .width(Stroke::REGULAR)
                    .alignment(BorderAlignment::Inner),
            )
            .child(tag)
            .child(title)
            .child(body)
            .maybe_child(explanation)
            .child(actions)
    }
}
