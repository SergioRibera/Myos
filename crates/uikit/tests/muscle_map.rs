//! Per-region, per-side painting and hit testing of the body map.

use freya::prelude::*;
use freya_testing::TestingRunner;
use myos_uikit::{BodyRegion, BodyView, MuscleHit, MuscleMap, MuscleMapState, RegionState, Side};

#[test]
fn state_is_per_region_and_side() {
    let state = MuscleMapState::default()
        .with(BodyRegion::Chest, RegionState::Load(2))
        .with_side(BodyRegion::Calves, Side::Left, RegionState::Excess)
        .with(
            BodyRegion::Forearms,
            RegionState::Custom(Color::from_rgb(1, 2, 3)),
        );

    for side in Side::BOTH {
        assert_eq!(state.get(BodyRegion::Chest, side), RegionState::Load(2));
        assert_eq!(state.get(BodyRegion::Biceps, side), RegionState::Empty);
    }
    assert_eq!(
        state.get(BodyRegion::Calves, Side::Left),
        RegionState::Excess
    );
    assert_eq!(
        state.get(BodyRegion::Calves, Side::Right),
        RegionState::Empty
    );
    assert_eq!(
        state.get(BodyRegion::Forearms, Side::Right),
        RegionState::Custom(Color::from_rgb(1, 2, 3))
    );
}

fn front_map() -> impl IntoElement {
    let mut hits = use_consume::<State<Vec<MuscleHit>>>();
    MuscleMap::new(BodyView::Front).on_region_press(move |hit| hits.write().push(hit))
}

#[test]
fn pressing_a_muscle_reports_its_anatomical_side() {
    let (mut test, hits) = TestingRunner::new(
        front_map,
        (220., 420.).into(),
        |runner| runner.provide_root_context(|| State::create(Vec::<MuscleHit>::new())),
        1.,
    );
    test.sync_and_update();

    // 220 × 420 fits the 1000 × 2000 model at 0.21, 5 px from the left. The
    // pecs sit around model (390, 500) and (610, 500).
    test.click_cursor((87., 105.));
    test.click_cursor((133., 105.));

    // Front view: the viewer's left is the person's right.
    assert_eq!(
        *hits.peek(),
        vec![
            MuscleHit {
                region: BodyRegion::Chest,
                side: Side::Right
            },
            MuscleHit {
                region: BodyRegion::Chest,
                side: Side::Left
            },
        ]
    );
}
