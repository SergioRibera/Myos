use freya::prelude::Color;

/// `0xRRGGBB` → opaque [`Color`].
const fn rgb(hex: u32) -> Color {
    Color::new(0xFF00_0000 | hex)
}

/// Semantic colors of one appearance (dark or light).
///
/// Field names follow the role, not the hue: `accent` is lime in dark mode and
/// deep olive in light mode, because lime fails contrast on light grounds.
/// Lime is never used as a background (brand rule).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    /// Screen background (`color.fondo`).
    pub ground: Color,
    /// Cards, bars, rows (`color.superficie`).
    pub surface: Color,
    /// Raised or selected surface, secondary buttons (`color.superficie-2`).
    pub surface_raised: Color,
    /// Pressed secondary surface, switch track (`color.superficie-3`).
    pub surface_sunken: Color,
    /// Borders and dividers (`color.linea`).
    pub line: Color,
    /// Primary text (`color.texto`).
    pub ink: Color,
    /// Secondary text (`color.texto-2`).
    pub ink_muted: Color,
    /// Tertiary text, inactive icons (`color.texto-3`).
    pub ink_subtle: Color,
    /// Brand accent: active states, "in range", highlights (`color.acento`).
    pub accent: Color,
    /// Primary call-to-action fill (`color.cta.fondo`).
    pub cta: Color,
    /// Text on [`Self::cta`] (`color.cta.texto`).
    pub on_cta: Color,
    /// [`Self::cta`] while pressed or hovered.
    pub cta_pressed: Color,
    /// Deficit marks: dashed outlines, dots (`color.estado.deficit`).
    pub deficit: Color,
    /// Excess and destructive marks (`color.estado.exceso`).
    pub excess: Color,
    /// Destructive button fill.
    pub danger_surface: Color,
    /// [`Self::danger_surface`] while pressed or hovered.
    pub danger_surface_pressed: Color,
    /// Muscle map fill ramp, from untrained (`0`) to on target (`3`)
    /// (`color.mapa.0` … `color.mapa.3`).
    pub map_levels: [Color; 4],
    /// Body silhouette under the muscles (`color.mapa.silueta`).
    pub map_silhouette: Color,
}

impl Palette {
    /// Dark appearance (`Tema/Oscuro`).
    pub const DARK: Self = Self {
        ground: rgb(0x15_171C),
        surface: rgb(0x1D_2027),
        surface_raised: rgb(0x26_2A33),
        surface_sunken: rgb(0x30_3542),
        line: rgb(0x36_3B47),
        ink: rgb(0xF4_F5F0),
        ink_muted: rgb(0xA9_AEB9),
        ink_subtle: rgb(0x8D_93A0),
        accent: rgb(0xC6_F432),
        cta: rgb(0xF4_F5F0),
        on_cta: rgb(0x15_171C),
        cta_pressed: rgb(0xD9_DBD3),
        deficit: rgb(0x6E_C8FF),
        excess: rgb(0xFF_8A5B),
        danger_surface: rgb(0x3A_241A),
        danger_surface_pressed: rgb(0x4A_2C1F),
        map_levels: [
            rgb(0x36_3C48),
            rgb(0x46_552A),
            rgb(0x7E_9F22),
            rgb(0xC6_F432),
        ],
        map_silhouette: rgb(0x2B_2F38),
    };

    /// Light appearance (`Tema/Claro`).
    pub const LIGHT: Self = Self {
        ground: rgb(0xF4_F5F0),
        surface: rgb(0xFF_FFFF),
        surface_raised: rgb(0xEC_EEE7),
        surface_sunken: rgb(0xDF_E2D9),
        line: rgb(0xE3_E5DD),
        ink: rgb(0x15_171C),
        ink_muted: rgb(0x56_5B66),
        ink_subtle: rgb(0x6E_747F),
        accent: rgb(0x5E_8000),
        cta: rgb(0x15_171C),
        on_cta: rgb(0xF4_F5F0),
        cta_pressed: rgb(0x3A_3F4A),
        deficit: rgb(0x2E_7FB8),
        excess: rgb(0xD2_561F),
        danger_surface: rgb(0xFC_E9DF),
        danger_surface_pressed: rgb(0xF6_D5C4),
        map_levels: [
            rgb(0xE1_E3DB),
            rgb(0xD6_E8A4),
            rgb(0xA8_CC3A),
            rgb(0x5E_8000),
        ],
        map_silhouette: rgb(0xD3_D6CD),
    };
}

/// Brand colors that never change with the appearance (logo, previews).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Brand;

impl Brand {
    /// Carbón.
    pub const CARBON: Color = rgb(0x15_171C);
    /// Lima. Only on dark grounds, never as a background.
    pub const LIME: Color = rgb(0xC6_F432);
    /// Tiza.
    pub const CHALK: Color = rgb(0xF4_F5F0);
}
