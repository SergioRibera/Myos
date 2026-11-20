use freya::prelude::TextStyleExt;

/// Family every role uses. The same string is registered by
/// [`crate::fonts`], so the name is never duplicated.
pub const FONT_FAMILY: &str = "Figtree";

/// Numeric values of one text role.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextStyle {
    /// Font size in logical px.
    pub size: f32,
    /// Weight, 100–900.
    pub weight: i32,
    /// Line height as a multiple of the size.
    pub line_height: f32,
    /// Tracking in px; negative tightens large titles.
    pub letter_spacing: f32,
    /// Tabular numerals, so columns of numbers line up.
    pub tabular: bool,
}

/// Semantic type scale (Penpot library typographies, plus the sizes the
/// components use for their labels).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TextRole {
    /// Hero numbers: records, totals (64/800).
    BigNumber,
    /// Onboarding and empty-state headlines (40/800).
    Display,
    /// Tab root titles (30/800).
    Title1,
    /// Section and secondary-screen titles, stepper values (20/800).
    Title2,
    /// Button labels (17/700).
    Action,
    /// Small inline button labels (12/700).
    ActionSmall,
    /// Card titles, list item titles (15/700).
    BodyStrong,
    /// Running text (14/500).
    #[default]
    Body,
    /// Chip labels and banner messages (13/700).
    Tag,
    /// Secondary lines under a title (13/500).
    Detail,
    /// Captions, helper text, small buttons (12/600).
    Caption,
    /// Overlines and tab labels (11/800, tracked).
    Label,
}

impl TextRole {
    /// Numeric values of this role.
    #[must_use]
    pub const fn style(self) -> TextStyle {
        const fn s(size: f32, weight: i32, line_height: f32, letter_spacing: f32) -> TextStyle {
            TextStyle {
                size,
                weight,
                line_height,
                letter_spacing,
                tabular: false,
            }
        }
        match self {
            Self::BigNumber => TextStyle {
                tabular: true,
                ..s(64., 800, 1.0, -2.5)
            },
            Self::Display => s(40., 800, 1.05, -1.2),
            Self::Title1 => s(30., 800, 1.15, -0.6),
            Self::Title2 => s(20., 800, 1.2, -0.3),
            Self::Action => s(17., 700, 1.2, 0.),
            Self::ActionSmall => s(12., 700, 1.25, 0.),
            Self::BodyStrong => s(15., 700, 1.3, 0.),
            Self::Body => s(14., 500, 1.4, 0.),
            Self::Tag => s(13., 700, 1.25, 0.),
            Self::Detail => s(13., 500, 1.35, 0.),
            Self::Caption => s(12., 600, 1.3, 0.),
            Self::Label => s(11., 800, 1.2, 1.4),
        }
    }
}

/// Applies a [`TextRole`] (family, size, weight, tracking) to any Freya text
/// element. Line height is element-specific; [`crate::Text`] sets it too.
pub trait TextRoleExt: TextStyleExt {
    /// Style this text with `role`.
    #[must_use]
    fn text_role(self, role: TextRole) -> Self {
        let style = role.style();
        let text = self
            .font_family(FONT_FAMILY)
            .font_size(style.size)
            .font_weight(style.weight)
            .letter_spacing(style.letter_spacing);
        if style.tabular {
            text.font_tabular()
        } else {
            text
        }
    }
}

impl<T: TextStyleExt> TextRoleExt for T {}
