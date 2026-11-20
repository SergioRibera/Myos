use std::borrow::Cow;

use freya::prelude::*;

use crate::theme::use_theme;
use crate::tokens::{Palette, TextRole, TextRoleExt};

/// Text color by meaning, resolved against the current palette.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Tone {
    /// Primary text.
    #[default]
    Primary,
    /// Secondary text.
    Muted,
    /// Tertiary text: hints, placeholders, disabled labels.
    Subtle,
    /// Brand accent. Use for short highlights only: in light mode the
    /// accent is olive and stays readable, in dark mode it is lime.
    Accent,
    /// Text on a call-to-action fill.
    OnCta,
    /// Deficit state.
    Deficit,
    /// Excess or destructive state.
    Excess,
}

impl Tone {
    /// Color of this tone in `palette`.
    #[must_use]
    pub const fn color(self, palette: &Palette) -> Color {
        match self {
            Self::Primary => palette.ink,
            Self::Muted => palette.ink_muted,
            Self::Subtle => palette.ink_subtle,
            Self::Accent => palette.accent,
            Self::OnCta => palette.on_cta,
            Self::Deficit => palette.deficit,
            Self::Excess => palette.excess,
        }
    }
}

/// A run of text in one [`TextRole`] and [`Tone`].
///
/// ```rust,no_run
/// # use freya::prelude::*;
/// # use myos_uikit::{Text, TextRole, Tone};
/// fn app() -> impl IntoElement {
///     Text::new("Isquios en déficit").role(TextRole::BodyStrong).tone(Tone::Muted)
/// }
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct Text {
    content: Cow<'static, str>,
    role: TextRole,
    tone: Tone,
    max_lines: Option<usize>,
    align: TextAlign,
    layout: LayoutData,
}

impl Text {
    /// Body text in the primary tone.
    #[must_use]
    pub fn new(text: impl Into<Cow<'static, str>>) -> Self {
        Self {
            content: text.into(),
            role: TextRole::default(),
            tone: Tone::default(),
            max_lines: None,
            align: TextAlign::default(),
            layout: LayoutData::default(),
        }
    }

    /// Type scale role.
    #[must_use]
    pub const fn role(mut self, role: TextRole) -> Self {
        self.role = role;
        self
    }

    /// Color by meaning.
    #[must_use]
    pub const fn tone(mut self, tone: Tone) -> Self {
        self.tone = tone;
        self
    }

    /// Cut after `lines` lines with an ellipsis.
    #[must_use]
    pub const fn max_lines(mut self, lines: usize) -> Self {
        self.max_lines = Some(lines);
        self
    }

    /// Horizontal alignment.
    #[must_use]
    pub const fn align(mut self, align: TextAlign) -> Self {
        self.align = align;
        self
    }
}

impl LayoutExt for Text {
    fn layout_mut(&mut self) -> &mut LayoutData {
        &mut self.layout
    }
}

impl ContainerExt for Text {}

impl Component for Text {
    fn render(&self) -> impl IntoElement {
        let theme = use_theme();
        label()
            .layout(self.layout.clone())
            .text(self.content.clone())
            .text_role(self.role)
            .line_height(self.role.style().line_height)
            .color(self.tone.color(theme.palette))
            .text_align(self.align)
            .max_lines(self.max_lines)
            .maybe(self.max_lines.is_some(), |l| {
                l.text_overflow(TextOverflow::Ellipsis)
            })
    }
}
