//! Base components of the Penpot proposal. Each one renders every variant of
//! its Penpot component set (type × state × theme) from the props alone.

mod alert_card;
mod button;
mod chip;
mod layout;
mod muscle_map;
mod nav;
mod press;
mod select_option;
mod set_row;
mod stepper;
mod suggestion_card;
mod switch;
mod sync_banner;
mod text;
mod theme_card;

pub use alert_card::{AlertCard, AlertKind};
pub use button::{Button, ButtonKind, ButtonSize};
pub use chip::{Chip, ChipKind, Status};
pub use layout::{Card, Screen, ThemeScope};
pub use muscle_map::{
    BodyRegion, BodyView, MuscleHit, MuscleMap, MuscleMapState, RegionState, Side,
};
pub use nav::{NavBar, TabBar, TabItem};
pub use select_option::SelectOption;
pub use set_row::{SetRow, SetState};
pub use stepper::Stepper;
pub use suggestion_card::{SuggestionCard, SuggestionLabels, SuggestionState};
pub use switch::Switch;
pub use sync_banner::{SyncBanner, SyncStatus};
pub use text::{Text, Tone};
pub use theme_card::ThemeCard;

/// Callback with no payload, the shape every `on_*` prop of this crate uses:
/// `.on_press(move || count += 1)`.
pub type Action = freya::prelude::NoArgCallback<()>;
