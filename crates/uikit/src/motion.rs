//! Motion policy and the transitions components share.
//!
//! Durations come from [`Motion`]; curves follow one rule set: color changes
//! ease in-out, movement eases out (quart), nothing bounces. State that exists
//! on mount never animates in.

use std::time::Duration;

use freya::animation::{
    AnimColor, AnimNum, Ease, Function, OnChange, OnCreation, use_animation_transition,
    use_animation_with_dependencies,
};
use freya::prelude::{Color, State, try_consume_context, use_provide_context};

use crate::tokens::Motion;

/// The user's "reduce motion" setting (Ajustes → Accesibilidad).
///
/// Freya does not read the operating system's, so the app owns it. When on,
/// things stop sliding; fades stay, because an instant change is more jarring than a
/// short fade.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ReducedMotion(pub bool);

/// Provide the [`ReducedMotion`] setting to every component below. Call it
/// once in the root, next to [`crate::use_provide_theme_mode`].
#[expect(
    clippy::must_use_candidate,
    reason = "a root that never changes the setting at runtime ignores the state"
)]
pub fn use_provide_reduced_motion(initial: ReducedMotion) -> State<ReducedMotion> {
    use_provide_context(|| State::create(initial))
}

/// Whether motion is reduced. Off without a provider.
#[must_use]
pub fn use_reduced_motion() -> bool {
    try_consume_context::<State<ReducedMotion>>().is_some_and(|setting| setting.read().0)
}

/// Milliseconds of a [`Motion`] duration, as the animation API takes them.
fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

/// Fade from the previous `target` to the new one whenever it changes
/// (press fills, state colors). Returns the color to paint this frame.
pub(crate) fn use_color_transition(target: Color, duration: Duration) -> Color {
    let animation = use_animation_transition(target, move |from: Color, to| {
        AnimColor::new(from, to)
            .time(millis(duration))
            .ease(Ease::InOut)
            .function(Function::Quad)
    });
    animation.get().value()
}

/// Ease a number from the previous `target` to the new one (positions,
/// progress). With reduced motion the value jumps.
pub(crate) fn use_number_transition(target: f32, duration: Duration) -> f32 {
    let duration = if use_reduced_motion() {
        Duration::ZERO
    } else {
        duration
    };
    // `f32` is not `PartialEq`-stable as a dependency key; track its bits.
    let animation = use_animation_transition(target.to_bits(), move |from: u32, to: u32| {
        AnimNum::new(f32::from_bits(from), f32::from_bits(to))
            .time(millis(duration))
            .ease(Ease::Out)
            .function(Function::Quart)
    });
    animation.get().value()
}

/// Press feedback fill: [`Motion::FAST`] color fade.
pub(crate) fn use_press_fill(target: Color) -> Color {
    use_color_transition(target, Motion::FAST)
}

/// Opacity that fades from `from` to 1 each time `key` changes, settled at 1
/// on mount (content already there does not fade in). Used for content that
/// swaps in place: an explanation opening, the body map turning around.
pub(crate) fn use_fade_on_change<K: Clone + PartialEq + 'static>(key: &K, from: f32) -> f32 {
    let animation = use_animation_with_dependencies(key, move |conf, _| {
        conf.on_creation(OnCreation::Finish);
        conf.on_change(OnChange::Rerun);
        AnimNum::new(from, 1.)
            .time(millis(Motion::BASE))
            .ease(Ease::Out)
            .function(Function::Quart)
    });
    animation.get().value()
}
