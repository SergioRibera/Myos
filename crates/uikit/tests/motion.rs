//! The switch knob slides instead of jumping, and lands in place.

use std::time::Duration;

use freya::prelude::*;
use freya_testing::{TestingNode, TestingRunner};
use myos_uikit as ui;

fn app() -> impl IntoElement {
    let mut on = use_consume::<State<bool>>();
    rect().child(ui::Switch::new(on()).on_toggle(move || on.toggle()))
}

/// Left edge of the 22 × 22 knob.
fn knob_x(test: &TestingRunner) -> f32 {
    test.find(|node: TestingNode, _| {
        let area = node.layout().area;
        ((area.width() - 22.).abs() < 0.01 && (area.height() - 22.).abs() < 0.01)
            .then(|| area.min_x())
    })
    .expect("the switch renders a knob")
}

#[test]
fn switch_knob_slides_on_toggle() {
    let (mut test, on) = TestingRunner::new(
        app,
        (100., 40.).into(),
        |runner| runner.provide_root_context(|| State::create(false)),
        1.,
    );
    test.sync_and_update();
    let start = knob_x(&test);

    test.click_cursor((24., 14.));
    assert!(*on.peek(), "press toggles the value");

    test.poll(Duration::from_millis(1), Duration::from_millis(40));
    test.sync_and_update();
    let midway = knob_x(&test);
    assert!(
        midway > start && midway < start + 20.,
        "knob is between ends at 40 ms: {midway}"
    );

    test.poll(Duration::from_millis(5), Duration::from_millis(300));
    test.sync_and_update();
    let end = knob_x(&test);
    assert!(
        (end - (start + 20.)).abs() < 0.01,
        "knob lands 20 px right: {end}"
    );
}

fn reduced_app() -> impl IntoElement {
    ui::use_provide_reduced_motion(ui::ReducedMotion(true));
    app()
}

#[test]
fn reduced_motion_moves_the_knob_without_sliding() {
    let (mut test, _) = TestingRunner::new(
        reduced_app,
        (100., 40.).into(),
        |runner| runner.provide_root_context(|| State::create(false)),
        1.,
    );
    test.sync_and_update();
    let start = knob_x(&test);

    test.click_cursor((24., 14.));
    test.poll(Duration::from_millis(1), Duration::from_millis(10));
    test.sync_and_update();
    let end = knob_x(&test);
    assert!(
        (end - (start + 20.)).abs() < 0.01,
        "knob is already at the end: {end}"
    );
}
