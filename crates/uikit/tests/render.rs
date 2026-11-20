//! Renders the gallery headless. Catches panics in any component and, with
//! `MYOS_UIKIT_SNAPSHOT=<file.png>`, writes the frame for a visual check.

#[path = "../examples/gallery/view.rs"]
mod view;

use std::collections::HashMap;

use freya_testing::TestingRunner;
use myos_uikit::{FONT_FAMILY, fonts};

#[test]
fn gallery_renders_in_both_themes() {
    let (mut test, ()) = TestingRunner::new(view::gallery, (900., 2400.).into(), |_| (), 1.);
    // One call per face: the map is keyed by family name.
    for face in fonts::FACES {
        test.set_fonts(HashMap::from([(FONT_FAMILY, face)]));
    }
    test.set_default_fonts(&[FONT_FAMILY.into()]);
    test.sync_and_update();
    let png = test.render();
    assert!(!png.is_empty());
    if let Some(path) = std::env::var_os("MYOS_UIKIT_SNAPSHOT") {
        std::fs::write(path, png.as_bytes()).expect("snapshot path is writable");
    }
}
