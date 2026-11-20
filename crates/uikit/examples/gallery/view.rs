//! Every component and variant, dark and light side by side. Shared by the
//! `gallery` example and the render test.

use freya::prelude::*;
use myos_uikit::{self as ui, IconName, TextRole, ThemeMode, Tone};

/// Gallery root: one column per appearance.
#[expect(
    unreachable_pub,
    reason = "included with #[path] by the example and the render test"
)]
pub fn gallery() -> impl IntoElement {
    rect()
        .expanded()
        .horizontal()
        .child(column(ThemeMode::Dark))
        .child(column(ThemeMode::Light))
}

fn section(title: &'static str) -> ui::Text {
    ui::Text::new(title)
        .role(TextRole::Label)
        .tone(Tone::Subtle)
}

fn row() -> Rect {
    rect()
        .horizontal()
        .spacing(ui::Space::SM)
        .cross_align(Alignment::Center)
}

fn column(mode: ThemeMode) -> impl IntoElement {
    let map = sample_map(mode);

    ui::ThemeScope::new(mode)
        .width(Size::px(ui::Space::LG.mul_add(2., ui::Dimen::MOBILE_WIDTH)))
        .height(Size::fill())
        .padding(ui::Space::LG)
        .spacing(ui::Space::MD)
        .child(ui::NavBar::new("Componentes").on_back(|| {}))
        .child(ui::SyncBanner::new(ui::SyncStatus::Syncing { progress: 0.6 }, "Sincronizando 3 de 5…"))
        .child(ui::SyncBanner::new(ui::SyncStatus::Error, "No pudimos sincronizar").on_retry(|| {}))
        .child(section("BOTONES"))
        .child(ui::Button::primary("Empezar sesión").expanded().icon(IconName::Play).on_press(|| {}))
        .child(
            row()
                .child(ui::Button::secondary("Secundario").on_press(|| {}))
                .child(ui::Button::ghost("Fantasma").on_press(|| {}))
                .child(ui::Button::primary("Off").enabled(false)),
        )
        .child(ui::Button::destructive("Borrar datos").expanded().on_press(|| {}))
        .child(section("CHIPS"))
        .child(
            row()
                .child(ui::Chip::new("45 min"))
                .child(ui::Chip::new("Mancuernas").selected(true).on_press(|| {}))
                .child(ui::Chip::new("Déficit · 2 sem").status(ui::Status::Deficit))
                .child(ui::Chip::new("Exceso").status(ui::Status::Excess)),
        )
        .child(section("ALERTAS"))
        .child(
            ui::AlertCard::new(
                ui::AlertKind::Deficit,
                "Isquios en déficit · 2 semanas",
                "2 de 10 series. Cuádriceps / isquios = 8 : 1.",
            )
            .on_press(|| {}),
        )
        .child(ui::AlertCard::new(ui::AlertKind::Resolved, "Espalda volvió a su rango", "+8 series esta semana.").on_press(|| {}))
        .child(section("SUGERENCIA"))
        .child(
            ui::SuggestionCard::new("Suma 2 series de isquios hoy", "Tu rutina no los llega a cubrir esta semana.")
                .explanation("2 de 10 series eq. en 14 días · rango por objetivo “ganar músculo”, intermedio · confianza media")
                .state(ui::SuggestionState::Explained)
                .on_accept(|| {})
                .on_explain(|| {})
                .on_dismiss(|| {}),
        )
        .child(section("SERIES"))
        .child(ui::SetRow::new(1, "24", "10").previous("22 × 9").effort("8").state(ui::SetState::Done).on_toggle(|| {}))
        .child(ui::SetRow::new(2, "24", "10").previous("22 × 9").state(ui::SetState::Active).on_toggle(|| {}))
        .child(ui::SetRow::new(3, "24", "10").previous("22 × 9").on_toggle(|| {}))
        .child(
            row()
                .child(ui::Stepper::new("24 kg").on_decrement(|| {}).on_increment(|| {}))
                .child(ui::Switch::new(true).on_toggle(|| {}))
                .child(ui::Switch::new(false).on_toggle(|| {})),
        )
        .child(section("OPCIONES"))
        .child(
            ui::SelectOption::new(IconName::Dumbbell, "Ganar músculo")
                .description("Más volumen en todos los grupos")
                .selected(true)
                .on_press(|| {}),
        )
        .child(
            row()
                .child(ui::ThemeCard::new(ThemeMode::System).selected(true).on_press(|| {}))
                .child(ui::ThemeCard::new(ThemeMode::Light).on_press(|| {}))
                .child(ui::ThemeCard::new(ThemeMode::Dark).on_press(|| {})),
        )
        .child(section("MAPA"))
        .child(
            row()
                .child(map_view(ui::BodyView::Front, &map))
                .child(map_view(ui::BodyView::Back, &map)),
        )
        .child(
            ui::TabBar::new()
                .tab(ui::TabItem::new(IconName::Home, "Hoy").active(true))
                .tab(ui::TabItem::new(IconName::Body, "Mapa"))
                .tab(ui::TabItem::new(IconName::Progress, "Progreso"))
                .tab(ui::TabItem::new(IconName::Profile, "Yo"))
                .center_action("Registrar", || {}),
        )
}

fn map_view(view: ui::BodyView, state: &ui::MuscleMapState) -> ui::MuscleMap {
    ui::MuscleMap::new(view)
        .state(*state)
        .width(Size::px(180.))
        .height(Size::px(344.))
}

/// A week with every kind of state, including one-sided calves and a custom
/// color.
fn sample_map(mode: ThemeMode) -> ui::MuscleMapState {
    let palette = match mode {
        ThemeMode::Light => &ui::Palette::LIGHT,
        ThemeMode::Dark | ThemeMode::System => &ui::Palette::DARK,
    };
    ui::MuscleMapState::default()
        .with(ui::BodyRegion::Chest, ui::RegionState::Load(3))
        .with(ui::BodyRegion::FrontDeltoids, ui::RegionState::Load(2))
        .with(ui::BodyRegion::Biceps, ui::RegionState::Load(2))
        .with(ui::BodyRegion::Abs, ui::RegionState::Load(1))
        .with(ui::BodyRegion::Obliques, ui::RegionState::Load(1))
        .with(ui::BodyRegion::Quadriceps, ui::RegionState::Load(3))
        .with(ui::BodyRegion::Trapezius, ui::RegionState::Load(1))
        .with(ui::BodyRegion::UpperBack, ui::RegionState::Deficit(0))
        .with(ui::BodyRegion::LowerBack, ui::RegionState::Excess)
        .with(ui::BodyRegion::Glutes, ui::RegionState::Load(2))
        .with(ui::BodyRegion::Hamstrings, ui::RegionState::Deficit(1))
        .with_side(
            ui::BodyRegion::Calves,
            ui::Side::Left,
            ui::RegionState::Load(3),
        )
        .with_side(
            ui::BodyRegion::Calves,
            ui::Side::Right,
            ui::RegionState::Load(1),
        )
        .with(
            ui::BodyRegion::Forearms,
            ui::RegionState::Custom(palette.deficit),
        )
        .with(ui::BodyRegion::Triceps, ui::RegionState::Load(3))
}
