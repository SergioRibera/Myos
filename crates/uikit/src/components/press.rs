//! Pointer, touch and keyboard press handling shared by every pressable
//! component, so hover, pressed and focus states behave the same everywhere.

use freya::prelude::*;

use super::Action;
use crate::tokens::{DISABLED_OPACITY, Stroke};

/// Interaction state of one pressable element.
#[derive(Clone, Copy)]
pub(super) struct Press {
    hovering: State<bool>,
    pressing: State<bool>,
    a11y_id: AccessibilityId,
    focus: Memo<Focus>,
}

/// Hook: interaction state for a pressable component.
pub(super) fn use_press() -> Press {
    let a11y_id = use_a11y();
    Press {
        hovering: use_state(|| false),
        pressing: use_state(|| false),
        a11y_id,
        focus: use_focus(a11y_id),
    }
}

impl Press {
    /// Hovered by a pointer or held down. Components show their "pressed"
    /// fill for both: the proposal has one interaction state, and on touch
    /// there is no hover.
    pub(super) fn active(&self) -> bool {
        *self.hovering.read() || *self.pressing.read()
    }

    /// Focus ring for keyboard navigation; pointer focus shows nothing.
    pub(super) fn focus_border(&self, color: Color, radius_inner: bool) -> Option<Border> {
        (*self.focus.read() == Focus::Keyboard).then(|| {
            Border::new()
                .fill(color)
                .width(Stroke::STRONG)
                .alignment(if radius_inner {
                    BorderAlignment::Inner
                } else {
                    BorderAlignment::Outer
                })
        })
    }

    /// Wire the events, accessibility and cursor of `element`. Disabled
    /// elements are dimmed and ignore input.
    pub(super) fn wire(
        self,
        element: Rect,
        role: AccessibilityRole,
        enabled: bool,
        on_press: Option<&Action>,
    ) -> Rect {
        let Self {
            mut hovering,
            mut pressing,
            a11y_id,
            ..
        } = self;
        let on_press = on_press.cloned();
        let interactive = enabled && on_press.is_some();
        element
            .a11y_id(a11y_id)
            .a11y_role(role)
            .a11y_enabled(enabled)
            .a11y_focusable(interactive)
            .maybe(!enabled, |el| el.opacity(DISABLED_OPACITY))
            .maybe(interactive, |el| {
                el.cursor(CursorIcon::Pointer)
                    .on_pointer_down(move |e: Event<PointerEventData>| {
                        if e.data().is_primary() {
                            pressing.set_if_modified(true);
                        }
                    })
                    .on_pointer_over(move |_| hovering.set_if_modified(true))
                    .on_pointer_out(move |_| hovering.set_if_modified(false))
                    .on_pointer_leave(move |_| pressing.set_if_modified(false))
                    .on_global_pointer_up(move |_| pressing.set_if_modified(false))
                    .on_all_press(move |e: Event<PressEventData>| {
                        let primary = match e.data() {
                            PressEventData::Mouse(data) => data.button == Some(MouseButton::Left),
                            PressEventData::Touch(_)
                            | PressEventData::Keyboard(_)
                            | PressEventData::Accessibility(_) => true,
                        };
                        if primary && let Some(on_press) = &on_press {
                            a11y_id.request_focus();
                            on_press.call();
                        }
                    })
            })
    }
}
