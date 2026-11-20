use std::borrow::Cow;

use freya::prelude::*;

use super::Action;
use super::button::Button;
use crate::icon::{Icon, IconName};
use crate::motion::use_number_transition;
use crate::theme::use_theme;
use crate::tokens::{Motion, Stroke, TextRole, TextRoleExt};

/// Offline-first sync state (Penpot property `Estado`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SyncStatus {
    /// Everything uploaded.
    UpToDate,
    /// Uploading; `progress` in `0.0..=1.0`.
    Syncing {
        /// Fraction done.
        progress: f32,
    },
    /// No connection; changes wait on the device.
    Offline,
    /// The last attempt failed.
    Error,
}

/// Pill that tells where the user's data is (Penpot component
/// `Banner de sincronización`). The message is the caller's
/// ("Sincronizando 3 de 5…").
#[derive(Clone, PartialEq)]
pub struct SyncBanner {
    status: SyncStatus,
    message: Cow<'static, str>,
    retry_label: Cow<'static, str>,
    on_retry: Option<Action>,
}

impl SyncBanner {
    /// Banner for `status` with `message`.
    #[must_use]
    pub fn new(status: SyncStatus, message: impl Into<Cow<'static, str>>) -> Self {
        Self {
            status,
            message: message.into(),
            retry_label: Cow::Borrowed("Reintentar"),
            on_retry: None,
        }
    }

    /// Retry button copy (default "Reintentar").
    #[must_use]
    pub fn retry_label(mut self, retry_label: impl Into<Cow<'static, str>>) -> Self {
        self.retry_label = retry_label.into();
        self
    }

    /// Shows the retry button in [`SyncStatus::Error`].
    #[must_use]
    pub fn on_retry(mut self, on_retry: impl Into<Action>) -> Self {
        self.on_retry = Some(on_retry.into());
        self
    }
}

impl Component for SyncBanner {
    fn render(&self) -> impl IntoElement {
        let palette = use_theme().palette;
        let (icon, icon_color, ink) = match self.status {
            SyncStatus::UpToDate => (IconName::Sync, palette.ink_subtle, palette.ink_subtle),
            SyncStatus::Syncing { .. } => (IconName::Sync, palette.accent, palette.ink),
            SyncStatus::Offline => (IconName::WifiOff, palette.ink_muted, palette.ink),
            SyncStatus::Error => (IconName::Alert, palette.excess, palette.ink),
        };
        let progress = match self.status {
            SyncStatus::Syncing { progress } => Some(progress.clamp(0., 1.)),
            SyncStatus::UpToDate | SyncStatus::Offline | SyncStatus::Error => None,
        };
        let progress_width = use_number_transition(progress.unwrap_or(0.) * 100., Motion::BASE);
        let retry = self
            .on_retry
            .as_ref()
            .filter(|_| self.status == SyncStatus::Error)
            .map(|on_retry| {
                Button::secondary(self.retry_label.clone())
                    .small()
                    .on_press(on_retry.clone())
            });
        rect()
            .width(Size::fill())
            .height(Size::px(44.))
            .padding((0., 14.))
            .horizontal()
            .content(Content::flex())
            .cross_align(Alignment::Center)
            .spacing(10.)
            .corner_radius(22.)
            .background(palette.surface)
            .maybe(self.status == SyncStatus::Error, |el| {
                el.border(
                    Border::new()
                        .fill(palette.excess.with_af32(0.6))
                        .width(Stroke::REGULAR)
                        .alignment(BorderAlignment::Inner),
                )
            })
            .child(Icon::new(icon).size(22.).color(icon_color))
            .child(
                label()
                    .width(Size::flex(1.))
                    .text(self.message.clone())
                    .text_role(TextRole::Tag)
                    .color(ink)
                    .max_lines(1)
                    .text_overflow(TextOverflow::Ellipsis),
            )
            .maybe_child(progress.map(|_| {
                rect()
                    .width(Size::px(82.))
                    .height(Size::px(6.))
                    .corner_radius(3.)
                    .background(palette.surface_sunken)
                    .child(
                        rect()
                            .width(Size::percent(progress_width))
                            .height(Size::fill())
                            .corner_radius(3.)
                            .background(palette.accent),
                    )
            }))
            .maybe_child(retry)
            .a11y_role(AccessibilityRole::Status)
    }
}
