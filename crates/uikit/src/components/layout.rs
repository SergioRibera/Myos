use freya::prelude::*;

use crate::theme::{ThemeMode, use_provide_theme_mode, use_theme};
use crate::tokens::{FONT_FAMILY, Radius, Space, Stroke};

/// Root of a screen: fills the window with the ground color and sets the
/// default font and text color for everything inside.
#[derive(Clone, PartialEq)]
pub struct Screen {
    children: Vec<Element>,
    layout: LayoutData,
}

impl Default for Screen {
    fn default() -> Self {
        Self {
            children: Vec::new(),
            layout: LayoutData::default(),
        }
        .expanded()
    }
}

impl Screen {
    /// An empty, expanded screen.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl ChildrenExt for Screen {
    fn children_mut(&mut self) -> &mut Vec<Element> {
        &mut self.children
    }
}

impl LayoutExt for Screen {
    fn layout_mut(&mut self) -> &mut LayoutData {
        &mut self.layout
    }
}

impl ContainerExt for Screen {}
impl ContainerWithContentExt for Screen {}

impl Component for Screen {
    fn render(&self) -> impl IntoElement {
        let palette = use_theme().palette;
        rect()
            .layout(self.layout.clone())
            .background(palette.ground)
            .color(palette.ink)
            .font_family(FONT_FAMILY)
            .children(self.children.clone())
    }
}

/// Grouped content on a surface: 24 px corners, 16 px padding, 12 px between
/// children. Layout setters override those defaults.
#[derive(Clone, PartialEq)]
pub struct Card {
    children: Vec<Element>,
    layout: LayoutData,
    outlined: bool,
}

impl Default for Card {
    fn default() -> Self {
        Self {
            children: Vec::new(),
            layout: LayoutData::default(),
            outlined: false,
        }
        .padding(Space::LG)
        .spacing(Space::MD)
    }
}

impl Card {
    /// A filled card.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Hairline border and no fill, for cards on a surface.
    #[must_use]
    pub const fn outlined(mut self) -> Self {
        self.outlined = true;
        self
    }
}

impl ChildrenExt for Card {
    fn children_mut(&mut self) -> &mut Vec<Element> {
        &mut self.children
    }
}

impl LayoutExt for Card {
    fn layout_mut(&mut self) -> &mut LayoutData {
        &mut self.layout
    }
}

impl ContainerExt for Card {}
impl ContainerWithContentExt for Card {}

impl Component for Card {
    fn render(&self) -> impl IntoElement {
        let palette = use_theme().palette;
        rect()
            .layout(self.layout.clone())
            .corner_radius(Radius::CARD)
            .map(
                (!self.outlined).then_some(palette.surface),
                BackgroundExt::background,
            )
            .maybe(self.outlined, |el| {
                el.border(
                    Border::new()
                        .fill(palette.line)
                        .width(Stroke::REGULAR)
                        .alignment(BorderAlignment::Inner),
                )
            })
            .children(self.children.clone())
    }
}

/// Forces a [`ThemeMode`] on its subtree, whatever the app-wide mode:
/// previews, the theme picker, side-by-side galleries.
#[derive(Clone, PartialEq)]
pub struct ThemeScope {
    mode: ThemeMode,
    children: Vec<Element>,
    layout: LayoutData,
}

impl ThemeScope {
    /// Subtree rendered in `mode`.
    #[must_use]
    pub fn new(mode: ThemeMode) -> Self {
        Self {
            mode,
            children: Vec::new(),
            layout: LayoutData::default(),
        }
    }
}

impl ChildrenExt for ThemeScope {
    fn children_mut(&mut self) -> &mut Vec<Element> {
        &mut self.children
    }
}

impl LayoutExt for ThemeScope {
    fn layout_mut(&mut self) -> &mut LayoutData {
        &mut self.layout
    }
}

impl ContainerExt for ThemeScope {}
impl ContainerWithContentExt for ThemeScope {}

impl Component for ThemeScope {
    fn render(&self) -> impl IntoElement {
        let mut mode = use_provide_theme_mode(self.mode);
        if *mode.peek() != self.mode {
            mode.set(self.mode);
        }
        let palette = use_theme().palette;
        rect()
            .layout(self.layout.clone())
            .background(palette.ground)
            .color(palette.ink)
            .font_family(FONT_FAMILY)
            .children(self.children.clone())
    }
}
